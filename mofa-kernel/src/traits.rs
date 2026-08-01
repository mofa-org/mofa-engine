//! Provider trait — the contract all model backends must implement.

use async_trait::async_trait;

use crate::error::EngineError;
use crate::types::{
    BackendFeature, BackendHealth, InferenceRequest, InferenceResponse, LifecycleResult, ModelCard,
    ProviderKind, StreamDelta, StreamSink,
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

    /// Stream inference output as it is produced.
    ///
    /// Providers push incremental text deltas to `sink` and return the final
    /// aggregate response (with the full text/file, tokens, etc.). The engine
    /// owns the surrounding `Started`/`Completed`/`Error` envelope.
    ///
    /// The default implementation provides *non-streaming compatibility*: it
    /// runs [`invoke`](Self::invoke) and emits the whole text output as a single
    /// delta. Backends that support incremental generation (e.g. Ollama) should
    /// override this to stream real tokens.
    async fn stream(
        &self,
        model_id: &str,
        request: &InferenceRequest,
        sink: StreamSink,
    ) -> Result<InferenceResponse, EngineError> {
        let response = self.invoke(model_id, request).await?;
        if let Some(text) = &response.text
            && !text.is_empty()
        {
            let _ = sink.send(StreamDelta::Text(text.clone())).await;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify the trait is object-safe by constructing a trait object type.
    fn _assert_object_safe(_: &dyn Provider) {}
}
