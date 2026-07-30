//! MoFA Engine — multimodal AI model orchestration engine.
//!
//! Binary entry point. Runs the HTTP server by default, and offers CLI
//! subcommands (`status`, `capabilities`, `invoke`, `refresh`,
//! `validate-config`) that talk to a running daemon over the `/v1` API.

use clap::{Parser, Subcommand};
use mofa_engine_core::{Engine, EngineConfig};
use mofa_engine_sdk::{DaemonClient, Server};
use mofa_kernel::{Capability, InferenceRequest, Message};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, prelude::*};

/// MoFA Engine — multimodal AI model orchestration
#[derive(Parser, Debug)]
#[command(name = "mofa-engine", version, about)]
struct Cli {
    /// Path to config.toml (default: auto-detect)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Override listen port (serve mode)
    #[arg(short, long)]
    port: Option<u16>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Base URL argument shared by the daemon-facing subcommands.
#[derive(clap::Args, Debug)]
struct DaemonArgs {
    /// Base URL of the running engine daemon.
    #[arg(long, default_value = "http://127.0.0.1:8420")]
    url: String,
    /// Bearer token for a secured daemon (defaults to $MOFA_API_TOKEN).
    #[arg(long)]
    token: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the HTTP server (default).
    Serve,
    /// Validate the configuration and exit.
    ValidateConfig,
    /// List a running daemon's capabilities.
    Capabilities(DaemonArgs),
    /// Show a running daemon's status.
    Status(DaemonArgs),
    /// Re-run discovery on a running daemon.
    Refresh(DaemonArgs),
    /// Invoke a model on a running daemon.
    Invoke {
        #[command(flatten)]
        daemon: DaemonArgs,
        /// Capability to request (e.g. chat, tts).
        #[arg(long)]
        capability: Option<String>,
        /// Specific model name to request.
        #[arg(long)]
        model: Option<String>,
        /// Text input.
        #[arg(long)]
        text: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Cli::init_tracing();
    Cli::parse().run().await
}

impl Cli {
    /// Dispatch the parsed command to its handler. Serve/validate run in-process;
    /// the remaining subcommands act on a running daemon over the `/v1` API.
    async fn run(self) -> anyhow::Result<()> {
        let Cli {
            config,
            port,
            command,
        } = self;
        match command {
            None | Some(Command::Serve) => Self::serve(config, port).await,
            Some(Command::ValidateConfig) => Self::validate_config(config),
            Some(Command::Capabilities(d)) => {
                Self::print_json(Self::daemon_client(d).capabilities().await)
            }
            Some(Command::Status(d)) => Self::print_json(Self::daemon_client(d).status().await),
            Some(Command::Refresh(d)) => Self::print_json(Self::daemon_client(d).refresh().await),
            Some(Command::Invoke {
                daemon,
                capability,
                model,
                text,
            }) => {
                let request = InferenceRequest {
                    capability: capability.as_deref().and_then(Capability::from_str_loose),
                    model,
                    messages: vec![Message {
                        role: "user".into(),
                        content: text,
                        ..Default::default()
                    }],
                    request_id: String::new(),
                    ..Default::default()
                };
                Self::print_json(Self::daemon_client(daemon).invoke(&request).await)
            }
        }
    }

    /// Initialise tracing. Set `MOFA_LOG_FORMAT=json` for structured JSON logs
    /// (suitable for aggregation); otherwise human-readable text. Levels honour
    /// `RUST_LOG`.
    fn init_tracing() {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let json = std::env::var("MOFA_LOG_FORMAT")
            .map(|v| v.eq_ignore_ascii_case("json"))
            .unwrap_or(false);

        let registry = tracing_subscriber::registry().with(filter);
        if json {
            registry
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false),
                )
                .init();
        } else {
            registry
                .with(tracing_subscriber::fmt::layer().with_target(false))
                .init();
        }
    }

    /// Build a daemon client from the shared args, taking the bearer token from
    /// `--token` or, failing that, `$MOFA_API_TOKEN`.
    fn daemon_client(args: DaemonArgs) -> DaemonClient {
        let token = args
            .token
            .or_else(|| std::env::var("MOFA_API_TOKEN").ok())
            .filter(|t| !t.is_empty());
        let client = DaemonClient::new(args.url);
        match token {
            Some(t) => client.with_token(t),
            None => client,
        }
    }

    /// Run the daemon in-process.
    async fn serve(config_path: Option<PathBuf>, port: Option<u16>) -> anyhow::Result<()> {
        let mut config = EngineConfig::load_checked(config_path.as_deref())?;
        if let Some(port) = port {
            config.listen.port = port;
        }
        let host = config.listen.host.clone();
        let port = config.listen.port;

        tracing::info!("MoFA Engine v{} starting", env!("CARGO_PKG_VERSION"));
        let engine = Engine::try_new(config).await?;
        Server::start(engine, &host, port)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Validate configuration without starting the engine.
    fn validate_config(config_path: Option<PathBuf>) -> anyhow::Result<()> {
        match EngineConfig::load_checked(config_path.as_deref()) {
            Ok(cfg) => {
                println!(
                    "configuration is valid: {} provider(s), listen {}:{}",
                    cfg.providers.len(),
                    cfg.listen.host,
                    cfg.listen.port
                );
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("invalid configuration: {e}")),
        }
    }

    /// Pretty-print a client result as JSON, or fail with the error.
    fn print_json<T: serde::Serialize, E: std::fmt::Display>(
        result: Result<T, E>,
    ) -> anyhow::Result<()> {
        match result {
            Ok(value) => {
                println!("{}", serde_json::to_string_pretty(&value)?);
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("{e}")),
        }
    }
}
