//! # mofa-kernel
//!
//! Trait definitions and core types for the MoFA Engine.
//! This crate contains **no implementations** — only the contracts
//! that providers and the engine must satisfy.

pub mod types;
pub mod traits;
pub mod error;

pub use error::EngineError;
pub use traits::Provider;
pub use types::*;
