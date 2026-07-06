use std::fs;

use tailgate::config::load_config_from_path;
use tailgate::health::{HealthRegistry, HealthUpdate};
use tailgate::selector::resolve_auto_candidates;
use tailgate::types::{
    AppConfig, CostTierConfig, DeepSeekSyncConfig, Endpoint, ModelConfig, OpenRouterSyncConfig,
    RouteConfig, RoutingConfig, ServerConfig,
};

#[test]
fn simple_config_expands_local_audio_translation_and_voice_models() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.yaml");
    fs::write(
        &path,
        r#"
server:
  host: 127.0.0.1
  port: 11435

local:
  base_url: http://qwen-local.test/v1
  api_key_env: LOCAL_API_KEY
  max_concurrency: 1
"#,
    )
    .unwrap();

    let config = load_config_from_path(&path).unwrap();

    assert_eq!(config.models["local/asr"].upstream_model, "local-asr");
    assert_eq!(
        config.models["local/asr"].endpoint,
        Endpoint::AudioTranscriptions
    );
    assert_eq!(
        config.models["local/translation"].upstream_model,
        "local-translation"
    );
    assert_eq!(
        config.models["local/translation"].endpoint,
        Endpoint::Translations
    );
    assert_eq!(
        config.models["local/tts-quality"].upstream_model,
        "local-tts-quality"
    );
    assert_eq!(config.models["local/tts-quality"].max_concurrency, Some(1));
    assert_eq!(
        config.models["local/tts-voice-design"].upstream_model,
        "local-tts-voice-design"
    );
    assert_eq!(
        config.models["local/tts-voice-design"].endpoint,
        Endpoint::AudioSpeech
    );
}

#[test]
fn auto_selector_uses_price_rank_then_config_order() {
    let config = selector_config();
    let health = HealthRegistry::new(&config);
    for model in config.models.keys() {
        health.finish_request(
            model,
            HealthUpdate {
                ok: true,
                total_latency_ms: 100,
                first_token_latency_ms: None,
                error: None,
            },
        );
        health.update_probe(model, true, 100, None);
    }

    let standard = resolve_auto_candidates(
        &config,
        &health,
        "standard/chat",
        Endpoint::Chat,
        None,
        None,
    )
    .unwrap();
    assert_eq!(standard[0].name, "deepseek/chat");
    assert_eq!(standard[1].name, "openrouter/auto");

    let premium =
        resolve_auto_candidates(&config, &health, "premium/chat", Endpoint::Chat, None, None)
            .unwrap();
    assert_eq!(premium[0].name, "deepseek/premium");
    assert_eq!(premium[1].name, "openrouter/premium");
}

fn selector_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            host: "127.0.0.1".into(),
            port: 11435,
            request_timeout_ms: 60_000,
            fallback_max_attempts: 2,
            cors_allowed_origins: vec!["*".into()],
        },
        models: [
            (
                "deepseek/chat",
                ModelConfig {
                    provider: "deepseek".into(),
                    upstream_model: "deepseek-v4-flash".into(),
                    base_url: "https://api.deepseek.com/v1".into(),
                    api_key_env: "DEEPSEEK_API_KEY".into(),
                    endpoint: Endpoint::Chat,
                    cost_tier: Some("standard".into()),
                    price_rank: Some(30),
                    context_window: None,
                    max_concurrency: None,
                    supported_parameters: None,
                },
            ),
            (
                "openrouter/auto",
                ModelConfig {
                    provider: "openrouter".into(),
                    upstream_model: "openrouter/auto".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    api_key_env: "OPENROUTER_API_KEY".into(),
                    endpoint: Endpoint::Chat,
                    cost_tier: Some("standard".into()),
                    price_rank: Some(30),
                    context_window: None,
                    max_concurrency: None,
                    supported_parameters: None,
                },
            ),
            (
                "deepseek/premium",
                ModelConfig {
                    provider: "deepseek".into(),
                    upstream_model: "deepseek-v4-pro".into(),
                    base_url: "https://api.deepseek.com/v1".into(),
                    api_key_env: "DEEPSEEK_API_KEY".into(),
                    endpoint: Endpoint::Chat,
                    cost_tier: Some("premium".into()),
                    price_rank: Some(10_000),
                    context_window: None,
                    max_concurrency: None,
                    supported_parameters: None,
                },
            ),
            (
                "openrouter/premium",
                ModelConfig {
                    provider: "openrouter".into(),
                    upstream_model: "moonshotai/kimi-k2.6".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    api_key_env: "OPENROUTER_API_KEY".into(),
                    endpoint: Endpoint::Chat,
                    cost_tier: Some("premium".into()),
                    price_rank: Some(10_000),
                    context_window: None,
                    max_concurrency: None,
                    supported_parameters: None,
                },
            ),
        ]
        .into_iter()
        .map(|(name, model)| (name.to_string(), model))
        .collect(),
        model_order: [
            ("deepseek/chat", 0),
            ("openrouter/auto", 1),
            ("deepseek/premium", 2),
            ("openrouter/premium", 3),
        ]
        .into_iter()
        .map(|(name, order)| (name.to_string(), order))
        .collect(),
        routes: [
            (
                "standard/chat",
                RouteConfig {
                    endpoint: Endpoint::Chat,
                    require_private: None,
                    cost_tier: Some("standard".into()),
                    latency: None,
                },
            ),
            (
                "premium/chat",
                RouteConfig {
                    endpoint: Endpoint::Chat,
                    require_private: None,
                    cost_tier: Some("premium".into()),
                    latency: None,
                },
            ),
        ]
        .into_iter()
        .map(|(name, route)| (name.to_string(), route))
        .collect(),
        routing: RoutingConfig { latency: None },
        openrouter_sync: OpenRouterSyncConfig {
            enabled: false,
            interval_seconds: 21_600,
            update_config_file: false,
            source_url: "https://openrouter.ai/api/v1/models".into(),
            include_unconfigured_models: false,
            allowlist: vec![],
            cost_tiers: CostTierConfig::default(),
        },
        deepseek_sync: DeepSeekSyncConfig {
            enabled: false,
            interval_seconds: 21_600,
            source_url: "https://api-docs.deepseek.com/quick_start/pricing/".into(),
            cost_tiers: CostTierConfig::default(),
        },
    }
}
