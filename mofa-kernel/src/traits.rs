//! Provider trait — the contract all model backends must implement.

use async_trait::async_trait;

use crate::error::EngineError;
use crate::types::{InferenceRequest, InferenceResponse, ModelCard, ProviderKind};

/// A model provider backend.
///
/// Implementations discover available models, manage their lifecycle,
/// and invoke inference. All methods are async and the trait is
/// object-safe (`Send + Sync`).
#[async_trait]
pub trait Provider: Send + Sync {
    /// Discover all models this provider can serve.
    async fn discover(&self) -> Vec<ModelCard>;

    /// Check if the provider is reachable and healthy.
    async fn health(&self) -> bool;

    /// Run inference on a specific model.
    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError>;

    /// Pre-warm a model so it is ready for fast inference.
    async fn warm(&self, model_id: &str);

    /// Evict a model from memory / cache.
    async fn evict(&self, model_id: &str);

    /// What kind of provider this is.
    fn kind(&self) -> ProviderKind;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify the trait is object-safe by constructing a trait object type.
    fn _assert_object_safe(_: &dyn Provider) {}
}
