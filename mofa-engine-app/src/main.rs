//! MoFA Engine — multimodal AI model orchestration engine.
//!
//! Binary entry point. Parses CLI arguments, loads configuration,
//! initialises the engine, and starts the HTTP server.

use clap::Parser;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_engine_sdk::start_server;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

/// MoFA Engine — multimodal AI model orchestration
#[derive(Parser, Debug)]
#[command(name = "mofa-engine", version, about)]
struct Cli {
    /// Path to config.toml (default: auto-detect)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Override listen port
    #[arg(short, long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise tracing
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Load configuration
    let mut config = EngineConfig::load(cli.config.as_deref());

    // CLI port override
    if let Some(port) = cli.port {
        config.listen.port = port;
    }

    let host = config.listen.host.clone();
    let port = config.listen.port;

    tracing::info!(
        "MoFA Engine v{} starting",
        env!("CARGO_PKG_VERSION")
    );

    // Create engine
    let engine = Engine::new(config).await;

    // Start server
    start_server(engine, &host, port)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}
