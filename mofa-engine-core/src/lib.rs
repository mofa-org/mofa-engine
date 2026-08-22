//! # mofa-engine-core
//!
//! Engine internals: configuration, provider backends, routing,
//! memory management, circuit breaker, preflight prediction,
//! and the main `Engine` orchestrator.

// Crate-internal machinery: reachable across the crate but not part of the
// published surface, so it can evolve freely.
pub(crate) mod artifacts;
pub(crate) mod backends;
pub(crate) mod circuit_breaker;
pub(crate) mod memory;
pub(crate) mod metrics;
pub(crate) mod router;

// Public surface consumed by the SDK, the app, and examples.
pub mod config;
pub mod engine;
pub mod preflight;
pub mod quality_gate;
pub mod subscription;

pub use config::EngineConfig;
pub use engine::Engine;
