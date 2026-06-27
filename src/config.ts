import fs from "node:fs";
import YAML from "yaml";
import { AppConfig, CostTier, DeepSeekSyncConfig, Endpoint, ModelConfig, OpenRouterSyncConfig, RouteConfig, RoutingConfig } from "./types.js";

const COST_TIERS: CostTier[] = ["free", "standard", "premium"];
const ENDPOINTS: Endpoint[] = ["chat", "embeddings", "audio_speech", "audio_transcriptions", "translations"];
const DEFAULT_CORS_ALLOWED_ORIGINS = ["http://127.0.0.1:5173", "http://localhost:5173"];

export function loadConfig(path = process.env.CONFIG_PATH || "./config.yaml"): AppConfig {
  if (!fs.existsSync(path)) {
    throw new Error(`Config file not found: ${path}`);
  }

  const parsed = YAML.parse(fs.readFileSync(path, "utf8")) as unknown;
  return validateConfig(parsed);
}

function validateConfig(value: unknown): AppConfig {
  if (!isRecord(value)) throw new Error("Config must be an object");

  const server = value.server;
  if (!isRecord(server)) throw new Error("Config server must be an object");

  if (!isRecord(value.models)) {
    return validateExpandedConfig(expandSimpleConfig(value));
  }

  return validateExpandedConfig(value);
}

function validateExpandedConfig(value: Record<string, unknown>): AppConfig {
  const server = value.server;
  if (!isRecord(server)) throw new Error("Config server must be an object");

  const models = value.models;
  if (!isRecord(models)) throw new Error("Config models must be an object");

  const appConfig: AppConfig = {
    server: {
      host: stringValue(server.host, "server.host"),
      port: numberValue(server.port, "server.port"),
      request_timeout_ms: numberValue(server.request_timeout_ms, "server.request_timeout_ms"),
      fallback_max_attempts: server.fallback_max_attempts === undefined ? 2 : numberValue(server.fallback_max_attempts, "server.fallback_max_attempts"),
      cors_allowed_origins:
        server.cors_allowed_origins === undefined ? DEFAULT_CORS_ALLOWED_ORIGINS : stringArray(server.cors_allowed_origins, "server.cors_allowed_origins"),
    },
    models: {},
    model_order: {},
    routes: builtinRoutes(validateRouting(value.routing)),
    routing: validateRouting(value.routing),
    openrouter_sync: validateOpenRouterSync(value.openrouter_sync),
    deepseek_sync: validateDeepSeekSync(value.deepseek_sync),
  };

  for (const [index, [name, rawModel]] of Object.entries(models).entries()) {
    appConfig.models[name] = validateModel(rawModel, `models.${name}`);
    appConfig.model_order[name] = index;
  }

  return appConfig;
}

function expandSimpleConfig(value: Record<string, unknown>): Record<string, unknown> {
  const server = isRecord(value.server) ? value.server : {};
  const sync = isRecord(value.sync) ? value.sync : {};
  const pricing = isRecord(value.pricing) ? value.pricing : {};
  const local = isRecord(value.local) ? value.local : {};
  const deepseek = isRecord(value.deepseek) ? value.deepseek : {};
  const openrouter = isRecord(value.openrouter) ? value.openrouter : {};
  const routing = isRecord(value.routing) ? value.routing : {};

  const localBaseUrl = stringOrDefault(local.base_url, "http://macbook-pro:8000/v1").replace(/\/+$/, "");
  const localApiKeyEnv = stringOrDefault(local.api_key_env, "LOCAL_API_KEY");
  const localMaxConcurrency = numberOrDefault(local.max_concurrency, 1);
  const localContextWindow = numberOrDefault(local.context_window, 8192);
  const localTtsVoiceDesignModel = stringOrDefault(local.tts_voice_design_model, "local-tts-voice-design");

  const deepseekBaseUrl = stringOrDefault(deepseek.base_url, "https://api.deepseek.com/v1").replace(/\/+$/, "");
  const deepseekApiKeyEnv = stringOrDefault(deepseek.api_key_env, "DEEPSEEK_API_KEY");
  const deepseekModel = stringOrDefault(deepseek.model, "deepseek-v4-flash");
  const deepseekPremiumModel = stringOrDefault(deepseek.premium_model, "deepseek-v4-pro");

  const openrouterBaseUrl = stringOrDefault(openrouter.base_url, "https://openrouter.ai/api/v1").replace(/\/+$/, "");
  const openrouterApiKeyEnv = stringOrDefault(openrouter.api_key_env, "OPENROUTER_API_KEY");
  const openrouterFreeModel = stringOrDefault(openrouter.free_model, "openrouter/free");
  const openrouterAutoModel = stringOrDefault(openrouter.standard_model ?? openrouter.auto_model, "openrouter/auto");
  const openrouterPremiumModel = stringOrDefault(openrouter.premium_model, "anthropic/claude-sonnet-4");
  const openrouterExtraModels = openrouter.extra_models === undefined ? [] : stringArray(openrouter.extra_models, "openrouter.extra_models");

  const syncIntervalSeconds = numberOrDefault(sync.interval_seconds, 21_600);
  const standardMaxUsd = numberOrDefault(pricing.standard_max_usd_per_1m_tokens, 2);

  return {
    server: {
      host: stringValue(server.host, "server.host"),
      port: numberValue(server.port, "server.port"),
      request_timeout_ms: numberOrDefault(server.request_timeout_ms, 60_000),
      fallback_max_attempts: numberOrDefault(server.fallback_max_attempts, 2),
      cors_allowed_origins: server.cors_allowed_origins === undefined ? DEFAULT_CORS_ALLOWED_ORIGINS : stringArray(server.cors_allowed_origins, "server.cors_allowed_origins"),
    },
    openrouter_sync: {
      enabled: booleanOrDefault(sync.openrouter, false),
      interval_seconds: syncIntervalSeconds,
      update_config_file: false,
      source_url: "https://openrouter.ai/api/v1/models",
      include_unconfigured_models: false,
      allowlist: uniqueStrings([openrouterFreeModel, openrouterAutoModel, openrouterPremiumModel, ...openrouterExtraModels]),
      cost_tiers: costTierDefaults(standardMaxUsd),
    },
    deepseek_sync: {
      enabled: booleanOrDefault(sync.deepseek, false),
      interval_seconds: syncIntervalSeconds,
      source_url: "https://api-docs.deepseek.com/quick_start/pricing/",
      cost_tiers: costTierDefaults(standardMaxUsd),
    },
    routing: {
      latency: {
        network_ms_max: numberOrDefault(routing.network_ms_max, 250),
        first_token_ms_max: numberOrDefault(routing.first_token_ms_max, 5000),
      },
    },
    models: {
      "local/chat": {
        provider: "local",
        upstream_model: stringOrDefault(local.chat_model, "local-llm"),
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "chat",
        context_window: localContextWindow,
        max_concurrency: localMaxConcurrency,
      },
      "local/embedding": {
        provider: "local",
        upstream_model: stringOrDefault(local.embedding_model, "local-embedding"),
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "embeddings",
        max_concurrency: localMaxConcurrency,
      },
      "local/tts": {
        provider: "local",
        upstream_model: stringOrDefault(local.tts_model, "local-tts"),
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "audio_speech",
        max_concurrency: localMaxConcurrency,
      },
      "local/tts-voice-design": {
        provider: "local",
        upstream_model: localTtsVoiceDesignModel,
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "audio_speech",
        max_concurrency: localMaxConcurrency,
      },
      "local/asr": {
        provider: "local",
        upstream_model: stringOrDefault(local.asr_model, "local-asr"),
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "audio_transcriptions",
        max_concurrency: localMaxConcurrency,
      },
      "local/translation": {
        provider: "local",
        upstream_model: stringOrDefault(local.translation_model, "local-translation"),
        base_url: localBaseUrl,
        api_key_env: localApiKeyEnv,
        endpoint: "translations",
        max_concurrency: localMaxConcurrency,
      },
      "deepseek/chat": {
        provider: "deepseek",
        upstream_model: deepseekModel,
        base_url: deepseekBaseUrl,
        api_key_env: deepseekApiKeyEnv,
        endpoint: "chat",
        context_window: numberOrDefault(deepseek.context_window, 64_000),
      },
      "deepseek/premium": {
        provider: "deepseek",
        upstream_model: deepseekPremiumModel,
        base_url: deepseekBaseUrl,
        api_key_env: deepseekApiKeyEnv,
        endpoint: "chat",
        cost_tier: "premium",
        context_window: numberOrDefault(deepseek.premium_context_window ?? deepseek.context_window, 64_000),
      },
      "openrouter/auto": {
        provider: "openrouter",
        upstream_model: openrouterAutoModel,
        base_url: openrouterBaseUrl,
        api_key_env: openrouterApiKeyEnv,
        endpoint: "chat",
        context_window: numberOrDefault(openrouter.standard_context_window ?? openrouter.auto_context_window, 128_000),
      },
      "openrouter/premium": {
        provider: "openrouter",
        upstream_model: openrouterPremiumModel,
        base_url: openrouterBaseUrl,
        api_key_env: openrouterApiKeyEnv,
        endpoint: "chat",
        context_window: numberOrDefault(openrouter.premium_context_window, 200_000),
      },
    },
  };
}

function validateModel(value: unknown, path: string): ModelConfig {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);

  const endpoint = stringValue(value.endpoint, `${path}.endpoint`) as Endpoint;
  if (!ENDPOINTS.includes(endpoint)) throw new Error(`${path}.endpoint is invalid`);

  const costTier = value.cost_tier === undefined ? undefined : (stringValue(value.cost_tier, `${path}.cost_tier`) as CostTier);
  if (costTier && !COST_TIERS.includes(costTier)) throw new Error(`${path}.cost_tier is invalid`);

  return {
    provider: stringValue(value.provider, `${path}.provider`),
    upstream_model: stringValue(value.upstream_model, `${path}.upstream_model`),
    base_url: stringValue(value.base_url, `${path}.base_url`).replace(/\/+$/, ""),
    api_key_env: stringValue(value.api_key_env, `${path}.api_key_env`),
    endpoint,
    cost_tier: costTier,
    price_rank: value.price_rank === undefined ? undefined : numberValue(value.price_rank, `${path}.price_rank`),
    context_window: value.context_window === undefined ? undefined : numberValue(value.context_window, `${path}.context_window`),
    max_concurrency: value.max_concurrency === undefined ? undefined : numberValue(value.max_concurrency, `${path}.max_concurrency`),
    supported_parameters: value.supported_parameters === undefined ? undefined : stringArray(value.supported_parameters, `${path}.supported_parameters`),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${path} must be a non-empty string`);
  return value;
}

function numberValue(value: unknown, path: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`${path} must be a number`);
  return value;
}

function booleanValue(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${path} must be a boolean`);
  return value;
}

function stringOrDefault(value: unknown, fallback: string): string {
  return value === undefined ? fallback : stringValue(value, "config value");
}

function numberOrDefault(value: unknown, fallback: number): number {
  return value === undefined ? fallback : numberValue(value, "config value");
}

function booleanOrDefault(value: unknown, fallback: boolean): boolean {
  return value === undefined ? fallback : booleanValue(value, "config value");
}

function uniqueStrings(values: string[]): string[] {
  return Array.from(new Set(values));
}

function costTierDefaults(standardMaxUsd: number) {
  return {
    free_max_usd_per_1m_tokens: 0,
    standard_max_usd_per_1m_tokens: standardMaxUsd,
    premium_max_usd_per_1m_tokens: 9999,
  };
}

function latencyConfig(value: unknown, path: string) {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);
  return {
    network_p95_ms_max:
      value.network_p95_ms_max === undefined
        ? value.network_ms_max === undefined
          ? undefined
          : numberValue(value.network_ms_max, `${path}.network_ms_max`)
        : numberValue(value.network_p95_ms_max, `${path}.network_p95_ms_max`),
    first_token_p95_ms_max:
      value.first_token_p95_ms_max === undefined
        ? value.first_token_ms_max === undefined
          ? undefined
          : numberValue(value.first_token_ms_max, `${path}.first_token_ms_max`)
        : numberValue(value.first_token_p95_ms_max, `${path}.first_token_p95_ms_max`),
  };
}

function validateRouting(value: unknown): RoutingConfig {
  const raw = isRecord(value) ? value : {};
  return {
    latency: raw.latency === undefined ? { network_p95_ms_max: 250, first_token_p95_ms_max: 5000 } : latencyConfig(raw.latency, "routing.latency"),
  };
}

function builtinRoutes(routing: RoutingConfig): Record<string, RouteConfig> {
  const latency = routing.latency;
  const route = (endpoint: Endpoint, costTier?: CostTier): RouteConfig => ({
    endpoint,
    cost_tier: costTier,
    latency,
  });

  const routes: Record<string, RouteConfig> = {
    "free/chat": route("chat", "free"),
    "free/embedding": route("embeddings", "free"),
    "free/tts": route("audio_speech", "free"),
    "free/asr": route("audio_transcriptions", "free"),
    "free/translation": route("translations", "free"),

    "standard/chat": route("chat", "standard"),
    "standard/embedding": route("embeddings", "standard"),
    "standard/tts": route("audio_speech", "standard"),
    "standard/asr": route("audio_transcriptions", "standard"),
    "standard/translation": route("translations", "standard"),

    "premium/chat": route("chat", "premium"),
    "premium/embedding": route("embeddings", "premium"),
    "premium/tts": route("audio_speech", "premium"),
    "premium/asr": route("audio_transcriptions", "premium"),
    "premium/translation": route("translations", "premium"),
  };
  return routes;
}

function validateOpenRouterSync(value: unknown): OpenRouterSyncConfig {
  const raw = isRecord(value) ? value : {};
  const costTiers = isRecord(raw.cost_tiers) ? raw.cost_tiers : {};

  return {
    enabled: raw.enabled === undefined ? false : booleanValue(raw.enabled, "openrouter_sync.enabled"),
    interval_seconds: raw.interval_seconds === undefined ? 21_600 : numberValue(raw.interval_seconds, "openrouter_sync.interval_seconds"),
    update_config_file: raw.update_config_file === undefined ? false : booleanValue(raw.update_config_file, "openrouter_sync.update_config_file"),
    source_url: raw.source_url === undefined ? "https://openrouter.ai/api/v1/models" : stringValue(raw.source_url, "openrouter_sync.source_url"),
    include_unconfigured_models: raw.include_unconfigured_models === undefined ? false : booleanValue(raw.include_unconfigured_models, "openrouter_sync.include_unconfigured_models"),
    allowlist: raw.allowlist === undefined ? [] : stringArray(raw.allowlist, "openrouter_sync.allowlist"),
    cost_tiers: {
      free_max_usd_per_1m_tokens:
        costTiers.free_max_usd_per_1m_tokens === undefined ? 0 : numberValue(costTiers.free_max_usd_per_1m_tokens, "openrouter_sync.cost_tiers.free_max_usd_per_1m_tokens"),
      standard_max_usd_per_1m_tokens:
        costTiers.standard_max_usd_per_1m_tokens === undefined ? 2 : numberValue(costTiers.standard_max_usd_per_1m_tokens, "openrouter_sync.cost_tiers.standard_max_usd_per_1m_tokens"),
      premium_max_usd_per_1m_tokens:
        costTiers.premium_max_usd_per_1m_tokens === undefined ? 9999 : numberValue(costTiers.premium_max_usd_per_1m_tokens, "openrouter_sync.cost_tiers.premium_max_usd_per_1m_tokens"),
    },
  };
}

function validateDeepSeekSync(value: unknown): DeepSeekSyncConfig {
  const raw = isRecord(value) ? value : {};
  const costTiers = isRecord(raw.cost_tiers) ? raw.cost_tiers : {};

  return {
    enabled: raw.enabled === undefined ? false : booleanValue(raw.enabled, "deepseek_sync.enabled"),
    interval_seconds: raw.interval_seconds === undefined ? 21_600 : numberValue(raw.interval_seconds, "deepseek_sync.interval_seconds"),
    source_url: raw.source_url === undefined ? "https://api-docs.deepseek.com/quick_start/pricing/" : stringValue(raw.source_url, "deepseek_sync.source_url"),
    cost_tiers: {
      free_max_usd_per_1m_tokens:
        costTiers.free_max_usd_per_1m_tokens === undefined ? 0 : numberValue(costTiers.free_max_usd_per_1m_tokens, "deepseek_sync.cost_tiers.free_max_usd_per_1m_tokens"),
      standard_max_usd_per_1m_tokens:
        costTiers.standard_max_usd_per_1m_tokens === undefined ? 2 : numberValue(costTiers.standard_max_usd_per_1m_tokens, "deepseek_sync.cost_tiers.standard_max_usd_per_1m_tokens"),
      premium_max_usd_per_1m_tokens:
        costTiers.premium_max_usd_per_1m_tokens === undefined ? 9999 : numberValue(costTiers.premium_max_usd_per_1m_tokens, "deepseek_sync.cost_tiers.premium_max_usd_per_1m_tokens"),
    },
  };
}

function stringArray(value: unknown, path: string): string[] {
  if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
  return value.map((item, index) => stringValue(item, `${path}.${index}`));
}
