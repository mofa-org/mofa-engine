//! # mofa-engine-sdk
//!
//! The unified call layer for the MoFA Engine:
//!
//! - [`server`] — the versioned Axum HTTP API, SSE streaming, and dashboard.
//! - [`client`] — a native Rust SDK with an embedded ([`client::EmbeddedEngine`])
//!   and a daemon ([`client::DaemonClient`]) mode.
//!
//! The embedded facade is synchronous and intended as the UniFFI target for
//! Python bindings; the daemon client speaks the same HTTP surface the server
//! exposes.

pub mod client;
pub mod server;

// Internal: the embedded dashboard HTML, served by `server`.
pub(crate) mod dashboard;

pub use client::{AsyncEmbeddedEngine, ClientError, DaemonClient, EmbeddedEngine};
pub use server::Server;

// Re-export the config type callers need to construct an embedded engine, so an
// app can depend on `mofa-engine-sdk` alone for engine access instead of also
// reaching into `mofa-engine-core`.
pub use mofa_engine_core::EngineConfig;
