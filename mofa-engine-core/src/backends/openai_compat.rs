//! Generic OpenAI-compatible provider backend.
//!
//! Works with any API that follows the OpenAI chat/completions contract:
//! OpenAI, DeepSeek, DashScope, NVIDIA NIM, Perplexity, Zhipu, etc.

use async_trait::async_trait;
use mofa_kernel::{
    Capability, CostTier, EngineError, InferenceRequest, InferenceResponse, ModelCard,
    ModelStatus, Provider, ProviderKind,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::ModelDef;

/// A provider for any OpenAI-compatible API.
pub struct OpenAiCompatProvider {
    /// Display name
    name: String,
    /// Base URL (e.g. `https://api.openai.com/v1`)
    base_url: String,
    /// Bearer token
    api_key: String,
    /// Configured models
    models: Vec<ModelDef>,
    /// Cost tier for all models from this provider
    cost_tier: CostTier,
    /// HTTP client
    client: Client,
}

impl OpenAiCompatProvider {
    /// Create a new OpenAI-compatible provider.
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
    ) -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default();

        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            models,
            cost_tier,
            client,
        }
    }
}

// ── OpenAI chat/completions request/response ────────────────────────────

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Option<Vec<ChatChoice>>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: Option<ChatMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    total_tokens: Option<u32>,
}

// ── TTS request ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
    async fn discover(&self) -> Vec<ModelCard> {
        self.models
            .iter()
            .map(|m| {
                let cap = Capability::from_str_loose(&m.capability)
                    .unwrap_or(Capability::Chat);
                let id = format!("{}::{}", self.name, m.name);

                ModelCard {
                    id,
                    name: m.name.clone(),
                    provider: self.name.clone(),
                    capability: cap,
                    status: ModelStatus::Cold,
                    cost_tier: self.cost_tier,
                    context_window: m.context_window.unwrap_or(4096),
                    memory_estimate_bytes: m.memory_mb.unwrap_or(0) * 1024 * 1024,
                }
            })
            .collect()
    }

    async fn health(&self) -> bool {
        // Try /models endpoint first
        let models_url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&models_url)
            .bearer_auth(&self.api_key)
            .send()
            .await;

        if let Ok(r) = resp {
            if r.status().is_success() {
                return true;
            }
            // DashScope quirk: /models may 404, try a minimal chat request
            if r.status().as_u16() == 404 {
                return self.health_via_chat().await;
            }
        }

        // Network error — try chat fallback
        self.health_via_chat().await
    }

    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let model_name = model_id.split("::").nth(1).unwrap_or(model_id);

        let capability = request.capability.unwrap_or(Capability::Chat);
        let start = std::time::Instant::now();

        match capability {
            Capability::Chat => self.invoke_chat(model_name, request, start).await,
            Capability::Tts => self.invoke_tts(model_name, request, start).await,
            Capability::Asr => self.invoke_asr(model_name, request, start).await,
            _ => self.invoke_chat(model_name, request, start).await,
        }
    }

    async fn warm(&self, _model_id: &str) {
        // Cloud providers are always warm — no-op
    }

    async fn evict(&self, _model_id: &str) {
        // Cloud providers don't need eviction — no-op
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }
}

impl OpenAiCompatProvider {
    /// Health check fallback via a minimal chat completion.
    async fn health_via_chat(&self) -> bool {
        let first_chat_model = self
            .models
            .iter()
            .find(|m| m.capability == "chat")
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "gpt-4o-mini".into());

        let body = ChatCompletionRequest {
            model: first_chat_model,
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            max_tokens: Some(1),
        };

        let url = format!("{}/chat/completions", self.base_url);
        matches!(
            self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await,
            Ok(r) if r.status().is_success()
        )
    }

    /// Invoke chat completion.
    async fn invoke_chat(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let messages: Vec<ChatMessage> = request
            .messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
        }

        let max_tokens = request
            .params
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let body = ChatCompletionRequest {
            model: model_name.to_string(),
            messages,
            max_tokens,
        };

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("HTTP {status}: {text}"),
            });
        }

        let chat: ChatCompletionResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("parse error: {e}"),
            })?;

        let text = chat
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.message.as_ref())
            .map(|m| m.content.clone());

        let tokens = chat.usage.and_then(|u| u.total_tokens);
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResponse {
            text,
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: tokens,
        })
    }

    /// Invoke TTS (text-to-speech) — returns a path to the generated audio file.
    async fn invoke_tts(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let input_text = request
            .messages
            .first()
            .map(|m| m.content.clone())
            .or_else(|| {
                request.params.get("input").and_then(|v| v.as_str()).map(String::from)
            })
            .ok_or_else(|| {
                EngineError::InvalidRequest("TTS requires text input".into())
            })?;

        let voice = request
            .params
            .get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or("alloy")
            .to_string();

        let body = TtsRequest {
            model: model_name.to_string(),
            input: input_text,
            voice,
            response_format: "mp3".into(),
        };

        let url = format!("{}/audio/speech", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("TTS HTTP {status}: {text}"),
            });
        }

        let bytes = resp.bytes().await.map_err(|e| EngineError::ProviderError {
            provider: self.name.clone(),
            detail: format!("TTS read error: {e}"),
        })?;

        // Write to temp file
        let tmp = tempfile::Builder::new()
            .suffix(".mp3")
            .tempfile()
            .map_err(|e| EngineError::Internal(format!("temp file error: {e}")))?;

        let path = tmp.path().to_string_lossy().to_string();
        std::fs::write(&path, &bytes).map_err(|e| {
            EngineError::Internal(format!("write error: {e}"))
        })?;

        // Keep the file alive (don't let tempfile delete it)
        let _ = tmp.into_temp_path();

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(InferenceResponse {
            text: None,
            file: Some(path),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: None,
        })
    }

    /// Invoke ASR (speech-to-text) via multipart upload.
    async fn invoke_asr(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let file_path = request.input_file.as_deref().ok_or_else(|| {
            EngineError::InvalidRequest("ASR requires input_file".into())
        })?;

        let file_bytes = std::fs::read(file_path).map_err(|e| {
            EngineError::InvalidRequest(format!("cannot read file '{file_path}': {e}"))
        })?;

        let file_name = std::path::Path::new(file_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "audio.mp3".into());

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|e| EngineError::Internal(format!("mime error: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .text("model", model_name.to_string())
            .part("file", part);

        let url = format!("{}/audio/transcriptions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("ASR HTTP {status}: {text}"),
            });
        }

        #[derive(Deserialize)]
        struct AsrResponse {
            text: Option<String>,
        }

        let asr: AsrResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("ASR parse error: {e}"),
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(InferenceResponse {
            text: asr.text,
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_returns_configured_models() {
        let provider = OpenAiCompatProvider::new(
            "test",
            "https://api.example.com/v1",
            "sk-test",
            vec![
                ModelDef {
                    name: "model-a".into(),
                    capability: "chat".into(),
                    context_window: Some(8192),
                    memory_mb: None,
                },
                ModelDef {
                    name: "model-b".into(),
                    capability: "tts".into(),
                    context_window: None,
                    memory_mb: Some(512),
                },
            ],
            CostTier::Medium,
        );

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let cards = rt.block_on(provider.discover());
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, "test::model-a");
        assert_eq!(cards[0].capability, Capability::Chat);
        assert_eq!(cards[1].capability, Capability::Tts);
        assert_eq!(cards[1].memory_estimate_bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn kind_is_openai_compat() {
        let p = OpenAiCompatProvider::new(
            "x",
            "https://example.com",
            "key",
            vec![],
            CostTier::Low,
        );
        assert_eq!(p.kind(), ProviderKind::OpenAiCompatible);
    }
}
