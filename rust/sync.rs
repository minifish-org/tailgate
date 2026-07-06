use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use regex::Regex;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use tracing::{info, warn};

use crate::pricing::{parse_openrouter_pricing, rank_per_1m_pricing};
use crate::runtime_catalog::RuntimeCatalog;
use crate::types::{AppConfig, ModelConfig, RuntimeModelMetadata};

#[derive(Clone)]
pub struct SyncRegistry {
    openrouter: Arc<RwLock<OpenRouterSyncStatus>>,
    deepseek: Arc<RwLock<DeepSeekSyncStatus>>,
}

impl SyncRegistry {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            openrouter: Arc::new(RwLock::new(OpenRouterSyncStatus {
                enabled: config.openrouter_sync.enabled,
                ..OpenRouterSyncStatus::default()
            })),
            deepseek: Arc::new(RwLock::new(DeepSeekSyncStatus {
                enabled: config.deepseek_sync.enabled,
                ..DeepSeekSyncStatus::default()
            })),
        }
    }

    pub fn openrouter_status(&self) -> OpenRouterSyncStatus {
        self.openrouter
            .read()
            .expect("openrouter sync lock poisoned")
            .clone()
    }

    pub fn deepseek_status(&self) -> DeepSeekSyncStatus {
        self.deepseek
            .read()
            .expect("deepseek sync lock poisoned")
            .clone()
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct OpenRouterSyncStatus {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_configured: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_added_virtual: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DeepSeekSyncStatus {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fetched_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated_configured: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OpenRouterSyncSummary {
    pub ok: bool,
    pub fetched: usize,
    pub updated_configured: usize,
    pub added_virtual: usize,
    pub warnings: Vec<String>,
    pub duration_ms: u128,
    pub synced_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeepSeekSyncSummary {
    pub ok: bool,
    pub fetched: usize,
    pub updated_configured: usize,
    pub warnings: Vec<String>,
    pub duration_ms: u128,
    pub synced_at: String,
}

#[derive(Clone, Debug)]
struct OpenRouterModel {
    id: String,
    name: Option<String>,
    context_length: Option<u64>,
    pricing: Option<Value>,
    supported_parameters: Option<Vec<String>>,
    created: Option<i64>,
}

#[derive(Clone, Debug)]
struct DeepSeekModelPricing {
    id: String,
    context_window: Option<u64>,
    prompt_per_1m: Option<f64>,
    completion_per_1m: Option<f64>,
    display_name: Option<String>,
}

pub async fn sync_openrouter(
    config: &AppConfig,
    catalog: &RuntimeCatalog,
    registry: &SyncRegistry,
    client: &Client,
) -> Result<OpenRouterSyncSummary> {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| anyhow!("OPENROUTER_API_KEY is not configured"))?;
    let started = std::time::Instant::now();
    let synced_at = now_stamp();
    let mut warnings = Vec::new();
    if config.openrouter_sync.update_config_file {
        warnings.push(
            "openrouter_sync.update_config_file is not implemented; using runtime metadata only"
                .to_string(),
        );
        warn!("openrouter_sync.update_config_file is not implemented; using runtime metadata only");
    }

    let result = async {
        let response = client
            .get(&config.openrouter_sync.source_url)
            .bearer_auth(api_key)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "OpenRouter models sync returned {}",
                response.status().as_u16()
            ));
        }
        let payload: Value = response.json().await?;
        let models = parse_openrouter_models(&payload);
        let by_id: HashMap<_, _> = models
            .iter()
            .map(|model| (model.id.clone(), model.clone()))
            .collect();
        let mut updated_configured = 0;
        let mut added_virtual = 0;

        for (tailgate_model_id, model) in &config.models {
            if model.provider != "openrouter" {
                continue;
            }
            let Some(openrouter_model) = by_id.get(&model.upstream_model) else {
                warnings.push(format!(
                    "configured OpenRouter model not found: {tailgate_model_id} -> {}",
                    model.upstream_model
                ));
                continue;
            };
            catalog.set_overlay(
                tailgate_model_id,
                metadata_from_openrouter_model(openrouter_model, config, &synced_at),
            );
            updated_configured += 1;
        }

        let configured = configured_upstream_ids(config);
        let allowed_ids: Vec<String> = if config.openrouter_sync.include_unconfigured_models {
            models.iter().map(|model| model.id.clone()).collect()
        } else {
            config.openrouter_sync.allowlist.clone()
        };
        for openrouter_id in allowed_ids {
            if configured.contains(&openrouter_id) {
                continue;
            }
            let Some(openrouter_model) = by_id.get(&openrouter_id) else {
                warnings.push(format!(
                    "allowlist OpenRouter model not found: {openrouter_id}"
                ));
                continue;
            };
            let virtual_id = virtual_model_id(&openrouter_id);
            let metadata = metadata_from_openrouter_model(openrouter_model, config, &synced_at);
            catalog.set_virtual_model(
                &virtual_id,
                virtual_model_config(openrouter_model, &metadata),
                metadata,
            );
            added_virtual += 1;
        }

        Ok(OpenRouterSyncSummary {
            ok: true,
            fetched: models.len(),
            updated_configured,
            added_virtual,
            warnings,
            duration_ms: started.elapsed().as_millis(),
            synced_at,
        })
    }
    .await;

    match result {
        Ok(summary) => {
            *registry
                .openrouter
                .write()
                .expect("openrouter sync lock poisoned") = OpenRouterSyncStatus {
                enabled: config.openrouter_sync.enabled,
                last_success_at: Some(summary.synced_at.clone()),
                last_fetched_count: Some(summary.fetched),
                last_updated_configured: Some(summary.updated_configured),
                last_added_virtual: Some(summary.added_virtual),
                ..OpenRouterSyncStatus::default()
            };
            info!(
                fetched = summary.fetched,
                updated_configured = summary.updated_configured,
                added_virtual = summary.added_virtual,
                warnings_count = summary.warnings.len(),
                duration_ms = summary.duration_ms,
                "openrouter sync completed"
            );
            Ok(summary)
        }
        Err(error) => {
            let message = truncate(&error.to_string());
            let mut status = registry
                .openrouter
                .write()
                .expect("openrouter sync lock poisoned");
            status.enabled = config.openrouter_sync.enabled;
            status.last_error_at = Some(now_stamp());
            status.last_error = Some(message.clone());
            warn!(error = %message, duration_ms = started.elapsed().as_millis(), "openrouter sync failed");
            Err(error)
        }
    }
}

pub async fn sync_deepseek(
    config: &AppConfig,
    catalog: &RuntimeCatalog,
    registry: &SyncRegistry,
    client: &Client,
) -> Result<DeepSeekSyncSummary> {
    let started = std::time::Instant::now();
    let synced_at = now_stamp();
    let mut warnings = Vec::new();
    let result = async {
        let response = client.get(&config.deepseek_sync.source_url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "DeepSeek pricing sync returned {}",
                response.status().as_u16()
            ));
        }
        let html = response.text().await?;
        let models = parse_deepseek_pricing_models(&html);
        let mut by_id = HashMap::new();
        for model in &models {
            by_id.insert(model.id.clone(), model.clone());
            if model.id == "deepseek-v4-flash" {
                by_id.insert("deepseek-chat".to_string(), model.clone());
                by_id.insert("deepseek-reasoner".to_string(), model.clone());
            }
        }

        let mut updated_configured = 0;
        for (tailgate_model_id, model) in &config.models {
            if model.provider != "deepseek" {
                continue;
            }
            let Some(pricing) = by_id.get(&model.upstream_model) else {
                warnings.push(format!(
                    "configured DeepSeek model not found: {tailgate_model_id} -> {}",
                    model.upstream_model
                ));
                continue;
            };
            catalog.set_overlay(
                tailgate_model_id,
                metadata_from_deepseek_pricing(pricing, config, &synced_at),
            );
            updated_configured += 1;
        }

        Ok(DeepSeekSyncSummary {
            ok: true,
            fetched: models.len(),
            updated_configured,
            warnings,
            duration_ms: started.elapsed().as_millis(),
            synced_at,
        })
    }
    .await;

    match result {
        Ok(summary) => {
            *registry
                .deepseek
                .write()
                .expect("deepseek sync lock poisoned") = DeepSeekSyncStatus {
                enabled: config.deepseek_sync.enabled,
                last_success_at: Some(summary.synced_at.clone()),
                last_fetched_count: Some(summary.fetched),
                last_updated_configured: Some(summary.updated_configured),
                ..DeepSeekSyncStatus::default()
            };
            info!(
                fetched = summary.fetched,
                updated_configured = summary.updated_configured,
                warnings_count = summary.warnings.len(),
                duration_ms = summary.duration_ms,
                "deepseek sync completed"
            );
            Ok(summary)
        }
        Err(error) => {
            let message = truncate(&error.to_string());
            let mut status = registry
                .deepseek
                .write()
                .expect("deepseek sync lock poisoned");
            status.enabled = config.deepseek_sync.enabled;
            status.last_error_at = Some(now_stamp());
            status.last_error = Some(message.clone());
            warn!(error = %message, duration_ms = started.elapsed().as_millis(), "deepseek sync failed");
            Err(error)
        }
    }
}

pub fn parse_deepseek_pricing_page(html: &str) -> Vec<RuntimeModelMetadata> {
    parse_deepseek_pricing_models(html)
        .into_iter()
        .map(|model| RuntimeModelMetadata {
            context_window: model.context_window,
            dynamic_price_prompt: model.prompt_per_1m,
            dynamic_price_completion: model.completion_per_1m,
            provider_model_name: model.display_name,
            ..RuntimeModelMetadata::default()
        })
        .collect()
}

fn parse_openrouter_models(payload: &Value) -> Vec<OpenRouterModel> {
    payload
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let object = value.as_object()?;
            let id = object.get("id")?.as_str()?.to_string();
            Some(OpenRouterModel {
                id,
                name: object
                    .get("name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                context_length: object.get("context_length").and_then(Value::as_u64),
                pricing: object.get("pricing").cloned(),
                supported_parameters: object
                    .get("supported_parameters")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(ToOwned::to_owned)
                            .collect()
                    }),
                created: object.get("created").and_then(Value::as_i64),
            })
        })
        .collect()
}

fn metadata_from_openrouter_model(
    model: &OpenRouterModel,
    config: &AppConfig,
    synced_at: &str,
) -> RuntimeModelMetadata {
    let pricing =
        parse_openrouter_pricing(model.pricing.as_ref(), &config.openrouter_sync.cost_tiers);
    let tier_override = openrouter_tier_override(&model.id);
    RuntimeModelMetadata {
        context_window: model.context_length,
        dynamic_price_prompt: pricing.prompt_per_1m,
        dynamic_price_completion: pricing.completion_per_1m,
        dynamic_price_request: pricing.request,
        dynamic_price_image: pricing.image,
        dynamic_cost_tier: tier_override
            .as_ref()
            .map(|(tier, _)| tier.to_string())
            .or(Some(pricing.cost_tier)),
        dynamic_price_rank: tier_override
            .map(|(_, rank)| rank)
            .or(Some(pricing.price_rank)),
        supported_parameters: model.supported_parameters.clone(),
        provider_model_name: model.name.clone(),
        openrouter_model_name: model.name.clone(),
        openrouter_created: model.created,
        last_price_sync_at: Some(synced_at.to_string()),
    }
}

fn openrouter_tier_override(model_id: &str) -> Option<(&'static str, i64)> {
    match model_id {
        "openrouter/free" => Some(("free", 0)),
        "openrouter/auto" => Some(("standard", 30)),
        _ => None,
    }
}

fn virtual_model_config(model: &OpenRouterModel, metadata: &RuntimeModelMetadata) -> ModelConfig {
    ModelConfig {
        provider: "openrouter".to_string(),
        upstream_model: model.id.clone(),
        base_url: "https://openrouter.ai/api/v1".to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        endpoint: crate::types::Endpoint::Chat,
        cost_tier: metadata
            .dynamic_cost_tier
            .clone()
            .or_else(|| Some("standard".to_string())),
        price_rank: metadata.dynamic_price_rank.or(Some(999_999)),
        context_window: metadata.context_window,
        supported_parameters: metadata.supported_parameters.clone(),
        max_concurrency: None,
    }
}

fn configured_upstream_ids(config: &AppConfig) -> HashSet<String> {
    config
        .models
        .values()
        .filter(|model| model.provider == "openrouter")
        .map(|model| model.upstream_model.clone())
        .collect()
}

fn virtual_model_id(openrouter_id: &str) -> String {
    if openrouter_id.starts_with("openrouter/") {
        return openrouter_id.to_string();
    }
    let sanitized = Regex::new(r"[^a-zA-Z0-9]+")
        .expect("valid regex")
        .replace_all(openrouter_id, "-")
        .trim_matches('-')
        .to_string();
    format!("openrouter/{sanitized}")
}

fn parse_deepseek_pricing_models(html: &str) -> Vec<DeepSeekModelPricing> {
    let text = html_to_text(html);
    let model_re = Regex::new(r"deepseek-v4-(?:flash|pro)").expect("valid regex");
    let mut ids = Vec::<String>::new();
    for capture in model_re.find_iter(&text) {
        let id = capture.as_str().to_string();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let cache_miss_prices = prices_after_label(&text, "1M INPUT TOKENS (CACHE MISS)");
    let output_prices = prices_after_label(&text, "1M OUTPUT TOKENS");
    let context_window = parse_context_window(&text);
    ids.into_iter()
        .enumerate()
        .map(|(index, id)| DeepSeekModelPricing {
            display_name: Some(id.clone()),
            id,
            context_window,
            prompt_per_1m: cache_miss_prices.get(index).copied(),
            completion_per_1m: output_prices.get(index).copied(),
        })
        .collect()
}

fn metadata_from_deepseek_pricing(
    pricing: &DeepSeekModelPricing,
    config: &AppConfig,
    synced_at: &str,
) -> RuntimeModelMetadata {
    let ranked = rank_per_1m_pricing(
        pricing.prompt_per_1m,
        pricing.completion_per_1m,
        &config.deepseek_sync.cost_tiers,
    );
    let tier_override = deepseek_tier_override(&pricing.id, ranked.price_rank);
    RuntimeModelMetadata {
        context_window: pricing.context_window,
        dynamic_price_prompt: pricing.prompt_per_1m,
        dynamic_price_completion: pricing.completion_per_1m,
        dynamic_cost_tier: tier_override
            .as_ref()
            .map(|(tier, _)| tier.to_string())
            .or(Some(ranked.cost_tier)),
        dynamic_price_rank: tier_override
            .map(|(_, rank)| rank)
            .or(Some(ranked.price_rank)),
        provider_model_name: pricing.display_name.clone(),
        last_price_sync_at: Some(synced_at.to_string()),
        ..RuntimeModelMetadata::default()
    }
}

fn deepseek_tier_override(model_id: &str, price_rank: i64) -> Option<(&'static str, i64)> {
    if model_id == "deepseek-v4-pro" {
        Some(("premium", price_rank.max(10_000)))
    } else {
        None
    }
}

fn prices_after_label(text: &str, label: &str) -> Vec<f64> {
    let Some(start) = text.find(label) else {
        return Vec::new();
    };
    let rest = &text[start + label.len()..];
    let boundary = Regex::new(r"1M [A-Z ]+TOKENS|Deduction Rules|\(\d\)").expect("valid regex");
    let end = boundary
        .find(rest)
        .map(|m| m.start())
        .unwrap_or(300.min(rest.len()));
    let segment = &rest[..end.min(rest.len())];
    Regex::new(r"\$([0-9]+(?:\.[0-9]+)?)")
        .expect("valid regex")
        .captures_iter(segment)
        .filter_map(|capture| capture.get(1)?.as_str().parse().ok())
        .collect()
}

fn parse_context_window(text: &str) -> Option<u64> {
    let re = Regex::new(r"(?i)CONTEXT LENGTH\s+([0-9]+(?:\.[0-9]+)?)([MK])").ok()?;
    let capture = re.captures(text)?;
    let value: f64 = capture.get(1)?.as_str().parse().ok()?;
    let unit = capture.get(2)?.as_str().to_uppercase();
    Some(if unit == "M" {
        (value * 1_000_000.0) as u64
    } else {
        (value * 1_000.0) as u64
    })
}

fn html_to_text(html: &str) -> String {
    let mut text = html.to_string();
    for pattern in [
        r"(?is)<script[\s\S]*?</script>",
        r"(?is)<style[\s\S]*?</style>",
        r"(?is)<[^>]+>",
    ] {
        text = Regex::new(pattern)
            .expect("valid regex")
            .replace_all(&text, " ")
            .to_string();
    }
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

fn now_stamp() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", duration.as_secs(), duration.subsec_millis())
}
