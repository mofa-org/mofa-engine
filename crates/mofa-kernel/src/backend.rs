//! Backend trait definitions.

use async_trait::async_trait;
use crate::{BackendType, EngineError, ModelInfo, ModelInput, ModelOutput, ModelType};

#[async_trait]
pub trait ModelBackend: Send + Sync {
    fn backend_type(&self) -> BackendType;

    async fn discover(&self) -> Result<Vec<ModelInfo>, EngineError>;

    async fn health_check(&self) -> Result<bool, EngineError>;

    async fn load_model(&self, model_id: &str) -> Result<(), EngineError>;

    async fn unload_model(&self, model_id: &str) -> Result<(), EngineError>;

    async fn run(&self, model_id: &str, input: &ModelInput) -> Result<ModelOutput, EngineError>;

    fn supports_type(&self, model_type: ModelType) -> bool;
}
