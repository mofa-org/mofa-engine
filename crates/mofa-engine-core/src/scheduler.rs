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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(id: &str, name: &str, mt: ModelType, bt: BackendType, status: ModelStatus) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            name: name.to_string(),
            model_type: mt,
            backend: bt,
            status,
            memory_bytes: 1000,
            priority: ModelPriority::Normal,
        }
    }

    #[tokio::test]
    async fn test_select_by_name() {
        let s = SmartScheduler::new();
        let models = vec![
            make_model("a", "gpt-4o", ModelType::Llm, BackendType::OpenAi, ModelStatus::Available),
        ];
        let id = s.select_model(None, Some("gpt-4o"), &models).await.unwrap();
        assert_eq!(id, "a");
    }

    #[tokio::test]
    async fn test_select_by_type_prefers_local() {
        let s = SmartScheduler::new();
        let models = vec![
            make_model("cloud", "gpt-4o", ModelType::Llm, BackendType::OpenAi, ModelStatus::Available),
            make_model("local", "qwen", ModelType::Llm, BackendType::Ollama, ModelStatus::Available),
        ];
        let id = s.select_model(Some(ModelType::Llm), None, &models).await.unwrap();
        assert_eq!(id, "local");
    }

    #[tokio::test]
    async fn test_select_by_type_prefers_loaded() {
        let s = SmartScheduler::new();
        let models = vec![
            make_model("a", "qwen", ModelType::Llm, BackendType::Ollama, ModelStatus::Available),
            make_model("b", "gpt-4o", ModelType::Llm, BackendType::OpenAi, ModelStatus::Loaded),
        ];
        let id = s.select_model(Some(ModelType::Llm), None, &models).await.unwrap();
        assert_eq!(id, "b");
    }

    #[tokio::test]
    async fn test_select_not_found() {
        let s = SmartScheduler::new();
        let models = vec![
            make_model("a", "tts-1", ModelType::Tts, BackendType::OpenAi, ModelStatus::Available),
        ];
        let result = s.select_model(Some(ModelType::Asr), None, &models).await;
        assert!(result.is_err());
    }
}
