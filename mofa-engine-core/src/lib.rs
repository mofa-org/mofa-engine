//! # mofa-engine-core
//!
//! Engine internals: configuration, provider backends, routing,
//! memory management, circuit breaker, preflight prediction,
//! and the main `Engine` orchestrator.

pub mod config;
pub mod backends;
pub mod router;
pub mod memory;
pub mod circuit_breaker;
pub mod preflight;
pub mod engine;

pub use config::EngineConfig;
pub use engine::Engine;
