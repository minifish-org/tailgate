import "dotenv/config";
import { randomUUID } from "node:crypto";
import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { requireRouterAuth } from "./auth.js";
import { loadConfig } from "./config.js";
import { errorResponse, GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { OpenRouterSyncService } from "./openrouterSync.js";
import { ENDPOINT_SPECS, modelsResponse, proxyOpenAIEndpoint, sanitizedConfig } from "./openaiProxy.js";

const config = loadConfig();
const health = new HealthRegistry(config);
const openRouterSync = new OpenRouterSyncService(config, health);
health.start();
openRouterSync.start();

const app = new Hono();

app.use("/v1/*", requireRouterAuth);
app.use("/tailgate/*", requireRouterAuth);

app.get("/v1/models", (c) => c.json(modelsResponse(config, openRouterSync)));

app.post("/v1/chat/completions", async (c) => {
  const requestId = c.req.header("x-request-id") || randomUUID();
  return proxyOpenAIEndpoint(c, config, health, openRouterSync, requestId, ENDPOINT_SPECS.chat);
});

app.post("/v1/embeddings", async (c) => {
  const requestId = c.req.header("x-request-id") || randomUUID();
  return proxyOpenAIEndpoint(c, config, health, openRouterSync, requestId, ENDPOINT_SPECS.embeddings);
});

app.post("/v1/audio/speech", async (c) => {
  const requestId = c.req.header("x-request-id") || randomUUID();
  return proxyOpenAIEndpoint(c, config, health, openRouterSync, requestId, ENDPOINT_SPECS.audio_speech);
});

app.post("/v1/audio/transcriptions", async (c) => {
  const requestId = c.req.header("x-request-id") || randomUUID();
  return proxyOpenAIEndpoint(c, config, health, openRouterSync, requestId, ENDPOINT_SPECS.audio_transcriptions);
});

app.get("/tailgate/health", (c) =>
  c.json({
    object: "tailgate.health",
    data: health.snapshot(openRouterSync.getModelEntries().map(([id, model]) => ({ id, provider: model.provider, endpoint: model.endpoint }))),
    openrouter_sync: openRouterSync.getStatus(),
  }),
);

app.get("/tailgate/config", (c) => c.json(sanitizedConfig(config, openRouterSync)));

app.post("/tailgate/sync/openrouter", async (c) => c.json(await openRouterSync.sync()));

app.notFound((c) => errorResponse(c, 404, "not_found", "Not found"));

app.onError((error, c) => {
  if (error instanceof GatewayError) {
    return errorResponse(c, error.status, error.code, error.message);
  }

  logger.error({ error: error instanceof Error ? error.message : String(error) }, "unhandled error");
  return errorResponse(c, 500, "internal_error", "Internal server error");
});

serve(
  {
    fetch: app.fetch,
    hostname: config.server.host,
    port: config.server.port,
  },
  (info) => {
    logger.info({ host: info.address, port: info.port }, "tailgate listening");
  },
);

process.on("SIGTERM", () => {
  logger.info("received SIGTERM");
  health.stop();
  openRouterSync.stop();
  process.exit(0);
});

process.on("SIGINT", () => {
  logger.info("received SIGINT");
  health.stop();
  openRouterSync.stop();
  process.exit(0);
});
