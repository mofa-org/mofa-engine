//! Axum HTTP server with REST API and SSE event streaming.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response,
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
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::Instrument;

use crate::dashboard;

/// Header carrying the request correlation id.
const REQUEST_ID_HEADER: &str = "x-request-id";
/// Maximum accepted request body size (16 MiB) — bounds base64/JSON payloads.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Shared application state.
#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    started_at: std::time::Instant,
    /// Optional bearer token; when set, `/v1` routes require it.
    api_token: Option<Arc<String>>,
}

/// Start the HTTP server.
///
/// Binds loopback by default (the caller passes the host). Set `MOFA_API_TOKEN`
/// to require `Authorization: Bearer <token>` on all `/v1` routes; when unset,
/// the API is open (appropriate only for a trusted local machine).
pub async fn start_server(
    engine: Arc<Engine>,
    host: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_token = std::env::var("MOFA_API_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .map(Arc::new);
    if api_token.is_some() {
        tracing::info!("API authentication enabled (bearer token required on /v1)");
    }

    let state = AppState {
        engine,
        started_at: std::time::Instant::now(),
        api_token,
    };

    let app = build_app(state);

    let addr = format!("{host}:{port}");
    tracing::info!("MoFA Engine listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Assemble the full router: public routes, an auth-gated `/v1` API, and the
/// cross-cutting middleware stack. Extracted so tests can exercise it without
/// binding a socket.
fn build_app(state: AppState) -> Router {
    // `/v1` routes sit behind the auth gate; public routes do not.
    let api = Router::new()
        .route("/v1/capabilities", get(capabilities_handler))
        .route("/v1/invoke", post(invoke_handler))
        .route("/v1/invoke/stream", post(invoke_stream_handler))
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
        .route("/v1/models/load", post(load_model_handler))
        .route("/v1/models/unload", post(unload_model_handler))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        // Public (unauthenticated) routes.
        .route("/", get(dashboard_handler))
        .route("/health", get(health_handler))
        .route("/metrics", get(metrics_handler))
        .merge(api)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        // Outermost, so the request-id span encloses the trace layer and every
        // response (including framework rejections) carries `x-request-id`.
        .layer(middleware::from_fn(correlation_middleware))
        .with_state(state)
}

/// Maximum accepted length of a client-supplied `x-request-id`. A longer (or
/// empty) value is replaced with a generated id so a caller cannot amplify log
/// or allocation cost through the correlation header.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Attach an `x-request-id` to every request/response and open a tracing span
/// carrying it, so logs can be correlated across a single request.
async fn correlation_middleware(req: Request, next: Next) -> Response {
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= MAX_REQUEST_ID_LEN)
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let span = tracing::info_span!("request", request_id = %request_id);
    let mut resp = next.run(req).instrument(span).await;

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        resp.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    resp
}

/// Enforce bearer-token auth on `/v1` routes when a token is configured.
async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = state.api_token.as_ref() else {
        return next.run(req).await;
    };
    let presented = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let authorized = presented
        .map(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
        .unwrap_or(false);
    if authorized {
        next.run(req).await
    } else {
        let err = EngineError::InvalidRequest("missing or invalid bearer token".into());
        (StatusCode::UNAUTHORIZED, Json(err.info())).into_response()
    }
}

/// Compare two byte strings in time independent of how many leading bytes match,
/// so token verification does not leak the secret through response timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
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

/// Stream inference output as Server-Sent Events.
///
/// Each SSE `data:` line is a JSON [`StreamChunk`](mofa_kernel::StreamChunk):
/// a `started` event, then `text` deltas, then a terminal `completed` or
/// `error`. Errors are delivered in-band as an `error` chunk (HTTP status is
/// always 200 once the stream opens), so clients handle failures uniformly.
async fn invoke_stream_handler(
    State(state): State<AppState>,
    Json(req): Json<InferenceRequest>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.engine.invoke_stream(req);
    let stream = ReceiverStream::new(rx).map(|chunk| {
        // Every SSE frame must be a valid StreamChunk JSON. Serialization of a
        // chunk cannot realistically fail, but if it did, emit a well-formed
        // error chunk rather than an empty (invalid) frame.
        let data = serde_json::to_string(&chunk).unwrap_or_else(|_| {
            r#"{"type":"error","code":"internal","message":"failed to serialize stream chunk","retryable":false,"source":null}"#.to_string()
        });
        Ok(Event::default().data(data))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

async fn status_handler(State(state): State<AppState>) -> Json<mofa_kernel::EngineStatus> {
    let status = state.engine.status().await;
    Json(status)
}

async fn refresh_handler(State(state): State<AppState>) -> Json<mofa_kernel::EngineStatus> {
    state.engine.refresh_resources().await;
    Json(state.engine.status().await)
}

/// Prometheus text-exposition metrics. Public (no auth) so scrapers can reach it.
async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let body = state.engine.metrics_prometheus();
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}

/// Map an engine error to the HTTP status used by the management endpoints.
fn error_status(e: &EngineError) -> StatusCode {
    match e {
        EngineError::NoCapableModel(_) => StatusCode::NOT_FOUND,
        EngineError::InvalidRequest(_) | EngineError::UnsupportedOperation(_) => {
            StatusCode::BAD_REQUEST
        }
        EngineError::CircuitOpen(_) => StatusCode::SERVICE_UNAVAILABLE,
        EngineError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Body for the manual model load/unload management endpoints.
#[derive(Deserialize)]
struct ModelActionRequest {
    /// Canonical model id (`provider/model`).
    model_id: String,
}

#[derive(Serialize)]
struct ModelActionResponse {
    model_id: String,
    changed: bool,
}

/// `POST /v1/models/load` — manually warm a model.
async fn load_model_handler(
    State(state): State<AppState>,
    Json(req): Json<ModelActionRequest>,
) -> Result<Json<ModelActionResponse>, (StatusCode, Json<ErrorInfo>)> {
    match state.engine.load_model(&req.model_id).await {
        Ok(()) => Ok(Json(ModelActionResponse {
            model_id: req.model_id,
            changed: true,
        })),
        Err(e) => Err((error_status(&e), Json(e.info()))),
    }
}

/// `POST /v1/models/unload` — manually unload a model.
async fn unload_model_handler(
    State(state): State<AppState>,
    Json(req): Json<ModelActionRequest>,
) -> Result<Json<ModelActionResponse>, (StatusCode, Json<ErrorInfo>)> {
    match state.engine.unload_model_manual(&req.model_id).await {
        Ok(changed) => Ok(Json(ModelActionResponse {
            model_id: req.model_id,
            changed,
        })),
        Err(e) => Err((error_status(&e), Json(e.info()))),
    }
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
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use mofa_engine_core::EngineConfig;
    use mofa_engine_core::config::{ListenConfig, MemoryConfig, PreflightConfig, TimeoutConfig};
    use tower::ServiceExt; // for `oneshot`

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

    async fn test_state(api_token: Option<&str>) -> AppState {
        let config = EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            artifacts: Default::default(),
            security: Default::default(),
            providers: vec![],
        };
        AppState {
            engine: mofa_engine_core::Engine::new(config).await,
            started_at: std::time::Instant::now(),
            api_token: api_token.map(|t| Arc::new(t.to_string())),
        }
    }

    #[tokio::test]
    async fn metrics_endpoint_serves_prometheus_text() {
        let app = build_app(test_state(None).await);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("text/plain")
        );
        let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("mofa_requests_total"));
    }

    #[tokio::test]
    async fn responses_carry_a_request_id_header() {
        let app = build_app(test_state(None).await);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.headers().contains_key(REQUEST_ID_HEADER));
    }

    #[tokio::test]
    async fn provided_request_id_is_echoed_back() {
        let app = build_app(test_state(None).await);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .header(REQUEST_ID_HEADER, "abc-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.headers().get(REQUEST_ID_HEADER).unwrap(), "abc-123");
    }

    #[tokio::test]
    async fn auth_gate_rejects_missing_token_but_allows_public_routes() {
        let app = build_app(test_state(Some("secret")).await);

        // A /v1 route without the token is unauthorized.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // The same route with the correct bearer token succeeds.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Public routes remain reachable without a token.
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
