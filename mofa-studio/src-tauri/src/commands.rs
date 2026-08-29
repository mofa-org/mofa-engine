//! Tauri commands: every one drives the engine through the SDK's in-process
//! `AsyncEmbeddedEngine`. Chat streams chunks to the webview as `chat://chunk`
//! events; image/video generation blocks and returns a result.

use mofa_engine_sdk::AsyncEmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, Message, ModelResidency, Prefer, StreamChunk};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// `AsyncEmbeddedEngine` is cheap to clone (an `Arc` inside); commands clone it out
/// before any `.await` so they never hold the `State` borrow across a suspension.
pub struct AppState {
    pub engine: AsyncEmbeddedEngine,
}

/// One row per (capability, provider) the engine can serve; drives the UI's
/// availability chips and on-device/cloud labelling.
#[derive(Serialize)]
pub struct CapabilityRow {
    pub capability: String,
    pub provider: String,
    pub available: bool,
    pub local: bool,
}

#[tauri::command]
pub async fn get_capabilities(state: State<'_, AppState>) -> Result<Vec<CapabilityRow>, String> {
    let engine = state.engine.clone();
    let rows = engine
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
            local: c.residency != ModelResidency::Remote,
        })
        .collect();
    Ok(rows)
}

/// A chat turn from the UI. `role` is `"system" | "user" | "assistant"`.
#[derive(Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// Per-chunk envelope for the webview. `stream_id` correlates chunks to the request
/// that produced them; `chunk` is the engine's `StreamChunk`, serialized tagged
/// (`{"type":"text","delta":"…"}`) so the UI switches on `chunk.type`.
#[derive(Serialize, Clone)]
struct ChunkEvent {
    stream_id: String,
    chunk: StreamChunk,
}

/// Stream a chat completion; content arrives as `chat://chunk` events keyed by
/// `stream_id`. Returns once the stream closes.
#[tauri::command]
pub async fn chat_stream(
    app: AppHandle,
    state: State<'_, AppState>,
    stream_id: String,
    messages: Vec<ChatMessage>,
) -> Result<(), String> {
    let engine = state.engine.clone();

    let request = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: messages
            .into_iter()
            .map(|m| Message {
                role: m.role,
                content: m.content,
                ..Default::default()
            })
            .collect(),
        prefer: Prefer::Auto,
        request_id: stream_id.clone(),
        ..Default::default()
    };

    let mut rx = engine.invoke_stream(request);
    while let Some(chunk) = rx.recv().await {
        app.emit(
            "chat://chunk",
            ChunkEvent {
                stream_id: stream_id.clone(),
                chunk,
            },
        )
        .map_err(|e| format!("emitting chunk: {e}"))?;
    }
    Ok(())
}

/// One-shot media result. `path` is an absolute artifact path (loaded by the webview
/// via `convertFileSrc`); `None` on failure, with `error` set for the UI.
#[derive(Serialize)]
pub struct GenResult {
    pub ok: bool,
    pub path: Option<String>,
    pub provider: Option<String>,
    pub local: Option<bool>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

impl GenResult {
    fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            path: None,
            provider: None,
            local: None,
            cost_usd: None,
            duration_ms: None,
            error: Some(msg.into()),
        }
    }
}

/// Shared image/video generation: one engine capability with capability-specific params.
async fn generate(
    engine: &AsyncEmbeddedEngine,
    capability: Capability,
    prompt: &str,
    params: serde_json::Value,
) -> GenResult {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return GenResult::err("a prompt is required");
    }

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

    match engine.invoke(request).await {
        Ok(resp) => {
            // Locality is where the model ran, not cost (a cloud model can bill $0):
            // read it from the serving provider's residency.
            let local = engine
                .capabilities()
                .await
                .iter()
                .find(|c| c.provider == resp.provider)
                .map(|c| c.residency != ModelResidency::Remote);
            match resp.file {
                Some(file) => GenResult {
                    ok: true,
                    path: Some(file),
                    provider: Some(resp.provider),
                    local,
                    cost_usd: resp.cost_usd,
                    duration_ms: Some(resp.duration_ms),
                    error: None,
                },
                None => GenResult::err("backend returned no downloadable artifact"),
            }
        }
        // A missing backend is the expected case offline — return a clean message.
        Err(e) => {
            let info = e.info();
            GenResult::err(format!("{:?}: {}", info.code, info.message))
        }
    }
}

#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    prompt: String,
    size: Option<String>,
) -> Result<GenResult, String> {
    let engine = state.engine.clone();
    let size = size.unwrap_or_else(|| "1024x1024".into());
    Ok(generate(
        &engine,
        Capability::ImageGen,
        &prompt,
        serde_json::json!({ "size": size }),
    )
    .await)
}

#[tauri::command]
pub async fn generate_video(
    state: State<'_, AppState>,
    prompt: String,
    resolution: Option<String>,
    duration: Option<u32>,
    ratio: Option<String>,
) -> Result<GenResult, String> {
    let engine = state.engine.clone();
    Ok(generate(
        &engine,
        Capability::VideoGen,
        &prompt,
        serde_json::json!({
            "ratio": ratio.unwrap_or_else(|| "16:9".into()),
            "resolution": resolution.unwrap_or_else(|| "720p".into()),
            "duration": duration.unwrap_or(5),
        }),
    )
    .await)
}
