import { GatewayError } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { AppConfig, CostTier, RouteConfig, SelectedModel } from "./types.js";

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

  return selectAutoModel(config, health, requestedModel, route);
}

function selectAutoModel(config: AppConfig, health: HealthRegistry, routeName: string, route: RouteConfig): SelectedModel {
  const candidates = Object.entries(config.models)
    .filter(([, model]) => model.endpoint === route.endpoint)
    .filter(([, model]) => capabilitiesMatch(model.capabilities, route.required_capabilities))
    .filter(([, model]) => !route.require_private || model.capabilities.private === true)
    .filter(([, model]) => route.allow_external !== false || model.capabilities.private === true)
    .filter(([, model]) => !route.max_cost_tier || COST_TIER_ORDER[model.cost_tier] <= COST_TIER_ORDER[route.max_cost_tier])
    .filter(([modelName]) => health.get(modelName).healthy)
    .sort((a, b) => a[1].price_rank - b[1].price_rank);

  const selected = candidates[0];
  if (!selected) {
    throw new GatewayError(`No healthy model matches route: ${routeName}`, 503, "no_healthy_model");
  }

  return { name: selected[0], config: selected[1] };
}

function capabilitiesMatch(modelCapabilities: Record<string, unknown>, required?: Record<string, number>): boolean {
  if (!required) return true;

  for (const [capability, minimum] of Object.entries(required)) {
    const actual = modelCapabilities[capability];
    if (typeof actual !== "number" || actual < minimum) return false;
  }

  return true;
}
