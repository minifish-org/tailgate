use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    Chat,
    Embeddings,
    AudioSpeech,
    AudioTranscriptions,
    Translations,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub request_timeout_ms: u64,
    pub fallback_max_attempts: usize,
    pub cors_allowed_origins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CostTierConfig {
    pub free_max_usd_per_1m_tokens: f64,
    pub standard_max_usd_per_1m_tokens: f64,
    pub premium_max_usd_per_1m_tokens: f64,
}

impl Default for CostTierConfig {
    fn default() -> Self {
        Self {
            free_max_usd_per_1m_tokens: 0.0,
            standard_max_usd_per_1m_tokens: 2.0,
            premium_max_usd_per_1m_tokens: 9999.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenRouterSyncConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub update_config_file: bool,
    pub source_url: String,
    pub include_unconfigured_models: bool,
    pub allowlist: Vec<String>,
    pub cost_tiers: CostTierConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeepSeekSyncConfig {
    pub enabled: bool,
    pub interval_seconds: u64,
    pub source_url: String,
    pub cost_tiers: CostTierConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelConfig {
    pub provider: String,
    pub upstream_model: String,
    pub base_url: String,
    pub api_key_env: String,
    pub endpoint: Endpoint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_parameters: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RouteLatencyConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_p95_ms_max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_p95_ms_max: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RoutingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<RouteLatencyConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RouteConfig {
    pub endpoint: Endpoint,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency: Option<RouteLatencyConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub models: HashMap<String, ModelConfig>,
    pub model_order: HashMap<String, usize>,
    pub routes: HashMap<String, RouteConfig>,
    pub routing: RoutingConfig,
    pub openrouter_sync: OpenRouterSyncConfig,
    pub deepseek_sync: DeepSeekSyncConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RuntimeModelMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_price_prompt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_price_completion: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_price_request: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_price_image: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_cost_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_price_rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_parameters: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter_model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter_created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_price_sync_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HealthState {
    pub healthy: bool,
    pub busy: bool,
    pub in_flight: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_latency_ms: Option<u64>,
    pub error_rate: f64,
    pub success_count: u64,
    pub error_count: u64,
}

impl Default for HealthState {
    fn default() -> Self {
        Self {
            healthy: false,
            busy: false,
            in_flight: 0,
            last_error: None,
            last_checked_at: None,
            network_latency_ms: None,
            first_token_latency_ms: None,
            total_latency_ms: None,
            error_rate: 0.0,
            success_count: 0,
            error_count: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SelectedModel {
    pub name: String,
    pub config: ModelConfig,
    pub metadata: Option<RuntimeModelMetadata>,
}
