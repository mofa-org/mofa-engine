//! Scheduler trait.

use async_trait::async_trait;
use crate::{EngineError, ModelInfo, ModelType, PrefetchHint};

#[async_trait]
pub trait Scheduler: Send + Sync {
    async fn select_model(
        &self,
        model_type: Option<ModelType>,
        model_name: Option<&str>,
        available: &[ModelInfo],
    ) -> Result<String, EngineError>;

    async fn on_hint(&self, hint: &PrefetchHint);

    async fn on_run_complete(&self, model_type: ModelType, model_id: &str);
}
