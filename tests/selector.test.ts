import assert from "node:assert/strict";
import { resolveAutoCandidates } from "../src/selector.js";
import { AppConfig, HealthState } from "../src/types.js";

const healthy = (): HealthState => ({
  healthy: true,
  busy: false,
  in_flight: 0,
  error_rate: 0,
  success_count: 0,
  error_count: 0,
  network_latency_ms: 100,
});

const config: AppConfig = {
  server: {
    host: "127.0.0.1",
    port: 11435,
    request_timeout_ms: 60_000,
    fallback_max_attempts: 2,
    cors_allowed_origins: ["*"],
  },
  models: {
    "deepseek/chat": {
      provider: "deepseek",
      upstream_model: "deepseek-v4-flash",
      base_url: "https://api.deepseek.com/v1",
      api_key_env: "DEEPSEEK_API_KEY",
      endpoint: "chat",
      cost_tier: "standard",
      price_rank: 30,
    },
    "openrouter/auto": {
      provider: "openrouter",
      upstream_model: "openrouter/auto",
      base_url: "https://openrouter.ai/api/v1",
      api_key_env: "OPENROUTER_API_KEY",
      endpoint: "chat",
      cost_tier: "standard",
      price_rank: 30,
    },
  },
  model_order: {
    "deepseek/chat": 0,
    "openrouter/auto": 1,
  },
  routes: {
    "standard/chat": {
      endpoint: "chat",
      cost_tier: "standard",
    },
  },
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

const health = {
  get: () => healthy(),
};

const candidates = resolveAutoCandidates(config, health as any, "standard/chat", "chat");

assert.equal(candidates[0]?.name, "deepseek/chat");
assert.equal(candidates[1]?.name, "openrouter/auto");

console.log("selector tests passed");
