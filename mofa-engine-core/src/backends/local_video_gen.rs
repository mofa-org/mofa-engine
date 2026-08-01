//! Local video-generation process-adapter backend.
//!
//! Runs a configured local command — a text-to-video CLI (e.g. an AnimateDiff /
//! Stable-Video-Diffusion wrapper, or a `Wan`/`mochi`-style generator) — to
//! render a short clip from a text prompt, returning the produced file as a
//! managed artifact. This completes the S4 Creative Generation track: with a
//! local backend an explainer clip can be generated on-device (`prefer=local`,
//! cost ≈ $0); liter-llm exposes no video endpoint, so unlike image generation
//! there is no cloud fallback and this local adapter is the whole path.
//!
//! Device- and runtime-specific concerns stay behind this `Provider` boundary,
//! so the engine treats a local video-gen model like any other backend: it is
//! discovered, memory-managed, warmed, and idle-unloaded. Because generation is
//! typically slow, an operator should size the `inference_secs` timeout
//! accordingly; the child is spawned with `kill_on_drop(true)` so a timeout that
//! drops the future terminates the render rather than leaking it.
//!
//! ## Command contract
//!
//! The command is spawned once per clip (a cold, stateless process). Argument
//! templates may contain:
//!   - `{prompt}` — the text prompt (required),
//!   - `{output}` — path the command must write the video to (required),
//!   - `{negative_prompt}` — from `params.negative_prompt` (default empty),
//!   - `{width}` / `{height}` — from `params.size` (`"WxH"`) or
//!     `params.width` / `params.height` (default 512×512),
//!   - `{seconds}` — target duration from `params.seconds` (default 4),
//!   - `{fps}` — target frame rate from `params.fps` (default 16).
//!
//! The produced clip should be verified with the [`quality_gate`](crate::quality_gate)
//! before it is presented as a finished S4 artifact ("no gate, no output").

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind,
};
use tokio::process::Command;

use crate::config::ModelDef;

/// Default output dimensions when a request does not specify a size.
const DEFAULT_EDGE: u32 = 512;
/// Default clip length in seconds.
const DEFAULT_SECONDS: u32 = 4;
/// Default frame rate.
const DEFAULT_FPS: u32 = 16;

/// A process-adapter provider that shells out to a local video-generation command.
pub(crate) struct LocalVideoGenProvider {
    /// Display name.
    name: String,
    /// Program to execute per clip.
    command: String,
    /// Argument template with `{prompt}`, `{output}`, `{negative_prompt}`,
    /// `{width}`, `{height}`, `{seconds}`, and `{fps}` placeholders.
    args: Vec<String>,
    /// Output video extension/container (e.g. `mp4`, `webm`).
    output_format: String,
    /// Directory for generated artifacts.
    output_dir: PathBuf,
    /// Configured models this backend serves.
    models: Vec<ModelDef>,
}

impl LocalVideoGenProvider {
    /// Create a new local video-generation process adapter.
    pub(crate) fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        output_format: Option<String>,
        output_dir: Option<String>,
        models: Vec<ModelDef>,
    ) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            output_format: output_format
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "mp4".into()),
            output_dir: output_dir
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            models,
        }
    }

    /// Resolve the configured program to an executable path, searching `PATH`
    /// for a bare name. Returns `None` when it cannot be found.
    fn resolve_program(&self) -> Option<PathBuf> {
        let program = Path::new(&self.command);
        if program.is_absolute() || self.command.contains(std::path::MAIN_SEPARATOR) {
            return program.is_file().then(|| program.to_path_buf());
        }
        let paths = std::env::var_os("PATH")?;
        std::env::split_paths(&paths)
            .map(|dir| dir.join(&self.command))
            .find(|candidate| candidate.is_file())
    }

    /// Whether a configured model serves the given capability.
    fn model_supports(&self, model_name: &str, capability: Capability) -> bool {
        self.models.iter().any(|m| {
            m.name == model_name && Capability::from_str_loose(&m.capability) == Some(capability)
        })
    }

    /// Resolve the argument template for one render. Pure over the configured
    /// template, so placeholder wiring is unit-testable without spawning.
    #[allow(clippy::too_many_arguments)]
    fn resolve_args(
        &self,
        prompt: &str,
        output: &str,
        negative_prompt: &str,
        width: u32,
        height: u32,
        seconds: u32,
        fps: u32,
    ) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| {
                arg.replace("{prompt}", prompt)
                    .replace("{output}", output)
                    .replace("{negative_prompt}", negative_prompt)
                    .replace("{width}", &width.to_string())
                    .replace("{height}", &height.to_string())
                    .replace("{seconds}", &seconds.to_string())
                    .replace("{fps}", &fps.to_string())
            })
            .collect()
    }

    async fn generate(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
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
            .ok_or_else(|| EngineError::InvalidRequest("video_gen requires a prompt".into()))?;

        let negative_prompt = request
            .params
            .get("negative_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (width, height) = Self::dimensions(&request.params);
        let seconds = Self::positive_u32(&request.params, "seconds").unwrap_or(DEFAULT_SECONDS);
        let fps = Self::positive_u32(&request.params, "fps").unwrap_or(DEFAULT_FPS);

        let output_path = self.output_dir.join(format!(
            "mofa_video_{}.{}",
            uuid::Uuid::new_v4(),
            self.output_format
        ));
        let output_str = output_path.to_string_lossy().to_string();

        let resolved_args = self.resolve_args(
            &prompt,
            &output_str,
            negative_prompt,
            width,
            height,
            seconds,
            fps,
        );

        let result = Command::new(&self.command)
            .args(&resolved_args)
            .kill_on_drop(true)
            .output()
            .await;

        let output = result.map_err(|e| EngineError::ProviderError {
            provider: self.name.clone(),
            detail: format!("failed to spawn '{}': {e}", self.command),
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = tokio::fs::remove_file(&output_path).await;
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!(
                    "video_gen command exited with {}: {}",
                    output.status,
                    stderr.trim()
                ),
            });
        }

        // A successful exit must have produced a non-empty video file.
        match tokio::fs::metadata(&output_path).await {
            Ok(meta) if meta.len() > 0 => {}
            _ => {
                let _ = tokio::fs::remove_file(&output_path).await;
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: "video_gen command produced no video output".into(),
                });
            }
        }

        Ok(InferenceResponse {
            text: None,
            file: Some(output_str),
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

/// Pure parameter parsing, grouped as private associated functions.
impl LocalVideoGenProvider {
    /// Requested dimensions from `params`, accepting a combined `size` (`"WxH"`)
    /// or explicit `width`/`height`, falling back to a square [`DEFAULT_EDGE`].
    fn dimensions(params: &serde_json::Value) -> (u32, u32) {
        if let Some((w, h)) = params
            .get("size")
            .and_then(|v| v.as_str())
            .and_then(Self::parse_size)
        {
            return (w, h);
        }
        (
            Self::positive_u32(params, "width").unwrap_or(DEFAULT_EDGE),
            Self::positive_u32(params, "height").unwrap_or(DEFAULT_EDGE),
        )
    }

    /// Parse a `"WIDTHxHEIGHT"` size string into positive dimensions (accepts `x`
    /// or `X` as the separator).
    fn parse_size(size: &str) -> Option<(u32, u32)> {
        let (w, h) = size.split_once(['x', 'X'])?;
        let w: u32 = w.trim().parse().ok()?;
        let h: u32 = h.trim().parse().ok()?;
        (w > 0 && h > 0).then_some((w, h))
    }

    /// Read a strictly-positive `u32` from a params key, or `None`.
    fn positive_u32(params: &serde_json::Value, key: &str) -> Option<u32> {
        params
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .filter(|v| *v > 0)
            .map(|v| v as u32)
    }
}

#[async_trait]
impl Provider for LocalVideoGenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalVideoGen
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![
            BackendFeature::Discovery,
            BackendFeature::Load,
            BackendFeature::Unload,
            BackendFeature::MemoryReporting,
        ]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        let cards = self
            .models
            .iter()
            .filter_map(|m| {
                let cap = Capability::from_str_loose(&m.capability)?;
                let mut card =
                    ModelCard::new(self.name.clone(), m.name.clone(), cap, CostTier::Free);
                card.id = ModelId::canonical(&self.name, &m.name);
                card.availability = ModelAvailability::Configured;
                card.residency = ModelResidency::Unloaded;
                card.memory_estimate_bytes = m.memory_mb.unwrap_or(0) * 1024 * 1024;
                card.refresh_status();
                Some(card)
            })
            .collect();
        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        if self.resolve_program().is_some() {
            Ok(BackendHealth::Healthy)
        } else {
            Ok(BackendHealth::Unavailable)
        }
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        // Readiness probe: the command must resolve before we claim residency.
        if self.resolve_program().is_none() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("video_gen command '{}' not found", self.command),
            });
        }
        let model_name = ModelId::name(model_id);
        let estimate = self
            .models
            .iter()
            .find(|m| m.name == model_name)
            .and_then(|m| m.memory_mb)
            .map(|mb| mb * 1024 * 1024);
        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, model_name),
            residency: ModelResidency::Loaded,
            memory_bytes: estimate,
            changed: true,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, ModelId::name(model_id)),
            residency: ModelResidency::Unloaded,
            memory_bytes: Some(0),
            changed: true,
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

        self.generate(model_name, request, std::time::Instant::now())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::Message;

    fn video_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "fixture".into(),
            capability: "video_gen".into(),
            context_window: None,
            memory_mb: Some(4096),
            ..Default::default()
        }]
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
    fn metadata_and_defaults() {
        let p = LocalVideoGenProvider::new("local-video", "sh", vec![], None, None, video_models());
        assert_eq!(p.kind(), ProviderKind::LocalVideoGen);
        assert!(p.kind().is_local());
        assert_eq!(p.output_format, "mp4");
    }

    #[test]
    fn dimensions_and_positive_params() {
        assert_eq!(
            LocalVideoGenProvider::dimensions(&serde_json::json!({ "size": "640x480" })),
            (640, 480)
        );
        assert_eq!(
            LocalVideoGenProvider::dimensions(&serde_json::json!({ "width": 256, "height": 128 })),
            (256, 128)
        );
        assert_eq!(
            LocalVideoGenProvider::dimensions(&serde_json::Value::Null),
            (DEFAULT_EDGE, DEFAULT_EDGE)
        );
        // Zero is rejected as non-positive and falls back to the default.
        assert_eq!(
            LocalVideoGenProvider::positive_u32(&serde_json::json!({ "fps": 0 }), "fps"),
            None
        );
        assert_eq!(
            LocalVideoGenProvider::positive_u32(&serde_json::json!({ "fps": 24 }), "fps"),
            Some(24)
        );
    }

    #[test]
    fn resolve_args_substitutes_all_placeholders() {
        let p = LocalVideoGenProvider::new(
            "local-video",
            "gen",
            vec![
                "--prompt".into(),
                "{prompt}".into(),
                "-W".into(),
                "{width}".into(),
                "-H".into(),
                "{height}".into(),
                "--secs".into(),
                "{seconds}".into(),
                "--fps".into(),
                "{fps}".into(),
                "-o".into(),
                "{output}".into(),
            ],
            None,
            None,
            video_models(),
        );
        let args = p.resolve_args("a rocket", "/tmp/o.mp4", "", 512, 512, 4, 16);
        assert_eq!(
            args,
            vec![
                "--prompt",
                "a rocket",
                "-W",
                "512",
                "-H",
                "512",
                "--secs",
                "4",
                "--fps",
                "16",
                "-o",
                "/tmp/o.mp4"
            ]
        );
    }

    #[tokio::test]
    async fn generates_video_artifact_via_fixture_command() {
        let dir = tempfile::tempdir().unwrap();
        // A deterministic "generator" fixture: write the prompt + geometry to the
        // output path, proving the placeholders reached the command.
        let p = LocalVideoGenProvider::new(
            "local-video",
            "sh",
            vec![
                "-c".into(),
                "printf 'MP4:%s:%sx%s@%s' \"$1\" \"$2\" \"$3\" \"$4\" > \"$5\"".into(),
                "sh".into(),
                "{prompt}".into(),
                "{width}".into(),
                "{height}".into(),
                "{fps}".into(),
                "{output}".into(),
            ],
            Some("mp4".into()),
            Some(dir.path().to_string_lossy().to_string()),
            video_models(),
        );

        let mut req = video_request("a neuron firing");
        req.params = serde_json::json!({ "size": "320x240", "fps": 24 });
        let resp = p.invoke("local-video/fixture", &req).await.unwrap();
        let file = resp.file.expect("a video artifact path");
        assert!(file.ends_with(".mp4"));
        let produced = std::fs::read_to_string(&file).unwrap();
        assert_eq!(produced, "MP4:a neuron firing:320x240@24");
    }

    #[tokio::test]
    async fn empty_prompt_is_invalid_request() {
        let p = LocalVideoGenProvider::new("local-video", "sh", vec![], None, None, video_models());
        let err = p
            .invoke("local-video/fixture", &video_request("   "))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn command_failure_is_a_retryable_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = LocalVideoGenProvider::new(
            "local-video",
            "sh",
            vec!["-c".into(), "echo boom >&2; exit 7".into()],
            None,
            Some(dir.path().to_string_lossy().to_string()),
            video_models(),
        );
        let err = p
            .invoke("local-video/fixture", &video_request("x"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        assert!(err.retryable());
    }

    #[tokio::test]
    async fn rejects_unsupported_capability() {
        let p = LocalVideoGenProvider::new("local-video", "sh", vec![], None, None, video_models());
        let mut req = video_request("x");
        req.capability = Some(Capability::Chat);
        let err = p.invoke("local-video/fixture", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }
}
