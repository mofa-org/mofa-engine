//! Axum HTTP server with REST API and SSE event streaming.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{
        Html,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use mofa_engine_core::Engine;
use mofa_kernel::InferenceRequest;
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::dashboard;

/// Shared application state.
#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    started_at: std::time::Instant,
}

/// Start the HTTP server.
pub async fn start_server(
    engine: Arc<Engine>,
    host: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState {
        engine,
        started_at: std::time::Instant::now(),
    };

    let app = Router::new()
        // Dashboard
        .route("/", get(dashboard_handler))
        // API routes
        .route("/health", get(health_handler))
        .route("/v1/capabilities", get(capabilities_handler))
        .route("/v1/invoke", post(invoke_handler))
        .route("/v1/status", get(status_handler))
        .route("/v1/events", get(events_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = format!("{host}:{port}");
    tracing::info!("MoFA Engine listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
}

async fn health_handler(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.started_at.elapsed().as_secs(),
    })
}

async fn capabilities_handler(
    State(state): State<AppState>,
) -> Json<Vec<mofa_kernel::ModelCard>> {
    let caps = state.engine.capabilities().await;
    Json(caps)
}

async fn invoke_handler(
    State(state): State<AppState>,
    Json(req): Json<InferenceRequest>,
) -> Result<Json<mofa_kernel::InferenceResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.engine.invoke(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let status = match &e {
                mofa_kernel::EngineError::NoCapableModel(_) => StatusCode::NOT_FOUND,
                mofa_kernel::EngineError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
                mofa_kernel::EngineError::CircuitOpen(_) => StatusCode::SERVICE_UNAVAILABLE,
                mofa_kernel::EngineError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((
                status,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            ))
        }
    }
}

/// Error response body.
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn status_handler(
    State(state): State<AppState>,
) -> Json<mofa_kernel::EngineStatus> {
    let status = state.engine.status().await;
    Json(status)
}

async fn events_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.engine.subscribe_events();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let data = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

async fn dashboard_handler() -> Html<&'static str> {
    Html(dashboard::DASHBOARD_HTML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_response_serializes() {
        let resp = HealthResponse {
            status: "ok",
            version: "0.1.0",
            uptime_secs: 42,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
    }

    #[test]
    fn error_response_serializes() {
        let resp = ErrorResponse {
            error: "something broke".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("something broke"));
    }
}
