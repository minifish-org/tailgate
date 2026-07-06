use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{
    health::HealthRegistry,
    types::{AppConfig, ModelConfig, RuntimeModelMetadata, SelectedModel},
};

#[derive(Clone)]
pub struct RuntimeCatalog {
    config: Arc<AppConfig>,
    health: HealthRegistry,
    overlays: Arc<RwLock<HashMap<String, RuntimeModelMetadata>>>,
    virtual_models: Arc<RwLock<HashMap<String, ModelConfig>>>,
}

impl RuntimeCatalog {
    pub fn new(config: Arc<AppConfig>, health: HealthRegistry) -> Self {
        Self {
            config,
            health,
            overlays: Arc::new(RwLock::new(HashMap::new())),
            virtual_models: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_overlay(&self, model_id: &str, metadata: RuntimeModelMetadata) {
        self.overlays
            .write()
            .expect("catalog overlay lock poisoned")
            .insert(model_id.to_string(), metadata);
    }

    pub fn set_virtual_model(
        &self,
        model_id: &str,
        model: ModelConfig,
        metadata: RuntimeModelMetadata,
    ) {
        self.virtual_models
            .write()
            .expect("catalog virtual lock poisoned")
            .insert(model_id.to_string(), model.clone());
        self.set_overlay(model_id, metadata);
        self.health.ensure_model(model_id, Some(model), false);
    }

    pub fn get_model(&self, model_name: &str) -> Option<SelectedModel> {
        let overlays = self.overlays.read().expect("catalog overlay lock poisoned");
        if let Some(configured) = self.config.models.get(model_name) {
            let metadata = overlays.get(model_name).cloned();
            return Some(SelectedModel {
                name: model_name.to_string(),
                config: effective_model(configured, metadata.as_ref()),
                metadata,
            });
        }
        let virtual_model = self
            .virtual_models
            .read()
            .expect("catalog virtual lock poisoned")
            .get(model_name)
            .cloned();
        virtual_model.map(|model| {
            let metadata = overlays.get(model_name).cloned();
            SelectedModel {
                name: model_name.to_string(),
                config: effective_model(&model, metadata.as_ref()),
                metadata,
            }
        })
    }

    pub fn get_model_entries(&self) -> Vec<(String, ModelConfig, Option<RuntimeModelMetadata>)> {
        let overlays = self.overlays.read().expect("catalog overlay lock poisoned");
        let mut entries: Vec<_> = self
            .config
            .models
            .iter()
            .map(|(id, model)| {
                let metadata = overlays.get(id).cloned();
                (
                    id.clone(),
                    effective_model(model, metadata.as_ref()),
                    metadata,
                )
            })
            .collect();
        entries.extend(
            self.virtual_models
                .read()
                .expect("catalog virtual lock poisoned")
                .iter()
                .map(|(id, model)| {
                    let metadata = overlays.get(id).cloned();
                    (
                        id.clone(),
                        effective_model(model, metadata.as_ref()),
                        metadata,
                    )
                }),
        );
        entries
    }

    pub fn get_configured_model_entries(&self) -> Vec<(String, ModelConfig)> {
        self.config
            .models
            .iter()
            .map(|(id, model)| (id.clone(), model.clone()))
            .collect()
    }

    pub fn get_virtual_model_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .virtual_models
            .read()
            .expect("catalog virtual lock poisoned")
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    pub fn get_runtime_metadata_summary(&self) -> HashMap<String, RuntimeModelMetadata> {
        self.overlays
            .read()
            .expect("catalog overlay lock poisoned")
            .clone()
    }
}

pub fn effective_model(
    model: &ModelConfig,
    metadata: Option<&RuntimeModelMetadata>,
) -> ModelConfig {
    let Some(metadata) = metadata else {
        return model.clone();
    };
    let mut model = model.clone();
    if metadata.context_window.is_some() {
        model.context_window = metadata.context_window;
    }
    if metadata.dynamic_cost_tier.is_some() {
        model.cost_tier = metadata.dynamic_cost_tier.clone();
    }
    if metadata.dynamic_price_rank.is_some() {
        model.price_rank = metadata.dynamic_price_rank;
    }
    if metadata.supported_parameters.is_some() {
        model.supported_parameters = metadata.supported_parameters.clone();
    }
    model
}
