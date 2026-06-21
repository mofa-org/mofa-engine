//! # mofa-engine-sdk
//!
//! HTTP API server (Axum), SSE event streaming, and embedded dashboard
//! for the MoFA Engine.

pub mod dashboard;
pub mod server;

pub use server::start_server;
