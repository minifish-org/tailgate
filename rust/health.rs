use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use tokio::time::sleep;
use tracing::debug;

use crate::types::{AppConfig, Endpoint, HealthState, ModelConfig};

const EWMA_ALPHA: f64 = 0.2;

#[derive(Clone)]
pub struct HealthRegistry {
    states: Arc<RwLock<HashMap<String, HealthState>>>,
    models: Arc<RwLock<HashMap<String, ModelConfig>>>,
    timer_models: Arc<RwLock<HashSet<String>>>,
}

#[derive(Clone, Debug)]
pub struct HealthUpdate {
    pub ok: bool,
    pub total_latency_ms: u64,
    pub first_token_latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct HealthSnapshotEntry {
    pub id: String,
    pub provider: String,
    pub endpoint: Endpoint,
    #[serde(flatten)]
    pub state: HealthState,
}

impl HealthRegistry {
    pub fn new(config: &AppConfig) -> Self {
        let models: HashMap<String, ModelConfig> = config.models.clone();
        let states = models
            .keys()
            .map(|name| (name.clone(), HealthState::default()))
            .collect();
        Self {
            states: Arc::new(RwLock::new(states)),
            models: Arc::new(RwLock::new(models)),
            timer_models: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub fn get(&self, model_name: &str) -> HealthState {
        self.states
            .read()
            .expect("health state lock poisoned")
            .get(model_name)
            .cloned()
            .unwrap_or_default()
    }

    pub fn ensure_model(&self, model_name: &str, model: Option<ModelConfig>, healthy: bool) {
        if let Some(model) = model {
            self.models
                .write()
                .expect("health model lock poisoned")
                .insert(model_name.to_string(), model);
        }
        let mut states = self.states.write().expect("health state lock poisoned");
        let state = states.entry(model_name.to_string()).or_default();
        if healthy {
            state.healthy = true;
        }
    }

    pub fn snapshot(&self, models: Vec<(String, String, Endpoint)>) -> Vec<HealthSnapshotEntry> {
        let mut entries: Vec<_> = models
            .into_iter()
            .map(|(id, provider, endpoint)| HealthSnapshotEntry {
                state: self.get(&id),
                id,
                provider,
                endpoint,
            })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        entries
    }

    pub fn begin_request(&self, model_name: &str) {
        let model = self
            .models
            .read()
            .expect("health model lock poisoned")
            .get(model_name)
            .cloned();
        let mut states = self.states.write().expect("health state lock poisoned");
        let state = states.entry(model_name.to_string()).or_default();
        state.in_flight += 1;
        state.busy = is_busy(model.as_ref(), state.in_flight);
    }

    pub fn finish_request(&self, model_name: &str, result: HealthUpdate) {
        let model = self
            .models
            .read()
            .expect("health model lock poisoned")
            .get(model_name)
            .cloned();
        let mut states = self.states.write().expect("health state lock poisoned");
        let state = states.entry(model_name.to_string()).or_default();
        state.in_flight = state.in_flight.saturating_sub(1);
        state.busy = is_busy(model.as_ref(), state.in_flight);
        state.total_latency_ms = ewma(state.total_latency_ms, result.total_latency_ms);
        if let Some(latency) = result.first_token_latency_ms {
            state.first_token_latency_ms = ewma(state.first_token_latency_ms, latency);
        }
        if result.ok {
            state.healthy = true;
            state.success_count += 1;
            state.last_error = None;
        } else {
            state.error_count += 1;
            state.last_error = Some(truncate_error(
                &result.error.unwrap_or_else(|| "request failed".to_string()),
            ));
        }
        let total = state.success_count + state.error_count;
        state.error_rate = if total == 0 {
            0.0
        } else {
            state.error_count as f64 / total as f64
        };
    }

    pub fn record_first_token(&self, model_name: &str, first_token_latency_ms: u64) {
        let mut states = self.states.write().expect("health state lock poisoned");
        let state = states.entry(model_name.to_string()).or_default();
        state.first_token_latency_ms = ewma(state.first_token_latency_ms, first_token_latency_ms);
    }

    pub fn update_probe(
        &self,
        model_name: &str,
        healthy: bool,
        latency_ms: u64,
        error: Option<String>,
    ) {
        let mut states = self.states.write().expect("health state lock poisoned");
        let state = states.entry(model_name.to_string()).or_default();
        state.healthy = healthy;
        state.last_checked_at = Some(now_iso_like());
        state.network_latency_ms = ewma(state.network_latency_ms, latency_ms);
        if let Some(error) = error {
            state.last_error = Some(truncate_error(&error));
        }
    }

    pub fn start(&self, config: AppConfig, client: Client) {
        let models = self
            .models
            .read()
            .expect("health model lock poisoned")
            .clone();
        for (model_name, model) in models {
            self.ensure_timer(model_name, model, config.clone(), client.clone());
        }
    }

    pub fn ensure_timer(
        &self,
        model_name: String,
        model: ModelConfig,
        config: AppConfig,
        client: Client,
    ) {
        {
            let mut timer_models = self
                .timer_models
                .write()
                .expect("health timer lock poisoned");
            if !timer_models.insert(model_name.clone()) {
                return;
            }
        }
        let registry = self.clone();
        tokio::spawn(async move {
            loop {
                registry
                    .check_model(&model_name, &model, &config, &client)
                    .await;
                let interval = if model.provider == "local" { 15 } else { 30 };
                sleep(Duration::from_secs(interval)).await;
            }
        });
    }

    async fn check_model(
        &self,
        model_name: &str,
        model: &ModelConfig,
        config: &AppConfig,
        client: &Client,
    ) {
        let started = Instant::now();
        let request = client.get(format!("{}/models", model.base_url));
        let request = if let Ok(api_key) = std::env::var(&model.api_key_env) {
            request.bearer_auth(api_key)
        } else {
            request
        };
        let result = tokio::time::timeout(
            Duration::from_millis(config.server.request_timeout_ms.min(10_000)),
            request.send(),
        )
        .await;
        let latency = started.elapsed().as_millis() as u64;
        match result {
            Ok(Ok(response)) if response.status().is_success() => {
                self.update_probe(model_name, true, latency, None);
            }
            Ok(Ok(response)) => {
                self.update_probe(
                    model_name,
                    false,
                    latency,
                    Some(format!(
                        "health check returned {}",
                        response.status().as_u16()
                    )),
                );
            }
            Ok(Err(error)) => {
                self.update_probe(model_name, false, latency, Some(error.to_string()));
                debug!(model = model_name, provider = model.provider, error = %error, "health check failed");
            }
            Err(error) => {
                self.update_probe(model_name, false, latency, Some(error.to_string()));
            }
        }
    }
}

fn is_busy(model: Option<&ModelConfig>, in_flight: u64) -> bool {
    model
        .and_then(|model| model.max_concurrency)
        .is_some_and(|max| in_flight >= max)
}

fn ewma(previous: Option<u64>, next: u64) -> Option<u64> {
    Some(match previous {
        Some(previous) => {
            ((previous as f64 * (1.0 - EWMA_ALPHA)) + (next as f64 * EWMA_ALPHA)).round() as u64
        }
        None => next,
    })
}

fn truncate_error(error: &str) -> String {
    error.split_whitespace().collect::<Vec<_>>().join(" ")[..]
        .chars()
        .take(240)
        .collect()
}

fn now_iso_like() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", duration.as_secs(), duration.subsec_millis())
}
