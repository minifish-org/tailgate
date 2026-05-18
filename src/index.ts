import "dotenv/config";
import { randomUUID } from "node:crypto";
import { serve } from "@hono/node-server";
import { Hono } from "hono";
import { requireRouterAuth } from "./auth.js";
import { loadConfig } from "./config.js";
import { errorResponse, GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { modelsResponse, proxyChatCompletions } from "./openaiProxy.js";

const config = loadConfig();
const health = new HealthRegistry(config);
health.start();

const app = new Hono();

app.use("/v1/*", requireRouterAuth);

app.get("/v1/models", (c) => c.json(modelsResponse(config)));

app.post("/v1/chat/completions", async (c) => {
  const requestId = c.req.header("x-request-id") || randomUUID();
  return proxyChatCompletions(c, config, health, requestId);
});

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
  process.exit(0);
});

process.on("SIGINT", () => {
  logger.info("received SIGINT");
  health.stop();
  process.exit(0);
});
