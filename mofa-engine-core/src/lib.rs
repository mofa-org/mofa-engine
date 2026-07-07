//! # mofa-engine-core
//!
//! Engine internals: configuration, provider backends, routing,
//! memory management, circuit breaker, preflight prediction,
//! and the main `Engine` orchestrator.

pub mod artifacts;
pub mod backends;
pub mod circuit_breaker;
pub mod config;
pub mod engine;
pub mod memory;
pub mod metrics;
pub mod preflight;
pub mod router;
pub mod subscription;

pub use config::EngineConfig;
pub use engine::Engine;
