//! Axum HTTP server with REST API and SSE event streaming.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{
        Html,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use mofa_engine_core::Engine;
use mofa_engine_core::engine::{LifecycleRecord, MemoryReport};
use mofa_engine_core::preflight::PreflightStats;
use mofa_engine_core::subscription::SubscriptionInfo;
use mofa_kernel::{Capability, EngineError, ErrorInfo, InferenceRequest};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
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
        .route("/v1/memory", get(memory_handler))
        .route("/v1/lifecycle", get(lifecycle_handler))
        .route("/v1/preflight", get(preflight_handler))
        .route(
            "/v1/subscriptions",
            get(list_subscriptions_handler).post(subscribe_handler),
        )
        .route("/v1/subscriptions/{id}", delete(unsubscribe_handler))
        .route("/v1/events", get(events_handler))
        .route("/v1/discovery/refresh", post(refresh_handler))
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

async fn capabilities_handler(State(state): State<AppState>) -> Json<Vec<mofa_kernel::ModelCard>> {
    let caps = state.engine.capabilities().await;
    Json(caps)
}

async fn invoke_handler(
    State(state): State<AppState>,
    Json(req): Json<InferenceRequest>,
) -> Result<Json<mofa_kernel::InferenceResponse>, (StatusCode, Json<ErrorInfo>)> {
    match state.engine.invoke(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let status = match &e {
                mofa_kernel::EngineError::NoCapableModel(_) => StatusCode::NOT_FOUND,
                mofa_kernel::EngineError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
                mofa_kernel::EngineError::CircuitOpen(_) => StatusCode::SERVICE_UNAVAILABLE,
                mofa_kernel::EngineError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
                mofa_kernel::EngineError::UnsupportedOperation(_) => StatusCode::BAD_REQUEST,
                mofa_kernel::EngineError::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((status, Json(e.info())))
        }
    }
}

async fn status_handler(State(state): State<AppState>) -> Json<mofa_kernel::EngineStatus> {
    let status = state.engine.status().await;
    Json(status)
}

async fn refresh_handler(State(state): State<AppState>) -> Json<mofa_kernel::EngineStatus> {
    state.engine.refresh_resources().await;
    Json(state.engine.status().await)
}

async fn memory_handler(State(state): State<AppState>) -> Json<MemoryReport> {
    Json(state.engine.memory_report())
}

async fn lifecycle_handler(State(state): State<AppState>) -> Json<Vec<LifecycleRecord>> {
    Json(state.engine.lifecycle_history())
}

async fn preflight_handler(State(state): State<AppState>) -> Json<PreflightStats> {
    Json(state.engine.preflight_stats())
}

async fn list_subscriptions_handler(State(state): State<AppState>) -> Json<Vec<SubscriptionInfo>> {
    Json(state.engine.subscriptions())
}

/// Body for `POST /v1/subscriptions`.
#[derive(Deserialize)]
struct SubscribeRequest {
    #[serde(default)]
    app_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    capabilities: Vec<Capability>,
    /// Optional lifetime in seconds; omitted means it lives until removed.
    #[serde(default)]
    ttl_secs: Option<u64>,
}

#[derive(Serialize)]
struct SubscribeResponse {
    id: u64,
}

async fn subscribe_handler(
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> Result<Json<SubscribeResponse>, (StatusCode, Json<ErrorInfo>)> {
    if req.capabilities.is_empty() {
        let err = EngineError::InvalidRequest("capabilities must not be empty".into());
        return Err((StatusCode::BAD_REQUEST, Json(err.info())));
    }
    let ttl = req.ttl_secs.map(Duration::from_secs);
    let id = state
        .engine
        .subscribe(req.app_id, req.session_id, req.capabilities, ttl);
    Ok(Json(SubscribeResponse { id }))
}

#[derive(Serialize)]
struct UnsubscribeResponse {
    removed: bool,
}

async fn unsubscribe_handler(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> (StatusCode, Json<UnsubscribeResponse>) {
    let removed = state.engine.unsubscribe(id);
    let status = if removed {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    (status, Json(UnsubscribeResponse { removed }))
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
}
