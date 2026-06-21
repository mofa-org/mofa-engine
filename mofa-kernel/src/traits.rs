//! Provider trait — the contract all model backends must implement.

use async_trait::async_trait;

use crate::error::EngineError;
use crate::types::{
    BackendFeature, BackendHealth, InferenceRequest, InferenceResponse, LifecycleResult, ModelCard,
    ProviderKind,
};

/// A model provider backend.
///
/// Providers own backend-specific protocol details. The engine owns routing,
/// lifecycle policy, memory admission, fallback, and observability.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider name used in canonical model identifiers.
    fn name(&self) -> &str;

    /// What kind of provider this is.
    fn kind(&self) -> ProviderKind;

    /// Static backend features.
    fn features(&self) -> Vec<BackendFeature>;

    /// Discover all models this provider can serve.
    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError>;

    /// Check if the provider is reachable and healthy.
    async fn health(&self) -> Result<BackendHealth, EngineError>;

    /// Load or warm a model so it is ready for inference.
    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError>;

    /// Evict a model from memory/cache when the backend supports it.
    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError>;

    /// Run inference on a specific model.
    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify the trait is object-safe by constructing a trait object type.
    fn _assert_object_safe(_: &dyn Provider) {}
}
