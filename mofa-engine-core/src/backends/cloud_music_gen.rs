//! Cloud music-generation backend (the gcui-art/suno-api gateway contract).
//!
//! Text-to-music is a *task* like cloud video: submit a generation, poll the
//! clips until one is ready, then download the audio — so this mirrors
//! [`CloudVideoGenProvider`](super::cloud_video_gen) behind the same
//! [`Provider`] boundary.
//!
//! ## API contract (https://github.com/gcui-art/suno-api)
//!
//! - `POST /api/generate` `{prompt, make_instrumental, wait_audio:false}` —
//!   standard mode; returns an array of two clip objects.
//! - `POST /api/custom_generate` — Custom Mode: additionally carries
//!   `lyrics`, `style`, and `title`.
//! - `GET /api/get?ids=a,b` — clip info; a clip is finished when its
//!   `status` is `streaming` (audio ready at `audio_url`).
//! - Auth: the Suno session rides in a `Cookie` header; the gateway may also
//!   hold it in its own `SUNO_COOKIE` env, in which case no cookie is needed
//!   here.
//!
//! ## Parameters (`request.params`)
//!
//! - `lyrics`, `style`, `title` — switch to Custom Mode when any is set;
//! - `instrumental` (bool) — instrumental-only track;
//! - `poll_interval_secs` — override the default poll cadence.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind,
};
use reqwest::Client;

use crate::config::ModelDef;

/// The de-facto community gateway reference; point `base_url` at any
/// suno-api compatible deployment (Vercel/Docker/self-host).
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3000";
/// Music renders are slower than chat; poll briskly (the reference client
/// uses 5s — we match it but start immediately after the first interval).
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Backstop so a stuck clip cannot poll forever; the engine's inference
/// timeout is the outer bound.
const MAX_POLL_ATTEMPTS: u32 = 120;
/// The env var a deployment without a configured cookie falls back to.
const API_KEY_ENV: &str = "SUNO_COOKIE";

/// A provider that renders music through a suno-api compatible gateway.
pub(crate) struct CloudMusicGenProvider {
    /// Display name.
    name: String,
    /// Gateway root, e.g. `https://suno.example.com`.
    base_url: String,
    /// Suno session cookie sent as the `Cookie` header. Empty means the
    /// gateway is expected to hold its own credentials.
    api_key: String,
    /// Configured model ids this backend serves (e.g. `suno-v4`).
    models: Vec<ModelDef>,
    /// Cost tier applied to this provider's models.
    cost_tier: CostTier,
    /// Directory for downloaded audio artifacts.
    output_dir: PathBuf,
    /// Shared HTTP client (connection reuse across submit/poll/download).
    client: Client,
}

/// The state of a clip as read from a poll response.
#[derive(Debug, PartialEq, Eq)]
enum ClipState {
    /// Submitted/queued/running — keep polling.
    Pending,
    /// Finished; carries the downloadable audio URL.
    Ready(String),
    /// Terminal failure; carries the gateway's reason.
    Failed(String),
}

/// Pure mapping of a poll clip object into a [`ClipState`].
///
/// The reference client treats `streaming` as "audio ready"; `complete` is
/// accepted as a synonym. Unknown/absent statuses keep polling — the poll
/// cap and engine timeout remain the safety net.
pub(crate) fn parse_clip_state(clip: &serde_json::Value) -> ClipState {
    let status = clip
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let url = clip
        .get("audio_url")
        .or_else(|| clip.get("audioUrl"))
        .and_then(|v| v.as_str())
        .filter(|u| !u.is_empty());
    match status.as_str() {
        "streaming" | "complete" | "completed" => match url {
            Some(u) => ClipState::Ready(u.to_string()),
            None => ClipState::Failed("clip finished but returned no audio URL".into()),
        },
        "error" | "failed" | "blocked" | "flagged" => ClipState::Failed(
            clip.get("error_message")
                .or_else(|| clip.get("errorMessage"))
                .and_then(|v| v.as_str())
                .unwrap_or("clip failed without a reason")
                .to_string(),
        ),
        _ => ClipState::Pending,
    }
}

/// Build the submit body. Any of lyrics/style/title switches to Custom Mode.
pub(crate) fn build_submit_body(prompt: &str, params: &serde_json::Value) -> serde_json::Value {
    let instrumental = params
        .get("instrumental")
        .or_else(|| params.get("make_instrumental"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let lyrics = params
        .get("lyrics")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let style = params
        .get("style")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let title = params
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    if lyrics.is_some() || style.is_some() || title.is_some() {
        let mut body = serde_json::json!({
            "prompt": lyrics.unwrap_or(prompt),
            "make_instrumental": instrumental,
            "wait_audio": false,
        });
        if let Some(lyrics) = lyrics {
            body["lyrics"] = serde_json::json!(lyrics);
        }
        if let Some(style) = style {
            // The gateway accepts the style under either name.
            body["tags"] = serde_json::json!(style);
            body["style"] = serde_json::json!(style);
        }
        if let Some(title) = title {
            body["title"] = serde_json::json!(title);
        }
        body
    } else {
        serde_json::json!({
            "prompt": prompt,
            "make_instrumental": instrumental,
            "wait_audio": false,
        })
    }
}

impl CloudMusicGenProvider {
    pub(crate) fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Result<Self, EngineError> {
        let name = name.into();
        let base_url = {
            let b = base_url.into();
            if b.is_empty() {
                DEFAULT_BASE_URL.to_string()
            } else {
                b.trim_end_matches('/').to_string()
            }
        };
        // The cookie is optional: gateways routinely hold SUNO_COOKIE
        // themselves. We only fall back to the env when none was configured.
        let api_key = api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var(API_KEY_ENV).ok())
            .unwrap_or_default();

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Audio downloads can be sizable; per-request timeout generous.
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| {
                EngineError::Config(format!("failed to build cloud music HTTP client: {e}"))
            })?;

        Ok(Self {
            name,
            base_url,
            api_key,
            models,
            cost_tier,
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
            client,
        })
    }

    fn provider_error(&self, detail: impl std::fmt::Display) -> EngineError {
        EngineError::ProviderError {
            provider: self.name.clone(),
            detail: detail.to_string(),
        }
    }

    fn model_supports(&self, model_name: &str, capability: Capability) -> bool {
        self.models.iter().any(|m| {
            m.name == model_name && Capability::from_str_loose(&m.capability) == Some(capability)
        })
    }

    /// Attach auth: the Suno session rides in the `Cookie` header.
    fn authed(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.is_empty() {
            request
        } else {
            request.header("Cookie", &self.api_key)
        }
    }

    async fn read_json(
        &self,
        resp: reqwest::Response,
        stage: &str,
    ) -> Result<serde_json::Value, EngineError> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| self.provider_error(format!("reading {stage} response body: {e}")))?;
        if !status.is_success() {
            let mut body = text;
            body.truncate(500);
            return Err(self.provider_error(format!("{stage} returned HTTP {status}: {body}")));
        }
        serde_json::from_str(&text)
            .map_err(|e| self.provider_error(format!("parsing {stage} response JSON: {e}")))
    }

    async fn generate(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let prompt = request
            .messages
            .first()
            .map(|m| m.content.clone())
            .or_else(|| {
                request
                    .params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| EngineError::InvalidRequest("music_gen requires a prompt".into()))?;

        let body = build_submit_body(&prompt, &request.params);
        let custom = body.get("lyrics").is_some()
            || body.get("style").is_some()
            || body.get("title").is_some();
        let path = if custom {
            "api/custom_generate"
        } else {
            "api/generate"
        };
        let resp = self
            .authed(self.client.post(format!("{}/{}", self.base_url, path)))
            .json(&body)
            .send()
            .await
            .map_err(|e| self.provider_error(format!("submit request failed: {e}")))?;
        let submitted = self.read_json(resp, "submit").await?;
        // The gateway returns an array of (usually two) clips; we track the first.
        let clip = submitted
            .as_array()
            .and_then(|clips| clips.first())
            .cloned()
            .ok_or_else(|| self.provider_error(format!("submit returned no clips: {submitted}")))?;
        let clip_id = clip
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.provider_error(format!("submit response clip has no id: {clip}")))?
            .to_string();
        tracing::info!(provider = %self.name, %clip_id, "submitted music generation");

        let interval = request
            .params
            .get("poll_interval_secs")
            .and_then(|v| v.as_u64())
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_POLL_INTERVAL);

        let mut audio_url = None;
        let mut last_clip = clip;
        for _ in 0..MAX_POLL_ATTEMPTS {
            tokio::time::sleep(interval).await;
            let resp = self
                .authed(
                    self.client
                        .get(format!("{}/api/get?ids={clip_id}", self.base_url)),
                )
                .send()
                .await
                .map_err(|e| self.provider_error(format!("poll request failed: {e}")))?;
            let polls = self.read_json(resp, "poll").await?;
            last_clip = polls
                .as_array()
                .and_then(|clips| clips.first())
                .cloned()
                .ok_or_else(|| self.provider_error(format!("poll returned no clips: {polls}")))?;
            match parse_clip_state(&last_clip) {
                ClipState::Pending => continue,
                ClipState::Ready(url) => {
                    audio_url = Some(url);
                    break;
                }
                ClipState::Failed(reason) => {
                    return Err(
                        self.provider_error(format!("music clip {clip_id} failed: {reason}"))
                    );
                }
            }
        }
        let audio_url = audio_url.ok_or_else(|| {
            self.provider_error(format!(
                "music clip {clip_id} did not finish within {MAX_POLL_ATTEMPTS} polls"
            ))
        })?;

        let resp = self
            .client
            .get(&audio_url)
            .send()
            .await
            .map_err(|e| self.provider_error(format!("downloading audio: {e}")))?;
        if !resp.status().is_success() {
            return Err(
                self.provider_error(format!("audio download returned HTTP {}", resp.status()))
            );
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| self.provider_error(format!("reading audio bytes: {e}")))?;
        if bytes.is_empty() {
            return Err(self.provider_error("audio download was empty"));
        }
        let file = self
            .output_dir
            .join(format!("mofa_music_{}.mp3", uuid::Uuid::new_v4()));
        tokio::fs::write(&file, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("audio write error: {e}")))?;

        // Carry the clip metadata so callers can label the artifact.
        let title = last_clip
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tags = last_clip.get("tags").and_then(|v| v.as_str()).unwrap_or("");
        let label = match (title.is_empty(), tags.is_empty()) {
            (false, false) => format!("{title} · {tags}"),
            (false, true) => title.to_string(),
            (true, false) => tags.to_string(),
            (true, true) => String::new(),
        };

        Ok(InferenceResponse {
            text: if label.is_empty() { None } else { Some(label) },
            file: Some(file.to_string_lossy().to_string()),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used: None,
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }
}

#[async_trait]
impl Provider for CloudMusicGenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::CloudMusicGen
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![
            BackendFeature::Discovery,
            BackendFeature::Load,
            BackendFeature::Unload,
        ]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        let cards = self
            .models
            .iter()
            .filter_map(|m| {
                let cap = Capability::from_str_loose(&m.capability)?;
                let mut card =
                    ModelCard::new(self.name.clone(), m.name.clone(), cap, self.cost_tier);
                card.id = ModelId::canonical(&self.name, &m.name);
                // The gateway may hold its own credentials, so a missing cookie
                // here is not unavailability; invoke surfaces auth errors honestly.
                card.availability = ModelAvailability::Configured;
                card.residency = ModelResidency::Remote;
                card.refresh_status();
                Some(card)
            })
            .collect();
        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        Ok(BackendHealth::Healthy)
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        // Nothing to warm: remote models are always "loaded" from our side.
        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, ModelId::name(model_id)),
            residency: ModelResidency::Remote,
            memory_bytes: Some(0),
            changed: false,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, ModelId::name(model_id)),
            residency: ModelResidency::Remote,
            memory_bytes: Some(0),
            changed: false,
        })
    }

    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let model_name = ModelId::name(model_id);
        let capability = request.capability.unwrap_or(Capability::MusicGen);

        if capability != Capability::MusicGen {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' only supports music_gen, not {capability}",
                self.name
            )));
        }
        if !self.model_supports(model_name, Capability::MusicGen) {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{model_name}' does not support music_gen",
                self.name
            )));
        }

        self.generate(model_name, request, Instant::now()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::Message;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    fn music_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "suno-v4".into(),
            capability: "music_gen".into(),
            ..Default::default()
        }]
    }

    fn provider() -> CloudMusicGenProvider {
        CloudMusicGenProvider::new(
            "suno",
            "http://127.0.0.1:9",
            Some("session=abc".into()),
            music_models(),
            CostTier::Medium,
            None,
        )
        .unwrap()
    }

    fn music_request(prompt: &str, params: serde_json::Value) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::MusicGen),
            messages: vec![Message {
                role: "user".into(),
                content: prompt.into(),
                images: vec![],
            }],
            params,
            ..Default::default()
        }
    }

    #[test]
    fn submit_body_standard_vs_custom() {
        let standard = build_submit_body("欢快的流行歌", &serde_json::json!({}));
        assert_eq!(standard["prompt"], "欢快的流行歌");
        assert_eq!(standard["wait_audio"], false);
        assert!(standard.get("lyrics").is_none());

        let custom = build_submit_body(
            "欢快的流行歌",
            &serde_json::json!({ "lyrics": "[ verse ]", "style": "pop, upbeat", "title": "晨跑" }),
        );
        assert_eq!(custom["lyrics"], "[ verse ]");
        assert_eq!(custom["tags"], "pop, upbeat");
        assert_eq!(custom["title"], "晨跑");

        let instrumental = build_submit_body("x", &serde_json::json!({ "instrumental": true }));
        assert_eq!(instrumental["make_instrumental"], true);
    }

    #[test]
    fn clip_state_mapping() {
        assert_eq!(
            parse_clip_state(&serde_json::json!({ "status": "queued" })),
            ClipState::Pending
        );
        assert_eq!(
            parse_clip_state(&serde_json::json!({ "status": "running" })),
            ClipState::Pending
        );
        assert_eq!(parse_clip_state(&serde_json::json!({})), ClipState::Pending);
        assert_eq!(
            parse_clip_state(&serde_json::json!({
                "status": "streaming", "audio_url": "https://cdn/x.mp3"
            })),
            ClipState::Ready("https://cdn/x.mp3".into())
        );
        // Finished without a URL is a failure, not a hang.
        assert!(matches!(
            parse_clip_state(&serde_json::json!({ "status": "streaming" })),
            ClipState::Failed(_)
        ));
        assert_eq!(
            parse_clip_state(&serde_json::json!({
                "status": "error", "error_message": "credits exhausted"
            })),
            ClipState::Failed("credits exhausted".into())
        );
    }

    #[tokio::test]
    async fn rejects_non_music_capability_and_unlisted_models() {
        let p = provider();
        let mut req = music_request("x", serde_json::json!({}));
        req.capability = Some(Capability::Chat);
        assert!(matches!(
            p.invoke("suno/suno-v4", &req).await.unwrap_err(),
            EngineError::UnsupportedOperation(_)
        ));
        let req = music_request("x", serde_json::json!({}));
        assert!(matches!(
            p.invoke("suno/other", &req).await.unwrap_err(),
            EngineError::UnsupportedOperation(_)
        ));
    }

    #[tokio::test]
    async fn empty_prompt_is_invalid_request() {
        let p = provider();
        assert!(matches!(
            p.invoke("suno/suno-v4", &music_request("   ", serde_json::json!({})))
                .await
                .unwrap_err(),
            EngineError::InvalidRequest(_)
        ));
    }

    /// Full lifecycle against a scripted mock gateway: submit returns two
    /// pending clips, the first poll is still running, the second is
    /// streaming with a downloadable mp3.
    #[tokio::test]
    async fn full_lifecycle_against_mock_gateway() {
        let polls = Arc::new(AtomicU8::new(0));
        let cookie_seen = Arc::new(AtomicUsize::new(0));
        let polls_handler = polls.clone();
        let cookie_handler = cookie_seen.clone();

        let app = axum::Router::new()
            .route(
                "/api/generate",
                axum::routing::post(move |headers: axum::http::HeaderMap| async move {
                    if headers.contains_key("cookie") {
                        cookie_handler.fetch_add(1, Ordering::SeqCst);
                    }
                    axum::Json(serde_json::json!([
                        { "id": "clip-a", "status": "queued" },
                        { "id": "clip-b", "status": "queued" }
                    ]))
                }),
            )
            .route(
                "/api/get",
                axum::routing::get(move || {
                    let polls = polls_handler.clone();
                    async move {
                        let n = polls.fetch_add(1, Ordering::SeqCst);
                        if n == 0 {
                            axum::Json(serde_json::json!([
                                { "id": "clip-a", "status": "running" }
                            ]))
                        } else {
                            axum::Json(serde_json::json!([
                                {
                                    "id": "clip-a",
                                    "status": "streaming",
                                    "audio_url": "http://127.0.0.1:9/nope.mp3",
                                    "title": "晨跑",
                                    "tags": "pop, upbeat"
                                }
                            ]))
                        }
                    }
                }),
            )
            // The audio_url in the poll points at a dead port, so we cannot
            // exercise the download over this mock — see the download test below.
            ;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let p = CloudMusicGenProvider::new(
            "suno-mock",
            format!("http://{addr}"),
            Some("session=abc".into()),
            music_models(),
            CostTier::Medium,
            None,
        )
        .unwrap();

        let req = music_request(
            "欢快的晨跑歌",
            serde_json::json!({ "poll_interval_secs": 1 }),
        );
        // The download URL is dead; the lifecycle must fail at download, which
        // proves submit + polling + parse all worked.
        let err = p.invoke("suno-mock/suno-v4", &req).await.unwrap_err();
        // Fails at download (connect error, or a proxy-intercepted 502) —
        // which proves submit + polling + clip parsing all worked.
        let message = err.to_string();
        assert!(
            message.contains("downloading audio") || message.contains("audio download"),
            "got: {message}"
        );
        assert_eq!(polls.load(Ordering::SeqCst), 2);
        assert_eq!(
            cookie_seen.load(Ordering::SeqCst),
            1,
            "cookie auth attached"
        );
    }

    /// Download + artifact write against a mock that also serves the mp3.
    #[tokio::test]
    async fn finished_clip_downloads_into_artifact() {
        let app = axum::Router::new()
            .route(
                "/api/generate",
                axum::routing::post(|| async {
                    axum::Json(serde_json::json!([{ "id": "clip-1", "status": "queued" }]))
                }),
            )
            .route(
                "/api/get",
                axum::routing::get(|| async {
                    axum::Json(serde_json::json!([{
                        "id": "clip-1",
                        "status": "streaming",
                        "audio_url": "AUDIO_URL_SENTINEL",
                        "title": "夜航",
                        "tags": "lofi"
                    }]))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let audio_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Serve the mp3 bytes from a second listener so the URL is real.
        let audio_app = axum::Router::new().route(
            "/clip.mp3",
            axum::routing::get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "audio/mpeg")],
                    b"ID3-fake-mp3-bytes".as_slice(),
                )
            }),
        );
        let audio_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mp3_addr = audio_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(audio_listener, audio_app).await.unwrap();
        });

        let p = CloudMusicGenProvider::new(
            "suno-mock2",
            format!("http://{audio_addr}"),
            Some("session=abc".into()),
            music_models(),
            CostTier::Medium,
            None,
        )
        .unwrap();

        // Point audio_url at the real mp3 server via params-free trick: the
        // poll response above uses a sentinel we cannot rewrite — instead run
        // a second gateway whose poll returns the mp3 server URL.
        let gateway_app = axum::Router::new()
            .route(
                "/api/generate",
                axum::routing::post(|| async {
                    axum::Json(serde_json::json!([{ "id": "clip-1", "status": "queued" }]))
                }),
            )
            .route(
                "/api/get",
                axum::routing::get(move || {
                    let url = format!("http://{mp3_addr}/clip.mp3");
                    async move {
                        axum::Json(serde_json::json!([{
                            "id": "clip-1",
                            "status": "streaming",
                            "audio_url": url,
                            "title": "夜航",
                            "tags": "lofi"
                        }]))
                    }
                }),
            );
        let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gw_addr = gw_listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(gw_listener, gateway_app).await.unwrap();
        });

        let p = CloudMusicGenProvider::new(
            "suno-mock3",
            format!("http://{gw_addr}"),
            Some("session=abc".into()),
            music_models(),
            CostTier::Medium,
            None,
        )
        .unwrap();

        let req = music_request("安静的夜航", serde_json::json!({ "poll_interval_secs": 1 }));
        let resp = p.invoke("suno-mock3/suno-v4", &req).await.unwrap();
        let file = resp.file.expect("artifact path");
        assert!(file.ends_with(".mp3"));
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, b"ID3-fake-mp3-bytes");
        assert_eq!(resp.text.as_deref(), Some("夜航 · lofi"));
    }
}
