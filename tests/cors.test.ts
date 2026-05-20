import assert from "node:assert/strict";
import { createApp } from "../src/app.js";
import { AppConfig } from "../src/types.js";

process.env.ROUTER_API_KEY = "test-router-key";

const config: AppConfig = {
  server: {
    host: "127.0.0.1",
    port: 11435,
    request_timeout_ms: 60_000,
    fallback_max_attempts: 2,
    cors_allowed_origins: ["http://127.0.0.1:5173", "http://localhost:5173"],
  },
  models: {},
  routes: {},
  routing: {},
  openrouter_sync: {
    enabled: false,
    interval_seconds: 21_600,
    update_config_file: false,
    source_url: "https://openrouter.ai/api/v1/models",
    include_unconfigured_models: false,
    allowlist: [],
    cost_tiers: {
      free_max_usd_per_1m_tokens: 0,
      standard_max_usd_per_1m_tokens: 2,
      premium_max_usd_per_1m_tokens: 9999,
    },
  },
  deepseek_sync: {
    enabled: false,
    interval_seconds: 21_600,
    source_url: "https://api-docs.deepseek.com/quick_start/pricing/",
    cost_tiers: {
      free_max_usd_per_1m_tokens: 0,
      standard_max_usd_per_1m_tokens: 2,
      premium_max_usd_per_1m_tokens: 9999,
    },
  },
};

const app = createApp({
  config,
  health: {
    snapshot: () => [],
  } as any,
  catalog: {
    getModelEntries: () => [],
  } as any,
  openRouterSync: {
    getStatus: () => ({ enabled: false }),
    sync: async () => ({}),
  } as any,
  deepSeekSync: {
    getStatus: () => ({ enabled: false }),
    sync: async () => ({}),
  } as any,
});

const optionsResponse = await app.request("/v1/chat/completions", {
  method: "OPTIONS",
  headers: {
    Origin: "http://127.0.0.1:5173",
    "Access-Control-Request-Method": "POST",
    "Access-Control-Request-Headers": "authorization,content-type",
  },
});

assert.equal(optionsResponse.status, 204);
assert.equal(optionsResponse.headers.get("access-control-allow-origin"), "http://127.0.0.1:5173");
assert.equal(optionsResponse.headers.get("access-control-allow-methods"), "POST, OPTIONS");
assert.equal(optionsResponse.headers.get("access-control-allow-headers"), "authorization, content-type");

const postResponse = await app.request("/v1/chat/completions", {
  method: "POST",
  headers: {
    Origin: "http://127.0.0.1:5173",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({ model: "standard/chat", messages: [] }),
});

assert.equal(postResponse.status, 401);
assert.equal(postResponse.headers.get("access-control-allow-origin"), "http://127.0.0.1:5173");

console.log("cors tests passed");
