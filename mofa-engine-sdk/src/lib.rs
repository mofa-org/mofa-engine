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
pub mod observability_bridge;
pub mod server;

// Internal: the embedded dashboard HTML, served by `server`.
pub(crate) mod dashboard;

pub use client::{ClientError, DaemonClient, EmbeddedEngine};
pub use server::Server;
