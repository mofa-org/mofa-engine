//! Cloud video-generation backend (Seedance / Volcengine Ark contract).
//!
//! This is the *API-level* video path the local process-adapter
//! ([`LocalVideoGenProvider`](super::local_video_gen)) could not offer: a real
//! text-to-video (and image-to-video) model reached over HTTP. It targets the
//! Volcengine Ark / BytePlus "content generation tasks" contract, which is what
//! ByteDance's **Seedance** models speak, so a scenario can render a genuine
//! clip from a prompt without any local generator installed.
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
const DEFAULT_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
/// Environment variable liter-llm-style key resolution falls back to.
const API_KEY_ENV: &str = "ARK_API_KEY";
/// How often to poll a running task when the request does not say otherwise.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Upper bound on poll attempts. Generation is slow but not unbounded; the engine
/// also enforces its own inference timeout, whose dropped future cancels the poll
/// loop. This cap is a backstop so a stuck task cannot poll forever.
const MAX_POLL_ATTEMPTS: u32 = 200;

/// A provider that renders video through the Ark / Seedance task API.
pub(crate) struct CloudVideoGenProvider {
    /// Display name.
    name: String,
    /// API root, e.g. `https://ark.cn-beijing.volces.com/api/v3`.
    base_url: String,
    /// Bearer token. Empty means "no credentials" → the provider reports itself
    /// unavailable rather than failing engine startup (mirrors the other cloud
    /// backends), so an offline/keyless config still boots.
    api_key: String,
    /// Configured Seedance model ids this backend serves.
    models: Vec<ModelDef>,
    /// Cost tier applied to this provider's models.
    cost_tier: CostTier,
    /// Directory for downloaded video artifacts.
    output_dir: PathBuf,
    /// Shared HTTP client (connection reuse across submit/poll/download).
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
    /// Build a cloud video provider.
    ///
    /// An empty `api_key` falls back to the [`API_KEY_ENV`] environment variable;
    /// if that too is empty the provider is constructed but stays unavailable, so
    /// a config that lists it without a key does not break engine startup.
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
                // Tolerate a trailing slash so `{base}/contents/...` never doubles it.
                b.trim_end_matches('/').to_string()
            }
        };

        let api_key = api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var(API_KEY_ENV).ok())
            .unwrap_or_default();
        if api_key.is_empty() {
            tracing::warn!(
                provider = %name,
                "cloud video provider has no {API_KEY_ENV}; marked unavailable"
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

    // ==========================================================================
    // Pure request/response mapping (unit-tested without the network)
    // ==========================================================================

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

    /// Build the JSON task body. Adds an `image_url` content item when
    /// `params.image_url` is set, selecting image-to-video.
    fn build_task_body(
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
            "succeeded" | "success" | "done" => {
                // The video URL nests under `content.video_url` in the Ark contract;
                // accept a top-level `video_url` too for forward-compatibility.
                let url = body
                    .get("content")
                    .and_then(|c| c.get("video_url"))
                    .or_else(|| body.get("video_url"))
                    .and_then(|v| v.as_str());
                match url {
                    Some(u) if !u.is_empty() => TaskState::Succeeded(u.to_string()),
                    _ => TaskState::Failed("task succeeded but returned no video_url".into()),
                }
            }
            "failed" | "error" | "cancelled" | "canceled" => {
                let reason = body
                    .get("error")
                    .and_then(|e| e.get("message"))
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

    async fn submit_task(&self, body: &serde_json::Value) -> Result<String, EngineError> {
        let url = format!("{}/contents/generations/tasks", self.base_url);
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
        let url = format!("{}/contents/generations/tasks/{task_id}", self.base_url);
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
            return Err(self.provider_error(format!("no {API_KEY_ENV} credentials configured")));
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

        let body = Self::build_task_body(model_name, &prompt, &request.params);
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
    fn empty_base_url_defaults_to_ark_endpoint() {
        let p = provider(Some("k".into()));
        assert_eq!(p.base_url, DEFAULT_BASE_URL);
        assert_eq!(p.kind(), ProviderKind::CloudVideoGen);
        // Cloud: not counted as a local backend for routing/memory.
        assert!(!p.kind().is_local());
    }

    #[test]
    fn trailing_slash_in_base_url_is_trimmed() {
        let p = CloudVideoGenProvider::new(
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
    fn build_task_body_text_only() {
        let body = CloudVideoGenProvider::build_task_body(
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
    fn build_task_body_image_to_video_appends_image_item() {
        let body = CloudVideoGenProvider::build_task_body(
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

    #[tokio::test]
    async fn keyless_provider_is_unavailable_and_refuses_to_generate() {
        // Ensure the env fallback can't accidentally make this available.
        // SAFETY: single-threaded test; no other thread reads the env concurrently.
        unsafe {
            std::env::remove_var(API_KEY_ENV);
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
