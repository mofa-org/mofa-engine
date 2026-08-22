//! # mofa-kernel
//!
//! Trait definitions and core types for the MoFA Engine.
//! This crate contains **no implementations** — only the contracts
//! that providers and the engine must satisfy.

pub mod error;
pub mod traits;
pub mod types;

pub use error::{EngineError, ErrorCode, ErrorInfo, FailedAttempt};
pub use traits::Provider;
pub use types::*;
