import { randomUUID } from "node:crypto";
import { Hono } from "hono";
import { requireRouterAuth } from "./auth.js";
import { openAICors } from "./cors.js";
import { DeepSeekSyncService } from "./deepseekSync.js";
import { errorResponse, GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { OpenRouterSyncService } from "./openrouterSync.js";
import { ENDPOINT_SPECS, modelsResponse, proxyOpenAIEndpoint, sanitizedConfig } from "./openaiProxy.js";
import { RuntimeCatalog } from "./runtimeCatalog.js";
import { AppConfig } from "./types.js";

export function createApp(deps: {
  config: AppConfig;
  health: HealthRegistry;
  catalog: RuntimeCatalog;
  openRouterSync: OpenRouterSyncService;
  deepSeekSync: DeepSeekSyncService;
}) {
  const { config, health, catalog, openRouterSync, deepSeekSync } = deps;
  const app = new Hono();

  app.use("*", openAICors(config.server.cors_allowed_origins));
  app.use("/v1/*", requireRouterAuth);
  app.use("/tailgate/*", requireRouterAuth);

  app.get("/v1/models", (c) => c.json(modelsResponse(config, catalog)));

  app.post("/v1/chat/completions", async (c) => {
    const requestId = c.req.header("x-request-id") || randomUUID();
    return proxyOpenAIEndpoint(c, config, health, catalog, requestId, ENDPOINT_SPECS.chat);
  });

  app.post("/v1/embeddings", async (c) => {
    const requestId = c.req.header("x-request-id") || randomUUID();
    return proxyOpenAIEndpoint(c, config, health, catalog, requestId, ENDPOINT_SPECS.embeddings);
  });

  app.post("/v1/audio/speech", async (c) => {
    const requestId = c.req.header("x-request-id") || randomUUID();
    return proxyOpenAIEndpoint(c, config, health, catalog, requestId, ENDPOINT_SPECS.audio_speech);
  });

  app.post("/v1/audio/transcriptions", async (c) => {
    const requestId = c.req.header("x-request-id") || randomUUID();
    return proxyOpenAIEndpoint(c, config, health, catalog, requestId, ENDPOINT_SPECS.audio_transcriptions);
  });

  app.post("/v1/translations", async (c) => {
    const requestId = c.req.header("x-request-id") || randomUUID();
    return proxyOpenAIEndpoint(c, config, health, catalog, requestId, ENDPOINT_SPECS.translations);
  });

  app.get("/tailgate/health", (c) =>
    c.json({
      object: "tailgate.health",
      data: health.snapshot(catalog.getModelEntries().map(([id, model]) => ({ id, provider: model.provider, endpoint: model.endpoint }))),
      openrouter_sync: openRouterSync.getStatus(),
      deepseek_sync: deepSeekSync.getStatus(),
    }),
  );

  app.get("/tailgate/config", (c) => c.json(sanitizedConfig(config, catalog)));

  app.post("/tailgate/sync/openrouter", async (c) => c.json(await openRouterSync.sync()));
  app.post("/tailgate/sync/deepseek", async (c) => c.json(await deepSeekSync.sync()));

  app.notFound((c) => errorResponse(c, 404, "not_found", "Not found"));

  app.onError((error, c) => {
    if (error instanceof GatewayError) {
      return errorResponse(c, error.status, error.code, error.message);
    }

    logger.error({ error: error instanceof Error ? error.message : String(error) }, "unhandled error");
    return errorResponse(c, 500, "internal_error", "Internal server error");
  });

  return app;
}
