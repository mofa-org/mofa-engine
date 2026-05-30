//! mofa-engine-core: Engine internals — scheduling, memory management, model lifecycle, preflight.

pub mod config;
pub mod engine;
pub mod memory_manager;
pub mod scheduler;
pub mod backends;
pub mod discovery;
pub mod preflight;

pub use engine::Engine;
pub use config::EngineConfig;
