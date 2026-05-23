import { GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { AppConfig, CostTier, Endpoint, ModelCatalog, ModelConfig, OpenAIJsonBody, RouteConfig, SelectedModel } from "./types.js";

export function resolveModel(config: AppConfig, health: HealthRegistry, requestedModel: string, catalog?: ModelCatalog): SelectedModel {
  const directModel = catalog?.getModel(requestedModel) ?? (config.models[requestedModel] ? { name: requestedModel, config: config.models[requestedModel] } : undefined);
  if (directModel) {
    return directModel;
  }

  const route = config.routes[requestedModel];
  if (!route) {
    throw new GatewayError(`Unknown model or route: ${requestedModel}`, 404, "model_not_found");
  }

  return selectAutoModels(config, health, requestedModel, route, undefined, catalog)[0]!;
}

export function isAutoRoute(config: AppConfig, requestedModel: string): boolean {
  return config.routes[requestedModel] !== undefined;
}

export function resolveAutoCandidates(
  config: AppConfig,
  health: HealthRegistry,
  requestedModel: string,
  endpoint: Endpoint,
  body?: OpenAIJsonBody,
  catalog?: ModelCatalog,
): SelectedModel[] {
  const route = config.routes[requestedModel];
  if (!route) {
    throw new GatewayError(`Unknown model or route: ${requestedModel}`, 404, "model_not_found");
  }
  if (route.endpoint !== endpoint) {
    throw new GatewayError(`Route ${requestedModel} does not support endpoint ${endpoint}`, 400, "endpoint_mismatch");
  }
  return selectAutoModels(config, health, requestedModel, route, estimateRequestTokens(body), catalog);
}

function selectAutoModels(config: AppConfig, health: HealthRegistry, routeName: string, route: RouteConfig, estimatedTokens?: number, catalog?: ModelCatalog): SelectedModel[] {
  const entries = catalog?.getModelEntries() ?? Object.entries(config.models).map(([name, model]) => [name, model, undefined] as const);
  const candidates = entries
    .filter(([, model]) => model.endpoint === route.endpoint)
    .filter(([, model]) => !route.require_private || model.provider === "local")
    .filter(([modelName, model]) => !route.cost_tier || modelTier(modelName, model) === route.cost_tier)
    .filter(([, model]) => !estimatedTokens || !model.context_window || estimatedTokens <= model.context_window)
    .filter(([modelName]) => {
      const state = health.get(modelName);
      return state.healthy && !state.busy && latencyMatches(state.network_latency_ms, route.latency?.network_p95_ms_max) && latencyMatches(state.first_token_latency_ms, route.latency?.first_token_p95_ms_max);
    })
    .sort((a, b) => {
      const priceDelta = modelRank(a[1]) - modelRank(b[1]);
      if (priceDelta !== 0) return priceDelta;
      const orderDelta = modelOrder(config, a[0]) - modelOrder(config, b[0]);
      if (orderDelta !== 0) return orderDelta;
      return (health.get(a[0]).network_latency_ms ?? Number.MAX_SAFE_INTEGER) - (health.get(b[0]).network_latency_ms ?? Number.MAX_SAFE_INTEGER);
    })
    .map(([name, model, metadata]) => ({ name, config: model, metadata }));

  if (candidates.length === 0) {
    throw new GatewayError(`No healthy model matches route: ${routeName}`, 503, "no_healthy_model");
  }

  return candidates;
}

function latencyMatches(actual: number | undefined, max: number | undefined): boolean {
  return max === undefined || actual === undefined || actual <= max;
}

function modelRank(model: { provider: string; price_rank?: number }): number {
  if (model.price_rank !== undefined) return model.price_rank;
  if (model.provider === "local") return 0;
  if (model.provider === "deepseek") return 10;
  if (model.provider === "openrouter") return 20;
  return 100;
}

function modelOrder(config: AppConfig, modelName: string): number {
  return config.model_order[modelName] ?? Number.MAX_SAFE_INTEGER;
}

function modelTier(modelName: string, model: ModelConfig): CostTier {
  if (model.cost_tier) return model.cost_tier;
  if (model.provider === "local") return "free";
  if (model.provider === "deepseek") return "standard";
  if (modelName.endsWith("/premium")) return "premium";
  return "standard";
}

function estimateRequestTokens(body?: OpenAIJsonBody): number | undefined {
  if (!body) return undefined;
  const text = JSON.stringify(body);
  return Math.ceil(text.length / 4);
}
