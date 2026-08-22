//! mofa-studio — the out-of-the-box creative studio.
//!
//!   cargo run -p mofa-studio            # then open http://127.0.0.1:8787
//!
//! This is the "usable, validated, users-directly-experience-it" deliverable: a
//! single binary that boots the MoFA engine and serves a browser UI where anyone
//! types a prompt and gets a **real image or video** back — no SDK, no CLI flags,
//! no code. It is deliberately *not* another example script; it is the framework
//! wearing a face a non-developer can use.
//!
//! ## What it proves about the framework
//!
//! Every generation goes through the ordinary engine contract
//! (`Capability::ImageGen` / `Capability::VideoGen`), so the UI can surface the
//! things that make MoFA more than a thin API wrapper: **which provider served
//! the request, whether it ran local or cloud, and what it cost**. That is the
//! "practical value" made visible.
//!
//! ## Bring-your-own model (the validation path)
//!
//! Offline, chat is local Ollama and there is no zero-config image/video backend,
//! so those buttons report honestly that no backend is configured. Set a cloud
//! key and the same UI renders real media:
//!
//!   AGNES_API_KEY=...  AGNES_BASE_URL=https://.../v1   cargo run -p mofa-studio
//!
//! `AGNES_*` wires the mentor-provided free omni-modal gateway (Agnes AI) as an
//! OpenAI-compatible provider for chat + image; `ARK_API_KEY` (see the
//! `cloud_video_gen` backend) wires Seedance for video. The model ids are
//! overridable via `AGNES_CHAT_MODEL` / `AGNES_IMAGE_MODEL`.
//!
//! TODO(video): once Agnes's video endpoint shape is confirmed, wire it here the
//! same way (either as a `cloud_video_gen` base_url or a dedicated adapter).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use clap::Parser;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, Prefer};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

/// Prompt → image / video, in the browser, via the MoFA engine.
#[derive(Parser, Debug)]
#[command(name = "mofa-studio", version, about)]
struct Cli {
    /// Port for the local web UI.
    #[arg(long, default_value_t = 8787)]
    port: u16,

    /// Use a specific engine config instead of the auto-provisioned one.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mofa_studio=info,mofa_engine_core=warn".into()),
        )
        .init();

    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let cli = Cli::parse();

    // The engine writes every artifact into this directory; we then serve it
    // statically so the browser can load the produced image/video by URL.
    let artifacts_dir = studio_artifacts_dir()?;

    let config_path = match &cli.config {
        Some(p) => Some(p.clone()),
        None => Some(provision_config(&artifacts_dir)?),
    };
    let engine = Engine::try_new(EngineConfig::load(config_path.as_deref()))
        .await
        .map_err(|e| format!("engine init failed: {e}"))?;
    engine.refresh_resources().await;

    let state = AppState {
        engine,
        artifacts_dir: artifacts_dir.clone(),
    };

    // The whole app: the embedded UI, a tiny JSON API, and static artifact serving.
    let app = Router::new()
        .route("/", get(index))
        .route("/api/capabilities", get(capabilities))
        .route("/api/generate", post(generate))
        .nest_service("/artifacts", ServeDir::new(&artifacts_dir))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("binding {addr}: {e}"))?;

    println!("\n  🎨 mofa-studio — open  http://{addr}\n");
    println!("  artifacts: {}\n", artifacts_dir.display());
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}

// ==============================================================================
// Shared state
// ==============================================================================

#[derive(Clone)]
struct AppState {
    engine: Arc<Engine>,
    artifacts_dir: PathBuf,
}

// ==============================================================================
// API — capabilities
// ==============================================================================

/// One row per (capability, provider) the engine can currently serve. The UI uses
/// this to tell the user *before* they click whether image/video is wired up.
#[derive(Serialize)]
struct CapabilityRow {
    capability: String,
    provider: String,
    available: bool,
    local: bool,
}

async fn capabilities(State(state): State<AppState>) -> Json<Vec<CapabilityRow>> {
    let rows = state
        .engine
        .capabilities()
        .await
        .into_iter()
        .map(|c| CapabilityRow {
            capability: c.capability.to_string(),
            provider: c.provider,
            available: matches!(
                c.availability,
                mofa_kernel::ModelAvailability::Discovered
                    | mofa_kernel::ModelAvailability::Configured
            ),
            local: matches!(
                c.residency,
                mofa_kernel::ModelResidency::Loaded
                    | mofa_kernel::ModelResidency::Unloaded
                    | mofa_kernel::ModelResidency::Loading
            ),
        })
        .collect();
    Json(rows)
}

// ==============================================================================
// API — generate
// ==============================================================================

#[derive(Deserialize)]
struct GenerateRequest {
    prompt: String,
    /// `"image"` or `"video"`.
    mode: String,
}

#[derive(Serialize)]
struct GenerateResponse {
    ok: bool,
    /// URL of the produced artifact under `/artifacts`, when successful.
    url: Option<String>,
    provider: Option<String>,
    /// `true` when served on-device (cost ≈ $0), `false` for a metered cloud call.
    local: Option<bool>,
    cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    /// Human-readable error when `ok` is false (e.g. "no image backend configured").
    error: Option<String>,
}

async fn generate(
    State(state): State<AppState>,
    Json(req): Json<GenerateRequest>,
) -> impl IntoResponse {
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(GenerateResponse::err("a prompt is required")),
        );
    }

    // Map the UI mode to an engine capability. Both are cloud-heavy, so we let the
    // router use cloud (`prefer=auto`) rather than pinning local.
    let (capability, params) = match req.mode.as_str() {
        "image" => (
            Capability::ImageGen,
            serde_json::json!({ "size": "1024x1024" }),
        ),
        "video" => (
            Capability::VideoGen,
            serde_json::json!({ "ratio": "16:9", "resolution": "720p", "duration": 5 }),
        ),
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(GenerateResponse::err(&format!("unknown mode '{other}'"))),
            );
        }
    };

    let request = InferenceRequest {
        capability: Some(capability),
        messages: vec![Message {
            role: "user".into(),
            content: prompt.to_string(),
            ..Default::default()
        }],
        params,
        prefer: Prefer::Auto,
        ..Default::default()
    };

    match state.engine.invoke(request).await {
        Ok(resp) => {
            let url = resp
                .file
                .as_deref()
                .and_then(|f| artifact_url(&state.artifacts_dir, f));
            // Locality is a property of where the model ran, not of cost (a cloud
            // model can bill $0). Read it from the serving provider's residency:
            // a remote card means cloud, anything else is on-device.
            let local = state
                .engine
                .capabilities()
                .await
                .iter()
                .find(|c| c.provider == resp.provider)
                .map(|c| c.residency != mofa_kernel::ModelResidency::Remote);
            (
                StatusCode::OK,
                Json(GenerateResponse {
                    ok: url.is_some(),
                    error: if url.is_some() {
                        None
                    } else {
                        Some("backend returned no downloadable artifact".into())
                    },
                    url,
                    provider: Some(resp.provider),
                    local,
                    cost_usd: resp.cost_usd,
                    duration_ms: Some(resp.duration_ms),
                }),
            )
        }
        Err(e) => {
            // A missing backend is the common, expected case offline — report it
            // as a clean message the UI can show, not a 500.
            let info = e.info();
            (
                StatusCode::OK,
                Json(GenerateResponse::err(&format!(
                    "{:?}: {}",
                    info.code, info.message
                ))),
            )
        }
    }
}

impl GenerateResponse {
    fn err(msg: &str) -> Self {
        Self {
            ok: false,
            url: None,
            provider: None,
            local: None,
            cost_usd: None,
            duration_ms: None,
            error: Some(msg.to_string()),
        }
    }
}

/// Turn an engine artifact path into a `/artifacts/<file>` URL, but only if the
/// file really lives under our served directory (defence against serving a path
/// a provider wrote elsewhere).
fn artifact_url(dir: &std::path::Path, file: &str) -> Option<String> {
    let path = std::path::Path::new(file);
    let name = path.file_name()?.to_str()?;
    let canonical_dir = std::fs::canonicalize(dir).ok()?;
    let canonical_file = std::fs::canonicalize(path).ok()?;
    canonical_file
        .starts_with(&canonical_dir)
        .then(|| format!("/artifacts/{name}"))
}

// ==============================================================================
// UI
// ==============================================================================

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// The whole front-end: one self-contained page, no build step, no external
/// assets. Kept intentionally small — the point is to *drive* the engine, not to
/// be a design showcase.
const INDEX_HTML: &str = include_str!("index.html");

// ==============================================================================
// Config provisioning
// ==============================================================================

/// The directory every generated artifact is written to (and served from). Uses
/// the OS cache dir so repeated runs share it and it survives a `cargo clean`.
fn studio_artifacts_dir() -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mofa-studio")
        .join("artifacts");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Write the engine config: always local Ollama for chat + the studio artifact
/// dir; add the Agnes gateway (chat + image) when `AGNES_API_KEY` is present, so
/// the exact same binary is offline-friendly *and* cloud-validated depending only
/// on the environment.
fn provision_config(artifacts_dir: &std::path::Path) -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mofa-studio");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    let mut toml = format!(
        "# Auto-generated by mofa-studio.\n\
         # Cloud video generation is a slow, poll-until-done task (a clip can take
         # several minutes), so the request/inference budgets are widened well past
         # the engine defaults — otherwise a video that IS rendering server-side is
         # killed mid-poll. `[timeouts]` is in seconds.\n\
         [timeouts]\n\
         request_secs = 900\n\
         inference_secs = 840\n\n\
         [artifacts]\n\
         dir = {artifacts:?}\n\n\
         [[providers]]\n\
         name = \"ollama\"\n\
         kind = \"ollama\"\n\
         base_url = \"http://127.0.0.1:11434\"\n\
         priority = 2\n\
         cost_tier = \"free\"\n",
        artifacts = artifacts_dir.to_string_lossy(),
    );

    // Agnes AI (mentor-provided free omni-modal gateway): OpenAI-compatible, so it
    // slots in as an `openai_compatible` provider for chat + image. Only added when
    // a key is present — otherwise the studio stays a clean offline build.
    if let Ok(key) = std::env::var("AGNES_API_KEY")
        && !key.trim().is_empty()
    {
        let base = std::env::var("AGNES_BASE_URL")
            .unwrap_or_else(|_| "https://apihub.agnes-ai.com/v1".into());
        let chat = std::env::var("AGNES_CHAT_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".into());
        let image =
            std::env::var("AGNES_IMAGE_MODEL").unwrap_or_else(|_| "agnes-image-2.1-flash".into());
        let video =
            std::env::var("AGNES_VIDEO_MODEL").unwrap_or_else(|_| "agnes-video-v2.0".into());
        // Chat + image share one OpenAI-compatible provider…
        toml.push_str(&format!(
            "\n[[providers]]\n\
             name = \"agnes\"\n\
             kind = \"openai_compatible\"\n\
             base_url = {base:?}\n\
             api_key = \"env:AGNES_API_KEY\"\n\
             priority = 1\n\
             cost_tier = \"low\"\n\
             [[providers.models]]\n\
             name = {chat:?}\n\
             capability = \"chat\"\n\
             [[providers.models]]\n\
             name = {image:?}\n\
             capability = \"image_gen\"\n",
        ));
        // …video is a separate task-based endpoint, so it needs the cloud_video_gen
        // backend with the `agnes` dialect (submit /videos → poll → download).
        toml.push_str(&format!(
            "\n[[providers]]\n\
             name = \"agnes-video\"\n\
             kind = \"cloud_video_gen\"\n\
             dialect = \"agnes\"\n\
             base_url = {base:?}\n\
             api_key = \"env:AGNES_API_KEY\"\n\
             priority = 1\n\
             cost_tier = \"low\"\n\
             [[providers.models]]\n\
             name = {video:?}\n\
             capability = \"video_gen\"\n",
        ));
        tracing::info!("Agnes gateway configured (chat + image + video)");
    }

    let config = dir.join("config.toml");
    std::fs::write(&config, toml).map_err(|e| format!("writing {}: {e}", config.display()))?;
    Ok(config)
}
