//! # mofa-engine-sdk
//!
//! HTTP API server (Axum), SSE event streaming, and embedded dashboard
//! for the MoFA Engine.

pub mod server;
pub mod dashboard;

pub use server::start_server;
