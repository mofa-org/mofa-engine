//! Core types shared across the entire MoFA Engine.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Capabilities that a model can provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Capability {
    /// Text chat / completion.
    Chat,
    /// Text-to-speech.
    Tts,
    /// Automatic speech recognition.
    Asr,
    /// Image generation.
    ImageGen,
    /// Video generation.
    VideoGen,
    /// Vision-language model.
    Vlm,
    /// Text embedding.
    Embedding,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Chat => "chat",
            Self::Tts => "tts",
            Self::Asr => "asr",
            Self::ImageGen => "imagegen",
            Self::VideoGen => "videogen",
            Self::Vlm => "vlm",
            Self::Embedding => "embedding",
        };
        f.write_str(s)
    }
}

impl Capability {
    /// Parse a capability from a string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "chat" | "llm" => Some(Self::Chat),
            "tts" => Some(Self::Tts),
            "asr" | "stt" => Some(Self::Asr),
            "imagegen" | "image_gen" | "image-gen" => Some(Self::ImageGen),
            "videogen" | "video_gen" | "video-gen" => Some(Self::VideoGen),
            "vlm" | "vision" => Some(Self::Vlm),
            "embedding" | "embeddings" => Some(Self::Embedding),
            _ => None,
        }
    }
}

/// The kind of provider backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderKind {
    /// Local Ollama instance.
    Ollama,
    /// Any OpenAI-compatible API.
    OpenAiCompatible,
}

/// Backend health is independent from model residency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendHealth {
    /// Health has not been checked yet.
    Unknown,
    /// Backend is reachable and can accept work.
    Healthy,
    /// Backend is reachable but not fully reliable.
    Degraded,
    /// Backend is not reachable or cannot serve requests.
    Unavailable,
}

impl BackendHealth {
    /// Whether the backend can be considered for routing.
    pub fn is_routable(self) -> bool {
        matches!(self, Self::Unknown | Self::Healthy | Self::Degraded)
    }
}

/// Whether a model exists in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAvailability {
    /// Model was discovered from a backend.
    Discovered,
    /// Model was configured manually.
    Configured,
    /// Model is known but cannot currently be used.
    Unavailable,
}

/// Whether a model is resident in local memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResidency {
    /// Residency is not known.
    Unknown,
    /// Model is not loaded locally.
    Unloaded,
    /// Model is currently being loaded.
    Loading,
    /// Model is loaded locally.
    Loaded,
    /// Model is currently being unloaded.
    Unloading,
    /// Model is remote/cloud-backed and does not consume local model memory.
    Remote,
}

/// Execution state and concurrency limits for a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionState {
    /// Number of active requests.
    pub active_requests: u32,
    /// Maximum requests the backend says this model can handle.
    pub max_concurrency: u32,
}

impl Default for ExecutionState {
    fn default() -> Self {
        Self {
            active_requests: 0,
            max_concurrency: 1,
        }
    }
}

impl ExecutionState {
    /// Whether the model can accept another request.
    pub fn has_capacity(&self) -> bool {
        self.active_requests < self.max_concurrency
    }
}

/// Runtime status retained for compatibility with the prototype API and dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ModelStatus {
    /// Not loaded, needs warming.
    Cold,
    /// Currently loading.
    Warming,
    /// Loaded and ready.
    Hot,
    /// Currently processing a request.
    Busy,
    /// Failed to load or crashed.
    Failed,
}

impl ModelStatus {
    /// Derive the legacy status from richer runtime state.
    pub fn from_state(
        availability: ModelAvailability,
        residency: ModelResidency,
        execution: &ExecutionState,
    ) -> Self {
        if availability == ModelAvailability::Unavailable {
            return Self::Failed;
        }
        if execution.active_requests > 0 {
            return Self::Busy;
        }
        match residency {
            ModelResidency::Loaded | ModelResidency::Remote => Self::Hot,
            ModelResidency::Loading => Self::Warming,
            ModelResidency::Unloading | ModelResidency::Unloaded | ModelResidency::Unknown => {
                Self::Cold
            }
        }
    }
}

/// Cost tier for routing preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CostTier {
    /// Free (local models).
    Free,
    /// Low cost.
    Low,
    /// Medium cost.
    Medium,
    /// High cost.
    High,
}

impl CostTier {
    /// Parse a cost tier from a string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "free" => Self::Free,
            "low" => Self::Low,
            "medium" | "med" => Self::Medium,
            "high" => Self::High,
            _ => Self::Medium,
        }
    }
}

/// Capabilities a backend supports outside model inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendFeature {
    /// Backend can discover models dynamically.
    Discovery,
    /// Backend can explicitly load or warm models.
    Load,
    /// Backend can explicitly unload models.
    Unload,
    /// Backend can report loaded/resident models.
    ResidencyInspection,
    /// Backend supports streaming output.
    Streaming,
    /// Backend can report memory usage.
    MemoryReporting,
}

/// Descriptor for a model available in the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCard {
    /// Unique model identifier (`provider/model_name`).
    pub id: String,
    /// Human-readable model name.
    pub name: String,
    /// Provider that hosts this model.
    pub provider: String,
    /// Primary capability retained for compatibility with the prototype API.
    pub capability: Capability,
    /// All capabilities this model can provide.
    pub capabilities: Vec<Capability>,
    /// Current runtime status retained for compatibility.
    pub status: ModelStatus,
    /// Whether the model was discovered, configured, or is unavailable.
    pub availability: ModelAvailability,
    /// Whether the model is loaded locally.
    pub residency: ModelResidency,
    /// Request concurrency state.
    pub execution: ExecutionState,
    /// Cost classification.
    pub cost_tier: CostTier,
    /// Maximum context window in tokens.
    pub context_window: u32,
    /// Estimated memory footprint in bytes.
    pub memory_estimate_bytes: u64,
}

impl ModelCard {
    /// Construct a model card with conservative defaults.
    pub fn new(
        provider: impl Into<String>,
        name: impl Into<String>,
        capability: Capability,
        cost_tier: CostTier,
    ) -> Self {
        let provider = provider.into();
        let name = name.into();
        let residency = ModelResidency::Unloaded;
        let availability = ModelAvailability::Discovered;
        let execution = ExecutionState::default();
        let status = ModelStatus::from_state(availability, residency, &execution);
        Self {
            id: canonical_model_id(&provider, &name),
            name,
            provider,
            capability,
            capabilities: vec![capability],
            status,
            availability,
            residency,
            execution,
            cost_tier,
            context_window: 4096,
            memory_estimate_bytes: 0,
        }
    }

    /// Update the compatibility `status` from the richer state fields.
    pub fn refresh_status(&mut self) {
        self.status = ModelStatus::from_state(self.availability, self.residency, &self.execution);
    }

    /// Whether the model supports the requested capability.
    pub fn supports(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability) || self.capability == capability
    }
}

/// Build a canonical model identifier.
pub fn canonical_model_id(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

/// Extract the provider segment from a canonical model identifier.
pub fn model_id_provider(model_id: &str) -> Option<&str> {
    model_id.split_once('/').map(|(provider, _)| provider)
}

/// Extract the model-name segment from a canonical model identifier.
pub fn model_id_name(model_id: &str) -> &str {
    model_id
        .split_once('/')
        .map(|(_, model)| model)
        .or_else(|| model_id.split_once("::").map(|(_, model)| model))
        .unwrap_or(model_id)
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "system", "user", "assistant".
    pub role: String,
    /// Message content.
    pub content: String,
}

/// Named-model fallback behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    /// Capability requests can fail over; named requests are strict.
    CapabilityOnly,
    /// Never fail over.
    Disabled,
    /// Allow fallback even for named requests.
    AllowNamed,
}

impl Default for FallbackPolicy {
    fn default() -> Self {
        Self::CapabilityOnly
    }
}

/// A request to the engine for inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    /// Desired capability.
    pub capability: Option<Capability>,
    /// Specific model to use.
    pub model: Option<String>,
    /// Application identifier for history learning and observability.
    pub app_id: Option<String>,
    /// Session identifier for scoped Preflight history.
    pub session_id: Option<String>,
    /// Named fallback policy.
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,
    /// Conversation messages.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Path to an input file on the engine host.
    pub input_file: Option<String>,
    /// Extra parameters passed through to the provider.
    #[serde(default)]
    pub params: serde_json::Value,
    /// Hint about what capability will be needed next.
    pub hint_next: Option<String>,
    /// Unique request identifier.
    #[serde(default = "generate_request_id")]
    pub request_id: String,
}

fn generate_request_id() -> String {
    Uuid::new_v4().to_string()
}

/// Response from an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Text output (for chat, ASR, etc.).
    pub text: Option<String>,
    /// File output path (for TTS, image gen, etc.).
    pub file: Option<String>,
    /// Which model actually handled the request.
    pub model_used: String,
    /// Which provider served it.
    pub provider: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Request correlation ID.
    pub request_id: String,
    /// Token usage, if reported by provider.
    pub tokens_used: Option<u32>,
    /// Whether the response came from a fallback candidate.
    #[serde(default)]
    pub fallback_used: bool,
    /// Machine-readable routing reason.
    pub routing_reason: Option<String>,
}

/// Result of a lifecycle operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleResult {
    /// Model affected by this operation.
    pub model_id: String,
    /// Resulting residency.
    pub residency: ModelResidency,
    /// Observed or estimated memory, if known.
    pub memory_bytes: Option<u64>,
    /// Whether the operation changed backend state.
    pub changed: bool,
}

/// Backend status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    /// Provider name.
    pub name: String,
    /// Provider kind.
    pub kind: ProviderKind,
    /// Health status.
    pub health: BackendHealth,
    /// Circuit breaker state.
    pub circuit_state: String,
    /// Static backend feature flags.
    pub features: Vec<BackendFeature>,
}

/// Events emitted by the engine for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineEvent {
    /// A model's compatibility status changed.
    ModelStatusChanged {
        /// Model identifier.
        model_id: String,
        /// Previous status.
        old: ModelStatus,
        /// New status.
        new: ModelStatus,
    },
    /// A model's richer residency state changed.
    ModelResidencyChanged {
        /// Model identifier.
        model_id: String,
        /// Previous residency.
        old: ModelResidency,
        /// New residency.
        new: ModelResidency,
    },
    /// A request started processing.
    RequestStarted {
        /// Request correlation ID.
        request_id: String,
        /// Requested capability.
        capability: Option<Capability>,
        /// Model selected for this request.
        model_id: String,
    },
    /// A request completed.
    RequestCompleted {
        /// Request correlation ID.
        request_id: String,
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Whether it succeeded.
        success: bool,
    },
    /// Memory allocation changed.
    MemoryChanged {
        /// Currently used bytes.
        used_bytes: u64,
        /// Total budget bytes.
        total_bytes: u64,
    },
    /// Provider health changed.
    ProviderHealthChanged {
        /// Provider name.
        provider: String,
        /// Current health.
        health: BackendHealth,
    },
    /// Discovery completed for one provider.
    DiscoveryCompleted {
        /// Provider name.
        provider: String,
        /// Number of models registered.
        models: usize,
        /// Whether discovery succeeded.
        success: bool,
    },
}

/// Overall engine status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    /// Total number of known models.
    pub total_models: usize,
    /// Number of currently loaded models.
    pub loaded_models: usize,
    /// Number of active providers.
    pub providers: usize,
    /// Memory used in bytes.
    pub memory_used_bytes: u64,
    /// Memory budget in bytes.
    pub memory_budget_bytes: u64,
    /// Engine uptime in seconds.
    pub uptime_secs: u64,
    /// Provider health map retained for compatibility.
    pub provider_health: Vec<ProviderHealth>,
    /// Backend status snapshots.
    #[serde(default)]
    pub backends: Vec<BackendStatus>,
}

/// Health status for a single provider retained for compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Provider name.
    pub name: String,
    /// Whether the backend is currently routable.
    pub healthy: bool,
    /// Circuit breaker state.
    pub circuit_state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_display_roundtrip() {
        let caps = [
            Capability::Chat,
            Capability::Tts,
            Capability::Asr,
            Capability::ImageGen,
            Capability::VideoGen,
            Capability::Vlm,
            Capability::Embedding,
        ];
        for cap in &caps {
            let s = cap.to_string();
            let parsed = Capability::from_str_loose(&s);
            assert_eq!(parsed, Some(*cap), "roundtrip failed for {cap:?}");
        }
    }

    #[test]
    fn cost_tier_parse() {
        assert_eq!(CostTier::from_str_loose("free"), CostTier::Free);
        assert_eq!(CostTier::from_str_loose("HIGH"), CostTier::High);
        assert_eq!(CostTier::from_str_loose("unknown"), CostTier::Medium);
    }

    #[test]
    fn request_id_auto_generated() {
        let json = r#"{"messages":[]}"#;
        let req: InferenceRequest = serde_json::from_str(json).unwrap();
        assert!(!req.request_id.is_empty());
        assert_eq!(req.fallback_policy, FallbackPolicy::CapabilityOnly);
    }

    #[test]
    fn canonical_id_uses_slash() {
        assert_eq!(canonical_model_id("ollama", "qwen"), "ollama/qwen");
        assert_eq!(model_id_name("ollama/qwen"), "qwen");
        assert_eq!(model_id_name("ollama::qwen"), "qwen");
    }

    #[test]
    fn model_status_derives_from_state() {
        let execution = ExecutionState {
            active_requests: 1,
            max_concurrency: 4,
        };
        assert_eq!(
            ModelStatus::from_state(
                ModelAvailability::Discovered,
                ModelResidency::Loaded,
                &execution
            ),
            ModelStatus::Busy
        );
    }

    #[test]
    fn engine_event_serialization() {
        let event = EngineEvent::ModelStatusChanged {
            model_id: "test".into(),
            old: ModelStatus::Cold,
            new: ModelStatus::Hot,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("model_status_changed"));
    }
}
