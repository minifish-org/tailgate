import fs from "node:fs";
import YAML from "yaml";
import { AppConfig, CostTier, Endpoint, ModelConfig, OpenRouterSyncConfig, RouteConfig, RouteLatencyConfig, RoutingConfig } from "./types.js";

const COST_TIERS: CostTier[] = ["free", "standard", "premium"];
const ENDPOINTS: Endpoint[] = ["chat", "embeddings", "audio_speech", "audio_transcriptions"];

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

  const models = value.models;
  if (!isRecord(models)) throw new Error("Config models must be an object");

  const routes = value.routes;
  if (routes !== undefined && !isRecord(routes)) throw new Error("Config routes must be an object");

  const appConfig: AppConfig = {
    server: {
      host: stringValue(server.host, "server.host"),
      port: numberValue(server.port, "server.port"),
      request_timeout_ms: numberValue(server.request_timeout_ms, "server.request_timeout_ms"),
      fallback_max_attempts: server.fallback_max_attempts === undefined ? 2 : numberValue(server.fallback_max_attempts, "server.fallback_max_attempts"),
    },
    models: {},
    routes: builtinRoutes(validateRouting(value.routing)),
    routing: validateRouting(value.routing),
    openrouter_sync: validateOpenRouterSync(value.openrouter_sync),
  };

  for (const [name, rawModel] of Object.entries(models)) {
    appConfig.models[name] = validateModel(rawModel, `models.${name}`);
  }

  if (isRecord(routes)) {
    for (const [name, rawRoute] of Object.entries(routes)) {
      appConfig.routes[name] = validateRoute(rawRoute, `routes.${name}`);
    }
  }

  return appConfig;
}

function validateModel(value: unknown, path: string): ModelConfig {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);

  const endpoint = stringValue(value.endpoint, `${path}.endpoint`) as Endpoint;
  if (!ENDPOINTS.includes(endpoint)) throw new Error(`${path}.endpoint is invalid`);

  const costTier = stringValue(value.cost_tier, `${path}.cost_tier`) as CostTier;
  if (!COST_TIERS.includes(costTier)) throw new Error(`${path}.cost_tier is invalid`);

  const capabilities = isRecord(value.capabilities) ? value.capabilities : {};

  return {
    provider: stringValue(value.provider, `${path}.provider`),
    upstream_model: stringValue(value.upstream_model, `${path}.upstream_model`),
    base_url: stringValue(value.base_url, `${path}.base_url`).replace(/\/+$/, ""),
    api_key_env: stringValue(value.api_key_env, `${path}.api_key_env`),
    endpoint,
    capabilities: capabilities as ModelConfig["capabilities"],
    cost_tier: costTier,
    price_rank: numberValue(value.price_rank, `${path}.price_rank`),
    context_window: value.context_window === undefined ? undefined : numberValue(value.context_window, `${path}.context_window`),
    max_concurrency: value.max_concurrency === undefined ? undefined : numberValue(value.max_concurrency, `${path}.max_concurrency`),
    supported_parameters: value.supported_parameters === undefined ? undefined : stringArray(value.supported_parameters, `${path}.supported_parameters`),
  };
}

function validateRoute(value: unknown, path: string): RouteConfig {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);

  const endpoint = stringValue(value.endpoint, `${path}.endpoint`) as Endpoint;
  if (!ENDPOINTS.includes(endpoint)) throw new Error(`${path}.endpoint is invalid`);

  const maxCostTier = value.max_cost_tier === undefined ? undefined : (stringValue(value.max_cost_tier, `${path}.max_cost_tier`) as CostTier);
  if (maxCostTier && !COST_TIERS.includes(maxCostTier)) throw new Error(`${path}.max_cost_tier is invalid`);

  return {
    endpoint,
    required_capabilities: value.required_capabilities === undefined ? undefined : numberRecord(value.required_capabilities, `${path}.required_capabilities`),
    require_private: value.require_private === undefined ? undefined : booleanValue(value.require_private, `${path}.require_private`),
    allow_external: value.allow_external === undefined ? undefined : booleanValue(value.allow_external, `${path}.allow_external`),
    max_cost_tier: maxCostTier,
    latency: value.latency === undefined ? undefined : latencyConfig(value.latency, `${path}.latency`),
    optimize: "cheapest",
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

function numberRecord(value: unknown, path: string): Record<string, number> {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);
  const result: Record<string, number> = {};
  for (const [key, raw] of Object.entries(value)) {
    result[key] = numberValue(raw, `${path}.${key}`);
  }
  return result;
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
  const route = (endpoint: Endpoint, capability: string, privateOnly: boolean): RouteConfig => ({
    endpoint,
    required_capabilities: { [capability]: 1 },
    require_private: privateOnly || undefined,
    allow_external: privateOnly ? false : true,
    latency,
    optimize: "cheapest",
  });

  const routes: Record<string, RouteConfig> = {
    "private/chat": route("chat", "general", true),
    "private/coding": route("chat", "coding", true),
    "private/reasoning": route("chat", "reasoning", true),
    "private/embedding": route("embeddings", "embedding", true),
    "private/tts": route("audio_speech", "tts", true),
    "private/asr": route("audio_transcriptions", "asr", true),

    "auto/chat": route("chat", "general", false),
    "auto/coding": route("chat", "coding", false),
    "auto/reasoning": route("chat", "reasoning", false),
    "auto/embedding": route("embeddings", "embedding", false),
    "auto/tts": route("audio_speech", "tts", false),
    "auto/asr": route("audio_transcriptions", "asr", false),
  };

  routes["auto/private"] = routes["private/chat"]!;
  routes["auto/default"] = routes["auto/chat"]!;
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

function stringArray(value: unknown, path: string): string[] {
  if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
  return value.map((item, index) => stringValue(item, `${path}.${index}`));
}
