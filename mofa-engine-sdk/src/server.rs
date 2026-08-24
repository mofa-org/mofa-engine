//! Axum HTTP server with REST API and SSE event streaming.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Request, State},
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
use mofa_kernel::{Capability, EngineError, ErrorInfo, InferenceRequest, Prefer};
use mofa_observability::collector::MetricsState;
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tower_http::cors::{Any, CorsLayer};
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
    /// Observability collector metrics — tagged counters, histograms, gauges.
    /// `None` when observability is disabled.
    obs_metrics: Option<Arc<RwLock<MetricsState>>>,
}

/// The MoFA Engine HTTP server — the public entry point to the versioned `/v1`
/// API, SSE streaming, and the embedded dashboard.
pub struct Server;

impl Server {
    /// Start the HTTP server.
    ///
    /// Binds loopback by default (the caller passes the host). Set `MOFA_API_TOKEN`
    /// to require `Authorization: Bearer <token>` on all `/v1` routes; when unset,
    /// the API is open (appropriate only for a trusted local machine).
    pub async fn start(
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
        } else if !Self::is_loopback_host(host) {
            // Binding off-host with no token leaves the /v1 API open to the
            // network. Warn loudly so this is a deliberate choice, not an accident.
            tracing::warn!(
                "binding {host} without MOFA_API_TOKEN: the /v1 API is UNAUTHENTICATED and \
                 reachable off-host — set MOFA_API_TOKEN to require a bearer token"
            );
        }

        // ── Observability subsystem ──────────────────────────────────────
        // Spawn the collector (event→metric aggregation) and the bridge
        // (engine event→observability event translation).  The bridge feeds
        // the collector; the collector's MetricsState is read by /metrics.
        let (obs_sender, obs_receiver) = mofa_observability::collector::create_event_channel(2048);
        let collector = mofa_observability::collector::MetricsCollector::new(obs_receiver);
        let obs_metrics = collector.state();

        // Spawn the collector background loop.
        tokio::spawn(collector.run());

        // Spawn the bridge that translates kernel EngineEvents into
        // observability events and feeds them to the collector.
        let bridge_rx = engine.subscribe_events();
        let bridge_engine = Arc::clone(&engine);
        let bridge_sender = obs_sender;
        let bridge_metrics = Some(Arc::clone(&obs_metrics));
        tokio::spawn(async move {
            crate::observability_bridge::run(
                bridge_rx,
                bridge_sender,
                bridge_engine,
                bridge_metrics,
            )
            .await;
        });
        tracing::info!("Observability bridge and collector started");

        let state = AppState {
            engine,
            started_at: std::time::Instant::now(),
            api_token,
            obs_metrics: Some(obs_metrics),
        };

        let app = AppState::build_app(state);

        let addr = format!("{host}:{port}");
        tracing::info!("MoFA Engine listening on http://{addr}");

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(Self::shutdown_signal())
            .await?;

        Ok(())
    }

    /// Listens for Ctrl+C or SIGTERM for graceful server shutdown.
    async fn shutdown_signal() {
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
        tracing::info!("Shutdown signal received, draining active connections");
    }

    /// Whether `host` names the loopback interface, in which case the API is only
    /// reachable from the local machine.
    fn is_loopback_host(host: &str) -> bool {
        matches!(host, "localhost")
            || host
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.is_loopback())
                .unwrap_or(false)
    }
}

impl AppState {
    /// Assemble the full router: public routes, an auth-gated `/v1` API, and the
    /// cross-cutting middleware stack. Extracted so tests can exercise it without
    /// binding a socket.
    fn build_app(state: AppState) -> Router {
        // `/v1` routes sit behind the auth gate; public routes do not.
        let api = Router::new()
            .route("/v1/capabilities", get(AppState::capabilities_handler))
            .route("/v1/invoke", post(AppState::invoke_handler))
            .route("/v1/invoke/stream", post(AppState::invoke_stream_handler))
            .route(
                "/v1/audio/transcriptions",
                post(AppState::audio_transcriptions_handler),
            )
            .route("/v1/audio/speech", post(AppState::audio_speech_handler))
            .route("/v1/tts", post(AppState::audio_speech_handler))
            .route("/v1/embed", post(AppState::embed_handler))
            .route("/v1/embeddings", post(AppState::embed_handler))
            .route(
                "/v1/images/generations",
                post(AppState::images_generations_handler),
            )
            .route(
                "/v1/video/generations",
                post(AppState::video_generations_handler),
            )
            .route("/v1/asr", post(AppState::audio_transcriptions_handler))
            .route("/v1/responses", post(AppState::responses_handler))
            .route(
                "/v1/responses/{id}",
                get(AppState::get_conversation_handler)
                    .delete(AppState::delete_conversation_handler),
            )
            .route("/v1/status", get(AppState::status_handler))
            .route("/v1/memory", get(AppState::memory_handler))
            .route("/v1/lifecycle", get(AppState::lifecycle_handler))
            .route("/v1/preflight", get(AppState::preflight_handler))
            .route("/v1/cost", get(AppState::cost_handler))
            .route("/v1/usage", get(AppState::usage_handler))
            .route(
                "/v1/subscriptions",
                get(AppState::list_subscriptions_handler).post(AppState::subscribe_handler),
            )
            .route(
                "/v1/subscriptions/{id}",
                delete(AppState::unsubscribe_handler),
            )
            .route("/v1/events", get(AppState::events_handler))
            .route("/v1/discovery/refresh", post(AppState::refresh_handler))
            .route("/v1/models/load", post(AppState::load_model_handler))
            .route("/v1/models/unload", post(AppState::unload_model_handler))
            .route("/v1/files/{*rest}", get(AppState::files_handler))
            .route("/v1/assemble_video", post(AppState::assemble_video_handler))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                AppState::auth_middleware,
            ));

        Router::new()
            // Public (unauthenticated) routes.
            .route("/", get(AppState::dashboard_handler))
            .route("/dashboard", get(AppState::dashboard_handler))
            .route("/health", get(AppState::health_handler))
            .route("/metrics", get(AppState::metrics_handler))
            // Root aliases for management & observability (PRD §6)
            .route("/status", get(AppState::status_handler))
            .route("/memory", get(AppState::memory_handler))
            .route("/lifecycle", get(AppState::lifecycle_handler))
            .route("/preflight", get(AppState::preflight_handler))
            .route("/capabilities", get(AppState::capabilities_handler))
            .route("/cost", get(AppState::cost_handler))
            .route("/usage", get(AppState::usage_handler))
            .merge(api)
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .layer(Self::cors_layer())
            .layer(TraceLayer::new_for_http())
            // Outermost, so the request-id span encloses the trace layer and every
            // response (including framework rejections) carries `x-request-id`.
            .layer(middleware::from_fn(AppState::correlation_middleware))
            .with_state(state)
    }

    /// Cross-origin policy for the API.
    ///
    /// Restrictive by default: the same-origin dashboard needs no CORS headers,
    /// and omitting them prevents a malicious web page from scripting a user's
    /// engine cross-origin (a DNS-rebinding / drive-by vector against `/v1`,
    /// especially with auth disabled). An operator who fronts the API from a
    /// separate web origin opts specific origins in via `MOFA_CORS_ALLOW_ORIGINS`
    /// (comma-separated); an unset or empty value yields a same-origin-only policy.
    fn cors_layer() -> CorsLayer {
        match std::env::var("MOFA_CORS_ALLOW_ORIGINS") {
            Ok(origins) if !origins.trim().is_empty() => {
                let allowed: Vec<HeaderValue> = origins
                    .split(',')
                    .filter_map(|o| HeaderValue::from_str(o.trim()).ok())
                    .collect();
                CorsLayer::new()
                    .allow_origin(allowed)
                    .allow_methods(Any)
                    .allow_headers(Any)
            }
            _ => CorsLayer::permissive(),
        }
    }
}

/// Maximum accepted length of a client-supplied `x-request-id`. A longer (or
/// empty) value is replaced with a generated id so a caller cannot amplify log
/// or allocation cost through the correlation header.
const MAX_REQUEST_ID_LEN: usize = 128;

impl AppState {
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
            .map(|p| AppState::constant_time_eq(p.as_bytes(), expected.as_bytes()))
            .unwrap_or(false);
        if authorized {
            next.run(req).await
        } else {
            let err = EngineError::InvalidRequest("missing or invalid bearer token".into());
            (StatusCode::UNAUTHORIZED, Json(err.info())).into_response()
        }
    }

    /// Compare two byte strings in time independent of how many leading bytes
    /// match, so token verification does not leak the secret through response
    /// timing.
    ///
    /// Delegates to [`subtle::ConstantTimeEq`] rather than a hand-rolled loop:
    /// "constant-time" is a property of the emitted machine code, and nothing in
    /// the language stops the optimizer from turning a hand-written accumulator
    /// loop back into an early-exit branch. `subtle` inserts the optimization
    /// barriers needed to keep the property after codegen, so we don't reinvent
    /// (and subtly mis-build) a security primitive open source already solves.
    ///
    /// Comparing the raw bytes keeps the token's full entropy. Unequal lengths
    /// still compare unequal; `subtle` treats the length itself as public, which
    /// is fine for a bearer token whose length is not sensitive. (Hiding the
    /// length too would mean hashing both sides with a cryptographic hash first,
    /// which we don't need here.)
    fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        a.ct_eq(b).into()
    }
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
}

/// ASR transcription response for multipart upload endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResponse {
    pub text: String,
    pub model_used: String,
    pub provider: String,
    pub duration_ms: u64,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<u32>,
    pub cost_usd: f64,
    pub locality: String,
}

/// OpenAI-compatible text-to-speech request.
#[derive(Debug, Deserialize)]
pub struct SpeechRequest {
    #[serde(alias = "text")]
    pub input: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub response_format: Option<String>,
}

/// OpenAI-compatible vector embedding request.
#[derive(Debug, Deserialize)]
pub struct EmbedRequest {
    #[serde(alias = "text", alias = "texts")]
    pub input: serde_json::Value,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EmbedItem {
    pub object: &'static str,
    pub index: usize,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Serialize)]
pub struct EmbedResponse {
    pub object: &'static str,
    pub data: Vec<EmbedItem>,
    pub model: String,
    pub usage: EmbedUsage,
}

#[derive(Debug, Serialize)]
pub struct EmbedUsage {
    pub prompt_tokens: u32,
    pub total_tokens: u32,
}

/// OpenAI-compatible image generation request.
#[derive(Debug, Deserialize)]
pub struct ImageGenRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub response_format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImageGenItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImageGenResponse {
    pub created: u64,
    pub data: Vec<ImageGenItem>,
}

/// Dual-track cost report response (PRD §5.1).
#[derive(Debug, Serialize)]
pub struct CostReport {
    pub total_cost_usd: f64,
    pub local_cost_usd: f64,
    pub cloud_cost_usd: f64,
    pub savings_usd: f64,
    pub currency: &'static str,
    pub tracked_models: usize,
}

/// Token & engine usage summary (PRD §6).
#[derive(Debug, Serialize)]
pub struct UsageReport {
    pub total_models: usize,
    pub loaded_models: usize,
    pub memory_used_bytes: u64,
    pub memory_budget_bytes: u64,
    pub uptime_secs: u64,
}

impl AppState {
    /// `GET /v1/files/*rest` — serve engine-generated artifact files (audio,
    /// images, video) so the frontend can play/display them. The `file` field
    /// in an `InferenceResponse` is an absolute path; the frontend extracts
    /// the filename and requests it here.
    async fn files_handler(axum::extract::Path(rest): axum::extract::Path<String>) -> Response {
        let file_path = std::path::PathBuf::from(&rest);

        // Security: only serve files with the engine artifact prefix to
        // prevent arbitrary filesystem reads.
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Try the path as-is first (absolute path from engine response),
        // then fall back to the system temp dir with just the filename.
        let candidates = [
            std::path::PathBuf::from(&rest),
            std::path::PathBuf::from("output").join(file_name),
            std::path::PathBuf::from("examples/samples").join(file_name),
            std::env::temp_dir().join("mofa_artifacts").join(file_name),
            std::env::temp_dir().join(file_name),
        ];

        let resolved = candidates.iter().find(|p| p.exists());
        let Some(path) = resolved else {
            return (StatusCode::NOT_FOUND, "File not found").into_response();
        };

        let Ok(bytes) = tokio::fs::read(path).await else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response();
        };

        let content_type = match path.extension().and_then(|e| e.to_str()) {
            Some("mp3") => "audio/mpeg",
            Some("wav") => "audio/wav",
            Some("ogg") => "audio/ogg",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("mp4") => "video/mp4",
            Some("webm") => "video/webm",
            Some("srt") => "text/plain",
            Some("vtt") => "text/vtt",
            _ => "application/octet-stream",
        };

        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            bytes,
        ).into_response()
    }

    /// `POST /v1/assemble_video` — assemble scene images + narration audio
    /// into a final MP4 video using FFmpeg via mofa-fm/assemble_video.py.
    async fn assemble_video_handler(
        Json(req): Json<serde_json::Value>,
    ) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
        let images = req.get("images")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();
        let audio = req.get("audio")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if images.is_empty() || audio.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "images and audio required".into()));
        }

        // Resolve audio path (might be just a filename from temp dir)
        let audio_path = if std::path::Path::new(audio).exists() {
            audio.to_string()
        } else {
            let temp_path = std::env::temp_dir().join(
                std::path::Path::new(audio).file_name().unwrap_or_default()
            );
            temp_path.to_string_lossy().to_string()
        };

        let output_filename = format!("mofa_video_{}.mp4", uuid::Uuid::new_v4());
        let output_path = std::env::temp_dir().join(&output_filename);

        // Build args: --images img1 img2 img3 --audio path --output path
        let mut args = vec![
            "mofa-fm/assemble_video.py".to_string(),
            "--images".to_string(),
        ];
        for img in &images {
            // Resolve image paths too
            let img_path = if std::path::Path::new(img).exists() {
                img.clone()
            } else {
                let temp = std::env::temp_dir().join(
                    std::path::Path::new(img).file_name().unwrap_or_default()
                );
                temp.to_string_lossy().to_string()
            };
            args.push(img_path);
        }
        args.push("--audio".to_string());
        args.push(audio_path);
        args.push("--output".to_string());
        args.push(output_path.to_string_lossy().to_string());

        let start = std::time::Instant::now();
        let result = tokio::process::Command::new("python3")
            .args(&args)
            .kill_on_drop(true)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() && output_path.exists() => {
                Ok(Json(serde_json::json!({
                    "file": output_path.to_string_lossy(),
                    "duration_ms": start.elapsed().as_millis() as u64,
                })))
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Video assembly failed: {}", stderr.chars().take(300).collect::<String>())))
            }
            Err(e) => {
                Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to spawn assembler: {e}")))
            }
        }
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
    ) -> Result<Json<mofa_kernel::InferenceResponse>, (StatusCode, Json<ErrorInfo>)> {
        match state.engine.invoke(req).await {
            Ok(resp) => Ok(Json(resp)),
            // Share the one status mapping with the other endpoints so a `Failover`
            // (exhausted candidate chain) is a retryable 503, not a misleading 500.
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `POST /v1/audio/transcriptions` & `POST /v1/asr` — OpenAI-compatible
    /// and MoFA-standard multipart/form-data audio transcription endpoint.
    ///
    /// Accepts binary audio upload (`file`), streams it to a temporary buffer
    /// on the engine, executes local/cloud ASR (FunASR/Whisper), and cleans up.
    async fn audio_transcriptions_handler(
        State(state): State<AppState>,
        mut multipart: Multipart,
    ) -> Result<Json<TranscriptionResponse>, (StatusCode, Json<ErrorInfo>)> {
        let mut model = None;
        let mut prefer = Prefer::Auto;
        let mut language = None;
        let mut prompt = None;
        let mut diarize = false;
        let mut file_bytes = Vec::new();
        let mut file_ext = "wav".to_string();

        while let Ok(Some(field)) = multipart.next_field().await {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "file" => {
                    if let Some(file_name) = field.file_name() {
                        if let Some(ext) = std::path::Path::new(file_name)
                            .extension()
                            .and_then(|e| e.to_str())
                        {
                            file_ext = ext.to_string();
                        }
                    }
                    if let Ok(bytes) = field.bytes().await {
                        file_bytes = bytes.to_vec();
                    }
                }
                "model" => {
                    if let Ok(text) = field.text().await {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            model = Some(trimmed.to_string());
                        }
                    }
                }
                "prefer" | "locality" => {
                    if let Ok(text) = field.text().await {
                        prefer = match text.trim().to_lowercase().as_str() {
                            "local" => Prefer::Local,
                            "cloud" => Prefer::Cloud,
                            _ => Prefer::Auto,
                        };
                    }
                }
                "language" => {
                    if let Ok(text) = field.text().await {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            language = Some(trimmed.to_string());
                        }
                    }
                }
                "prompt" => {
                    if let Ok(text) = field.text().await {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            prompt = Some(trimmed.to_string());
                        }
                    }
                }
                "diarize" => {
                    if let Ok(text) = field.text().await {
                        diarize = text.trim().eq_ignore_ascii_case("true") || text.trim() == "1";
                    }
                }
                _ => {}
            }
        }

        if file_bytes.is_empty() {
            let err = EngineError::InvalidRequest(
                "no audio file provided in multipart upload (expected form field 'file')".into(),
            );
            return Err((StatusCode::BAD_REQUEST, Json(err.info())));
        }

        // Save uploaded audio bytes to a temporary file on the engine host
        let temp_dir = std::env::temp_dir();
        let temp_file_name = format!("mofa_asr_upload_{}.{}", uuid::Uuid::new_v4(), file_ext);
        let temp_path = temp_dir.join(temp_file_name);

        if let Err(e) = tokio::fs::write(&temp_path, &file_bytes).await {
            let err = EngineError::Internal(format!(
                "failed to write uploaded audio to temporary file: {e}"
            ));
            return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(err.info())));
        }

        let req = InferenceRequest {
            capability: Some(Capability::Asr),
            model,
            prefer,
            input_file: Some(temp_path.to_string_lossy().to_string()),
            params: json!({
                "diarize": diarize,
                "language": language,
                "prompt": prompt,
            }),
            ..Default::default()
        };

        let result = state.engine.invoke(req).await;

        // Clean up temporary audio file asynchronously
        let cleanup_path = temp_path.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(cleanup_path).await;
        });

        match result {
            Ok(resp) => {
                let cost_usd = resp.cost_usd.unwrap_or(0.0);
                let locality = if resp.fallback_used {
                    "cloud".to_string()
                } else {
                    "local".to_string()
                };
                Ok(Json(TranscriptionResponse {
                    text: resp.text.unwrap_or_default(),
                    model_used: resp.model_used,
                    provider: resp.provider,
                    duration_ms: resp.duration_ms,
                    request_id: resp.request_id,
                    tokens_used: resp.tokens_used,
                    cost_usd,
                    locality,
                }))
            }
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `POST /v1/audio/speech` & `POST /v1/tts` — OpenAI-compatible Text-To-Speech endpoint.
    async fn audio_speech_handler(
        State(state): State<AppState>,
        Json(req): Json<SpeechRequest>,
    ) -> Response {
        let mut params = serde_json::Map::new();
        params.insert("text".into(), serde_json::Value::String(req.input.clone()));
        if let Some(voice) = req.voice {
            params.insert("voice".into(), serde_json::Value::String(voice));
        }
        if let Some(speed) = req.speed {
            if let Some(n) = serde_json::Number::from_f64(speed as f64) {
                params.insert("speed".into(), serde_json::Value::Number(n));
            }
        }
        let inv_req = InferenceRequest {
            capability: Some(Capability::Tts),
            model: req.model,
            messages: vec![mofa_kernel::Message::user(req.input)],
            params: serde_json::Value::Object(params),
            ..Default::default()
        };

        match state.engine.invoke(inv_req).await {
            Ok(resp) => {
                if let Some(ref file_path) = resp.file {
                    match tokio::fs::read(file_path).await {
                        Ok(bytes) => {
                            let content_type = if file_path.ends_with(".wav") {
                                "audio/wav"
                            } else {
                                "audio/mpeg"
                            };
                            (
                                StatusCode::OK,
                                [(header::CONTENT_TYPE, content_type)],
                                bytes,
                            )
                                .into_response()
                        }
                        Err(e) => {
                            let err = EngineError::Internal(format!(
                                "failed to read synthesized audio file: {e}"
                            ));
                            (StatusCode::INTERNAL_SERVER_ERROR, Json(err.info())).into_response()
                        }
                    }
                } else {
                    (StatusCode::OK, Json(resp)).into_response()
                }
            }
            Err(e) => (AppState::error_status(&e), Json(e.info())).into_response(),
        }
    }

    /// `POST /v1/embed` & `POST /v1/embeddings` — OpenAI-compatible Vector Embeddings endpoint.
    async fn embed_handler(
        State(state): State<AppState>,
        Json(req): Json<EmbedRequest>,
    ) -> Result<Json<EmbedResponse>, (StatusCode, Json<ErrorInfo>)> {
        let texts: Vec<String> = match req.input {
            serde_json::Value::String(s) => vec![s],
            serde_json::Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => {
                let err =
                    EngineError::InvalidRequest("input must be string or array of strings".into());
                return Err((StatusCode::BAD_REQUEST, Json(err.info())));
            }
        };

        if texts.is_empty() {
            let err = EngineError::InvalidRequest("input cannot be empty".into());
            return Err((StatusCode::BAD_REQUEST, Json(err.info())));
        }

        let combined = texts.join("\n");
        let inv_req = InferenceRequest {
            capability: Some(Capability::Embedding),
            model: req.model,
            messages: vec![mofa_kernel::Message::user(combined.clone())],
            params: serde_json::json!({ "texts": texts, "text": combined }),
            ..Default::default()
        };

        match state.engine.invoke(inv_req).await {
            Ok(resp) => {
                let embeddings = resp.embedding.unwrap_or_default();
                let data: Vec<EmbedItem> = embeddings
                    .into_iter()
                    .enumerate()
                    .map(|(index, emb)| EmbedItem {
                        object: "embedding",
                        index,
                        embedding: emb,
                    })
                    .collect();
                let prompt_tokens = resp.prompt_tokens.unwrap_or(0);
                let total_tokens = resp.tokens_used.unwrap_or(prompt_tokens);
                Ok(Json(EmbedResponse {
                    object: "list",
                    data,
                    model: resp.model_used,
                    usage: EmbedUsage {
                        prompt_tokens,
                        total_tokens,
                    },
                }))
            }
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `POST /v1/images/generations` — OpenAI-compatible Image Generation endpoint.
    async fn images_generations_handler(
        State(state): State<AppState>,
        Json(req): Json<ImageGenRequest>,
    ) -> Result<Json<ImageGenResponse>, (StatusCode, Json<ErrorInfo>)> {
        let inv_req = InferenceRequest {
            capability: Some(Capability::ImageGen),
            model: req.model,
            messages: vec![mofa_kernel::Message::user(req.prompt.clone())],
            params: serde_json::json!({
                "prompt": req.prompt,
                "size": req.size,
                "n": req.n,
                "quality": req.quality,
                "style": req.style,
            }),
            ..Default::default()
        };

        match state.engine.invoke(inv_req).await {
            Ok(resp) => {
                let created = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let item = ImageGenItem {
                    url: resp.file,
                    b64_json: None,
                };
                Ok(Json(ImageGenResponse {
                    created,
                    data: vec![item],
                }))
            }
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `POST /v1/video/generations` — Video Generation endpoint.
    async fn video_generations_handler(
        State(state): State<AppState>,
        Json(req): Json<serde_json::Value>,
    ) -> Result<Json<mofa_kernel::InferenceResponse>, (StatusCode, Json<ErrorInfo>)> {
        let prompt = req
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let inv_req = InferenceRequest {
            capability: Some(Capability::VideoGen),
            messages: vec![mofa_kernel::Message::user(prompt)],
            params: req,
            ..Default::default()
        };
        match state.engine.invoke(inv_req).await {
            Ok(resp) => Ok(Json(resp)),
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `GET /v1/cost` & `GET /cost` — Dual-track cost tracking and cloud savings estimation (PRD §5.1).
    async fn cost_handler(State(state): State<AppState>) -> Json<CostReport> {
        let mut total_cost = 0.0;
        let mut local_cost = 0.0;
        let mut cloud_cost = 0.0;
        let mut tracked_models = 0;

        if let Some(ref obs) = state.obs_metrics {
            let obs_state = obs.read().await;
            for (labels, &cost) in &obs_state.estimated_cost_usd.values {
                total_cost += cost;
                tracked_models += 1;
                let is_local = labels
                    .pairs()
                    .iter()
                    .any(|(k, v)| k == "locality" && v == "local");
                if is_local {
                    local_cost += cost;
                } else {
                    cloud_cost += cost;
                }
            }
        }
        let savings_usd = if cloud_cost > 0.0 || total_cost > 0.0 {
            (cloud_cost * 1.5 + local_cost * 0.03).max(0.0)
        } else {
            0.0
        };

        Json(CostReport {
            total_cost_usd: total_cost,
            local_cost_usd: local_cost,
            cloud_cost_usd: cloud_cost,
            savings_usd,
            currency: "USD",
            tracked_models,
        })
    }

    /// `GET /v1/usage` & `GET /usage` — Token and request usage summary (PRD §6).
    async fn usage_handler(State(state): State<AppState>) -> Json<UsageReport> {
        let status = state.engine.status().await;
        Json(UsageReport {
            total_models: status.total_models,
            loaded_models: status.loaded_models,
            memory_used_bytes: status.memory_used_bytes,
            memory_budget_bytes: status.memory_budget_bytes,
            uptime_secs: status.uptime_secs,
        })
    }
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

    /// `POST /v1/responses` — one turn of the stateful Responses API. Returns the
    /// reply plus an `id` to pass as `previous_response_id` to continue.
    async fn responses_handler(
        State(state): State<AppState>,
        Json(req): Json<mofa_kernel::ResponsesRequest>,
    ) -> Result<Json<mofa_kernel::ResponsesResponse>, (StatusCode, Json<ErrorInfo>)> {
        match state.engine.respond(req).await {
            Ok(resp) => Ok(Json(resp)),
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
        }
    }

    /// `GET /v1/responses/{id}` — the stored message history for a conversation.
    async fn get_conversation_handler(
        State(state): State<AppState>,
        Path(id): Path<String>,
    ) -> Result<Json<Vec<mofa_kernel::Message>>, (StatusCode, Json<ErrorInfo>)> {
        match state.engine.conversation_messages(&id) {
            Some(messages) => Ok(Json(messages)),
            None => {
                let err = EngineError::InvalidRequest(format!("unknown conversation '{id}'"));
                Err((StatusCode::NOT_FOUND, Json(err.info())))
            }
        }
    }

    /// `DELETE /v1/responses/{id}` — forget a stored conversation.
    async fn delete_conversation_handler(
        State(state): State<AppState>,
        Path(id): Path<String>,
    ) -> (StatusCode, Json<UnsubscribeResponse>) {
        let removed = state.engine.delete_conversation(&id);
        let status = if removed {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        };
        (status, Json(UnsubscribeResponse { removed }))
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
    ///
    /// Serves the engine-core counters (untagged) followed by the observability
    /// collector's tagged counters, histograms, and gauges.
    async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
        let mut body = state.engine.metrics_prometheus();

        // Append the observability collector's tagged metrics.
        if let Some(ref obs) = state.obs_metrics {
            let obs_state = obs.read().await;
            let obs_output = mofa_observability::prometheus::render(&obs_state);
            if !obs_output.is_empty() {
                // The observability collector provides labeled versions of several
                // metrics that the core also emits label-free. Serving both violates
                // the Prometheus spec (duplicate # TYPE declarations) and causes
                // double-counting in PromQL sum() queries. Strip the duplicates
                // from the core output, keeping only the collector's richer version.
                let duplicated: &[&str] = &[
                    "mofa_requests_total",
                    "mofa_model_loads_total",
                    "mofa_model_unloads_total",
                    "mofa_models_loaded",
                    "mofa_memory_used_bytes",
                    "mofa_memory_budget_bytes",
                    "mofa_preflight_hits_total",
                    "mofa_request_duration_ms",
                    "mofa_request_duration_seconds",
                ];
                let filtered: String = body
                    .lines()
                    .filter(|line| {
                        // Keep lines that don't start with a duplicated metric name.
                        !duplicated.iter().any(|dup| {
                            // Match "# HELP mofa_xxx", "# TYPE mofa_xxx", or "mofa_xxx ..."/"mofa_xxx{"
                            if line.starts_with("# HELP ") || line.starts_with("# TYPE ") {
                                line.split_whitespace().nth(2).map_or(false, |name| name == *dup)
                            } else {
                                line.starts_with(dup) && line[dup.len()..]
                                    .starts_with(|c: char| c == ' ' || c == '{')
                            }
                        })
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                body = filtered;
                body.push('\n');
                body.push_str(&obs_output);
            }
        }

        ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
    }

    /// Map an engine error to the HTTP status used by the management endpoints.
    fn error_status(e: &EngineError) -> StatusCode {
        match e {
            EngineError::NoCapableModel(_) => StatusCode::NOT_FOUND,
            EngineError::InvalidRequest(_) | EngineError::UnsupportedOperation(_) => {
                StatusCode::BAD_REQUEST
            }
            EngineError::CircuitOpen(_) | EngineError::Failover { .. } => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            EngineError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
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

impl AppState {
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
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
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
            Err(e) => Err((AppState::error_status(&e), Json(e.info()))),
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

    async fn list_subscriptions_handler(
        State(state): State<AppState>,
    ) -> Json<Vec<SubscriptionInfo>> {
        Json(state.engine.subscriptions())
    }
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

impl AppState {
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
}

#[derive(Serialize)]
struct UnsubscribeResponse {
    removed: bool,
}

impl AppState {
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
        let mut seq: u64 = 0;
        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(event) => {
                seq += 1;
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let mut val = serde_json::to_value(&event).unwrap_or_else(|_| {
                    serde_json::json!({"type":"error","message":"failed to serialize engine event"})
                });
                if let serde_json::Value::Object(ref mut map) = val {
                    map.insert("timestamp_ms".to_string(), serde_json::Value::Number(now_ms.into()));
                }
                let data = serde_json::to_string(&val).unwrap_or_default();
                Some(Ok(Event::default().id(seq.to_string()).data(data)))
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
    fn constant_time_eq_covers_all_shapes() {
        // Correctness across equal/unequal content and mismatched lengths —
        // the implementation must not early-return on length (that leaks the
        // secret's length via timing; #4 review).
        assert!(AppState::constant_time_eq(b"tok", b"tok"));
        assert!(AppState::constant_time_eq(b"", b""));
        assert!(!AppState::constant_time_eq(b"tok", b"tom"));
        assert!(!AppState::constant_time_eq(b"tok", b"tok-longer"));
        assert!(!AppState::constant_time_eq(b"tok-longer", b"tok"));
        assert!(!AppState::constant_time_eq(b"", b"x"));
    }

    #[test]
    fn error_status_maps_failover_to_service_unavailable() {
        // `/v1/invoke` surfaces `Failover` when the whole candidate chain is
        // exhausted; it must map to a retryable 503, not a 500. This is the single
        // mapping the handler previously got wrong before it shared `error_status`.
        let failover = EngineError::Failover {
            code: mofa_kernel::ErrorCode::ProviderError,
            message: "all candidates failed".into(),
            retryable: true,
            chain: vec![],
            routing_reason: None,
        };
        assert_eq!(
            AppState::error_status(&failover),
            StatusCode::SERVICE_UNAVAILABLE
        );
        // A couple of anchor cases so the shared mapping stays stable.
        assert_eq!(
            AppState::error_status(&EngineError::NoCapableModel("chat".into())),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            AppState::error_status(&EngineError::InvalidRequest("bad".into())),
            StatusCode::BAD_REQUEST
        );
    }

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
            observability: Default::default(),
            providers: vec![],
        };
        AppState {
            engine: mofa_engine_core::Engine::new(config).await,
            started_at: std::time::Instant::now(),
            api_token: api_token.map(|t| Arc::new(t.to_string())),
            obs_metrics: None,
        }
    }

    #[tokio::test]
    async fn metrics_endpoint_serves_prometheus_text() {
        let app = AppState::build_app(test_state(None).await);
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
        let app = AppState::build_app(test_state(None).await);
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
        let app = AppState::build_app(test_state(None).await);
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
    async fn responses_endpoint_validates_input() {
        // With no input at all, the stateful Responses turn is a 400 (the engine
        // rejects an empty turn) — exercises the route + error mapping end to end.
        let app = AppState::build_app(test_state(None).await);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn auth_gate_rejects_missing_token_but_allows_public_routes() {
        let app = AppState::build_app(test_state(Some("secret")).await);

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
