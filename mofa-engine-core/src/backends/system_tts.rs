//! Built-in, zero-config local TTS using the operating system's native voice.
//!
//! Every other TTS path ([`super::local_tts`]) shells out to a command the user
//! must install (Crane/Kokoro). This provider needs nothing installed: it uses the
//! voice already present on the machine — `say` on macOS, `espeak-ng`/`espeak` on
//! Linux — so a fresh checkout has a working local voice out of the box. The
//! engine auto-registers it (see `Engine::try_new`) as a low-priority fallback
//! whenever a real config declares no TTS backend, so configured voices still win.
//!
//! ## Output format
//!
//! The native tool produces an intermediate (AIFF on macOS, WAV on Linux) which is
//! transcoded to the requested `params.format`. `wav` is the zero-dependency
//! default (macOS `afconvert` and Linux `espeak` both produce it natively); `mp3`
//! is honored when `lame` or `ffmpeg` is available, otherwise the response falls
//! back to `wav` and its `file` carries the real extension. `voice` and `speed`
//! (`params.voice`/`params.speed`) pass through to the native tool.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind,
};
use tokio::process::Command;

/// The canonical name and model id for the built-in system voice.
pub(crate) const SYSTEM_TTS_NAME: &str = "system-tts";
const SYSTEM_TTS_MODEL: &str = "system";

/// Which OS-native TTS tool this build will drive, resolved once at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Voice {
    /// macOS `say` (+ `afconvert` for WAV/M4A).
    MacSay,
    /// Linux `espeak-ng` or `espeak` (writes WAV directly).
    Espeak,
    /// No system voice available on this machine.
    None,
}

/// A zero-config TTS provider backed by the operating system's voice.
pub(crate) struct SystemTtsProvider {
    output_dir: PathBuf,
    voice: Voice,
}

impl SystemTtsProvider {
    /// Build the provider, detecting the OS-native voice tool once.
    pub(crate) fn new(output_dir: Option<String>) -> Self {
        Self {
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
            voice: detect_voice(),
        }
    }

    /// Whether this machine has a usable system voice.
    pub(crate) fn is_available(&self) -> bool {
        self.voice != Voice::None
    }

    async fn synthesize(
        &self,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        // Text comes from the first message, or `params.input` (parity with local_tts).
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

        let voice_name = request
            .params
            .get("voice")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        // Optional speaking rate in words-per-minute (`params.speed` as a number or
        // numeric string). Left to the tool's default when absent or unparseable.
        let rate_wpm = request.params.get("speed").and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        });
        let want_format = request
            .params
            .get("format")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("wav")
            .to_ascii_lowercase();

        // A per-request scratch space; dropped (and cleaned) at the end.
        let stem = uuid::Uuid::new_v4().to_string();
        let text_file = self.output_dir.join(format!("mofa_tts_{stem}.txt"));
        tokio::fs::write(&text_file, &text)
            .await
            .map_err(|e| EngineError::Internal(format!("cannot write TTS input file: {e}")))?;

        // Synthesize to the tool's native container, then transcode to the request's
        // format (falling back to the native format when no encoder is available).
        let result = self
            .render(&text_file, voice_name, rate_wpm, &stem, &want_format)
            .await;
        let _ = tokio::fs::remove_file(&text_file).await;
        let output = result?;

        Ok(InferenceResponse {
            text: None,
            file: Some(output.to_string_lossy().to_string()),
            model_used: SYSTEM_TTS_MODEL.to_string(),
            provider: SYSTEM_TTS_NAME.to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used: None,
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }

    /// Run the native voice and transcode to `want_format`, returning the final
    /// audio path. The returned file's extension reflects what was actually
    /// produced (which may differ from `want_format` when no encoder exists).
    async fn render(
        &self,
        text_file: &Path,
        voice: Option<&str>,
        rate_wpm: Option<f64>,
        stem: &str,
        want_format: &str,
    ) -> Result<PathBuf, EngineError> {
        // 1. Native synthesis → an intermediate file.
        let native = match self.voice {
            Voice::MacSay => {
                let aiff = self.output_dir.join(format!("mofa_tts_{stem}.aiff"));
                let mut cmd = Command::new("say");
                if let Some(v) = voice {
                    cmd.arg("-v").arg(v);
                }
                if let Some(wpm) = rate_wpm.filter(|w| *w > 0.0) {
                    cmd.arg("-r").arg(format!("{}", wpm.round() as i64));
                }
                cmd.arg("-f").arg(text_file).arg("-o").arg(&aiff);
                run(cmd).await?;
                aiff
            }
            Voice::Espeak => {
                let wav = self.output_dir.join(format!("mofa_tts_{stem}.wav"));
                let program = if which("espeak-ng") {
                    "espeak-ng"
                } else {
                    "espeak"
                };
                let mut cmd = Command::new(program);
                if let Some(v) = voice {
                    cmd.arg("-v").arg(v);
                }
                if let Some(wpm) = rate_wpm.filter(|w| *w > 0.0) {
                    cmd.arg("-s").arg(format!("{}", wpm.round() as i64));
                }
                cmd.arg("-w").arg(&wav).arg("-f").arg(text_file);
                run(cmd).await?;
                wav
            }
            Voice::None => {
                return Err(EngineError::ProviderError {
                    provider: SYSTEM_TTS_NAME.into(),
                    detail: "no system voice available on this platform".into(),
                });
            }
        };

        // A successful command must have left a non-empty file.
        match tokio::fs::metadata(&native).await {
            Ok(m) if m.len() > 0 => {}
            _ => {
                let _ = tokio::fs::remove_file(&native).await;
                return Err(EngineError::ProviderError {
                    provider: SYSTEM_TTS_NAME.into(),
                    detail: "system voice produced no audio".into(),
                });
            }
        }

        // 2. Transcode to the requested format when it differs from the native one.
        let native_ext = native
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if want_format == native_ext {
            return Ok(native);
        }

        let target = self
            .output_dir
            .join(format!("mofa_tts_{stem}.{want_format}"));
        let transcoded = self.transcode(&native, &target, want_format).await;
        match transcoded {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&native).await;
                Ok(target)
            }
            // No encoder for the requested format: keep the native file so the
            // caller still gets real, playable audio (with its true extension).
            Err(_) => {
                let _ = tokio::fs::remove_file(&target).await;
                Ok(native)
            }
        }
    }

    /// Transcode `src` → `dst` in `format`, preferring built-in tools. Returns an
    /// error (handled by the caller as "keep the native file") when no suitable
    /// encoder is available.
    async fn transcode(&self, src: &Path, dst: &Path, format: &str) -> Result<(), EngineError> {
        // macOS `afconvert` covers wav/aiff/m4a with zero extra installs.
        if which("afconvert") {
            let spec = match format {
                "wav" => Some(("WAVE", "LEI16")),
                "aiff" | "aif" => Some(("AIFF", "BEI16")),
                "m4a" | "aac" => Some(("m4af", "aac")),
                _ => None,
            };
            if let Some((file_fmt, data_fmt)) = spec {
                let mut cmd = Command::new("afconvert");
                cmd.arg("-f")
                    .arg(file_fmt)
                    .arg("-d")
                    .arg(data_fmt)
                    .arg(src)
                    .arg(dst);
                return run(cmd).await;
            }
        }

        // mp3 (and anything afconvert can't do) via lame or ffmpeg when present.
        if format == "mp3" && which("lame") {
            let mut cmd = Command::new("lame");
            cmd.arg("--quiet").arg(src).arg(dst);
            return run(cmd).await;
        }
        if which("ffmpeg") {
            let mut cmd = Command::new("ffmpeg");
            cmd.arg("-y")
                .arg("-loglevel")
                .arg("error")
                .arg("-i")
                .arg(src)
                .arg(dst);
            return run(cmd).await;
        }

        Err(EngineError::ProviderError {
            provider: SYSTEM_TTS_NAME.into(),
            detail: format!("no encoder available for '{format}'"),
        })
    }
}

#[async_trait]
impl Provider for SystemTtsProvider {
    fn name(&self) -> &str {
        SYSTEM_TTS_NAME
    }

    fn kind(&self) -> ProviderKind {
        // Reported as local TTS so routing/observability treat it like any other
        // on-device voice (local, free).
        ProviderKind::LocalTts
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![
            BackendFeature::Discovery,
            BackendFeature::Load,
            BackendFeature::Unload,
        ]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        if !self.is_available() {
            return Ok(vec![]);
        }
        let mut card = ModelCard::new(
            SYSTEM_TTS_NAME.to_string(),
            SYSTEM_TTS_MODEL.to_string(),
            Capability::Tts,
            CostTier::Free,
        );
        card.id = ModelId::canonical(SYSTEM_TTS_NAME, SYSTEM_TTS_MODEL);
        card.availability = ModelAvailability::Configured;
        card.residency = ModelResidency::Unloaded;
        card.refresh_status();
        Ok(vec![card])
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        if self.is_available() {
            Ok(BackendHealth::Healthy)
        } else {
            Ok(BackendHealth::Unavailable)
        }
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: ModelId::canonical(SYSTEM_TTS_NAME, ModelId::name(model_id)),
            residency: ModelResidency::Loaded,
            memory_bytes: Some(0),
            changed: true,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: ModelId::canonical(SYSTEM_TTS_NAME, ModelId::name(model_id)),
            residency: ModelResidency::Unloaded,
            memory_bytes: Some(0),
            changed: true,
        })
    }

    async fn invoke(
        &self,
        _model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let capability = request.capability.unwrap_or(Capability::Tts);
        if capability != Capability::Tts {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{SYSTEM_TTS_NAME}' only supports tts, not {capability}"
            )));
        }
        self.synthesize(request, std::time::Instant::now()).await
    }
}

// ==============================================================================
// Platform detection helpers
// ==============================================================================

/// Detect the OS-native voice tool available on this machine.
fn detect_voice() -> Voice {
    if cfg!(target_os = "macos") && which("say") {
        Voice::MacSay
    } else if which("espeak-ng") || which("espeak") {
        Voice::Espeak
    } else {
        Voice::None
    }
}

/// Whether a bare command name resolves on `PATH`.
fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(program);
                p.is_file()
            })
        })
        .unwrap_or(false)
}

/// Run a prepared command, mapping a non-zero exit or spawn failure to a
/// retryable provider error (so the engine can fail over to a cloud voice).
async fn run(mut cmd: Command) -> Result<(), EngineError> {
    let output = cmd
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| EngineError::ProviderError {
            provider: SYSTEM_TTS_NAME.into(),
            detail: format!("failed to spawn command: {e}"),
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(EngineError::ProviderError {
            provider: SYSTEM_TTS_NAME.into(),
            detail: format!(
                "command exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::Message;

    fn tts_request(text: &str, params: serde_json::Value) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::Tts),
            messages: vec![Message {
                role: "user".into(),
                content: text.into(),
                ..Default::default()
            }],
            params,
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[test]
    fn detects_a_voice_on_this_platform() {
        // CI/dev macOS always has `say`; the point is detection doesn't panic and
        // availability agrees with `discover`/`health`.
        let p = SystemTtsProvider::new(None);
        assert_eq!(p.kind(), ProviderKind::LocalTts);
        assert_eq!(p.name(), SYSTEM_TTS_NAME);
    }

    #[tokio::test]
    async fn discover_and_health_agree_on_availability() {
        let p = SystemTtsProvider::new(None);
        let cards = p.discover().await.unwrap();
        let health = p.health().await.unwrap();
        if p.is_available() {
            assert_eq!(cards.len(), 1);
            assert_eq!(cards[0].capability, Capability::Tts);
            assert_eq!(cards[0].id, "system-tts/system");
            assert_eq!(health, BackendHealth::Healthy);
        } else {
            assert!(cards.is_empty());
            assert_eq!(health, BackendHealth::Unavailable);
        }
    }

    #[tokio::test]
    async fn empty_text_is_an_invalid_request() {
        let p = SystemTtsProvider::new(None);
        let err = p
            .invoke(
                "system-tts/system",
                &tts_request("   ", serde_json::Value::Null),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn rejects_non_tts_capability() {
        let p = SystemTtsProvider::new(None);
        let mut req = tts_request("hi", serde_json::Value::Null);
        req.capability = Some(Capability::Chat);
        let err = p.invoke("system-tts/system", &req).await.unwrap_err();
        assert!(matches!(err, EngineError::UnsupportedOperation(_)));
    }

    // A real synthesis test would depend on `say`/`espeak` being installed and is
    // covered end-to-end by the `mofa-podcast`/`mofa-explainer` app runs; omitted
    // here to keep the unit tests hermetic.
}
