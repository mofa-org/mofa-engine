//! Error types for the MoFA Engine.

use thiserror::Error;

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
        /// Which provider failed
        provider: String,
        /// Error detail
        detail: String,
    },

    /// The circuit breaker for a provider is open.
    #[error("circuit open for provider: {0}")]
    CircuitOpen(String),

    /// Not enough memory to load the requested model.
    #[error("memory pressure: need {need} bytes, only {available} available")]
    MemoryPressure {
        /// Bytes needed
        need: u64,
        /// Bytes available
        available: u64,
    },

    /// The request was malformed or missing required fields.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// The request timed out.
    #[error("timeout: {0}")]
    Timeout(String),

    /// An internal/unexpected error.
    #[error("internal error: {0}")]
    Internal(String),
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
}
