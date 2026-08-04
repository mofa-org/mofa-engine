//! Local TTS process-adapter backend.
//!
//! Runs a configured local command — an MLX/Kokoro or Piper-style TTS CLI — to
//! synthesize speech, returning the produced audio as a managed artifact.
//!
//! Runtime- and device-specific concerns stay behind this `Provider` boundary,
//! so the engine treats a local TTS model like any other backend: it is
//! discovered, admitted through memory reservation, warmed, idle-unloaded, and
//! can fail over to a cloud TTS backend when the local one is unavailable.
//!
//! ## Lifecycle model
//!
//! This adapter spawns the command once per synthesis (a cold, stateless
//! process). `load` performs a cheap readiness probe (the program resolves and
//! is executable) and reports the model as resident with its configured memory
//! estimate; that conservative reservation lets the engine's coexistence and
//! idle-eviction logic apply to local TTS exactly as it does to Ollama. A
//! long-running server variant can later make `load` start the process without
//! changing the engine contract.
//!
//! ## Cancellation
//!
//! Child processes are spawned with `kill_on_drop(true)`, so when the engine's
//! inference timeout fires and drops the invocation future, the underlying
//! synthesis process is terminated rather than leaked.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind,
};
use tokio::process::Command;

use crate::config::ModelDef;

/// A process-adapter provider that shells out to a local TTS command.
pub(crate) struct LocalTtsProvider {
    /// Display name.
    name: String,
    /// Program to execute per synthesis.
    command: String,
    /// Argument template with `{text}`, `{text_file}`, and `{output}` placeholders.
    args: Vec<String>,
    /// Output audio extension/container (e.g. `wav`, `mp3`).
    output_format: String,
    /// Directory for generated artifacts.
    output_dir: PathBuf,
    /// Configured models this backend serves.
    models: Vec<ModelDef>,
}

impl LocalTtsProvider {
    /// Create a new local TTS process adapter.
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
                .unwrap_or_else(|| "wav".into()),
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
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

    async fn synthesize(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let text = request
            .messages
            .first()
            .map(|m| m.content.clone())
            .or_else(|| {
                request
                    .params
                    .get("input")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| EngineError::InvalidRequest("TTS requires text input".into()))?;

        let output_path = self.output_dir.join(format!(
            "mofa_tts_{}.{}",
            uuid::Uuid::new_v4(),
            self.output_format
        ));

        // Materialize a temp text file only if the command template references it.
        let needs_text_file = self.args.iter().any(|a| a.contains("{text_file}"));
        let text_file = if needs_text_file {
            let path = self
                .output_dir
                .join(format!("mofa_tts_{}.txt", uuid::Uuid::new_v4()));
            tokio::fs::write(&path, &text)
                .await
                .map_err(|e| EngineError::Internal(format!("cannot write TTS input file: {e}")))?;
            Some(path)
        } else {
            None
        };

        let output_str = output_path.to_string_lossy().to_string();
        let text_file_str = text_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let resolved_args: Vec<String> = self
            .args
            .iter()
            .map(|arg| {
                arg.replace("{text}", &text)
                    .replace("{text_file}", &text_file_str)
                    .replace("{output}", &output_str)
            })
            .collect();

        let result = Command::new(&self.command)
            .args(&resolved_args)
            .kill_on_drop(true)
            .output()
            .await;

        // Best-effort cleanup of the transient input file regardless of outcome.
        if let Some(path) = &text_file {
            let _ = tokio::fs::remove_file(path).await;
        }

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
                    "TTS command exited with {}: {}",
                    output.status,
                    stderr.trim()
                ),
            });
        }

        // A successful exit must have produced a non-empty artifact.
        match tokio::fs::metadata(&output_path).await {
            Ok(meta) if meta.len() > 0 => {}
            _ => {
                // Remove any empty/partial file the command may have created.
                let _ = tokio::fs::remove_file(&output_path).await;
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: "TTS command produced no audio output".into(),
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

#[async_trait]
impl Provider for LocalTtsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalTts
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
                detail: format!("TTS command '{}' not found", self.command),
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
        let capability = request.capability.unwrap_or(Capability::Tts);

        if capability != Capability::Tts {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' only supports tts, not {capability}",
                self.name
            )));
        }
        if !self.model_supports(model_name, Capability::Tts) {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{model_name}' does not support tts",
                self.name
            )));
        }

        self.synthesize(model_name, request, std::time::Instant::now())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::Message;

    fn tts_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "fixture".into(),
            capability: "tts".into(),
            context_window: None,
            memory_mb: Some(64),
            ..Default::default()
        }]
    }

    fn tts_request(text: &str) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::Tts),
            model: None,
            app_id: None,
            session_id: None,
            fallback_policy: Default::default(),
            messages: vec![Message {
                role: "user".into(),
                content: text.into(),
                ..Default::default()
            }],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    fn metadata_and_defaults() {
        let p = LocalTtsProvider::new("local-tts", "sh", vec![], None, None, tts_models());
        assert_eq!(p.kind(), ProviderKind::LocalTts);
        assert_eq!(p.name(), "local-tts");
        assert_eq!(p.output_format, "wav");
        assert!(p.features().contains(&BackendFeature::Load));
    }

    #[tokio::test]
    async fn discover_reports_configured_tts_model() {
        let p = LocalTtsProvider::new("local-tts", "sh", vec![], None, None, tts_models());
        let cards = p.discover().await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "local-tts/fixture");
        assert_eq!(cards[0].capability, Capability::Tts);
        assert_eq!(cards[0].residency, ModelResidency::Unloaded);
        assert_eq!(cards[0].memory_estimate_bytes, 64 * 1024 * 1024);
    }

    #[tokio::test]
    async fn health_reflects_command_availability() {
        // `sh` resolves on any POSIX PATH.
        let ok = LocalTtsProvider::new("local-tts", "sh", vec![], None, None, tts_models());
        assert_eq!(ok.health().await.unwrap(), BackendHealth::Healthy);

        let missing = LocalTtsProvider::new(
            "local-tts",
            "definitely-not-a-real-binary-xyz",
            vec![],
            None,
            None,
            tts_models(),
        );
        assert_eq!(missing.health().await.unwrap(), BackendHealth::Unavailable);
    }

    #[tokio::test]
    async fn synthesizes_audio_artifact_via_fixture_command() {
        let dir = tempfile::tempdir().unwrap();
        // A deterministic "TTS" fixture: write bytes to the requested output path.
        let p = LocalTtsProvider::new(
            "local-tts",
            "sh",
            vec![
                "-c".into(),
                "printf 'RIFF-fake-audio' > \"$1\"".into(),
                "sh".into(),
                "{output}".into(),
            ],
            Some("wav".into()),
            Some(dir.path().to_string_lossy().to_string()),
            tts_models(),
        );

        let resp = p
            .invoke("local-tts/fixture", &tts_request("hello world"))
            .await
            .unwrap();
        let file = resp.file.expect("a TTS artifact path");
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, b"RIFF-fake-audio");
        assert!(file.ends_with(".wav"));
    }

    #[tokio::test]
    async fn passes_text_through_a_temp_file_when_templated() {
        let dir = tempfile::tempdir().unwrap();
        // Copy the input text file to the output so we can assert it round-trips.
        let p = LocalTtsProvider::new(
            "local-tts",
            "sh",
            vec![
                "-c".into(),
                "cat \"$1\" > \"$2\"".into(),
                "sh".into(),
                "{text_file}".into(),
                "{output}".into(),
            ],
            Some("wav".into()),
            Some(dir.path().to_string_lossy().to_string()),
            tts_models(),
        );

        let resp = p
            .invoke("local-tts/fixture", &tts_request("spoken words"))
            .await
            .unwrap();
        let produced = std::fs::read_to_string(resp.file.unwrap()).unwrap();
        assert_eq!(produced, "spoken words");
    }

    #[tokio::test]
    async fn command_failure_is_a_retryable_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = LocalTtsProvider::new(
            "local-tts",
            "sh",
            vec!["-c".into(), "echo boom >&2; exit 3".into()],
            Some("wav".into()),
            Some(dir.path().to_string_lossy().to_string()),
            tts_models(),
        );

        let err = p
            .invoke("local-tts/fixture", &tts_request("hi"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        // Retryable so the engine can fail over to a cloud TTS backend.
        assert!(err.retryable());
    }

    #[tokio::test]
    async fn empty_text_is_an_invalid_request() {
        let p = LocalTtsProvider::new("local-tts", "sh", vec![], None, None, tts_models());
        let err = p
            .invoke("local-tts/fixture", &tts_request("   "))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn rejects_unsupported_capability() {
        let p = LocalTtsProvider::new("local-tts", "sh", vec![], None, None, tts_models());
        let mut req = tts_request("hi");
        req.capability = Some(Capability::Chat);
        let err = p.invoke("local-tts/fixture", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }
}
