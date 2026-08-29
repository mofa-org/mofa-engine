//! Error types for the MoFA Engine.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable machine-readable error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No model supports the requested constraints.
    NoCapableModel,
    /// A provider failed while handling a request.
    ProviderError,
    /// The circuit breaker is open.
    CircuitOpen,
    /// Local memory budget cannot admit the request.
    MemoryPressure,
    /// The request is malformed.
    InvalidRequest,
    /// The request timed out.
    Timeout,
    /// Backend does not support the requested operation.
    UnsupportedOperation,
    /// Configuration is invalid.
    Config,
    /// Internal error.
    Internal,
}

/// One failed candidate in a failover chain: which provider/model was tried and
/// why it failed. Lets an Agent see *all* attempts, not just the last error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedAttempt {
    pub provider: String,
    /// Canonical short name.
    pub model: String,
    pub reason: String,
}

/// Structured error body suitable for HTTP and SDKs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: ErrorCode,
    pub message: String,
    /// Whether retrying the same request may succeed.
    pub retryable: bool,
    /// Source provider/backend.
    pub source: Option<String>,
    /// Complete per-candidate failure chain (empty unless failover was attempted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_chain: Vec<FailedAttempt>,
    /// Machine-readable routing reason for the primary candidate, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_reason: Option<String>,
}

/// Top-level engine error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// No model available that supports the requested capability.
    #[error("no capable model: {0}")]
    NoCapableModel(String),

    /// A provider returned an error.
    #[error("provider '{provider}' error: {detail}")]
    ProviderError {
        provider: String,
        detail: String,
    },

    /// The circuit breaker for a provider is open.
    #[error("circuit open for provider: {0}")]
    CircuitOpen(String),

    /// Not enough memory to load the requested model.
    #[error("memory pressure: need {need} bytes, only {available} available")]
    MemoryPressure {
        need: u64,
        available: u64,
    },

    /// The request was malformed or missing required fields.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The request timed out.
    #[error("timeout: {0}")]
    Timeout(String),

    /// The backend does not support the requested operation.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// Configuration is invalid.
    #[error("config error: {0}")]
    Config(String),

    /// An internal/unexpected error.
    #[error("internal error: {0}")]
    Internal(String),

    /// Every routing candidate was exhausted during failover. Carries the full
    /// per-candidate failure chain so an Agent can decide whether to retry or
    /// switch tiers, rather than seeing only the last error.
    #[error("all candidates failed: {message}")]
    Failover {
        /// Code of the final (last-tried) failure.
        code: ErrorCode,
        /// Aggregate human-readable message (the last failure's message).
        message: String,
        /// Whether retrying the whole request might succeed.
        retryable: bool,
        /// Per-candidate failures, in attempt order.
        chain: Vec<FailedAttempt>,
        /// Machine-readable routing reason for the primary candidate.
        routing_reason: Option<String>,
    },
}

impl EngineError {
    /// Stable machine-readable code for this error.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NoCapableModel(_) => ErrorCode::NoCapableModel,
            Self::ProviderError { .. } => ErrorCode::ProviderError,
            Self::CircuitOpen(_) => ErrorCode::CircuitOpen,
            Self::MemoryPressure { .. } => ErrorCode::MemoryPressure,
            Self::InvalidRequest(_) => ErrorCode::InvalidRequest,
            Self::Timeout(_) => ErrorCode::Timeout,
            Self::UnsupportedOperation(_) => ErrorCode::UnsupportedOperation,
            Self::Config(_) => ErrorCode::Config,
            Self::Internal(_) => ErrorCode::Internal,
            Self::Failover { code, .. } => *code,
        }
    }

    /// Whether retrying the same request may succeed.
    pub fn retryable(&self) -> bool {
        match self {
            Self::ProviderError { .. }
            | Self::CircuitOpen(_)
            | Self::MemoryPressure { .. }
            | Self::Timeout(_)
            | Self::Internal(_) => true,
            Self::Failover { retryable, .. } => *retryable,
            _ => false,
        }
    }

    /// Optional source provider/backend for this error.
    pub fn source_name(&self) -> Option<&str> {
        match self {
            Self::ProviderError { provider, .. } | Self::CircuitOpen(provider) => Some(provider),
            _ => None,
        }
    }

    /// Convert to a structured error body.
    pub fn info(&self) -> ErrorInfo {
        if let Self::Failover {
            code,
            message,
            retryable,
            chain,
            routing_reason,
        } = self
        {
            return ErrorInfo {
                code: *code,
                message: message.clone(),
                retryable: *retryable,
                source: chain.last().map(|a| a.provider.clone()),
                failed_chain: chain.clone(),
                routing_reason: routing_reason.clone(),
            };
        }
        ErrorInfo {
            code: self.code(),
            message: self.to_string(),
            retryable: self.retryable(),
            source: self.source_name().map(ToOwned::to_owned),
            failed_chain: Vec::new(),
            routing_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = EngineError::NoCapableModel("tts".into());
        assert!(e.to_string().contains("tts"));

        let e = EngineError::MemoryPressure {
            need: 1024,
            available: 512,
        };
        assert!(e.to_string().contains("1024"));
    }

    #[test]
    fn error_info_is_stable() {
        let e = EngineError::CircuitOpen("ollama".into());
        let info = e.info();
        assert_eq!(info.code, ErrorCode::CircuitOpen);
        assert!(info.retryable);
        assert_eq!(info.source.as_deref(), Some("ollama"));
        // Non-failover errors carry an empty chain that is omitted from JSON.
        assert!(info.failed_chain.is_empty());
        assert!(
            !serde_json::to_string(&info)
                .unwrap()
                .contains("failed_chain")
        );
    }

    #[test]
    fn failover_info_carries_full_chain() {
        let e = EngineError::Failover {
            code: ErrorCode::ProviderError,
            message: "provider 'openai' error: 503".into(),
            retryable: true,
            chain: vec![
                FailedAttempt {
                    provider: "ollama".into(),
                    model: "qwen".into(),
                    reason: "circuit open".into(),
                },
                FailedAttempt {
                    provider: "openai".into(),
                    model: "gpt-4o".into(),
                    reason: "503".into(),
                },
            ],
            routing_reason: Some("resident local preferred".into()),
        };
        let info = e.info();
        assert_eq!(info.code, ErrorCode::ProviderError);
        assert!(info.retryable);
        assert_eq!(info.failed_chain.len(), 2);
        assert_eq!(info.failed_chain[0].provider, "ollama");
        assert_eq!(info.failed_chain[1].model, "gpt-4o");
        assert_eq!(
            info.routing_reason.as_deref(),
            Some("resident local preferred")
        );
        // `source` points at the last-tried provider.
        assert_eq!(info.source.as_deref(), Some("openai"));

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("failed_chain"));
        assert!(json.contains("routing_reason"));
    }
}
