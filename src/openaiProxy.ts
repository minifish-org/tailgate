import { Context } from "hono";
import { GatewayError, messageFromUnknown } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { isAutoRoute, resolveAutoCandidates, resolveModel } from "./selector.js";
import { AppConfig, CostTier, Endpoint, ModelCatalog, ModelConfig, OpenAIJsonBody, ProxyEndpointSpec, RouteConfig, SelectedModel } from "./types.js";

export const ENDPOINT_SPECS: Record<Endpoint, ProxyEndpointSpec> = {
  chat: {
    endpoint: "chat",
    upstreamPath: "/chat/completions",
    bodyKind: "json",
    supportsStream: true,
  },
  embeddings: {
    endpoint: "embeddings",
    upstreamPath: "/embeddings",
    bodyKind: "json",
    supportsStream: false,
  },
  audio_speech: {
    endpoint: "audio_speech",
    upstreamPath: "/audio/speech",
    bodyKind: "json",
    supportsStream: false,
  },
  audio_transcriptions: {
    endpoint: "audio_transcriptions",
    upstreamPath: "/audio/transcriptions",
    bodyKind: "multipart",
    supportsStream: false,
  },
  translations: {
    endpoint: "translations",
    upstreamPath: "/translations",
    bodyKind: "json",
    supportsStream: false,
  },
};

export function modelsResponse(config: AppConfig, catalog?: ModelCatalog) {
  const modelEntries = catalog?.getModelEntries() ?? Object.entries(config.models).map(([id, model]) => [id, model, undefined] as const);
  const routeEntries = Object.entries(config.routes);
  const ids = [...modelEntries.map(([id]) => id), ...routeEntries.map(([id]) => id)].sort();
  return {
    object: "list",
    data: ids.map((id) => {
      const model = modelEntries.find(([modelId]) => modelId === id);
      const route = config.routes[id];
      return {
      id,
      object: "model",
      created: 0,
      owned_by: "tailgate",
      ...(model
        ? {
            provider: model[1].provider,
            upstream_model: model[1].upstream_model,
            context_window: model[1].context_window,
            cost_tier: displayCostTier(id, model[1]),
            price_rank: model[1].price_rank,
          }
        : {}),
      ...(route ? routeMetadata(route) : {}),
    };
    }),
  };
}

function routeMetadata(route: RouteConfig) {
  return {
    endpoint: route.endpoint,
    cost_tier: route.cost_tier,
    route: true,
  };
}

function displayCostTier(modelName: string, model: ModelConfig): CostTier {
  if (model.cost_tier) return model.cost_tier;
  if (model.provider === "local") return "free";
  if (model.provider === "deepseek") return "standard";
  if (modelName.endsWith("/premium")) return "premium";
  return "standard";
}

export function sanitizedConfig(config: AppConfig, catalog?: ModelCatalog) {
  return {
    ...JSON.parse(JSON.stringify(config)),
    runtime_overlay: catalog?.getRuntimeMetadataSummary() ?? {},
    virtual_model_ids: catalog?.getVirtualModelIds() ?? [],
  };
}

export async function proxyOpenAIEndpoint(c: Context, config: AppConfig, health: HealthRegistry, catalog: ModelCatalog | undefined, requestId: string, spec: ProxyEndpointSpec) {
  const startedAt = Date.now();
  const path = new URL(c.req.url).pathname;
  const sessionId = c.req.header("x-session-id");
  let requestedModel: string | undefined;
  let selectedModel: string | undefined;
  let provider: string | undefined;
  let upstreamModel: string | undefined;
  let stream = false;
  let status = 500;
  let errorCode: string | undefined;
  let fallbackCount = 0;
  let firstTokenLatencyMs: number | undefined;

  try {
    const requestBody = spec.bodyKind === "json" ? await readJsonBody(c) : await readMultipartRequest(c);
    requestedModel = requestBody.model;
    stream = spec.supportsStream && requestBody.kind === "json" && requestBody.stream === true;

    const candidates = candidateModels(config, health, catalog, requestedModel, spec.endpoint, requestBody.kind === "json" ? requestBody.jsonBody : undefined);
    let lastFailure: unknown;

    for (let index = 0; index < candidates.length; index += 1) {
      const selected = candidates[index]!;
      selectedModel = selected.name;
      provider = selected.config.provider;
      upstreamModel = selected.config.upstream_model;

      try {
        const response = await forwardToSelected(c, config, health, selected, spec, requestBody, stream, requestedModel, index > 0);
        status = response.status;
        fallbackCount = index;
        firstTokenLatencyMs = response.firstTokenLatencyMs;

        logRequest("request completed", {
          requestId,
          sessionId,
          method: c.req.method,
          path,
          endpoint: spec.endpoint,
          requestedModel,
          selectedModel,
          provider,
          upstreamModel,
          stream,
          status,
          durationMs: Date.now() - startedAt,
          firstTokenLatencyMs,
          fallbackCount,
        });

        return response.response;
      } catch (error) {
        lastFailure = error;
        fallbackCount = index;
        if (!isAutoRoute(config, requestedModel) || index + 1 >= candidates.length) throw error;
        logger.info(
          {
            request_id: requestId,
            session_id: sessionId,
            requested_model: requestedModel,
            selected_model: selectedModel,
            provider,
            upstream_model: upstreamModel,
            endpoint: spec.endpoint,
            fallback_count: index + 1,
            error_code: error instanceof GatewayError ? error.code : "upstream_error",
          },
          "fallback to next candidate",
        );
      }
    }

    throw lastFailure instanceof Error ? lastFailure : new GatewayError("No model candidate succeeded", 502, "upstream_error");
  } catch (error) {
    if (error instanceof GatewayError) {
      status = error.status;
      errorCode = error.code;
      throw error;
    }

    status = 502;
    errorCode = "upstream_error";
    throw new GatewayError(messageFromUnknown(error), 502, "upstream_error");
  } finally {
    if (errorCode) {
      logRequest("request failed", {
        requestId,
        sessionId,
        method: c.req.method,
        path,
        endpoint: spec.endpoint,
        requestedModel,
        selectedModel,
        provider,
        upstreamModel,
        stream,
        status,
        durationMs: Date.now() - startedAt,
        firstTokenLatencyMs,
        fallbackCount,
        errorCode,
      });
    }
  }
}

function candidateModels(config: AppConfig, health: HealthRegistry, catalog: ModelCatalog | undefined, requestedModel: string, endpoint: Endpoint, body?: OpenAIJsonBody): SelectedModel[] {
  if (isAutoRoute(config, requestedModel)) {
    return resolveAutoCandidates(config, health, requestedModel, endpoint, body, catalog).slice(0, Math.max(1, config.server.fallback_max_attempts));
  }

  const selected = resolveModel(config, health, requestedModel, catalog);
  if (selected.config.endpoint !== endpoint) {
    throw new GatewayError(`Model ${requestedModel} does not support endpoint ${endpoint}`, 400, "endpoint_mismatch");
  }
  return [selected];
}

async function forwardToSelected(
  c: Context,
  config: AppConfig,
  health: HealthRegistry,
  selected: SelectedModel,
  spec: ProxyEndpointSpec,
  requestBody: ParsedRequestBody,
  stream: boolean,
  requestedModel: string,
  isFallback: boolean,
): Promise<{ response: Response; status: number; firstTokenLatencyMs?: number }> {
  const apiKey = process.env[selected.config.api_key_env];
  if (!apiKey) {
    throw new GatewayError(`Provider API key is not configured for model: ${selected.name}`, 500, "provider_key_not_configured");
  }

  const startedAt = Date.now();
  let firstTokenLatencyMs: number | undefined;
  let requestFinished = false;
  health.beginRequest(selected.name);

  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), config.server.request_timeout_ms);

    let upstream: Response;
    try {
      upstream = await fetch(`${selected.config.base_url}${spec.upstreamPath}`, {
        method: "POST",
        headers: upstreamHeaders(apiKey, requestBody),
        body: upstreamBody(requestBody, selected.config.upstream_model),
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timeout);
    }

    const responseHeaders = filteredUpstreamHeaders(upstream.headers);
    responseHeaders.set("X-Tailgate-Model", selected.name);
    responseHeaders.set("X-Tailgate-Provider", selected.config.provider);
    responseHeaders.set("X-Tailgate-Route", requestedModel);
    responseHeaders.set("X-Tailgate-Fallback", String(isFallback));

    if (!upstream.ok) {
      requestFinished = true;
      health.finishRequest(selected.name, {
        ok: false,
        totalLatencyMs: Date.now() - startedAt,
        error: `upstream returned ${upstream.status}`,
      });
      throw new GatewayError(`Upstream ${selected.name} returned ${upstream.status}`, upstream.status >= 500 ? 502 : upstream.status, "upstream_error");
    }

    if (stream && upstream.body) {
      const measuredBody = measureStreamingBody(upstream.body, startedAt, (latency) => {
        firstTokenLatencyMs = latency;
        health.recordFirstToken(selected.name, latency);
      }, () => {
        requestFinished = true;
        health.finishRequest(selected.name, {
          ok: true,
          totalLatencyMs: Date.now() - startedAt,
          firstTokenLatencyMs,
        });
      });
      return {
        response: new Response(measuredBody, { status: upstream.status, statusText: upstream.statusText, headers: responseHeaders }),
        status: upstream.status,
        firstTokenLatencyMs,
      };
    }

    requestFinished = true;
    health.finishRequest(selected.name, {
      ok: true,
      totalLatencyMs: Date.now() - startedAt,
    });

    return {
      response: new Response(upstream.body, { status: upstream.status, statusText: upstream.statusText, headers: responseHeaders }),
      status: upstream.status,
    };
  } catch (error) {
    if (!requestFinished) {
      requestFinished = true;
      health.finishRequest(selected.name, {
        ok: false,
        totalLatencyMs: Date.now() - startedAt,
        error: messageFromUnknown(error),
      });
    }
    if (error instanceof GatewayError) throw error;
    throw new GatewayError(messageFromUnknown(error), 502, "upstream_error");
  }
}

function measureStreamingBody(body: ReadableStream<Uint8Array>, startedAt: number, onFirstChunk: (latencyMs: number) => void, onDone: () => void): ReadableStream<Uint8Array> {
  const reader = body.getReader();
  let measured = false;
  let done = false;

  const finish = () => {
    if (!done) {
      done = true;
      onDone();
    }
  };

  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const next = await reader.read();
        if (next.done) {
          finish();
          controller.close();
          return;
        }
        if (!measured) {
          measured = true;
          onFirstChunk(Date.now() - startedAt);
        }
        controller.enqueue(next.value);
      } catch (error) {
        finish();
        controller.error(error);
      }
    },
    async cancel(reason) {
      finish();
      await reader.cancel(reason);
    },
  });
}

async function readJsonBody(c: Context): Promise<ParsedRequestBody> {
  try {
    const parsed = (await c.req.json()) as unknown;
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      throw new GatewayError("Request body must be a JSON object", 400, "invalid_request");
    }
    const jsonBody = parsed as OpenAIJsonBody;
    const model = modelFromValue(jsonBody.model);
    return { kind: "json", model, stream: jsonBody.stream, jsonBody };
  } catch (error) {
    if (error instanceof GatewayError) throw error;
    throw new GatewayError("Invalid JSON request body", 400, "invalid_json");
  }
}

async function readMultipartRequest(c: Context): Promise<ParsedRequestBody> {
  const contentType = c.req.header("content-type");
  if (!contentType?.toLowerCase().includes("multipart/form-data")) {
    throw new GatewayError("Request must be multipart/form-data", 400, "invalid_content_type");
  }

  const formData = await c.req.formData();
  const model = modelFromValue(formData.get("model"));
  return { kind: "multipart", model, formData };
}

function modelFromValue(value: unknown): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new GatewayError("Request body must include a model", 400, "model_required");
  }
  return value;
}

function upstreamHeaders(apiKey: string, requestBody: ParsedRequestBody): HeadersInit {
  const headers: HeadersInit = {
    Authorization: `Bearer ${apiKey}`,
  };
  if (requestBody.kind === "json") headers["Content-Type"] = "application/json";
  return headers;
}

function upstreamBody(requestBody: ParsedRequestBody, upstreamModel: string): BodyInit {
  if (requestBody.kind === "json") {
    return JSON.stringify({ ...requestBody.jsonBody, model: upstreamModel });
  }

  const formData = new FormData();
  requestBody.formData.forEach((value, key) => {
    formData.append(key, key === "model" ? upstreamModel : value);
  });
  return formData;
}

export function filteredUpstreamHeaders(headers: Headers): Headers {
  const result = new Headers();
  const blocked = new Set([
    "connection",
    "content-encoding",
    "content-length",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
  ]);

  headers.forEach((value, key) => {
    if (!blocked.has(key.toLowerCase())) result.set(key, value);
  });

  return result;
}

function logRequest(message: string, fields: RequestLogFields) {
  logger.info(
    {
      request_id: fields.requestId,
      session_id: fields.sessionId,
      method: fields.method,
      path: fields.path,
      endpoint: fields.endpoint,
      requested_model: fields.requestedModel,
      selected_model: fields.selectedModel,
      provider: fields.provider,
      upstream_model: fields.upstreamModel,
      stream: fields.stream,
      status: fields.status,
      duration_ms: fields.durationMs,
      first_token_latency_ms: fields.firstTokenLatencyMs,
      fallback_count: fields.fallbackCount,
      error_code: fields.errorCode,
    },
    message,
  );
}

type ParsedRequestBody =
  | { kind: "json"; model: string; stream: unknown; jsonBody: OpenAIJsonBody }
  | { kind: "multipart"; model: string; formData: FormData };

interface RequestLogFields {
  requestId: string;
  sessionId?: string;
  method: string;
  path: string;
  endpoint: Endpoint;
  requestedModel?: string;
  selectedModel?: string;
  provider?: string;
  upstreamModel?: string;
  stream: boolean;
  status: number;
  durationMs: number;
  firstTokenLatencyMs?: number;
  fallbackCount: number;
  errorCode?: string;
}
