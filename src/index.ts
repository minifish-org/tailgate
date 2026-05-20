import "dotenv/config";
import { serve } from "@hono/node-server";
import { createApp } from "./app.js";
import { loadConfig } from "./config.js";
import { DeepSeekSyncService } from "./deepseekSync.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { OpenRouterSyncService } from "./openrouterSync.js";
import { RuntimeCatalog } from "./runtimeCatalog.js";

const config = loadConfig();
const health = new HealthRegistry(config);
const catalog = new RuntimeCatalog(config, health);
const openRouterSync = new OpenRouterSyncService(config, catalog);
const deepSeekSync = new DeepSeekSyncService(config, catalog);
health.start();
openRouterSync.start();
deepSeekSync.start();

const app = createApp({ config, health, catalog, openRouterSync, deepSeekSync });

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
  deepSeekSync.stop();
  process.exit(0);
});

process.on("SIGINT", () => {
  logger.info("received SIGINT");
  health.stop();
  openRouterSync.stop();
  deepSeekSync.stop();
  process.exit(0);
});
