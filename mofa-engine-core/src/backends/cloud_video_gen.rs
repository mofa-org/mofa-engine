//! Cloud video-generation backend (task-based APIs; see [`VideoDialect`]).
//!
//! This is the *API-level* video path the local process-adapter
//! ([`LocalVideoGenProvider`](super::local_video_gen)) could not offer: a real
//! text-to-video (and image-to-video) model reached over HTTP. Two vendor
//! dialects are supported behind one backend: **Ark** (the Volcengine Ark /
//! BytePlus "content generation tasks" contract that ByteDance's **Seedance**
//! models speak) and **Agnes** (the Agnes AI gateway's `/videos` task API). So a
//! scenario can render a genuine clip from a prompt without any local generator
//! installed.
//!
//! ## Why this is a distinct backend
//!
//! Our multi-vendor gateway (`liter-llm`) exposes chat and image endpoints but
//! **no video endpoint**, and Seedance is not synchronous the way chat/image are:
//! generation is a *task* you submit, poll for completion, and then download.
//! That submit → poll → download shape does not fit the chat/image providers, so
//! it lives here behind the same [`Provider`] boundary as every other backend —
//! the engine discovers it, routes to it, and manages it identically.
//!
//! ## API contract
//!
//! Submit a task (`POST {base_url}/contents/generations/tasks`):
//!
//! ```json
//! { "model": "<seedance model>",
//!   "content": [ { "type": "text",
//!                  "text": "<prompt> --ratio 16:9 --resolution 1080p --duration 5" } ] }
//! ```
//!
//! For image-to-video an extra `{"type":"image_url","image_url":{"url":…}}` item is
//! appended. The response carries a task `id`. We then poll
//! `GET {base_url}/contents/generations/tasks/{id}` until `status` is `succeeded`
//! (yielding `content.video_url`) or `failed`, and finally download the video into
//! a managed artifact. The produced clip should still clear the
//! [`quality_gate`](crate::quality_gate) before being presented as an S4 artifact.
//!
//! ## Parameters (`request.params`)
//!
//! Seedance takes generation knobs as text-command suffixes appended to the
//! prompt, so we translate the familiar keys into that form:
//!   - `ratio` (e.g. `"16:9"`), `resolution` (e.g. `"1080p"`),
//!   - `duration` / `seconds` (integer seconds), `fps`, `seed`,
//!   - `image_url` — a reference image for image-to-video,
//!   - `poll_interval_secs` — override the default poll cadence.

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

/// Default Ark endpoint (Volcengine, China). Operators pointing at BytePlus
/// (international) or a private gateway override this via `base_url`.
const DEFAULT_ARK_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
/// Default Agnes AI gateway endpoint.
const DEFAULT_AGNES_BASE_URL: &str = "https://apihub.agnes-ai.com/v1";
/// How often to poll a running task when the request does not say otherwise.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Upper bound on poll attempts. Generation is slow but not unbounded; the engine
/// also enforces its own inference timeout, whose dropped future cancels the poll
/// loop. This cap is a backstop so a stuck task cannot poll forever.
const MAX_POLL_ATTEMPTS: u32 = 200;

/// Which vendor's task-based video API this provider speaks. Both share the
/// submit → poll → download shape but differ in endpoint paths, request body, and
/// where the finished URL sits in the poll response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoDialect {
    /// Volcengine Ark / BytePlus (ByteDance Seedance): `/contents/generations/tasks`,
    /// prompt carried as a `content` array, URL at `content.video_url`.
    Ark,
    /// Agnes AI gateway: `/videos`, flat `{prompt,width,height,num_frames,frame_rate}`
    /// body, URL at `metadata.url`.
    Agnes,
}

impl VideoDialect {
    /// Parse the config `dialect` string; unknown/empty defaults to [`Ark`].
    fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "agnes" => Self::Agnes,
            _ => Self::Ark,
        }
    }

    /// The environment variable this dialect resolves a missing key from.
    fn api_key_env(self) -> &'static str {
        match self {
            Self::Ark => "ARK_API_KEY",
            Self::Agnes => "AGNES_API_KEY",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::Ark => DEFAULT_ARK_BASE_URL,
            Self::Agnes => DEFAULT_AGNES_BASE_URL,
        }
    }
}

/// A provider that renders video through a task-based cloud API (see [`VideoDialect`]).
pub(crate) struct CloudVideoGenProvider {
    name: String,
    dialect: VideoDialect,
    base_url: String,
    /// Empty means "no credentials" → the provider reports itself unavailable
    /// rather than failing engine startup, so an offline/keyless config boots.
    api_key: String,
    models: Vec<ModelDef>,
    cost_tier: CostTier,
    output_dir: PathBuf,
    client: Client,
}

/// The state of a generation task as read from a poll response.
#[derive(Debug, PartialEq, Eq)]
enum TaskState {
    /// Queued or running — keep polling.
    Pending,
    /// Finished; carries the downloadable video URL.
    Succeeded(String),
    /// Terminal failure; carries the vendor's reason.
    Failed(String),
}

impl CloudVideoGenProvider {
    /// Build a cloud video provider for the given `dialect` (`"ark"` or `"agnes"`).
    ///
    /// An empty `api_key` falls back to the dialect's environment variable
    /// ([`VideoDialect::api_key_env`]); if that too is empty the provider is
    /// constructed but stays unavailable, so a config that lists it without a key
    /// does not break engine startup.
    pub(crate) fn new(
        name: impl Into<String>,
        dialect: &str,
        base_url: impl Into<String>,
        api_key: Option<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Result<Self, EngineError> {
        let name = name.into();
        let dialect = VideoDialect::from_str_loose(dialect);
        let base_url = {
            let b = base_url.into();
            if b.is_empty() {
                dialect.default_base_url().to_string()
            } else {
                // Tolerate a trailing slash so `{base}/…` never doubles it.
                b.trim_end_matches('/').to_string()
            }
        };

        let key_env = dialect.api_key_env();
        let api_key = api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var(key_env).ok())
            .unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!(
                provider = %name,
                "cloud video provider has no {key_env}; marked unavailable"
            );
        }

        // Video renders are slow; give the per-request calls a generous read
        // timeout. The overall wall-clock is still bounded by MAX_POLL_ATTEMPTS
        // and the engine's inference timeout.
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| {
                EngineError::Config(format!("failed to build cloud video HTTP client: {e}"))
            })?;

        Ok(Self {
            name,
            dialect,
            base_url,
            api_key,
            models,
            cost_tier,
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
            client,
        })
    }

    /// Whether usable credentials were resolved.
    fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn provider_error(&self, detail: impl std::fmt::Display) -> EngineError {
        EngineError::ProviderError {
            provider: self.name.clone(),
            detail: detail.to_string(),
        }
    }

    /// Whether a configured model serves the given capability.
    fn model_supports(&self, model_name: &str, capability: Capability) -> bool {
        self.models.iter().any(|m| {
            m.name == model_name && Capability::from_str_loose(&m.capability) == Some(capability)
        })
    }

    // Pure request/response mapping (unit-tested without the network).

    /// Translate the familiar size/duration knobs into the Seedance text-command
    /// suffix appended to the prompt (e.g. ` --ratio 16:9 --resolution 1080p`).
    ///
    /// Only keys actually present are emitted, so we never override a model's
    /// defaults with guesses.
    fn command_suffix(params: &serde_json::Value) -> String {
        let mut suffix = String::new();
        let mut push_str = |flag: &str, key: &str| {
            if let Some(v) = params.get(key).and_then(|v| v.as_str())
                && !v.trim().is_empty()
            {
                suffix.push_str(&format!(" --{flag} {}", v.trim()));
            }
        };
        push_str("ratio", "ratio");
        push_str("resolution", "resolution");

        // `duration` is preferred; `seconds` is accepted as an alias so callers can
        // reuse the local video-gen vocabulary.
        let duration = params
            .get("duration")
            .or_else(|| params.get("seconds"))
            .and_then(serde_json::Value::as_u64);
        if let Some(d) = duration {
            suffix.push_str(&format!(" --duration {d}"));
        }
        if let Some(fps) = params.get("fps").and_then(serde_json::Value::as_u64) {
            suffix.push_str(&format!(" --fps {fps}"));
        }
        if let Some(seed) = params.get("seed").and_then(serde_json::Value::as_i64) {
            suffix.push_str(&format!(" --seed {seed}"));
        }
        suffix
    }

    /// Pixel dimensions for the Agnes body: an explicit `width`/`height`, a
    /// `size` string (`"WxH"`), or a 3:2 720p-ish default the gateway will snap
    /// to its nearest preset.
    fn dimensions(params: &serde_json::Value) -> (u32, u32) {
        if let (Some(w), Some(h)) = (
            Self::positive_u32(params, "width"),
            Self::positive_u32(params, "height"),
        ) {
            return (w, h);
        }
        if let Some((w, h)) = params
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(|s| s.split_once(['x', 'X']))
            .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))
        {
            return (w, h);
        }
        (1152, 768)
    }

    /// Read a strictly-positive `u32` from a params key, or `None`.
    fn positive_u32(params: &serde_json::Value, key: &str) -> Option<u32> {
        params
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .filter(|v| *v > 0)
            .map(|v| v as u32)
    }

    /// Build the JSON task body for the given [`VideoDialect`].
    fn build_task_body(
        dialect: VideoDialect,
        model_name: &str,
        prompt: &str,
        params: &serde_json::Value,
    ) -> serde_json::Value {
        match dialect {
            VideoDialect::Ark => Self::build_ark_body(model_name, prompt, params),
            VideoDialect::Agnes => Self::build_agnes_body(model_name, prompt, params),
        }
    }

    /// Ark/Seedance body: prompt (plus generation knobs as text-command suffixes)
    /// carried in a `content` array. Adds an `image_url` item for image-to-video.
    fn build_ark_body(
        model_name: &str,
        prompt: &str,
        params: &serde_json::Value,
    ) -> serde_json::Value {
        let text = format!("{prompt}{}", Self::command_suffix(params));
        let mut content = vec![serde_json::json!({ "type": "text", "text": text })];
        if let Some(url) = params.get("image_url").and_then(|v| v.as_str())
            && !url.is_empty()
        {
            content.push(serde_json::json!({
                "type": "image_url",
                "image_url": { "url": url }
            }));
        }
        serde_json::json!({ "model": model_name, "content": content })
    }

    /// Agnes body: a flat object with explicit pixel dimensions and frame count.
    /// The familiar `ratio`/`resolution`/`duration` knobs are translated into
    /// `width`/`height`/`num_frames` (Agnes snaps to its nearest size preset).
    fn build_agnes_body(
        model_name: &str,
        prompt: &str,
        params: &serde_json::Value,
    ) -> serde_json::Value {
        let (width, height) = Self::dimensions(params);
        let frame_rate = Self::positive_u32(params, "fps")
            .or_else(|| Self::positive_u32(params, "frame_rate"))
            .unwrap_or(24);
        // Prefer an explicit num_frames; otherwise derive from duration × fps (+1,
        // since generators count an inclusive final frame). Default ~5s.
        let num_frames = Self::positive_u32(params, "num_frames").unwrap_or_else(|| {
            let seconds = params
                .get("duration")
                .or_else(|| params.get("seconds"))
                .and_then(serde_json::Value::as_u64)
                .filter(|s| *s > 0)
                .unwrap_or(5) as u32;
            seconds * frame_rate + 1
        });
        serde_json::json!({
            "model": model_name,
            "prompt": prompt,
            "width": width,
            "height": height,
            "num_frames": num_frames,
            "frame_rate": frame_rate,
        })
    }

    /// Extract the task id from a submit response.
    fn parse_task_id(body: &serde_json::Value) -> Result<String, String> {
        body.get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .ok_or_else(|| format!("submit response missing task id: {body}"))
    }

    /// Interpret a poll response into a [`TaskState`].
    ///
    /// Unknown/absent statuses are treated as still-pending so a transient shape
    /// we do not recognise makes us wait rather than crash — the poll cap and
    /// engine timeout remain the safety net.
    fn parse_task_state(body: &serde_json::Value) -> TaskState {
        let status = body
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match status.as_str() {
            "succeeded" | "success" | "done" | "completed" => {
                // The finished URL location varies by dialect/version: `content.video_url`
                // (Ark), a top-level `video_url`, a top-level `url` (Agnes `/videos`
                // actually returns the clip here), or `metadata.url` — accept any.
                let url = body
                    .get("content")
                    .and_then(|c| c.get("video_url"))
                    .or_else(|| body.get("video_url"))
                    .or_else(|| body.get("url"))
                    .or_else(|| body.get("metadata").and_then(|m| m.get("url")))
                    .and_then(|v| v.as_str());
                match url {
                    Some(u) if !u.is_empty() => TaskState::Succeeded(u.to_string()),
                    _ => TaskState::Failed("task succeeded but returned no video URL".into()),
                }
            }
            "failed" | "error" | "cancelled" | "canceled" => {
                let reason = body
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .or_else(|| body.get("metadata").and_then(|m| m.get("message")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("task failed without a reason");
                TaskState::Failed(reason.to_string())
            }
            _ => TaskState::Pending,
        }
    }

    // ==========================================================================
    // HTTP orchestration (submit → poll → download)
    // ==========================================================================

    /// Endpoint paths differ per dialect (Ark nests under `contents/generations`).
    fn tasks_path(&self) -> &'static str {
        match self.dialect {
            VideoDialect::Ark => "contents/generations/tasks",
            VideoDialect::Agnes => "videos",
        }
    }

    async fn submit_task(&self, body: &serde_json::Value) -> Result<String, EngineError> {
        let url = format!("{}/{}", self.base_url, self.tasks_path());
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| self.provider_error(format!("submit request failed: {e}")))?;
        let json = self.read_json(resp, "submit").await?;
        Self::parse_task_id(&json).map_err(|e| self.provider_error(e))
    }

    async fn poll_task(&self, task_id: &str) -> Result<serde_json::Value, EngineError> {
        let url = format!("{}/{}/{task_id}", self.base_url, self.tasks_path());
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| self.provider_error(format!("poll request failed: {e}")))?;
        self.read_json(resp, "poll").await
    }

    /// Read a JSON body, turning a non-2xx status into a descriptive error that
    /// includes the (truncated) response body — the vendor puts the actionable
    /// reason there.
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

    /// Download the finished clip into a managed artifact and return its path.
    async fn download(&self, video_url: &str) -> Result<String, EngineError> {
        let resp = self
            .client
            .get(video_url)
            .send()
            .await
            .map_err(|e| self.provider_error(format!("downloading video: {e}")))?;
        if !resp.status().is_success() {
            return Err(
                self.provider_error(format!("video download returned HTTP {}", resp.status()))
            );
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| self.provider_error(format!("reading video bytes: {e}")))?;
        if bytes.is_empty() {
            return Err(self.provider_error("video download was empty"));
        }
        let path = self
            .output_dir
            .join(format!("mofa_video_{}.mp4", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("video write error: {e}")))?;
        Ok(path.to_string_lossy().to_string())
    }

    async fn generate(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: Instant,
    ) -> Result<InferenceResponse, EngineError> {
        if !self.is_available() {
            return Err(self.provider_error(format!(
                "no {} credentials configured",
                self.dialect.api_key_env()
            )));
        }

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
            .ok_or_else(|| EngineError::InvalidRequest("video_gen requires a prompt".into()))?;

        let body = Self::build_task_body(self.dialect, model_name, &prompt, &request.params);
        let task_id = self.submit_task(&body).await?;
        tracing::info!(provider = %self.name, %task_id, "submitted video generation task");

        let interval = request
            .params
            .get("poll_interval_secs")
            .and_then(serde_json::Value::as_u64)
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_POLL_INTERVAL);

        // Poll until the task reaches a terminal state or we exhaust the backstop.
        let mut video_url = None;
        for _ in 0..MAX_POLL_ATTEMPTS {
            tokio::time::sleep(interval).await;
            let poll = self.poll_task(&task_id).await?;
            match Self::parse_task_state(&poll) {
                TaskState::Pending => continue,
                TaskState::Succeeded(url) => {
                    video_url = Some(url);
                    break;
                }
                TaskState::Failed(reason) => {
                    return Err(
                        self.provider_error(format!("video task {task_id} failed: {reason}"))
                    );
                }
            }
        }
        let video_url = video_url.ok_or_else(|| {
            self.provider_error(format!(
                "video task {task_id} did not finish within {MAX_POLL_ATTEMPTS} polls"
            ))
        })?;

        let file = self.download(&video_url).await?;

        Ok(InferenceResponse {
            text: None,
            file: Some(file),
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
impl Provider for CloudVideoGenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::CloudVideoGen
    }

    fn features(&self) -> Vec<BackendFeature> {
        // No memory reporting: the model runs remotely and consumes no local RAM.
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
                card.availability = if self.is_available() {
                    ModelAvailability::Configured
                } else {
                    ModelAvailability::Unavailable
                };
                // Cloud models are remote: they hold no local model memory.
                card.residency = ModelResidency::Remote;
                card.refresh_status();
                Some(card)
            })
            .collect();
        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        if self.is_available() {
            Ok(BackendHealth::Healthy)
        } else {
            Ok(BackendHealth::Unavailable)
        }
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
        let capability = request.capability.unwrap_or(Capability::VideoGen);

        if capability != Capability::VideoGen {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' only supports video_gen, not {capability}",
                self.name
            )));
        }
        if !self.model_supports(model_name, Capability::VideoGen) {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{model_name}' does not support video_gen",
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

    fn video_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "seedance-1-0-pro".into(),
            capability: "video_gen".into(),
            context_window: None,
            memory_mb: None,
            ..Default::default()
        }]
    }

    fn provider(api_key: Option<String>) -> CloudVideoGenProvider {
        CloudVideoGenProvider::new(
            "ark",
            "ark",
            "",
            api_key,
            video_models(),
            CostTier::High,
            Some(std::env::temp_dir().to_string_lossy().to_string()),
        )
        .expect("HTTP client builds")
    }

    fn video_request(prompt: &str) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::VideoGen),
            messages: vec![Message {
                role: "user".into(),
                content: prompt.into(),
                ..Default::default()
            }],
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_base_url_defaults_per_dialect() {
        let ark = provider(Some("k".into()));
        assert_eq!(ark.base_url, DEFAULT_ARK_BASE_URL);
        assert_eq!(ark.dialect, VideoDialect::Ark);
        assert_eq!(ark.kind(), ProviderKind::CloudVideoGen);
        // Cloud: not counted as a local backend for routing/memory.
        assert!(!ark.kind().is_local());

        let agnes = CloudVideoGenProvider::new(
            "agnes-video",
            "agnes",
            "",
            Some("k".into()),
            video_models(),
            CostTier::Low,
            None,
        )
        .unwrap();
        assert_eq!(agnes.base_url, DEFAULT_AGNES_BASE_URL);
        assert_eq!(agnes.dialect, VideoDialect::Agnes);
    }

    #[test]
    fn trailing_slash_in_base_url_is_trimmed() {
        let p = CloudVideoGenProvider::new(
            "ark",
            "ark",
            "https://example.com/api/v3/",
            Some("k".into()),
            video_models(),
            CostTier::High,
            None,
        )
        .unwrap();
        assert_eq!(p.base_url, "https://example.com/api/v3");
    }

    #[test]
    fn command_suffix_only_emits_present_keys() {
        let params = serde_json::json!({
            "ratio": "16:9",
            "resolution": "1080p",
            "duration": 5,
            "fps": 24,
            "seed": 42
        });
        assert_eq!(
            CloudVideoGenProvider::command_suffix(&params),
            " --ratio 16:9 --resolution 1080p --duration 5 --fps 24 --seed 42"
        );
        // `seconds` is accepted as an alias for `duration`.
        assert_eq!(
            CloudVideoGenProvider::command_suffix(&serde_json::json!({ "seconds": 8 })),
            " --duration 8"
        );
        // Nothing present → no suffix, so we never override model defaults.
        assert_eq!(
            CloudVideoGenProvider::command_suffix(&serde_json::Value::Null),
            ""
        );
    }

    #[test]
    fn ark_body_text_only() {
        let body = CloudVideoGenProvider::build_task_body(
            VideoDialect::Ark,
            "seedance-1-0-pro",
            "a rocket launch",
            &serde_json::json!({ "ratio": "16:9" }),
        );
        assert_eq!(body["model"], "seedance-1-0-pro");
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "a rocket launch --ratio 16:9");
    }

    #[test]
    fn ark_body_image_to_video_appends_image_item() {
        let body = CloudVideoGenProvider::build_task_body(
            VideoDialect::Ark,
            "seedance-1-0-pro",
            "make it move",
            &serde_json::json!({ "image_url": "https://img/x.png" }),
        );
        let content = body["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "https://img/x.png");
    }

    #[test]
    fn agnes_body_is_flat_with_derived_frames() {
        // duration 5 × fps 24 (+1) = 121 frames; explicit width/height pass through.
        let body = CloudVideoGenProvider::build_task_body(
            VideoDialect::Agnes,
            "agnes-video-v2.0",
            "a paper boat",
            &serde_json::json!({ "width": 1152, "height": 768, "duration": 5, "fps": 24 }),
        );
        assert_eq!(body["model"], "agnes-video-v2.0");
        assert_eq!(body["prompt"], "a paper boat");
        assert_eq!(body["width"], 1152);
        assert_eq!(body["height"], 768);
        assert_eq!(body["frame_rate"], 24);
        assert_eq!(body["num_frames"], 121);
        // Defaults when nothing is specified.
        let d = CloudVideoGenProvider::build_task_body(
            VideoDialect::Agnes,
            "agnes-video-v2.0",
            "x",
            &serde_json::Value::Null,
        );
        assert_eq!(d["width"], 1152);
        assert_eq!(d["height"], 768);
        assert_eq!(d["frame_rate"], 24);
        assert_eq!(d["num_frames"], 121);
    }

    #[test]
    fn parse_task_id_reads_id_or_errors() {
        assert_eq!(
            CloudVideoGenProvider::parse_task_id(&serde_json::json!({ "id": "cgt-1" })).unwrap(),
            "cgt-1"
        );
        assert!(CloudVideoGenProvider::parse_task_id(&serde_json::json!({})).is_err());
    }

    #[test]
    fn parse_task_state_covers_terminal_and_pending() {
        // Success with a nested video URL.
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({
                "status": "succeeded",
                "content": { "video_url": "https://v/clip.mp4" }
            })),
            TaskState::Succeeded("https://v/clip.mp4".into())
        );
        // Success without a URL is a failure, not a hang.
        assert!(matches!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({ "status": "succeeded" })),
            TaskState::Failed(_)
        ));
        // Explicit failure carries the vendor reason.
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({
                "status": "failed",
                "error": { "message": "nsfw" }
            })),
            TaskState::Failed("nsfw".into())
        );
        // Running / unknown → keep polling.
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({ "status": "running" })),
            TaskState::Pending
        );
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({})),
            TaskState::Pending
        );
    }

    #[test]
    fn parse_task_state_agnes_shape() {
        // Agnes `/videos` actually returns `completed` with the clip at a *top-level*
        // `url` (regression: the code previously only looked at `metadata.url`, so a
        // finished task failed with "task succeeded but returned no video URL").
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({
                "status": "completed",
                "url": "https://out/clip.mp4",
                "video_id": "video_abc"
            })),
            TaskState::Succeeded("https://out/clip.mp4".into())
        );
        // `metadata.url` remains accepted for any dialect/version that uses it.
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(&serde_json::json!({
                "status": "completed",
                "metadata": { "url": "https://out/clip.mp4" }
            })),
            TaskState::Succeeded("https://out/clip.mp4".into())
        );
        // Agnes progress states keep polling.
        assert_eq!(
            CloudVideoGenProvider::parse_task_state(
                &serde_json::json!({ "status": "in_progress", "progress": 30 })
            ),
            TaskState::Pending
        );
    }

    #[tokio::test]
    async fn keyless_provider_is_unavailable_and_refuses_to_generate() {
        // Ensure the env fallback can't accidentally make this available.
        // SAFETY: single-threaded test; no other thread reads the env concurrently.
        unsafe {
            std::env::remove_var("ARK_API_KEY");
        }
        let p = provider(None);
        assert!(!p.is_available());
        assert_eq!(p.health().await.unwrap(), BackendHealth::Unavailable);
        let cards = p.discover().await.unwrap();
        assert_eq!(cards[0].availability, ModelAvailability::Unavailable);
        let err = p
            .invoke("ark/seedance-1-0-pro", &video_request("x"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
    }

    #[tokio::test]
    async fn rejects_non_video_capability() {
        let p = provider(Some("k".into()));
        let mut req = video_request("x");
        req.capability = Some(Capability::Chat);
        let err = p.invoke("ark/seedance-1-0-pro", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }

    #[tokio::test]
    async fn empty_prompt_is_invalid_request() {
        let p = provider(Some("k".into()));
        let err = p
            .invoke("ark/seedance-1-0-pro", &video_request("   "))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }
}
