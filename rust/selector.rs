use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::{
    health::HealthRegistry,
    runtime_catalog::RuntimeCatalog,
    types::{AppConfig, Endpoint, ModelConfig, RouteConfig, SelectedModel},
};

pub fn resolve_model(
    config: &AppConfig,
    health: &HealthRegistry,
    requested_model: &str,
    catalog: Option<&RuntimeCatalog>,
) -> Result<SelectedModel> {
    if let Some(model) = catalog.and_then(|catalog| catalog.get_model(requested_model)) {
        return Ok(model);
    }
    if let Some(model) = config.models.get(requested_model) {
        return Ok(SelectedModel {
            name: requested_model.to_string(),
            config: model.clone(),
            metadata: None,
        });
    }
    let route = config
        .routes
        .get(requested_model)
        .ok_or_else(|| anyhow!("Unknown model or route: {requested_model}"))?;
    select_auto_models(config, health, requested_model, route, None, catalog)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No healthy model matches route: {requested_model}"))
}

pub fn is_auto_route(config: &AppConfig, requested_model: &str) -> bool {
    config.routes.contains_key(requested_model)
}

pub fn resolve_auto_candidates(
    config: &AppConfig,
    health: &HealthRegistry,
    requested_model: &str,
    endpoint: Endpoint,
    body: Option<&Value>,
    catalog: Option<&RuntimeCatalog>,
) -> Result<Vec<SelectedModel>> {
    let route = config
        .routes
        .get(requested_model)
        .ok_or_else(|| anyhow!("Unknown model or route: {requested_model}"))?;
    if route.endpoint != endpoint {
        return Err(anyhow!(
            "Route {requested_model} does not support endpoint {:?}",
            endpoint
        ));
    }
    let mut candidates = select_auto_models(
        config,
        health,
        requested_model,
        route,
        estimate_request_tokens(body),
        catalog,
    )?;
    candidates.truncate(config.server.fallback_max_attempts.max(1));
    Ok(candidates)
}

fn select_auto_models(
    config: &AppConfig,
    health: &HealthRegistry,
    route_name: &str,
    route: &RouteConfig,
    estimated_tokens: Option<u64>,
    catalog: Option<&RuntimeCatalog>,
) -> Result<Vec<SelectedModel>> {
    let entries = if let Some(catalog) = catalog {
        catalog.get_model_entries()
    } else {
        config
            .models
            .iter()
            .map(|(name, model)| (name.clone(), model.clone(), None))
            .collect()
    };
    let mut candidates: Vec<_> = entries
        .into_iter()
        .filter(|(_, model, _)| model.endpoint == route.endpoint)
        .filter(|(_, model, _)| {
            !route.require_private.unwrap_or(false) || model.provider == "local"
        })
        .filter(|(name, model, _)| {
            route
                .cost_tier
                .as_ref()
                .is_none_or(|tier| model_tier(name, model) == *tier)
        })
        .filter(|(_, model, _)| {
            estimated_tokens
                .is_none_or(|tokens| model.context_window.is_none_or(|window| tokens <= window))
        })
        .filter(|(name, _, _)| {
            let state = health.get(name);
            state.healthy
                && !state.busy
                && latency_matches(
                    state.network_latency_ms,
                    route
                        .latency
                        .as_ref()
                        .and_then(|latency| latency.network_p95_ms_max),
                )
                && latency_matches(
                    state.first_token_latency_ms,
                    route
                        .latency
                        .as_ref()
                        .and_then(|latency| latency.first_token_p95_ms_max),
                )
        })
        .map(|(name, config, metadata)| SelectedModel {
            name,
            config,
            metadata,
        })
        .collect();

    candidates.sort_by(|a, b| {
        model_rank(&a.config)
            .cmp(&model_rank(&b.config))
            .then_with(|| model_order(config, &a.name).cmp(&model_order(config, &b.name)))
            .then_with(|| {
                health
                    .get(&a.name)
                    .network_latency_ms
                    .unwrap_or(u64::MAX)
                    .cmp(&health.get(&b.name).network_latency_ms.unwrap_or(u64::MAX))
            })
    });

    if candidates.is_empty() {
        return Err(anyhow!("No healthy model matches route: {route_name}"));
    }
    Ok(candidates)
}

fn latency_matches(actual: Option<u64>, max: Option<u64>) -> bool {
    max.is_none_or(|max| actual.is_none_or(|actual| actual <= max))
}

fn model_rank(model: &ModelConfig) -> i64 {
    if let Some(rank) = model.price_rank {
        return rank;
    }
    match model.provider.as_str() {
        "local" => 0,
        "deepseek" => 10,
        "openrouter" => 20,
        _ => 100,
    }
}

fn model_order(config: &AppConfig, model_name: &str) -> usize {
    config
        .model_order
        .get(model_name)
        .copied()
        .unwrap_or(usize::MAX)
}

fn model_tier(model_name: &str, model: &ModelConfig) -> String {
    if let Some(tier) = &model.cost_tier {
        return tier.clone();
    }
    if model.provider == "local" {
        return "free".to_string();
    }
    if model.provider == "deepseek" {
        return "standard".to_string();
    }
    if model_name.ends_with("/premium") {
        return "premium".to_string();
    }
    "standard".to_string()
}

fn estimate_request_tokens(body: Option<&Value>) -> Option<u64> {
    let body = body?;
    let text = serde_json::to_string(body).ok()?;
    Some((text.len() as u64).div_ceil(4))
}
