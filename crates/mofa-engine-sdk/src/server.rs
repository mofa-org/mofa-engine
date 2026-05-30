//! HTTP API server using Axum.

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json,
};
use mofa_engine_core::Engine;
use mofa_kernel::*;
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing::info;

type AppState = Arc<Engine>;

pub async fn start_server(engine: Arc<Engine>, host: &str, port: u16) {
    let app = Router::new()
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/run", post(run_model))
        .route("/v1/status", get(engine_status))
        .route("/health", get(health))
        .route("/", get(dashboard))
        .with_state(engine)
        .layer(CorsLayer::permissive());

    let addr = format!("{host}:{port}");
    info!(addr = %addr, "starting MoFA Engine HTTP server");

    let listener = tokio::net::TcpListener::bind(&addr).await
        .expect("failed to bind");
    axum::serve(listener, app).await.expect("server error");
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "engine": "mofa-engine", "version": "0.1.0"}))
}

async fn capabilities(State(engine): State<AppState>) -> impl IntoResponse {
    Json(engine.capabilities())
}

async fn run_model(
    State(engine): State<AppState>,
    Json(request): Json<RunRequest>,
) -> Result<Json<RunResponse>, (StatusCode, Json<ErrorResponse>)> {
    match engine.run(request).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let (status, code) = match &e {
                EngineError::ModelNotFound(_) => (StatusCode::NOT_FOUND, "model_not_found"),
                EngineError::InvalidInput(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
                EngineError::InsufficientMemory { .. } => (StatusCode::SERVICE_UNAVAILABLE, "insufficient_memory"),
                EngineError::BackendNotAvailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "backend_unavailable"),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };
            Err((status, Json(ErrorResponse {
                error: code.to_string(),
                message: e.to_string(),
            })))
        }
    }
}

async fn engine_status(State(engine): State<AppState>) -> impl IntoResponse {
    Json(engine.status())
}

async fn dashboard(State(engine): State<AppState>) -> impl IntoResponse {
    let status = engine.status();
    let caps = engine.capabilities();

    let models_html: String = caps.models.iter().map(|m| {
        let status_class = match m.status {
            ModelStatus::Loaded | ModelStatus::Running => "loaded",
            ModelStatus::Available => "available",
            _ => "other",
        };
        let mem = if m.memory_bytes > 0 {
            format!("{:.1} GB", m.memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0)
        } else {
            "cloud".to_string()
        };
        format!(
            r#"<tr><td>{}</td><td>{}</td><td>{}</td><td class="{}">{:?}</td><td>{}</td></tr>"#,
            m.name, m.model_type, m.backend, status_class, m.status, mem
        )
    }).collect();

    let html = format!(r#"<!DOCTYPE html>
<html><head><title>MoFA Engine</title>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
body {{ font-family: -apple-system, sans-serif; max-width: 900px; margin: 40px auto; padding: 0 20px; background: #0d1117; color: #c9d1d9; }}
h1 {{ color: #58a6ff; }}
h2 {{ color: #8b949e; border-bottom: 1px solid #21262d; padding-bottom: 8px; }}
table {{ width: 100%; border-collapse: collapse; margin: 16px 0; }}
th, td {{ text-align: left; padding: 10px 14px; border-bottom: 1px solid #21262d; }}
th {{ color: #8b949e; font-weight: 600; }}
.loaded {{ color: #3fb950; font-weight: bold; }}
.available {{ color: #8b949e; }}
.other {{ color: #d29922; }}
.stat {{ display: inline-block; background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 16px 24px; margin: 8px; }}
.stat .label {{ color: #8b949e; font-size: 12px; }}
.stat .value {{ color: #58a6ff; font-size: 24px; font-weight: bold; }}
</style>
<script>setTimeout(()=>location.reload(), 5000)</script>
</head><body>
<h1>🔧 MoFA Engine</h1>
<div>
<div class="stat"><div class="label">Total Memory</div><div class="value">{:.1} GB</div></div>
<div class="stat"><div class="label">Used</div><div class="value">{:.1} GB</div></div>
<div class="stat"><div class="label">Available</div><div class="value">{:.1} GB</div></div>
<div class="stat"><div class="label">Models</div><div class="value">{}</div></div>
<div class="stat"><div class="label">Backends</div><div class="value">{}</div></div>
</div>
<h2>Models</h2>
<table><tr><th>Name</th><th>Type</th><th>Backend</th><th>Status</th><th>Memory</th></tr>
{models_html}
</table>
<h2>Backends</h2>
<table><tr><th>Type</th><th>Healthy</th><th>Models</th></tr>
{backends_html}
</table>
</body></html>"#,
        status.total_memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        status.used_memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        status.available_memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        caps.models.len(),
        status.backends.len(),
        backends_html = status.backends.iter().map(|b| {
            format!(r#"<tr><td>{}</td><td>{}</td><td>{}</td></tr>"#,
                b.backend_type,
                if b.healthy { "✅" } else { "❌" },
                b.model_count)
        }).collect::<String>(),
    );

    axum::response::Html(html)
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    message: String,
}
