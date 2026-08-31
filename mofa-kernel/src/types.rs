//! Core types shared across the entire MoFA Engine.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ErrorInfo;

/// Capabilities that a model can provide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
            Self::ImageGen => "image_gen",
            Self::VideoGen => "video_gen",
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
    /// Local process-adapter backend (e.g. an MLX/Kokoro or Piper TTS CLI).
    LocalTts,
    /// Local process-adapter ASR backend (e.g. a FunASR or whisper.cpp CLI).
    LocalAsr,
    /// Local process-adapter image-generation backend (e.g. a Stable Diffusion CLI).
    LocalImageGen,
    /// Local process-adapter video-generation backend (e.g. an AnimateDiff / SVD
    /// or Wan-style CLI).
    LocalVideoGen,
    /// Cloud video-generation API (the Volcengine Ark / BytePlus task contract
    /// that ByteDance's Seedance models speak): submit → poll → download.
    CloudVideoGen,
    /// Multi-vendor cloud gateway via the `liter-llm` crate (143+ providers,
    /// unified OpenAI-style contract).
    LiterLlm,
}

impl ProviderKind {
    /// Whether this backend runs on the local machine (as opposed to a remote
    /// API). Used by routing to prefer local models and by the memory manager
    /// to account for on-device residency.
    pub fn is_local(self) -> bool {
        matches!(
            self,
            Self::Ollama
                | Self::LocalTts
                | Self::LocalAsr
                | Self::LocalImageGen
                | Self::LocalVideoGen
        )
    }
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
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
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
    /// Reasoning tier this model serves, when configured. Lets `reasoning.effort`
    /// route `low | medium | high` to cheaper/stronger models (S2). `None` for
    /// non-tiered models.
    #[serde(default)]
    pub reasoning_tier: Option<ReasoningEffort>,
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
            id: ModelId::canonical(&provider, &name),
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
            reasoning_tier: None,
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

/// Namespace for canonical model-identifier operations.
///
/// Model identifiers have the canonical form `provider/model` (a legacy
/// `provider::model` form is still accepted when parsing). Construction and
/// parsing live here as associated functions on a zero-sized type so that all id
/// handling has a single, discoverable home rather than scattered free functions.
pub struct ModelId;

impl ModelId {
    /// Build a canonical `provider/model` identifier.
    pub fn canonical(provider: &str, model: &str) -> String {
        format!("{provider}/{model}")
    }

    /// Extract the provider segment, accepting the canonical `provider/model` and
    /// the legacy `provider::model` forms.
    pub fn provider(model_id: &str) -> Option<&str> {
        model_id
            .split_once('/')
            .map(|(provider, _)| provider)
            .or_else(|| model_id.split_once("::").map(|(provider, _)| provider))
    }

    /// Extract the model-name segment, falling back to the whole id when it
    /// carries no provider prefix.
    pub fn name(model_id: &str) -> &str {
        model_id
            .split_once('/')
            .map(|(_, model)| model)
            .or_else(|| model_id.split_once("::").map(|(_, model)| model))
            .unwrap_or(model_id)
    }
}

/// A single message in a conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Message {
    /// Role: "system", "user", "assistant".
    pub role: String,
    /// Message text content.
    pub content: String,
    /// Image references for multimodal (VLM) requests — HTTP(S) URLs, `data:`
    /// URLs, or local file paths. Empty for text-only messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl Message {
    /// Create a new user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    /// Create a new system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            images: Vec::new(),
        }
    }

    /// Create a new assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            images: Vec::new(),
        }
    }
}

/// Named-model fallback behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    /// Capability requests can fail over; named requests are strict.
    #[default]
    CapabilityOnly,
    /// Never fail over.
    Disabled,
    /// Allow fallback even for named requests.
    AllowNamed,
}

/// Backend-locality preference for routing (privacy / data-residency guardrail).
///
/// Matches the PRD `prefer` request field: `auto | local | cloud`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    /// Default seven-dimensional scoring: local models are preferred but a cloud
    /// model can be selected or used as a fallback.
    #[default]
    Auto,
    /// Hard constraint: only local models may serve the request. If none can, the
    /// request fails rather than leaving the device (fail-not-fallback).
    #[serde(alias = "local_only")]
    Local,
    /// Hard constraint: only cloud models may serve the request.
    Cloud,
}

/// Extended-thinking effort for reasoning-capable models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Minimal deliberation — cheapest, routes to the lightest tier.
    Low,
    /// Balanced deliberation (default).
    #[default]
    Medium,
    /// Maximum deliberation — routes to the strongest reasoning tier.
    High,
}

impl ReasoningEffort {
    /// Parse a loose config/string value (`low`/`medium`/`high`, any case).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Data-sensitivity classification for a request (S5 privacy moat).
///
/// `Confidential` is a hard data-residency constraint: the request is pinned to
/// local backends regardless of `prefer`, and fails rather than sending sensitive
/// data to the cloud. Every request's effective locality is written to the audit
/// log so data flow is traceable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    /// No residency constraint (default).
    #[default]
    Public,
    /// Organization-internal; no hard constraint, but audited.
    Internal,
    /// Sensitive: must never leave the device (implies local-only routing).
    Confidential,
}

impl DataClass {
    /// Whether this class forbids sending the request to a cloud backend.
    pub fn requires_local(self) -> bool {
        matches!(self, Self::Confidential)
    }
}

/// Deep-thinking controls for a request (S2 Code/PR Review).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Reasoning {
    /// How much thinking effort to spend; also used for tier routing.
    #[serde(default)]
    pub effort: ReasoningEffort,
    /// Whether the thought chain should be surfaced (streamed as `reasoning`
    /// increments and returned) rather than stripped from the output.
    #[serde(default)]
    pub include: bool,
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
    /// Backend-locality preference / privacy guardrail (`auto | local | cloud`).
    #[serde(default, alias = "locality")]
    pub prefer: Prefer,
    /// Data-sensitivity class. `confidential` pins the request to local backends
    /// regardless of `prefer` (privacy moat).
    #[serde(default)]
    pub data_class: DataClass,
    /// Deep-thinking controls (effort tier + thought-chain visibility). `None`
    /// keeps standard behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    /// Per-request spend ceiling in USD. A candidate whose *estimated* cost
    /// exceeds this is skipped during candidate selection (and recorded in
    /// `failed_chain`), so spend stays bounded and cheaper/local models win. Free
    /// and local models estimate to `$0` and are always affordable; `0.0`
    /// therefore means "free/local only". `None` = no ceiling.
    ///
    /// This is a **soft** ceiling: it is enforced against a pre-flight token
    /// *estimate*, so a model that generates more than estimated can still exceed
    /// it. To bound actual spend, also cap generation via `params.max_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
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
    #[serde(default = "InferenceRequest::generate_request_id")]
    pub request_id: String,
}

impl Default for InferenceRequest {
    fn default() -> Self {
        Self {
            capability: None,
            model: None,
            app_id: None,
            session_id: None,
            fallback_policy: FallbackPolicy::default(),
            prefer: Prefer::default(),
            data_class: DataClass::default(),
            reasoning: None,
            max_cost_usd: None,
            messages: Vec::new(),
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: Self::generate_request_id(),
        }
    }
}

impl InferenceRequest {
    /// Generate a fresh unique request identifier. Also the serde default for
    /// `request_id`, so a deserialized request without one still gets a unique id.
    fn generate_request_id() -> String {
        Uuid::new_v4().to_string()
    }
}

/// A turn in the stateful Responses API (S2 multi-turn deep reasoning).
///
/// The engine stores each turn's full message history keyed by the returned
/// [`ResponsesResponse::id`], so a caller continues a conversation by passing
/// that id as `previous_response_id` on the next turn instead of resending the
/// whole history. All routing knobs (capability, model, `prefer`, `reasoning`,
/// `params`, …) come from the flattened [`InferenceRequest`], so a Responses
/// turn routes exactly like a one-shot `invoke`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesRequest {
    /// Continue the conversation stored under this prior response id. `None`
    /// starts a new conversation. An unknown id is a (non-retryable) error rather
    /// than a silent fresh start, so a caller notices an expired/evicted chain.
    #[serde(default)]
    pub previous_response_id: Option<String>,
    /// System instructions, applied only when *starting* a new conversation
    /// (ignored when continuing, since the stored history already carries them).
    #[serde(default)]
    pub instructions: Option<String>,
    /// Convenience shorthand for a single new user message this turn. Appended
    /// after any explicit `messages`.
    #[serde(default)]
    pub input: Option<String>,
    /// Routing knobs and any explicit `messages` for this turn, flattened so a
    /// Responses request is a superset of an [`InferenceRequest`] on the wire.
    #[serde(flatten)]
    pub request: InferenceRequest,
}

/// Result of a [`ResponsesRequest`] turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResponsesResponse {
    /// Id of *this* response. Pass it as `previous_response_id` to continue the
    /// conversation from here.
    pub id: String,
    /// Total messages now stored in the conversation (including this turn's reply).
    pub message_count: usize,
    /// The underlying inference result (text, tokens, cost, provider, routing),
    /// flattened so a Responses reply is a superset of an [`InferenceResponse`].
    #[serde(flatten)]
    pub response: InferenceResponse,
}

/// Response from an inference call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Text output (for chat, ASR, etc.).
    pub text: Option<String>,
    /// File output path (for TTS, image gen, video gen, etc.).
    pub file: Option<String>,
    /// Embedding vectors (for the `embedding` capability): one row per input, in
    /// input order. `None` for non-embedding responses so the field is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<Vec<f32>>>,
    /// Which model actually handled the request.
    pub model_used: String,
    /// Which provider served it.
    pub provider: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Request correlation ID.
    pub request_id: String,
    /// Total token usage, if reported by provider.
    pub tokens_used: Option<u32>,
    /// Prompt/input tokens, if reported by provider.
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    /// Completion/output tokens, if reported by provider.
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    /// Estimated cost in USD, if a price is configured for the serving provider.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// Whether the response came from a fallback candidate.
    #[serde(default)]
    pub fallback_used: bool,
    /// Machine-readable routing reason.
    pub routing_reason: Option<String>,
}

/// A single event in a streaming inference response.
///
/// The streaming interface is versioned and can operate in a *non-streaming
/// compatibility mode*: a backend that cannot emit incremental tokens sends
/// `Started`, one `Text` chunk carrying the full output, then `Completed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StreamChunk {
    /// Emitted once before any content; identifies the serving model.
    Started {
        /// Request correlation ID.
        request_id: String,
        /// Model actually serving the request.
        model_used: String,
        /// Provider serving the request.
        provider: String,
    },
    /// An incremental piece of text output.
    Text {
        /// Text delta appended to the output so far.
        delta: String,
    },
    /// An incremental piece of the model's thought chain (reasoning-capable
    /// models). Kept distinct from `Text` so callers can display or audit the
    /// thought chain separately from the final answer (S2).
    Reasoning {
        /// Thought-chain delta.
        delta: String,
    },
    /// Terminal success event with aggregate metadata and any file output.
    Completed {
        /// Wall-clock duration in milliseconds.
        duration_ms: u64,
        /// Total token usage, if reported.
        tokens_used: Option<u32>,
        /// Prompt/input tokens, if reported.
        #[serde(default)]
        prompt_tokens: Option<u32>,
        /// Completion/output tokens, if reported.
        #[serde(default)]
        completion_tokens: Option<u32>,
        /// Estimated cost in USD, if a price is configured for the provider.
        #[serde(default)]
        cost_usd: Option<f64>,
        /// File output path (for TTS, image gen, etc.).
        file: Option<String>,
        /// Whether a fallback candidate served the request.
        fallback_used: bool,
        /// Machine-readable routing reason.
        routing_reason: Option<String>,
    },
    /// Terminal error event.
    Error(ErrorInfo),
}

/// One incremental delta a provider pushes while streaming: either final-answer
/// text or a piece of the model's thought chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamDelta {
    /// A piece of the final answer.
    Text(String),
    /// A piece of the thought chain (reasoning-capable models).
    Reasoning(String),
}

impl StreamDelta {
    /// Borrow the delta text regardless of kind.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s,
        }
    }
}

/// Channel a provider uses to emit incremental deltas while streaming.
///
/// The engine owns the surrounding envelope (`Started`/`Completed`/`Error`);
/// providers push [`StreamDelta`] items here and return the final aggregate
/// response. `Text` deltas become `StreamChunk::Text`, `Reasoning` deltas become
/// `StreamChunk::Reasoning`.
pub type StreamSink = tokio::sync::mpsc::Sender<StreamDelta>;

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
    /// A model was evicted from local memory.
    ModelEvicted {
        /// Model identifier.
        model_id: String,
        /// Why it was evicted, e.g. `memory_pressure` or `idle_timeout`.
        reason: String,
    },
    /// Predictive (Preflight) warming started for a model.
    PreflightWarmStarted {
        /// Model identifier being warmed.
        model_id: String,
        /// What triggered the warm: `hint`, `subscription`, or `history`.
        source: String,
    },
    /// Predictive (Preflight) warming finished.
    PreflightWarmCompleted {
        /// Model identifier that was warmed.
        model_id: String,
        /// Whether the warm succeeded.
        success: bool,
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
        assert_eq!(ModelId::canonical("ollama", "qwen"), "ollama/qwen");
        assert_eq!(ModelId::name("ollama/qwen"), "qwen");
        assert_eq!(ModelId::name("ollama::qwen"), "qwen");
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

    #[test]
    fn message_images_are_backward_compatible() {
        // A legacy text-only message (no `images`) still deserializes, and a
        // text-only message serializes without an `images` field.
        let m: Message = serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
        assert!(m.images.is_empty());
        assert!(!serde_json::to_string(&m).unwrap().contains("images"));

        // A multimodal message round-trips its image references.
        let mm: Message = serde_json::from_str(
            r#"{"role":"user","content":"what is this?","images":["https://x/a.png"]}"#,
        )
        .unwrap();
        assert_eq!(mm.images, vec!["https://x/a.png"]);
    }

    #[test]
    fn prefer_and_reasoning_deserialize_from_contract() {
        // PRD wire contract: `prefer: local` + `reasoning: { effort: high }`.
        let req: InferenceRequest = serde_json::from_str(
            r#"{"prefer":"local","reasoning":{"effort":"high","include":true}}"#,
        )
        .unwrap();
        assert_eq!(req.prefer, Prefer::Local);
        let r = req.reasoning.unwrap();
        assert_eq!(r.effort, ReasoningEffort::High);
        assert!(r.include);
        // Legacy alias still accepted.
        let legacy: InferenceRequest =
            serde_json::from_str(r#"{"locality":"local_only"}"#).unwrap();
        assert_eq!(legacy.prefer, Prefer::Local);
    }

    #[test]
    fn data_class_defaults_public_and_confidential_requires_local() {
        let default: InferenceRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(default.data_class, DataClass::Public);
        assert!(!default.data_class.requires_local());

        let sensitive: InferenceRequest =
            serde_json::from_str(r#"{"data_class":"confidential"}"#).unwrap();
        assert_eq!(sensitive.data_class, DataClass::Confidential);
        assert!(sensitive.data_class.requires_local());
    }

    #[test]
    fn reasoning_stream_chunk_has_distinct_tag() {
        let chunk = StreamChunk::Reasoning {
            delta: "thinking".into(),
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(json.contains(r#""type":"reasoning""#));
        assert!(json.contains(r#""delta":"thinking""#));
    }
}
