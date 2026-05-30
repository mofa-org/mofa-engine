//! Engine error types.

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("backend not available: {0}")]
    BackendNotAvailable(String),
    #[error("insufficient memory: need {need} bytes, available {available} bytes")]
    InsufficientMemory { need: u64, available: u64 },
    #[error("model load failed: {0}")]
    LoadFailed(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("backend error: {0}")]
    BackendError(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("config error: {0}")]
    ConfigError(String),
    #[error("internal error: {0}")]
    Internal(String),
}
