import fs from "node:fs";
import YAML from "yaml";
import { AppConfig, CostTier, Endpoint, ModelConfig, RouteConfig } from "./types.js";

const COST_TIERS: CostTier[] = ["free", "standard", "premium"];

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
  if (!isRecord(routes)) throw new Error("Config routes must be an object");

  const appConfig: AppConfig = {
    server: {
      host: stringValue(server.host, "server.host"),
      port: numberValue(server.port, "server.port"),
      request_timeout_ms: numberValue(server.request_timeout_ms, "server.request_timeout_ms"),
    },
    models: {},
    routes: {},
  };

  for (const [name, rawModel] of Object.entries(models)) {
    appConfig.models[name] = validateModel(rawModel, `models.${name}`);
  }

  for (const [name, rawRoute] of Object.entries(routes)) {
    appConfig.routes[name] = validateRoute(rawRoute, `routes.${name}`);
  }

  return appConfig;
}

function validateModel(value: unknown, path: string): ModelConfig {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);

  const endpoint = stringValue(value.endpoint, `${path}.endpoint`) as Endpoint;
  if (endpoint !== "chat") throw new Error(`${path}.endpoint must be chat`);

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
    context_window: numberValue(value.context_window, `${path}.context_window`),
    max_concurrency: value.max_concurrency === undefined ? undefined : numberValue(value.max_concurrency, `${path}.max_concurrency`),
  };
}

function validateRoute(value: unknown, path: string): RouteConfig {
  if (!isRecord(value)) throw new Error(`${path} must be an object`);

  const endpoint = stringValue(value.endpoint, `${path}.endpoint`) as Endpoint;
  if (endpoint !== "chat") throw new Error(`${path}.endpoint must be chat`);

  const maxCostTier = value.max_cost_tier === undefined ? undefined : (stringValue(value.max_cost_tier, `${path}.max_cost_tier`) as CostTier);
  if (maxCostTier && !COST_TIERS.includes(maxCostTier)) throw new Error(`${path}.max_cost_tier is invalid`);

  return {
    endpoint,
    required_capabilities: value.required_capabilities === undefined ? undefined : numberRecord(value.required_capabilities, `${path}.required_capabilities`),
    require_private: value.require_private === undefined ? undefined : booleanValue(value.require_private, `${path}.require_private`),
    allow_external: value.allow_external === undefined ? undefined : booleanValue(value.allow_external, `${path}.allow_external`),
    max_cost_tier: maxCostTier,
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
