//! MoFA Engine — multimodal AI model orchestration engine.
//!
//! Binary entry point. Parses CLI arguments, loads configuration,
//! initialises the engine, and starts the HTTP server.

use clap::Parser;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_engine_sdk::start_server;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, fmt};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

fn init_tracer(endpoint: &str) -> Result<opentelemetry_sdk::trace::SdkTracer, opentelemetry::trace::TraceError> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;
        
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder().with_service_name("mofa-engine").build()
        )
        .build();
        
    opentelemetry::global::set_tracer_provider(provider.clone());
    
    use opentelemetry::trace::TracerProvider;
    Ok(provider.tracer("mofa-engine"))
}

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
    let cli = Cli::parse();

    // Load and validate configuration.
    let mut config = EngineConfig::load_checked(cli.config.as_deref())?;

    // CLI port override
    if let Some(port) = cli.port {
        config.listen.port = port;
    }

    let host = config.listen.host.clone();
    let port = config.listen.port;

    #[allow(unused_variables)]
    let observability_enabled = config.observability.enabled;
    #[allow(unused_variables)]
    let otlp_endpoint = config.observability.otlp_endpoint.clone();

    // Initialize Tracing Subscriber
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = fmt::layer().with_target(false);

    let otel_layer = if observability_enabled {
        if let Some(endpoint) = &otlp_endpoint {
            match init_tracer(endpoint) {
                Ok(tracer) => Some(tracing_opentelemetry::layer().with_tracer(tracer)),
                Err(e) => {
                    eprintln!("Failed to initialize OTLP tracer: {e}");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    let mut metrics_state = None;
    let mut obs_sender = None;

    if observability_enabled {
        tracing::info!("Initializing observability pipeline...");
        let (metrics, sender) = mofa_observability::collector::init_pipeline(otlp_endpoint.as_deref());
        metrics_state = Some(metrics);
        obs_sender = Some(sender);
    }

    tracing::info!("MoFA Engine v{} starting", env!("CARGO_PKG_VERSION"));

    // Create engine
    let engine = Engine::try_new(config).await?;

    // Start Observability Bridge
    if let Some(sender) = obs_sender {
        let rx = engine.subscribe_events();
        let engine_clone = Arc::clone(&engine);
        tokio::spawn(async move {
            mofa_engine_sdk::observability_bridge::run(rx, sender, engine_clone).await;
        });
    }

    // Start server
    start_server(engine, metrics_state, &host, port)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(())
}
