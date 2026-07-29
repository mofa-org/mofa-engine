//! Local image-generation process-adapter backend.
//!
//! Runs a configured local command — a Stable Diffusion CLI (e.g. `sd`/sd.cpp,
//! `mflux`, or a wrapper script) — to render an image from a text prompt,
//! returning the produced file as a managed artifact. This is the flagship S4
//! enabler for **fully offline** operation: with a local SD backend, an explainer
//! video's scene images can be generated on-device (`prefer=local`, cost ≈ $0)
//! rather than falling back to a cloud image API.
//!
//! Device- and runtime-specific concerns stay behind this `Provider` boundary,
//! so the engine treats a local image-gen model like any other backend: it is
//! discovered, memory-managed, warmed, idle-unloaded, and can fail over to a
//! cloud image backend (via liter-llm) when the local one is unavailable.
//!
//! ## Command contract
//!
//! The command is spawned once per image (a cold, stateless process). Argument
//! templates may contain:
//!   - `{prompt}` — the text prompt (required),
//!   - `{output}` — path the command must write the image to (required),
//!   - `{negative_prompt}` — from `params.negative_prompt` (default empty),
//!   - `{width}` / `{height}` — parsed from `params.size` (e.g. `"1024x1024"`,
//!     the default), or `params.width` / `params.height`.
//!
//! Child processes are spawned with `kill_on_drop(true)`, so an inference timeout
//! that drops the future terminates the render rather than leaking it.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelResidency, Provider,
    ProviderKind, canonical_model_id, model_id_name,
};
use tokio::process::Command;

use crate::config::ModelDef;

/// Default output image dimensions when a request does not specify a size.
const DEFAULT_EDGE: u32 = 1024;

/// A process-adapter provider that shells out to a local image-generation command.
pub struct LocalImageGenProvider {
    /// Display name.
    name: String,
    /// Program to execute per image.
    command: String,
    /// Argument template with `{prompt}`, `{output}`, `{negative_prompt}`,
    /// `{width}`, and `{height}` placeholders.
    args: Vec<String>,
    /// Output image extension/container (e.g. `png`, `jpg`).
    output_format: String,
    /// Directory for generated artifacts.
    output_dir: PathBuf,
    /// Configured models this backend serves.
    models: Vec<ModelDef>,
}

impl LocalImageGenProvider {
    /// Create a new local image-generation process adapter.
    pub fn new(
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
                .unwrap_or_else(|| "png".into()),
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

    /// Resolve the argument template for one render, substituting the prompt,
    /// output path, negative prompt, and dimensions. Pure over the configured
    /// template, so placeholder wiring is unit-testable without spawning.
    fn resolve_args(
        &self,
        prompt: &str,
        output: &str,
        negative_prompt: &str,
        width: u32,
        height: u32,
    ) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| {
                arg.replace("{prompt}", prompt)
                    .replace("{output}", output)
                    .replace("{negative_prompt}", negative_prompt)
                    .replace("{width}", &width.to_string())
                    .replace("{height}", &height.to_string())
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
            .ok_or_else(|| EngineError::InvalidRequest("image_gen requires a prompt".into()))?;

        let negative_prompt = request
            .params
            .get("negative_prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let (width, height) = image_dimensions(&request.params);

        let output_path = self.output_dir.join(format!(
            "mofa_img_{}.{}",
            uuid::Uuid::new_v4(),
            self.output_format
        ));
        let output_str = output_path.to_string_lossy().to_string();

        let resolved_args = self.resolve_args(&prompt, &output_str, negative_prompt, width, height);

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
                    "image_gen command exited with {}: {}",
                    output.status,
                    stderr.trim()
                ),
            });
        }

        // A successful exit must have produced a non-empty image file.
        match tokio::fs::metadata(&output_path).await {
            Ok(meta) if meta.len() > 0 => {}
            _ => {
                let _ = tokio::fs::remove_file(&output_path).await;
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: "image_gen command produced no image output".into(),
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

/// Parse the requested image dimensions from a request's `params`, accepting a
/// combined `size` (`"WxH"`) or explicit `width`/`height`, and falling back to a
/// square [`DEFAULT_EDGE`]. Pure so the parsing is unit-testable.
fn image_dimensions(params: &serde_json::Value) -> (u32, u32) {
    if let Some((w, h)) = params
        .get("size")
        .and_then(|v| v.as_str())
        .and_then(parse_size)
    {
        return (w, h);
    }
    let as_edge = |key: &str| {
        params
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .filter(|v| *v > 0)
            .map(|v| v as u32)
    };
    (
        as_edge("width").unwrap_or(DEFAULT_EDGE),
        as_edge("height").unwrap_or(DEFAULT_EDGE),
    )
}

/// Parse a `"WIDTHxHEIGHT"` size string into positive dimensions (accepts `x` or
/// `X` as the separator).
fn parse_size(size: &str) -> Option<(u32, u32)> {
    let (w, h) = size.split_once(['x', 'X'])?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

#[async_trait]
impl Provider for LocalImageGenProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalImageGen
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
                card.id = canonical_model_id(&self.name, &m.name);
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
                detail: format!("image_gen command '{}' not found", self.command),
            });
        }
        let model_name = model_id_name(model_id);
        let estimate = self
            .models
            .iter()
            .find(|m| m.name == model_name)
            .and_then(|m| m.memory_mb)
            .map(|mb| mb * 1024 * 1024);
        Ok(LifecycleResult {
            model_id: canonical_model_id(&self.name, model_name),
            residency: ModelResidency::Loaded,
            memory_bytes: estimate,
            changed: true,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: canonical_model_id(&self.name, model_id_name(model_id)),
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
        let model_name = model_id_name(model_id);
        let capability = request.capability.unwrap_or(Capability::ImageGen);

        if capability != Capability::ImageGen {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' only supports image_gen, not {capability}",
                self.name
            )));
        }
        if !self.model_supports(model_name, Capability::ImageGen) {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{model_name}' does not support image_gen",
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

    fn img_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "fixture".into(),
            capability: "image_gen".into(),
            context_window: None,
            memory_mb: Some(2048),
            ..Default::default()
        }]
    }

    fn img_request(prompt: &str) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::ImageGen),
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
        let p = LocalImageGenProvider::new("local-sd", "sh", vec![], None, None, img_models());
        assert_eq!(p.kind(), ProviderKind::LocalImageGen);
        assert!(p.kind().is_local());
        assert_eq!(p.output_format, "png");
    }

    #[test]
    fn parse_size_and_dimensions() {
        assert_eq!(parse_size("1024x768"), Some((1024, 768)));
        assert_eq!(parse_size("512X512"), Some((512, 512)));
        assert_eq!(parse_size("bad"), None);
        assert_eq!(parse_size("0x100"), None);

        // size wins; else width/height; else the square default.
        assert_eq!(
            image_dimensions(&serde_json::json!({ "size": "640x480" })),
            (640, 480)
        );
        assert_eq!(
            image_dimensions(&serde_json::json!({ "width": 256, "height": 128 })),
            (256, 128)
        );
        assert_eq!(
            image_dimensions(&serde_json::Value::Null),
            (DEFAULT_EDGE, DEFAULT_EDGE)
        );
    }

    #[test]
    fn resolve_args_substitutes_all_placeholders() {
        let p = LocalImageGenProvider::new(
            "local-sd",
            "sd",
            vec![
                "--prompt".into(),
                "{prompt}".into(),
                "--neg".into(),
                "{negative_prompt}".into(),
                "-W".into(),
                "{width}".into(),
                "-H".into(),
                "{height}".into(),
                "-o".into(),
                "{output}".into(),
            ],
            None,
            None,
            img_models(),
        );
        let args = p.resolve_args("a cat", "/tmp/o.png", "blurry", 1024, 768);
        assert_eq!(
            args,
            vec![
                "--prompt",
                "a cat",
                "--neg",
                "blurry",
                "-W",
                "1024",
                "-H",
                "768",
                "-o",
                "/tmp/o.png"
            ]
        );
    }

    #[tokio::test]
    async fn generates_image_artifact_via_fixture_command() {
        let dir = tempfile::tempdir().unwrap();
        // A deterministic "SD" fixture: write the prompt + size into the output
        // path, proving prompt/size placeholders reached the command.
        let p = LocalImageGenProvider::new(
            "local-sd",
            "sh",
            vec![
                "-c".into(),
                "printf 'PNG:%s:%sx%s' \"$1\" \"$2\" \"$3\" > \"$4\"".into(),
                "sh".into(),
                "{prompt}".into(),
                "{width}".into(),
                "{height}".into(),
                "{output}".into(),
            ],
            Some("png".into()),
            Some(dir.path().to_string_lossy().to_string()),
            img_models(),
        );

        let mut req = img_request("a neuron");
        req.params = serde_json::json!({ "size": "512x512" });
        let resp = p.invoke("local-sd/fixture", &req).await.unwrap();
        let file = resp.file.expect("an image artifact path");
        assert!(file.ends_with(".png"));
        let produced = std::fs::read_to_string(&file).unwrap();
        assert_eq!(produced, "PNG:a neuron:512x512");
    }

    #[tokio::test]
    async fn empty_prompt_is_invalid_request() {
        let p = LocalImageGenProvider::new("local-sd", "sh", vec![], None, None, img_models());
        let err = p
            .invoke("local-sd/fixture", &img_request("   "))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn command_failure_is_a_retryable_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = LocalImageGenProvider::new(
            "local-sd",
            "sh",
            vec!["-c".into(), "echo boom >&2; exit 5".into()],
            None,
            Some(dir.path().to_string_lossy().to_string()),
            img_models(),
        );
        let err = p
            .invoke("local-sd/fixture", &img_request("x"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        // Retryable so the engine can fail over to a cloud image backend.
        assert!(err.retryable());
    }

    #[tokio::test]
    async fn rejects_unsupported_capability() {
        let p = LocalImageGenProvider::new("local-sd", "sh", vec![], None, None, img_models());
        let mut req = img_request("x");
        req.capability = Some(Capability::Chat);
        let err = p.invoke("local-sd/fixture", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }
}
