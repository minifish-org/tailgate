use std::{collections::HashMap, fs, path::Path};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

use crate::types::{
    AppConfig, CostTierConfig, DeepSeekSyncConfig, Endpoint, ModelConfig, OpenRouterSyncConfig,
    RouteConfig, RouteLatencyConfig, RoutingConfig, ServerConfig,
};

const DEFAULT_CORS_ALLOWED_ORIGINS: [&str; 2] = ["http://127.0.0.1:5173", "http://localhost:5173"];

pub fn load_config() -> Result<AppConfig> {
    let path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "./config.yaml".to_string());
    load_config_from_path(path)
}

pub fn load_config_from_path(path: impl AsRef<Path>) -> Result<AppConfig> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .with_context(|| format!("Config file not found: {}", path.display()))?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text)?;
    let json = serde_json::to_value(yaml)?;
    load_config_from_value(json)
}

pub fn load_config_from_value(value: Value) -> Result<AppConfig> {
    let object = as_object(&value, "Config")?;
    if object.get("models").and_then(Value::as_object).is_none() {
        let mut expanded = expand_simple_config(object)?;
        if let Some(local) = object.get("local").and_then(Value::as_object) {
            let allowed = [
                "chat",
                "embedding",
                "tts",
                "tts-quality",
                "tts-voice-design",
                "asr",
                "translation",
            ];
            let capabilities = match local.get("enabled_capabilities") {
                None => None,
                Some(value) => {
                    let values = string_array(Some(value), "local.enabled_capabilities")?;
                    if values.iter().any(|item| !allowed.contains(&item.as_str())) {
                        return Err(anyhow!(
                            "local.enabled_capabilities contains an unknown capability"
                        ));
                    }
                    Some(values)
                }
            };
            let timeout =
                optional_number(local.get("request_timeout_ms"), "local.request_timeout_ms")?;
            if timeout == Some(0) {
                return Err(anyhow!("local.request_timeout_ms must be positive"));
            }
            let models = expanded
                .get_mut("models")
                .and_then(Value::as_object_mut)
                .unwrap();
            models.retain(
                |name, _| match (name.strip_prefix("local/"), &capabilities) {
                    (Some(capability), Some(enabled)) => {
                        enabled.iter().any(|item| item == capability)
                    }
                    _ => true,
                },
            );
            if let Some(timeout) = timeout {
                for (name, model) in models.iter_mut() {
                    if name.starts_with("local/") {
                        model
                            .as_object_mut()
                            .unwrap()
                            .insert("request_timeout_ms".into(), json!(timeout));
                    }
                }
            }
        }
        return validate_expanded_config(expanded);
    }
    validate_expanded_config(value)
}

fn validate_expanded_config(value: Value) -> Result<AppConfig> {
    let object = as_object(&value, "Config")?;
    let server_raw = object
        .get("server")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Config server must be an object"))?;
    let models_raw = object
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Config models must be an object"))?;
    let routing = validate_routing(object.get("routing"))?;

    let server = ServerConfig {
        host: string_value(server_raw.get("host"), "server.host")?,
        port: number_value(server_raw.get("port"), "server.port")? as u16,
        request_timeout_ms: number_or_default(
            server_raw.get("request_timeout_ms"),
            60_000,
            "server.request_timeout_ms",
        )?,
        fallback_max_attempts: number_or_default(
            server_raw.get("fallback_max_attempts"),
            2,
            "server.fallback_max_attempts",
        )? as usize,
        cors_allowed_origins: if server_raw.get("cors_allowed_origins").is_none() {
            DEFAULT_CORS_ALLOWED_ORIGINS
                .iter()
                .map(|origin| origin.to_string())
                .collect()
        } else {
            string_array(
                server_raw.get("cors_allowed_origins"),
                "server.cors_allowed_origins",
            )?
        },
    };

    let mut models = HashMap::new();
    let mut model_order = HashMap::new();
    for (index, (name, raw_model)) in models_raw.iter().enumerate() {
        models.insert(
            name.clone(),
            validate_model(raw_model, &format!("models.{name}"))?,
        );
        model_order.insert(name.clone(), index);
    }

    Ok(AppConfig {
        server,
        models,
        model_order,
        routes: builtin_routes(routing.clone()),
        routing,
        openrouter_sync: validate_openrouter_sync(object.get("openrouter_sync"))?,
        deepseek_sync: validate_deepseek_sync(object.get("deepseek_sync"))?,
    })
}

fn expand_simple_config(object: &Map<String, Value>) -> Result<Value> {
    let server = object
        .get("server")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let sync = object
        .get("sync")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let pricing = object
        .get("pricing")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let local = object
        .get("local")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let deepseek = object
        .get("deepseek")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let openrouter = object
        .get("openrouter")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let routing = object
        .get("routing")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let local_base_url = trim_url(&string_or_default(
        local.get("base_url"),
        "http://macbook-pro:8000/v1",
        "local.base_url",
    )?);
    let local_api_key_env = string_or_default(
        local.get("api_key_env"),
        "LOCAL_API_KEY",
        "local.api_key_env",
    )?;
    let local_max_concurrency =
        number_or_default(local.get("max_concurrency"), 1, "local.max_concurrency")?;
    let local_context_window =
        number_or_default(local.get("context_window"), 8192, "local.context_window")?;
    let local_tts_quality_model = string_or_default(
        local.get("tts_quality_model"),
        "local-tts-quality",
        "local.tts_quality_model",
    )?;
    let local_tts_voice_design_model = string_or_default(
        local.get("tts_voice_design_model"),
        "local-tts-voice-design",
        "local.tts_voice_design_model",
    )?;

    let deepseek_base_url = trim_url(&string_or_default(
        deepseek.get("base_url"),
        "https://api.deepseek.com/v1",
        "deepseek.base_url",
    )?);
    let deepseek_api_key_env = string_or_default(
        deepseek.get("api_key_env"),
        "DEEPSEEK_API_KEY",
        "deepseek.api_key_env",
    )?;
    let deepseek_model =
        string_or_default(deepseek.get("model"), "deepseek-v4-flash", "deepseek.model")?;
    let deepseek_premium_model = string_or_default(
        deepseek.get("premium_model"),
        "deepseek-v4-pro",
        "deepseek.premium_model",
    )?;

    let openrouter_base_url = trim_url(&string_or_default(
        openrouter.get("base_url"),
        "https://openrouter.ai/api/v1",
        "openrouter.base_url",
    )?);
    let openrouter_api_key_env = string_or_default(
        openrouter.get("api_key_env"),
        "OPENROUTER_API_KEY",
        "openrouter.api_key_env",
    )?;
    let openrouter_free_model = string_or_default(
        openrouter.get("free_model"),
        "openrouter/free",
        "openrouter.free_model",
    )?;
    let openrouter_auto_model = string_or_default(
        openrouter
            .get("standard_model")
            .or_else(|| openrouter.get("auto_model")),
        "openrouter/auto",
        "openrouter.standard_model",
    )?;
    let openrouter_premium_model = string_or_default(
        openrouter.get("premium_model"),
        "anthropic/claude-sonnet-4",
        "openrouter.premium_model",
    )?;
    let openrouter_extra_models = if let Some(value) = openrouter.get("extra_models") {
        string_array(Some(value), "openrouter.extra_models")?
    } else {
        Vec::new()
    };

    let sync_interval_seconds = number_or_default(
        sync.get("interval_seconds"),
        21_600,
        "sync.interval_seconds",
    )?;
    let standard_max_usd = number_or_default(
        pricing.get("standard_max_usd_per_1m_tokens"),
        2,
        "pricing.standard_max_usd_per_1m_tokens",
    )?;

    let mut allowlist = vec![
        openrouter_free_model.clone(),
        openrouter_auto_model.clone(),
        openrouter_premium_model.clone(),
    ];
    allowlist.extend(openrouter_extra_models);
    allowlist.dedup();

    Ok(json!({
        "server": {
            "host": string_value(server.get("host"), "server.host")?,
            "port": number_value(server.get("port"), "server.port")?,
            "request_timeout_ms": number_or_default(server.get("request_timeout_ms"), 60_000, "server.request_timeout_ms")?,
            "fallback_max_attempts": number_or_default(server.get("fallback_max_attempts"), 2, "server.fallback_max_attempts")?,
            "cors_allowed_origins": if server.get("cors_allowed_origins").is_none() {
                DEFAULT_CORS_ALLOWED_ORIGINS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            } else {
                string_array(server.get("cors_allowed_origins"), "server.cors_allowed_origins")?
            },
        },
        "openrouter_sync": {
            "enabled": boolean_or_default(sync.get("openrouter"), false, "sync.openrouter")?,
            "interval_seconds": sync_interval_seconds,
            "update_config_file": false,
            "source_url": "https://openrouter.ai/api/v1/models",
            "include_unconfigured_models": false,
            "allowlist": allowlist,
            "cost_tiers": cost_tier_defaults(standard_max_usd as f64),
        },
        "deepseek_sync": {
            "enabled": boolean_or_default(sync.get("deepseek"), false, "sync.deepseek")?,
            "interval_seconds": sync_interval_seconds,
            "source_url": "https://api-docs.deepseek.com/quick_start/pricing/",
            "cost_tiers": cost_tier_defaults(standard_max_usd as f64),
        },
        "routing": {
            "latency": {
                "network_ms_max": number_or_default(routing.get("network_ms_max"), 250, "routing.network_ms_max")?,
                "first_token_ms_max": number_or_default(routing.get("first_token_ms_max"), 5000, "routing.first_token_ms_max")?,
            }
        },
        "models": {
            "local/chat": {
                "provider": "local",
                "upstream_model": string_or_default(local.get("chat_model"), "local-llm", "local.chat_model")?,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "chat",
                "context_window": local_context_window,
                "max_concurrency": local_max_concurrency,
            },
            "local/embedding": {
                "provider": "local",
                "upstream_model": string_or_default(local.get("embedding_model"), "local-embedding", "local.embedding_model")?,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "embeddings",
                "max_concurrency": local_max_concurrency,
            },
            "local/tts": {
                "provider": "local",
                "upstream_model": string_or_default(local.get("tts_model"), "local-tts", "local.tts_model")?,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "audio_speech",
                "max_concurrency": local_max_concurrency,
            },
            "local/tts-quality": {
                "provider": "local",
                "upstream_model": local_tts_quality_model,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "audio_speech",
                "max_concurrency": local_max_concurrency,
            },
            "local/tts-voice-design": {
                "provider": "local",
                "upstream_model": local_tts_voice_design_model,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "audio_speech",
                "max_concurrency": local_max_concurrency,
            },
            "local/asr": {
                "provider": "local",
                "upstream_model": string_or_default(local.get("asr_model"), "local-asr", "local.asr_model")?,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "audio_transcriptions",
                "max_concurrency": local_max_concurrency,
            },
            "local/translation": {
                "provider": "local",
                "upstream_model": string_or_default(local.get("translation_model"), "local-translation", "local.translation_model")?,
                "base_url": local_base_url,
                "api_key_env": local_api_key_env,
                "endpoint": "translations",
                "max_concurrency": local_max_concurrency,
            },
            "deepseek/chat": {
                "provider": "deepseek",
                "upstream_model": deepseek_model,
                "base_url": deepseek_base_url,
                "api_key_env": deepseek_api_key_env,
                "endpoint": "chat",
                "context_window": number_or_default(deepseek.get("context_window"), 64_000, "deepseek.context_window")?,
            },
            "deepseek/premium": {
                "provider": "deepseek",
                "upstream_model": deepseek_premium_model,
                "base_url": deepseek_base_url,
                "api_key_env": deepseek_api_key_env,
                "endpoint": "chat",
                "cost_tier": "premium",
                "context_window": number_or_default(deepseek.get("premium_context_window").or_else(|| deepseek.get("context_window")), 64_000, "deepseek.premium_context_window")?,
            },
            "openrouter/auto": {
                "provider": "openrouter",
                "upstream_model": openrouter_auto_model,
                "base_url": openrouter_base_url,
                "api_key_env": openrouter_api_key_env,
                "endpoint": "chat",
                "context_window": number_or_default(openrouter.get("standard_context_window").or_else(|| openrouter.get("auto_context_window")), 128_000, "openrouter.standard_context_window")?,
            },
            "openrouter/premium": {
                "provider": "openrouter",
                "upstream_model": openrouter_premium_model,
                "base_url": openrouter_base_url,
                "api_key_env": openrouter_api_key_env,
                "endpoint": "chat",
                "context_window": number_or_default(openrouter.get("premium_context_window"), 200_000, "openrouter.premium_context_window")?,
            }
        }
    }))
}

fn validate_model(value: &Value, path: &str) -> Result<ModelConfig> {
    let object = as_object(value, path)?;
    let endpoint: Endpoint = serde_json::from_value(
        object
            .get("endpoint")
            .cloned()
            .ok_or_else(|| anyhow!("{path}.endpoint must be a non-empty string"))?,
    )?;
    let cost_tier = optional_string(object.get("cost_tier"), &format!("{path}.cost_tier"))?;
    if let Some(tier) = &cost_tier {
        if tier != "free" && tier != "standard" && tier != "premium" {
            return Err(anyhow!("{path}.cost_tier is invalid"));
        }
    }
    Ok(ModelConfig {
        request_timeout_ms: {
            let timeout = optional_number(
                object.get("request_timeout_ms"),
                &format!("{path}.request_timeout_ms"),
            )?;
            if timeout == Some(0) {
                return Err(anyhow!("{path}.request_timeout_ms must be positive"));
            }
            timeout
        },
        provider: string_value(object.get("provider"), &format!("{path}.provider"))?,
        upstream_model: string_value(
            object.get("upstream_model"),
            &format!("{path}.upstream_model"),
        )?,
        base_url: trim_url(&string_value(
            object.get("base_url"),
            &format!("{path}.base_url"),
        )?),
        api_key_env: string_value(object.get("api_key_env"), &format!("{path}.api_key_env"))?,
        endpoint,
        cost_tier,
        price_rank: optional_number(object.get("price_rank"), &format!("{path}.price_rank"))?
            .map(|n| n as i64),
        context_window: optional_number(
            object.get("context_window"),
            &format!("{path}.context_window"),
        )?,
        max_concurrency: optional_number(
            object.get("max_concurrency"),
            &format!("{path}.max_concurrency"),
        )?,
        supported_parameters: if object.get("supported_parameters").is_none() {
            None
        } else {
            Some(string_array(
                object.get("supported_parameters"),
                &format!("{path}.supported_parameters"),
            )?)
        },
    })
}

fn validate_routing(value: Option<&Value>) -> Result<RoutingConfig> {
    let raw = value.and_then(Value::as_object);
    let latency_value = raw.and_then(|object| object.get("latency"));
    let latency = if let Some(latency) = latency_value {
        let object = as_object(latency, "routing.latency")?;
        Some(RouteLatencyConfig {
            network_p95_ms_max: optional_number(
                object
                    .get("network_p95_ms_max")
                    .or_else(|| object.get("network_ms_max")),
                "routing.latency.network_ms_max",
            )?,
            first_token_p95_ms_max: optional_number(
                object
                    .get("first_token_p95_ms_max")
                    .or_else(|| object.get("first_token_ms_max")),
                "routing.latency.first_token_ms_max",
            )?,
        })
    } else {
        Some(RouteLatencyConfig {
            network_p95_ms_max: Some(250),
            first_token_p95_ms_max: Some(5000),
        })
    };
    Ok(RoutingConfig { latency })
}

fn builtin_routes(routing: RoutingConfig) -> HashMap<String, RouteConfig> {
    let latency = routing.latency;
    let mut routes = HashMap::new();
    for (tier, endpoint, name) in [
        ("free", Endpoint::Chat, "free/chat"),
        ("free", Endpoint::Embeddings, "free/embedding"),
        ("free", Endpoint::AudioSpeech, "free/tts"),
        ("free", Endpoint::AudioTranscriptions, "free/asr"),
        ("free", Endpoint::Translations, "free/translation"),
        ("standard", Endpoint::Chat, "standard/chat"),
        ("standard", Endpoint::Embeddings, "standard/embedding"),
        ("standard", Endpoint::AudioSpeech, "standard/tts"),
        ("standard", Endpoint::AudioTranscriptions, "standard/asr"),
        ("standard", Endpoint::Translations, "standard/translation"),
        ("premium", Endpoint::Chat, "premium/chat"),
        ("premium", Endpoint::Embeddings, "premium/embedding"),
        ("premium", Endpoint::AudioSpeech, "premium/tts"),
        ("premium", Endpoint::AudioTranscriptions, "premium/asr"),
        ("premium", Endpoint::Translations, "premium/translation"),
    ] {
        routes.insert(
            name.to_string(),
            RouteConfig {
                endpoint,
                require_private: None,
                cost_tier: Some(tier.to_string()),
                latency: latency.clone(),
            },
        );
    }
    routes
}

fn validate_openrouter_sync(value: Option<&Value>) -> Result<OpenRouterSyncConfig> {
    let raw = value.and_then(Value::as_object);
    let cost_tiers = raw
        .and_then(|object| object.get("cost_tiers"))
        .and_then(Value::as_object);
    Ok(OpenRouterSyncConfig {
        enabled: bool_field(raw, "enabled", false, "openrouter_sync.enabled")?,
        interval_seconds: number_field(
            raw,
            "interval_seconds",
            21_600,
            "openrouter_sync.interval_seconds",
        )?,
        update_config_file: bool_field(
            raw,
            "update_config_file",
            false,
            "openrouter_sync.update_config_file",
        )?,
        source_url: string_field(
            raw,
            "source_url",
            "https://openrouter.ai/api/v1/models",
            "openrouter_sync.source_url",
        )?,
        include_unconfigured_models: bool_field(
            raw,
            "include_unconfigured_models",
            false,
            "openrouter_sync.include_unconfigured_models",
        )?,
        allowlist: if let Some(raw) = raw.and_then(|object| object.get("allowlist")) {
            string_array(Some(raw), "openrouter_sync.allowlist")?
        } else {
            Vec::new()
        },
        cost_tiers: validate_cost_tiers(cost_tiers, "openrouter_sync.cost_tiers")?,
    })
}

fn validate_deepseek_sync(value: Option<&Value>) -> Result<DeepSeekSyncConfig> {
    let raw = value.and_then(Value::as_object);
    let cost_tiers = raw
        .and_then(|object| object.get("cost_tiers"))
        .and_then(Value::as_object);
    Ok(DeepSeekSyncConfig {
        enabled: bool_field(raw, "enabled", false, "deepseek_sync.enabled")?,
        interval_seconds: number_field(
            raw,
            "interval_seconds",
            21_600,
            "deepseek_sync.interval_seconds",
        )?,
        source_url: string_field(
            raw,
            "source_url",
            "https://api-docs.deepseek.com/quick_start/pricing/",
            "deepseek_sync.source_url",
        )?,
        cost_tiers: validate_cost_tiers(cost_tiers, "deepseek_sync.cost_tiers")?,
    })
}

fn validate_cost_tiers(raw: Option<&Map<String, Value>>, path: &str) -> Result<CostTierConfig> {
    Ok(CostTierConfig {
        free_max_usd_per_1m_tokens: number_f64_field(
            raw,
            "free_max_usd_per_1m_tokens",
            0.0,
            &format!("{path}.free_max_usd_per_1m_tokens"),
        )?,
        standard_max_usd_per_1m_tokens: number_f64_field(
            raw,
            "standard_max_usd_per_1m_tokens",
            2.0,
            &format!("{path}.standard_max_usd_per_1m_tokens"),
        )?,
        premium_max_usd_per_1m_tokens: number_f64_field(
            raw,
            "premium_max_usd_per_1m_tokens",
            9999.0,
            &format!("{path}.premium_max_usd_per_1m_tokens"),
        )?,
    })
}

fn cost_tier_defaults(standard_max_usd: f64) -> Value {
    json!({
        "free_max_usd_per_1m_tokens": 0,
        "standard_max_usd_per_1m_tokens": standard_max_usd,
        "premium_max_usd_per_1m_tokens": 9999,
    })
}

fn as_object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow!("{path} must be an object"))
}

fn string_value(value: Option<&Value>, path: &str) -> Result<String> {
    let value = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("{path} must be a non-empty string"))?;
    Ok(value.to_string())
}

fn string_or_default(value: Option<&Value>, fallback: &str, path: &str) -> Result<String> {
    match value {
        Some(_) => string_value(value, path),
        None => Ok(fallback.to_string()),
    }
}

fn optional_string(value: Option<&Value>, path: &str) -> Result<Option<String>> {
    match value {
        Some(_) => string_value(value, path).map(Some),
        None => Ok(None),
    }
}

fn number_value(value: Option<&Value>, path: &str) -> Result<u64> {
    value
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{path} must be a number"))
}

fn number_f64_value(value: Option<&Value>, path: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("{path} must be a number"))
}

fn optional_number(value: Option<&Value>, path: &str) -> Result<Option<u64>> {
    match value {
        Some(_) => number_value(value, path).map(Some),
        None => Ok(None),
    }
}

fn number_or_default(value: Option<&Value>, fallback: u64, path: &str) -> Result<u64> {
    match value {
        Some(_) => number_value(value, path),
        None => Ok(fallback),
    }
}

fn bool_value(value: Option<&Value>, path: &str) -> Result<bool> {
    value
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow!("{path} must be a boolean"))
}

fn boolean_or_default(value: Option<&Value>, fallback: bool, path: &str) -> Result<bool> {
    match value {
        Some(_) => bool_value(value, path),
        None => Ok(fallback),
    }
}

fn string_array(value: Option<&Value>, path: &str) -> Result<Vec<String>> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{path} must be an array"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, item)| string_value(Some(item), &format!("{path}.{index}")))
        .collect()
}

fn bool_field(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: bool,
    path: &str,
) -> Result<bool> {
    boolean_or_default(raw.and_then(|object| object.get(key)), fallback, path)
}

fn number_field(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: u64,
    path: &str,
) -> Result<u64> {
    number_or_default(raw.and_then(|object| object.get(key)), fallback, path)
}

fn number_f64_field(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: f64,
    path: &str,
) -> Result<f64> {
    match raw.and_then(|object| object.get(key)) {
        Some(_) => number_f64_value(raw.and_then(|object| object.get(key)), path),
        None => Ok(fallback),
    }
}

fn string_field(
    raw: Option<&Map<String, Value>>,
    key: &str,
    fallback: &str,
    path: &str,
) -> Result<String> {
    string_or_default(raw.and_then(|object| object.get(key)), fallback, path)
}

fn trim_url(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}
