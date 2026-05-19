import { GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { AppConfig, CostTier, Endpoint, OpenAIJsonBody, RouteConfig, SelectedModel } from "./types.js";

const COST_TIER_ORDER: Record<CostTier, number> = {
  free: 0,
  standard: 1,
  premium: 2,
};

export function resolveModel(config: AppConfig, health: HealthRegistry, requestedModel: string): SelectedModel {
  const directModel = config.models[requestedModel];
  if (directModel) {
    return { name: requestedModel, config: directModel };
  }

  const route = config.routes[requestedModel];
  if (!route) {
    throw new GatewayError(`Unknown model or route: ${requestedModel}`, 404, "model_not_found");
  }

  return selectAutoModels(config, health, requestedModel, route)[0]!;
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
): SelectedModel[] {
  const route = config.routes[requestedModel];
  if (!route) {
    throw new GatewayError(`Unknown model or route: ${requestedModel}`, 404, "model_not_found");
  }
  if (route.endpoint !== endpoint) {
    throw new GatewayError(`Route ${requestedModel} does not support endpoint ${endpoint}`, 400, "endpoint_mismatch");
  }
  return selectAutoModels(config, health, requestedModel, route, estimateRequestTokens(body));
}

function selectAutoModels(config: AppConfig, health: HealthRegistry, routeName: string, route: RouteConfig, estimatedTokens?: number): SelectedModel[] {
  const candidates = Object.entries(config.models)
    .filter(([, model]) => model.endpoint === route.endpoint)
    .filter(([, model]) => capabilitiesMatch(model.capabilities, route.required_capabilities))
    .filter(([, model]) => !route.require_private || model.capabilities.private === true)
    .filter(([, model]) => route.allow_external !== false || model.capabilities.private === true)
    .filter(([, model]) => !route.max_cost_tier || COST_TIER_ORDER[model.cost_tier] <= COST_TIER_ORDER[route.max_cost_tier])
    .filter(([, model]) => !estimatedTokens || !model.context_window || estimatedTokens <= model.context_window)
    .filter(([modelName]) => {
      const state = health.get(modelName);
      return state.healthy && !state.busy && latencyMatches(state.network_latency_ms, route.latency?.network_p95_ms_max) && latencyMatches(state.first_token_latency_ms, route.latency?.first_token_p95_ms_max);
    })
    .sort((a, b) => {
      const priceDelta = a[1].price_rank - b[1].price_rank;
      if (priceDelta !== 0) return priceDelta;
      return (health.get(a[0]).network_latency_ms ?? Number.MAX_SAFE_INTEGER) - (health.get(b[0]).network_latency_ms ?? Number.MAX_SAFE_INTEGER);
    })
    .map(([name, model]) => ({ name, config: model }));

  if (candidates.length === 0) {
    throw new GatewayError(`No healthy model matches route: ${routeName}`, 503, "no_healthy_model");
  }

  return candidates;
}

function capabilitiesMatch(modelCapabilities: Record<string, unknown>, required?: Record<string, number>): boolean {
  if (!required) return true;

  for (const [capability, minimum] of Object.entries(required)) {
    const actual = modelCapabilities[capability];
    if (typeof actual !== "number" || actual < minimum) return false;
  }

  return true;
}

function latencyMatches(actual: number | undefined, max: number | undefined): boolean {
  return max === undefined || actual === undefined || actual <= max;
}

function estimateRequestTokens(body?: OpenAIJsonBody): number | undefined {
  if (!body) return undefined;
  const text = JSON.stringify(body);
  return Math.ceil(text.length / 4);
}
