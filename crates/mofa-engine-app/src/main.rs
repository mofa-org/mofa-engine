use mofa_engine_core::{Engine, EngineConfig};
use mofa_engine_sdk::start_server;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .init();

    let config = EngineConfig::load();
    let host = config.host.clone();
    let port = config.port;

    let engine = Engine::new(config).await;
    start_server(engine, &host, port).await;
}
