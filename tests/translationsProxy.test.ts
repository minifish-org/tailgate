import assert from "node:assert/strict";
import { createApp } from "../src/app.js";
import { AppConfig, HealthState } from "../src/types.js";

process.env.ROUTER_API_KEY = "test-router-key";
process.env.LOCAL_API_KEY = "local-test-key";

const healthy = (): HealthState => ({
  healthy: true,
  busy: false,
  in_flight: 0,
  error_rate: 0,
  success_count: 0,
  error_count: 0,
  network_latency_ms: 30,
});

const config: AppConfig = {
  server: {
    host: "127.0.0.1",
    port: 11435,
    request_timeout_ms: 60_000,
    fallback_max_attempts: 2,
    cors_allowed_origins: ["http://127.0.0.1:5173"],
  },
  models: {
    "local/translation": {
      provider: "local",
      upstream_model: "qwen-local-translation",
      base_url: "http://qwen-local.test/v1",
      api_key_env: "LOCAL_API_KEY",
      endpoint: "translations",
      max_concurrency: 1,
    },
  },
  model_order: {
    "local/translation": 0,
  },
  routes: {
    "free/translation": {
      endpoint: "translations",
      cost_tier: "free",
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
  snapshot: () => [],
  beginRequest: () => undefined,
  finishRequest: () => undefined,
  recordFirstToken: () => undefined,
};

const catalog = {
  getModel: (modelName: string) => (config.models[modelName] ? { name: modelName, config: config.models[modelName] } : undefined),
  getModelEntries: () => Object.entries(config.models).map(([name, model]) => [name, model, undefined] as const),
  getConfiguredModelEntries: () => Object.entries(config.models),
  getVirtualModelIds: () => [],
  getRuntimeMetadataSummary: () => ({}),
};

const app = createApp({
  config,
  health: health as any,
  catalog: catalog as any,
  openRouterSync: {
    getStatus: () => ({ enabled: false }),
    sync: async () => ({}),
  } as any,
  deepSeekSync: {
    getStatus: () => ({ enabled: false }),
    sync: async () => ({}),
  } as any,
});

const originalFetch = globalThis.fetch;
const upstreamPayload = {
  object: "translation",
  translation: "你好",
  source_language: "en",
  target_language: "zh",
};
const fetchCalls: Array<{ url: string; init: RequestInit; body: Record<string, unknown> }> = [];

globalThis.fetch = async (input, init) => {
  const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
  fetchCalls.push({ url: String(input), init: init ?? {}, body });

  return new Response(JSON.stringify(upstreamPayload), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
};

try {
  const unauthorizedResponse = await app.request("/v1/translations", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: "local/translation",
      source_language: "en",
      target_language: "zh",
      text: "hello",
    }),
  });

  assert.equal(unauthorizedResponse.status, 401);
  assert.equal(fetchCalls.length, 0);

  const response = await app.request("/v1/translations", {
    method: "POST",
    headers: {
      Authorization: "Bearer test-router-key",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: "free/translation",
      source_language: "en",
      target_language: "zh",
      text: "hello",
    }),
  });

  assert.equal(response.status, 200);
  assert.equal(await response.text(), JSON.stringify(upstreamPayload));
  assert.equal(response.headers.get("x-tailgate-model"), "local/translation");
  assert.equal(response.headers.get("x-tailgate-provider"), "local");
  assert.equal(response.headers.get("x-tailgate-route"), "free/translation");

  assert.equal(fetchCalls.length, 1);
  assert.equal(fetchCalls[0]?.url, "http://qwen-local.test/v1/translations");
  assert.equal(fetchCalls[0]?.init.method, "POST");
  assert.equal((fetchCalls[0]?.init.headers as Record<string, string>).Authorization, "Bearer local-test-key");
  assert.deepEqual(fetchCalls[0]?.body, {
    model: "qwen-local-translation",
    source_language: "en",
    target_language: "zh",
    text: "hello",
  });
} finally {
  globalThis.fetch = originalFetch;
}

console.log("translation proxy tests passed");
