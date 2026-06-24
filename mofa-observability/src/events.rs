//! # MoFA Engine — Event Type Definitions
//!
//! Structured event types for the observability subsystem.
//! Every interesting thing the engine does is captured as one of these events.
//!
//! Design principles:
//! - Zero side effects: events are data, not actions.
//! - Privacy: no prompt text, file contents, API keys, or user-identifying info. Ever.
//! - Bounded enums: capability, reason, source, status use enums, not free-form strings.
//!
//! Reference: observability_plan.md §3

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ─── Shared Enums ────────────────────────────────────────────────────────────

/// What the caller asked for. Bounded enum — no free-form strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Chat,
    Tts,
    Asr,
    Vlm,
    ImageGen,
    VideoGen,
    Embedding,
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Capability::Chat => write!(f, "chat"),
            Capability::Tts => write!(f, "tts"),
            Capability::Asr => write!(f, "asr"),
            Capability::Vlm => write!(f, "vlm"),
            Capability::ImageGen => write!(f, "image_gen"),
            Capability::VideoGen => write!(f, "video_gen"),
            Capability::Embedding => write!(f, "embedding"),
        }
    }
}

/// Why a model was unloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnloadReason {
    /// Ollama's keep_alive timer expired.
    IdleTimeout,
    /// Memory pressure forced eviction.
    Eviction,
    /// Caller explicitly requested unload.
    Explicit,
}

impl std::fmt::Display for UnloadReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnloadReason::IdleTimeout => write!(f, "idle_timeout"),
            UnloadReason::Eviction => write!(f, "eviction"),
            UnloadReason::Explicit => write!(f, "explicit"),
        }
    }
}

/// Where the Preflight prediction came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalSource {
    /// App explicitly said what comes next. Confidence = 1.0.
    Hint,
    /// Learned from past session transitions. Confidence < 1.0.
    History,
}

impl std::fmt::Display for SignalSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalSource::Hint => write!(f, "hint"),
            SignalSource::History => write!(f, "history"),
        }
    }
}

// ─── Event Envelope ──────────────────────────────────────────────────────────

/// Every event carries this standard envelope.
/// Reference: observability_plan.md §3.1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Correlation ID — ties events from the same request together.
    pub request_id: Option<String>,
    /// App or user session (if provided by caller).
    pub session_id: Option<String>,
    /// OpenTelemetry trace ID (128-bit hex string).
    pub trace_id: Option<String>,
    /// OpenTelemetry span ID (64-bit hex string).
    pub span_id: Option<String>,
    /// OpenTelemetry span end timestamp (Unix ms).
    pub span_end_timestamp: Option<u64>,
    /// The event payload.
    pub event: EngineEvent,
}

impl EventEnvelope {
    /// Create an envelope with the current timestamp.
    pub fn now(event: EngineEvent) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            timestamp_ms,
            request_id: None,
            session_id: None,
            trace_id: None,
            span_id: None,
            span_end_timestamp: None,
            event,
        }
    }

    /// Attach a request ID.
    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    /// Attach a session ID.
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Attach OpenTelemetry tracing context.
    pub fn with_trace(mut self, trace_id: impl Into<String>, span_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self.span_id = Some(span_id.into());
        self
    }

    /// Set span end timestamp.
    pub fn with_span_end(mut self, timestamp_ms: u64) -> Self {
        self.span_end_timestamp = Some(timestamp_ms);
        self
    }
}

// ─── Engine Event (discriminated union) ──────────────────────────────────────

/// All possible engine events. The collector pattern-matches on this.
/// Reference: observability_plan.md §3.2
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    // ── Request Events ───────────────────────────────────────────────────
    RequestReceived(RequestReceived),
    RoutingDecision(RoutingDecision),
    RequestCompleted(RequestCompleted),

    // ── Model Lifecycle Events ───────────────────────────────────────────
    ModelLoaded(ModelLoaded),
    ModelUnloaded(ModelUnloaded),

    // ── Memory Events ────────────────────────────────────────────────────
    EvictionTriggered(EvictionTriggered),

    // ── Preflight Events ─────────────────────────────────────────────────
    PreflightSignal(PreflightSignal),
    PreflightHit(PreflightHit),
    PreflightMiss(PreflightMiss),

    // ── Infrastructure Events ────────────────────────────────────────────
    ProviderDiscovered(ProviderDiscovered),
    FailoverTriggered(FailoverTriggered),
}

// ─── Request Events ──────────────────────────────────────────────────────────

/// Emitted when the engine accepts a new inference request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestReceived {
    /// What the caller asked for.
    pub capability: Capability,
    /// Specific model name if the caller used named routing.
    pub model: Option<String>,
    /// The `hint_next` value if provided.
    pub hint: Option<String>,
}

/// Emitted after the router selects a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Requested capability.
    pub capability: Capability,
    /// How many models were eligible.
    pub candidates_count: u32,
    /// The model chosen.
    pub selected_model: String,
    /// The backend hosting that model.
    pub selected_backend: String,
    /// Whether this was a fallback selection.
    pub is_fallback: bool,
    /// Why this model was chosen (e.g., "local_first", "only_candidate", "fallback_after_failure").
    pub reason: String,
}

/// Emitted when inference finishes (success or failure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestCompleted {
    /// Which model ran.
    pub model_id: String,
    /// Which backend.
    pub backend: String,
    /// What capability was served.
    pub capability: Capability,
    /// Total request time in milliseconds.
    pub duration_ms: u64,
    /// Time to first token in milliseconds (for streaming).
    pub ttft_ms: Option<u64>,
    /// Input token count (text models).
    pub tokens_in: Option<u64>,
    /// Output token count (text models).
    pub tokens_out: Option<u64>,
    /// Was the model already loaded when the request arrived?
    /// Populated by the engine. Enables warm vs cold latency split.
    pub model_was_hot: Option<bool>,
    /// Whether it succeeded.
    pub success: bool,
    /// Error code if failed.
    pub error_code: Option<String>,
}

// ─── Model Lifecycle Events ──────────────────────────────────────────────────

/// A model finished loading into memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLoaded {
    /// Which model.
    pub model_id: String,
    /// Which backend.
    pub backend: String,
    /// What capability.
    pub capability: Capability,
    /// How long the load took in milliseconds.
    pub load_duration_ms: u64,
    /// How much memory it occupies in bytes.
    pub memory_bytes: u64,
}

/// A model was removed from memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUnloaded {
    /// Which model.
    pub model_id: String,
    /// Why it was unloaded.
    pub reason: UnloadReason,
    /// How much memory was released in bytes.
    pub memory_freed_bytes: u64,
}

// ─── Memory Events ───────────────────────────────────────────────────────────

/// Memory pressure forced a model eviction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvictionTriggered {
    /// Which model was evicted.
    pub evicted_model: String,
    /// Used memory before eviction in bytes.
    pub memory_before_bytes: u64,
    /// Used memory after eviction in bytes.
    pub memory_after_bytes: u64,
    /// Total memory budget in bytes.
    pub budget_bytes: u64,
}

// ─── Preflight Events ────────────────────────────────────────────────────────

/// Preflight made a prediction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightSignal {
    /// What Preflight thinks comes next.
    pub predicted_capability: Capability,
    /// 0.0–1.0 (hints are always 1.0).
    pub confidence: f64,
    /// Where the prediction came from.
    pub source: SignalSource,
}

/// The prediction was correct — the predicted model was used next.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightHit {
    /// What was predicted.
    pub predicted_capability: Capability,
    /// How much load time was saved in milliseconds.
    /// Computed by engine/scheduler, not Preflight.
    pub cold_start_avoided_ms: u64,
}

/// The prediction was wrong — a different capability was requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightMiss {
    /// What was predicted.
    pub predicted_capability: Capability,
    /// What actually happened.
    pub actual_capability: Capability,
}

// ─── Infrastructure Events ───────────────────────────────────────────────────

/// A backend was found during discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDiscovered {
    /// Backend name.
    pub provider_name: String,
    /// How many models were discovered.
    pub models_found: u32,
    /// What capabilities are available.
    pub capabilities: Vec<Capability>,
}

/// Primary model failed, switching to fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverTriggered {
    /// The model that failed.
    pub failed_model: String,
    /// Its backend.
    pub failed_backend: String,
    /// The replacement model.
    pub fallback_model: String,
    /// Its backend.
    pub fallback_backend: String,
}

// ─── Privacy Contract ────────────────────────────────────────────────────────
//
// These fields are NEVER present in any event:
//   - Prompt text or generated text
//   - File contents or file paths
//   - API keys, tokens, or credentials
//   - User-identifying information
//
// All categorical fields use bounded enums, not free-form strings:
//   - capability → Capability enum (7 variants)
//   - reason → UnloadReason enum (3 variants)
//   - source → SignalSource enum (2 variants)
//   - status → bool (success/failure)
//
// Reference: observability_plan.md §3.3

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_display() {
        assert_eq!(Capability::Chat.to_string(), "chat");
        assert_eq!(Capability::Tts.to_string(), "tts");
        assert_eq!(Capability::ImageGen.to_string(), "image_gen");
    }

    #[test]
    fn test_unload_reason_display() {
        assert_eq!(UnloadReason::IdleTimeout.to_string(), "idle_timeout");
        assert_eq!(UnloadReason::Eviction.to_string(), "eviction");
        assert_eq!(UnloadReason::Explicit.to_string(), "explicit");
    }

    #[test]
    fn test_signal_source_display() {
        assert_eq!(SignalSource::Hint.to_string(), "hint");
        assert_eq!(SignalSource::History.to_string(), "history");
    }

    #[test]
    fn test_envelope_creation() {
        let event = EngineEvent::RequestReceived(RequestReceived {
            capability: Capability::Chat,
            model: None,
            hint: Some("tts".into()),
        });

        let envelope = EventEnvelope::now(event)
            .with_request_id("req-001")
            .with_session_id("sess-001");

        assert!(envelope.timestamp_ms > 0);
        assert_eq!(envelope.request_id.as_deref(), Some("req-001"));
        assert_eq!(envelope.session_id.as_deref(), Some("sess-001"));
    }

    #[test]
    fn test_request_completed_with_model_was_hot() {
        let event = RequestCompleted {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            duration_ms: 15000,
            ttft_ms: Some(115),
            tokens_in: Some(50),
            tokens_out: Some(277),
            model_was_hot: Some(true),
            success: true,
            error_code: None,
        };

        // Warm TTFT should be ~0.1s (Phase 0 measured 0.115s)
        assert_eq!(event.model_was_hot, Some(true));
        assert!(event.ttft_ms.unwrap() < 200); // warm is fast
    }

    #[test]
    fn test_preflight_signal_hint() {
        let signal = PreflightSignal {
            predicted_capability: Capability::Tts,
            confidence: 1.0,
            source: SignalSource::Hint,
        };

        // Phase 0 proved: hints have accuracy 1.0
        assert_eq!(signal.confidence, 1.0);
        assert_eq!(signal.source, SignalSource::Hint);
    }

    #[test]
    fn test_preflight_hit() {
        let hit = PreflightHit {
            predicted_capability: Capability::Tts,
            // Phase 0 measured: Kokoro TTS saving = 2404ms on Mac M4
            cold_start_avoided_ms: 2404,
        };

        assert_eq!(hit.predicted_capability, Capability::Tts);
        assert!(hit.cold_start_avoided_ms > 0);
    }

    #[test]
    fn test_preflight_miss() {
        let miss = PreflightMiss {
            predicted_capability: Capability::Tts,
            actual_capability: Capability::Asr,
        };

        assert_ne!(miss.predicted_capability, miss.actual_capability);
    }

    #[test]
    fn test_model_loaded_event() {
        let loaded = ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            // Phase 0 measured: 1532ms cold load on Mac M4
            load_duration_ms: 1532,
            // Phase 0 measured: 4.7 GB
            memory_bytes: 4_700_000_000,
        };

        assert!(loaded.load_duration_ms > 0);
        assert!(loaded.memory_bytes > 0);
    }

    #[test]
    fn test_eviction_event() {
        let eviction = EvictionTriggered {
            evicted_model: "gemma3:4b".into(),
            memory_before_bytes: 8_000_000_000,
            memory_after_bytes: 4_700_000_000,
            budget_bytes: 11_000_000_000,
        };

        assert!(eviction.memory_after_bytes < eviction.memory_before_bytes);
        assert!(eviction.memory_after_bytes < eviction.budget_bytes);
    }

    #[test]
    fn test_event_serialization_roundtrip() {
        let original = EventEnvelope::now(EngineEvent::PreflightSignal(PreflightSignal {
            predicted_capability: Capability::Tts,
            confidence: 1.0,
            source: SignalSource::Hint,
        }))
        .with_request_id("req-042");

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: EventEnvelope = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(original.timestamp_ms, restored.timestamp_ms);
        assert_eq!(original.request_id, restored.request_id);
    }

    #[test]
    fn test_all_event_variants_serialize() {
        // Every variant must serialize without panic.
        let events: Vec<EngineEvent> = vec![
            EngineEvent::RequestReceived(RequestReceived {
                capability: Capability::Chat,
                model: None,
                hint: None,
            }),
            EngineEvent::RoutingDecision(RoutingDecision {
                capability: Capability::Chat,
                candidates_count: 2,
                selected_model: "qwen2.5:7b".into(),
                selected_backend: "ollama".into(),
                is_fallback: false,
                reason: "local_first".into(),
            }),
            EngineEvent::RequestCompleted(RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: Some(115),
                tokens_in: Some(50),
                tokens_out: Some(277),
                model_was_hot: Some(true),
                success: true,
                error_code: None,
            }),
            EngineEvent::ModelLoaded(ModelLoaded {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                load_duration_ms: 1532,
                memory_bytes: 4_700_000_000,
            }),
            EngineEvent::ModelUnloaded(ModelUnloaded {
                model_id: "gemma3:4b".into(),
                reason: UnloadReason::IdleTimeout,
                memory_freed_bytes: 3_300_000_000,
            }),
            EngineEvent::EvictionTriggered(EvictionTriggered {
                evicted_model: "gemma3:4b".into(),
                memory_before_bytes: 8_000_000_000,
                memory_after_bytes: 4_700_000_000,
                budget_bytes: 11_000_000_000,
            }),
            EngineEvent::PreflightSignal(PreflightSignal {
                predicted_capability: Capability::Tts,
                confidence: 1.0,
                source: SignalSource::Hint,
            }),
            EngineEvent::PreflightHit(PreflightHit {
                predicted_capability: Capability::Tts,
                cold_start_avoided_ms: 2404,
            }),
            EngineEvent::PreflightMiss(PreflightMiss {
                predicted_capability: Capability::Tts,
                actual_capability: Capability::Chat,
            }),
            EngineEvent::ProviderDiscovered(ProviderDiscovered {
                provider_name: "ollama".into(),
                models_found: 3,
                capabilities: vec![Capability::Chat, Capability::Vlm],
            }),
            EngineEvent::FailoverTriggered(FailoverTriggered {
                failed_model: "qwen2.5:7b".into(),
                failed_backend: "ollama".into(),
                fallback_model: "gemma3:4b".into(),
                fallback_backend: "ollama".into(),
            }),
        ];

        for event in events {
            let envelope = EventEnvelope::now(event);
            let json = serde_json::to_string(&envelope);
            assert!(json.is_ok(), "Failed to serialize: {:?}", envelope);
        }
    }
}
