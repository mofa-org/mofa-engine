//! Smart scheduler with preflight support.

use async_trait::async_trait;
use dashmap::DashMap;
use mofa_kernel::*;


pub struct SmartScheduler {
    history: DashMap<ModelType, Vec<ModelType>>,
    subscriptions: DashMap<String, Vec<ModelType>>,
}

impl SmartScheduler {
    pub fn new() -> Self {
        Self {
            history: DashMap::new(),
            subscriptions: DashMap::new(),
        }
    }

    pub fn subscribe(&self, app_id: &str, types: Vec<ModelType>) {
        self.subscriptions.insert(app_id.to_string(), types);
    }

    pub fn predict_next(&self, current_type: ModelType) -> Option<ModelType> {
        self.history.get(&current_type).and_then(|followers| {
            let mut counts: std::collections::HashMap<ModelType, usize> = std::collections::HashMap::new();
            for t in followers.iter() {
                *counts.entry(*t).or_default() += 1;
            }
            counts.into_iter().max_by_key(|(_, c)| *c).map(|(t, _)| t)
        })
    }
}

#[async_trait]
impl Scheduler for SmartScheduler {
    async fn select_model(
        &self,
        model_type: Option<ModelType>,
        model_name: Option<&str>,
        available: &[ModelInfo],
    ) -> Result<String, EngineError> {
        if let Some(name) = model_name {
            if let Some(m) = available.iter().find(|m| m.name.eq_ignore_ascii_case(name)) {
                return Ok(m.id.clone());
            }
            return Err(EngineError::ModelNotFound(name.to_string()));
        }

        if let Some(mt) = model_type {
            let candidates: Vec<_> = available.iter()
                .filter(|m| m.model_type == mt)
                .collect();

            if candidates.is_empty() {
                return Err(EngineError::ModelNotFound(format!("no model for type {mt}")));
            }

            // prefer already-loaded
            if let Some(loaded) = candidates.iter().find(|m| m.status == ModelStatus::Loaded) {
                return Ok(loaded.id.clone());
            }

            // prefer local backends over cloud
            let local_first = candidates.iter().find(|m| {
                matches!(m.backend, BackendType::Ollama | BackendType::Mlx | BackendType::Cuda | BackendType::CpuOnnx)
            });
            if let Some(m) = local_first {
                return Ok(m.id.clone());
            }

            return Ok(candidates[0].id.clone());
        }

        Err(EngineError::InvalidInput("must specify model type or model name".to_string()))
    }

    async fn on_hint(&self, _hint: &PrefetchHint) {
        // hint processing is done at the engine level
    }

    async fn on_run_complete(&self, _model_type: ModelType, _model_id: &str) {
        // record for history learning — this is called by engine after each run
    }
}
