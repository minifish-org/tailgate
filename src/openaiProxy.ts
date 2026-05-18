import { Context } from "hono";
import { GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { resolveModel } from "./selector.js";
import { AppConfig, ChatCompletionBody } from "./types.js";

export function modelsResponse(config: AppConfig) {
  const ids = [...Object.keys(config.models), ...Object.keys(config.routes)].sort();
  return {
    object: "list",
    data: ids.map((id) => ({
      id,
      object: "model",
      created: 0,
      owned_by: "tailgate",
    })),
  };
}

export async function proxyChatCompletions(c: Context, config: AppConfig, health: HealthRegistry, requestId: string) {
  const startedAt = Date.now();
  let requestedModel: string | undefined;
  let selectedModel: string | undefined;
  let provider: string | undefined;
  let stream = false;
  let status = 500;
  let errorCode: string | undefined;

  try {
    const body = await readJsonBody(c);
    requestedModel = modelFromBody(body);
    stream = body.stream === true;

    const selected = resolveModel(config, health, requestedModel);
    selectedModel = selected.name;
    provider = selected.config.provider;

    const apiKey = process.env[selected.config.api_key_env];
    if (!apiKey) {
      throw new GatewayError(`Provider API key is not configured for model: ${selectedModel}`, 500, "provider_key_not_configured");
    }

    const upstreamBody: ChatCompletionBody = {
      ...body,
      model: selected.config.upstream_model,
    };

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), config.server.request_timeout_ms);
    let upstream: Response;

    try {
      upstream = await fetch(`${selected.config.base_url}/chat/completions`, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${apiKey}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(upstreamBody),
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timeout);
    }

    status = upstream.status;
    const responseHeaders = filteredUpstreamHeaders(upstream.headers);
    responseHeaders.set("X-Tailgate-Model", selectedModel);
    responseHeaders.set("X-Tailgate-Provider", provider);

    logger.info(
      {
        request_id: requestId,
        method: c.req.method,
        path: new URL(c.req.url).pathname,
        requested_model: requestedModel,
        selected_model: selectedModel,
        provider,
        status,
        duration_ms: Date.now() - startedAt,
        stream,
      },
      "request completed",
    );

    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: responseHeaders,
    });
  } catch (error) {
    if (error instanceof GatewayError) {
      status = error.status;
      errorCode = error.code;
      throw error;
    }
    status = 502;
    errorCode = "upstream_error";
    throw new GatewayError(error instanceof Error ? error.message : "Upstream request failed", 502, "upstream_error");
  } finally {
    if (errorCode) {
      logger.info(
        {
          request_id: requestId,
          method: c.req.method,
          path: new URL(c.req.url).pathname,
          requested_model: requestedModel,
          selected_model: selectedModel,
          provider,
          status,
          duration_ms: Date.now() - startedAt,
          stream,
          error_code: errorCode,
        },
        "request failed",
      );
    }
  }
}

async function readJsonBody(c: Context): Promise<ChatCompletionBody> {
  try {
    const parsed = (await c.req.json()) as unknown;
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new GatewayError("Request body must be a JSON object", 400, "invalid_request");
    }
    return parsed as ChatCompletionBody;
  } catch (error) {
    if (error instanceof GatewayError) throw error;
    throw new GatewayError("Invalid JSON request body", 400, "invalid_json");
  }
}

function modelFromBody(body: ChatCompletionBody): string {
  if (typeof body.model !== "string" || body.model.length === 0) {
    throw new GatewayError("Request body must include a model", 400, "model_required");
  }
  return body.model;
}

function filteredUpstreamHeaders(headers: Headers): Headers {
  const result = new Headers();
  const blocked = new Set(["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "te", "trailer", "transfer-encoding", "upgrade"]);

  headers.forEach((value, key) => {
    if (!blocked.has(key.toLowerCase())) result.set(key, value);
  });

  return result;
}
