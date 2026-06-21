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

/// Structured error body suitable for HTTP and SDKs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// Stable code.
    pub code: ErrorCode,
    /// Human-readable message.
    pub message: String,
    /// Whether retrying the same request may succeed.
    pub retryable: bool,
    /// Optional source provider/backend.
    pub source: Option<String>,
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
        /// Which provider failed.
        provider: String,
        /// Error detail.
        detail: String,
    },

    /// The circuit breaker for a provider is open.
    #[error("circuit open for provider: {0}")]
    CircuitOpen(String),

    /// Not enough memory to load the requested model.
    #[error("memory pressure: need {need} bytes, only {available} available")]
    MemoryPressure {
        /// Bytes needed.
        need: u64,
        /// Bytes available.
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
        }
    }

    /// Whether retrying the same request may succeed.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::ProviderError { .. }
                | Self::CircuitOpen(_)
                | Self::MemoryPressure { .. }
                | Self::Timeout(_)
                | Self::Internal(_)
        )
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
        ErrorInfo {
            code: self.code(),
            message: self.to_string(),
            retryable: self.retryable(),
            source: self.source_name().map(ToOwned::to_owned),
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
    }
}
