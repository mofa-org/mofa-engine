//! Local ASR process-adapter backend.
//!
//! Runs a configured local command — a FunASR or whisper.cpp-style CLI — to
//! transcribe an audio file, returning the recognized text. This is the S1
//! Meeting Brief enabler: long, confidential audio can be transcribed on-device
//! (`prefer=local`) without leaving the machine.
//!
//! Runtime- and device-specific concerns stay behind this `Provider` boundary,
//! so the engine treats a local ASR model like any other backend: it is
//! discovered, admitted through memory reservation, warmed, idle-unloaded, and
//! can fail over to a cloud ASR backend (non-confidential only) when the local
//! one is unavailable.
//!
//! ## Command contract
//!
//! The command is spawned once per transcription (a cold, stateless process).
//! Argument templates may contain:
//!   - `{input}` — path to the source audio file (required),
//!   - `{output}` — path the command must write the transcript text to; when
//!     present the transcript is read back from this file, otherwise the
//!     command's stdout is taken as the transcript,
//!   - `{language}` — language hint from `params.language` (default `auto`).
//!
//! When a request sets `params.diarize = true` (S1 speaker separation) the
//! configured `diarize_args` are appended (with the same placeholder
//! substitution); if none are configured the request degrades to a plain
//! transcript rather than failing.
//!
//! Child processes are spawned with `kill_on_drop(true)`, so an inference
//! timeout that drops the future terminates the process rather than leaking it.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind,
};
use tokio::process::Command;

use crate::config::ModelDef;

/// A process-adapter provider that shells out to a local ASR command.
pub(crate) struct LocalAsrProvider {
    name: String,
    /// Program to execute per transcription.
    command: String,
    /// Argument template with `{input}`, `{output}`, and `{language}` placeholders.
    args: Vec<String>,
    /// Extra arguments appended (with the same placeholder substitution) when a
    /// request asks for speaker diarization via `params.diarize = true`. Empty
    /// when the configured CLI does not do speaker separation, in which case a
    /// diarize request degrades to a plain transcript.
    diarize_args: Vec<String>,
    output_dir: PathBuf,
    models: Vec<ModelDef>,
}

impl LocalAsrProvider {
    pub(crate) fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        output_dir: Option<String>,
        models: Vec<ModelDef>,
    ) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            diarize_args: Vec::new(),
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
            models,
        }
    }

    /// Set the arguments appended when a request enables speaker diarization
    /// (`params.diarize = true`). These are placeholder-substituted like the base
    /// `args`, so a CLI flag such as `--speaker-diarize` (or a value derived from
    /// `{input}`/`{output}`) can be supplied.
    pub(crate) fn with_diarize_args(mut self, diarize_args: Vec<String>) -> Self {
        self.diarize_args = diarize_args;
        self
    }

    /// Build the resolved argument vector for one transcription, substituting the
    /// `{input}`/`{output}`/`{language}` placeholders and, when `diarize` is set,
    /// appending the (also-substituted) diarization arguments. Pure over the
    /// provider's configured templates, so placeholder + diarize wiring is
    /// unit-testable without spawning a process.
    fn resolve_args(
        &self,
        input: &str,
        output: &str,
        language: &str,
        diarize: bool,
    ) -> Vec<String> {
        let subst = |arg: &str| {
            arg.replace("{input}", input)
                .replace("{output}", output)
                .replace("{language}", language)
        };
        let mut resolved: Vec<String> = self.args.iter().map(|a| subst(a)).collect();
        if diarize {
            resolved.extend(self.diarize_args.iter().map(|a| subst(a)));
        }
        resolved
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

    async fn transcribe(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let input_file = request
            .input_file
            .as_deref()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| EngineError::InvalidRequest("ASR requires input_file".into()))?;

        let language = request
            .params
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        // Speaker diarization (S1): requested per-call, honored only when the
        // backend has diarize arguments configured (otherwise transcribe plainly).
        let diarize = request
            .params
            .get("diarize")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if diarize && self.diarize_args.is_empty() {
            tracing::debug!(
                provider = %self.name,
                "diarization requested but no diarize_args configured; transcribing without speaker separation"
            );
        }

        // Materialize a transcript output path only if the template references it;
        // otherwise the command's stdout is the transcript.
        let uses_output = self
            .args
            .iter()
            .chain(self.diarize_args.iter())
            .any(|a| a.contains("{output}"));
        let output_file = uses_output.then(|| {
            self.output_dir
                .join(format!("mofa_asr_{}.txt", uuid::Uuid::new_v4()))
        });
        let output_str = output_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let resolved_args = self.resolve_args(input_file, &output_str, language, diarize);

        let result = Command::new(&self.command)
            .args(&resolved_args)
            .kill_on_drop(true)
            .output()
            .await;

        let output = match result {
            Ok(output) => output,
            Err(e) => {
                if let Some(path) = &output_file {
                    let _ = tokio::fs::remove_file(path).await;
                }
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("failed to spawn '{}': {e}", self.command),
                });
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(path) = &output_file {
                let _ = tokio::fs::remove_file(path).await;
            }
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!(
                    "ASR command exited with {}: {}",
                    output.status,
                    stderr.trim()
                ),
            });
        }

        // Read the transcript from the output file (if templated) or from stdout.
        let transcript = match &output_file {
            Some(path) => {
                let text = tokio::fs::read_to_string(path).await;
                let _ = tokio::fs::remove_file(path).await;
                text.map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("cannot read ASR transcript: {e}"),
                })?
            }
            None => String::from_utf8_lossy(&output.stdout).into_owned(),
        };

        let transcript = transcript.trim().to_string();
        if transcript.is_empty() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "ASR command produced no transcript".into(),
            });
        }

        Ok(InferenceResponse {
            text: Some(transcript),
            file: None,
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
impl Provider for LocalAsrProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalAsr
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
                detail: format!("ASR command '{}' not found", self.command),
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
        let capability = request.capability.unwrap_or(Capability::Asr);

        if capability != Capability::Asr {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' only supports asr, not {capability}",
                self.name
            )));
        }
        if !self.model_supports(model_name, Capability::Asr) {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{model_name}' does not support asr",
                self.name
            )));
        }

        self.transcribe(model_name, request, std::time::Instant::now())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asr_models() -> Vec<ModelDef> {
        vec![ModelDef {
            name: "fixture".into(),
            capability: "asr".into(),
            context_window: None,
            memory_mb: Some(256),
            ..Default::default()
        }]
    }

    fn asr_request(input_file: Option<&str>) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::Asr),
            input_file: input_file.map(String::from),
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    fn metadata_and_kind() {
        let p = LocalAsrProvider::new("local-asr", "sh", vec![], None, asr_models());
        assert_eq!(p.kind(), ProviderKind::LocalAsr);
        assert!(p.kind().is_local());
        assert_eq!(p.name(), "local-asr");
    }

    #[tokio::test]
    async fn discover_reports_configured_asr_model() {
        let p = LocalAsrProvider::new("local-asr", "sh", vec![], None, asr_models());
        let cards = p.discover().await.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "local-asr/fixture");
        assert_eq!(cards[0].capability, Capability::Asr);
        assert_eq!(cards[0].memory_estimate_bytes, 256 * 1024 * 1024);
    }

    #[tokio::test]
    async fn transcribes_from_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("clip.wav");
        std::fs::write(&audio, b"fake-audio").unwrap();
        // A deterministic "ASR" fixture: echo a transcript to stdout.
        let p = LocalAsrProvider::new(
            "local-asr",
            "sh",
            vec!["-c".into(), "echo 'hello from audio'".into()],
            Some(dir.path().to_string_lossy().to_string()),
            asr_models(),
        );

        let resp = p
            .invoke(
                "local-asr/fixture",
                &asr_request(Some(audio.to_str().unwrap())),
            )
            .await
            .unwrap();
        assert_eq!(resp.text.as_deref(), Some("hello from audio"));
    }

    #[tokio::test]
    async fn transcribes_from_output_file_with_input_and_language() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("clip.wav");
        std::fs::write(&audio, b"fake-audio").unwrap();
        // Fixture writes "<language>:<input basename>" to the output file so we can
        // assert the placeholders were substituted and the file was read back.
        let p = LocalAsrProvider::new(
            "local-asr",
            "sh",
            vec![
                "-c".into(),
                "printf '%s:%s' \"$1\" \"$(basename \"$2\")\" > \"$3\"".into(),
                "sh".into(),
                "{language}".into(),
                "{input}".into(),
                "{output}".into(),
            ],
            Some(dir.path().to_string_lossy().to_string()),
            asr_models(),
        );

        let mut req = asr_request(Some(audio.to_str().unwrap()));
        req.params = serde_json::json!({ "language": "en" });
        let resp = p.invoke("local-asr/fixture", &req).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("en:clip.wav"));
    }

    #[tokio::test]
    async fn missing_input_file_is_invalid_request() {
        let p = LocalAsrProvider::new("local-asr", "sh", vec![], None, asr_models());
        let err = p
            .invoke("local-asr/fixture", &asr_request(None))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn command_failure_is_a_retryable_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("clip.wav");
        std::fs::write(&audio, b"x").unwrap();
        let p = LocalAsrProvider::new(
            "local-asr",
            "sh",
            vec!["-c".into(), "echo boom >&2; exit 4".into()],
            Some(dir.path().to_string_lossy().to_string()),
            asr_models(),
        );
        let err = p
            .invoke(
                "local-asr/fixture",
                &asr_request(Some(audio.to_str().unwrap())),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        assert!(err.retryable());
    }

    #[tokio::test]
    async fn empty_transcript_is_a_provider_error() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("clip.wav");
        std::fs::write(&audio, b"x").unwrap();
        let p = LocalAsrProvider::new(
            "local-asr",
            "sh",
            vec!["-c".into(), "true".into()], // no output
            Some(dir.path().to_string_lossy().to_string()),
            asr_models(),
        );
        let err = p
            .invoke(
                "local-asr/fixture",
                &asr_request(Some(audio.to_str().unwrap())),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
    }

    #[test]
    fn resolve_args_appends_diarize_args_only_when_requested() {
        let p = LocalAsrProvider::new(
            "local-asr",
            "asr",
            vec!["{input}".into(), "--lang".into(), "{language}".into()],
            None,
            asr_models(),
        )
        .with_diarize_args(vec!["--diarize".into(), "--out".into(), "{output}".into()]);

        // Without diarization: base args only.
        let plain = p.resolve_args("clip.wav", "out.txt", "en", false);
        assert_eq!(plain, vec!["clip.wav", "--lang", "en"]);

        // With diarization: the diarize args are appended, placeholders substituted.
        let diarized = p.resolve_args("clip.wav", "out.txt", "en", true);
        assert_eq!(
            diarized,
            vec!["clip.wav", "--lang", "en", "--diarize", "--out", "out.txt"]
        );
    }

    #[tokio::test]
    async fn diarize_flag_from_params_reaches_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("clip.wav");
        std::fs::write(&audio, b"fake-audio").unwrap();
        // The fixture echoes its first argument, so the transcript reveals whether
        // the diarize marker was passed through from `params.diarize`.
        let p = LocalAsrProvider::new(
            "local-asr",
            "sh",
            vec!["-c".into(), "echo \"$1\"".into(), "sh".into()],
            Some(dir.path().to_string_lossy().to_string()),
            asr_models(),
        )
        .with_diarize_args(vec!["SPEAKERS".into()]);

        // diarize = true → the marker is appended and echoed back.
        let mut req = asr_request(Some(audio.to_str().unwrap()));
        req.params = serde_json::json!({ "diarize": true });
        let resp = p.invoke("local-asr/fixture", &req).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("SPEAKERS"));

        // diarize omitted → no marker (empty first arg).
        let plain = asr_request(Some(audio.to_str().unwrap()));
        let resp = p.invoke("local-asr/fixture", &plain).await.unwrap_err();
        // No first arg → empty stdout → treated as an empty transcript.
        assert!(matches!(resp, EngineError::ProviderError { .. }));
    }

    #[tokio::test]
    async fn rejects_unsupported_capability() {
        let p = LocalAsrProvider::new("local-asr", "sh", vec![], None, asr_models());
        let mut req = asr_request(Some("/tmp/x.wav"));
        req.capability = Some(Capability::Chat);
        let err = p.invoke("local-asr/fixture", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }
}
