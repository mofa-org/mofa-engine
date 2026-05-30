//! Core types shared across the entire MoFA Engine.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Capabilities that a model can provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Capability {
    /// Text chat / completion
    Chat,
    /// Text-to-speech
    Tts,
    /// Automatic speech recognition
    Asr,
    /// Image generation
    ImageGen,
    /// Video generation
    VideoGen,
    /// Vision-language model
    Vlm,
    /// Text embedding
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
            "chat" => Some(Self::Chat),
            "tts" => Some(Self::Tts),
            "asr" => Some(Self::Asr),
            "imagegen" | "image_gen" | "image-gen" => Some(Self::ImageGen),
            "videogen" | "video_gen" | "video-gen" => Some(Self::VideoGen),
            "vlm" => Some(Self::Vlm),
            "embedding" => Some(Self::Embedding),
            _ => None,
        }
    }
}

/// The kind of provider backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderKind {
    /// Local Ollama instance
    Ollama,
    /// Any OpenAI-compatible API
    OpenAiCompatible,
}

/// Runtime status of a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ModelStatus {
    /// Not loaded, needs warming
    Cold,
    /// Currently loading
    Warming,
    /// Loaded and ready
    Hot,
    /// Currently processing a request
    Busy,
    /// Failed to load or crashed
    Failed,
}

/// Cost tier for routing preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CostTier {
    /// Free (local models)
    Free,
    /// Low cost
    Low,
    /// Medium cost
    Medium,
    /// High cost
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

/// Descriptor for a model available in the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCard {
    /// Unique model identifier (provider::model_name)
    pub id: String,
    /// Human-readable model name
    pub name: String,
    /// Provider that hosts this model
    pub provider: String,
    /// What this model can do
    pub capability: Capability,
    /// Current runtime status
    pub status: ModelStatus,
    /// Cost classification
    pub cost_tier: CostTier,
    /// Maximum context window in tokens
    pub context_window: u32,
    /// Estimated memory footprint in bytes
    pub memory_estimate_bytes: u64,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Role: "system", "user", "assistant"
    pub role: String,
    /// Message content
    pub content: String,
}

/// A request to the engine for inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    /// Desired capability (if None, inferred from context)
    pub capability: Option<Capability>,
    /// Specific model to use (if None, auto-routed)
    pub model: Option<String>,
    /// Conversation messages
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Path to an input file (for ASR, VLM, etc.)
    pub input_file: Option<String>,
    /// Extra parameters passed through to the provider
    #[serde(default)]
    pub params: serde_json::Value,
    /// Hint about what capability will be needed next (for preflight warming)
    pub hint_next: Option<String>,
    /// Unique request identifier (auto-generated if empty)
    #[serde(default = "generate_request_id")]
    pub request_id: String,
}

fn generate_request_id() -> String {
    Uuid::new_v4().to_string()
}

/// Response from an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Text output (for chat, ASR, etc.)
    pub text: Option<String>,
    /// File output path (for TTS, image gen, etc.)
    pub file: Option<String>,
    /// Which model actually handled the request
    pub model_used: String,
    /// Which provider served it
    pub provider: String,
    /// Wall-clock duration in milliseconds
    pub duration_ms: u64,
    /// Request correlation ID
    pub request_id: String,
    /// Token usage (if reported by provider)
    pub tokens_used: Option<u32>,
}

/// Events emitted by the engine for observability.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineEvent {
    /// A model's status changed
    ModelStatusChanged {
        /// Model identifier
        model_id: String,
        /// Previous status
        old: ModelStatus,
        /// New status
        new: ModelStatus,
    },
    /// A request started processing
    RequestStarted {
        /// Request correlation ID
        request_id: String,
        /// Requested capability
        capability: Option<Capability>,
        /// Model selected for this request
        model_id: String,
    },
    /// A request completed
    RequestCompleted {
        /// Request correlation ID
        request_id: String,
        /// Duration in milliseconds
        duration_ms: u64,
        /// Whether it succeeded
        success: bool,
    },
    /// Memory allocation changed
    MemoryChanged {
        /// Currently used bytes
        used_bytes: u64,
        /// Total budget bytes
        total_bytes: u64,
    },
    /// Provider health changed
    ProviderHealthChanged {
        /// Provider name
        provider: String,
        /// Whether provider is healthy
        healthy: bool,
    },
}

/// Overall engine status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    /// Total number of known models
    pub total_models: usize,
    /// Number of currently loaded (Hot) models
    pub loaded_models: usize,
    /// Number of active providers
    pub providers: usize,
    /// Memory used in bytes
    pub memory_used_bytes: u64,
    /// Memory budget in bytes
    pub memory_budget_bytes: u64,
    /// Engine uptime in seconds
    pub uptime_secs: u64,
    /// Provider health map
    pub provider_health: Vec<ProviderHealth>,
}

/// Health status for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    /// Provider name
    pub name: String,
    /// Whether health check passed
    pub healthy: bool,
    /// Circuit breaker state
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
