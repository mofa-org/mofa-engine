//! Generic OpenAI-compatible provider backend.
//!
//! Works with APIs that follow OpenAI-style chat, TTS, and ASR contracts.

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelResidency, Provider,
    ProviderKind, StreamDelta, StreamSink, canonical_model_id, model_id_name,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::ModelDef;

/// A provider for any OpenAI-compatible API.
pub struct OpenAiCompatProvider {
    /// Display name.
    name: String,
    /// Base URL.
    base_url: String,
    /// Bearer token.
    api_key: String,
    /// Configured models.
    models: Vec<ModelDef>,
    /// Cost tier for all models from this provider.
    cost_tier: CostTier,
    /// Directory for generated TTS artifacts.
    output_dir: std::path::PathBuf,
    /// HTTP client.
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
        Self::with_output_dir(name, base_url, api_key, models, cost_tier, None)
    }

    /// Replace the built-in HTTP client. The default client follows system
    /// proxy settings (cloud providers may need one); callers talking to a
    /// loopback endpoint can inject a `no_proxy()` client instead.
    pub fn with_http_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// Create a provider, writing TTS artifacts into `output_dir` (or the system
    /// temp dir when `None`) so they land where the artifact sweeper looks.
    pub fn with_output_dir(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Self {
        // A failure here means the system TLS/HTTP stack is unusable, so fail
        // loudly rather than falling back to a client without our timeouts.
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");

        Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            models,
            cost_tier,
            output_dir: output_dir
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            client,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    /// Hybrid reasoning models (e.g. Qwen3) gate their thinking mode with
    /// this body flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_thinking: Option<bool>,
}

/// `stream_options` payload: ask OpenAI-compatible providers to attach a
/// final usage frame to the SSE stream.
#[derive(Debug, Serialize)]
struct StreamOptions {
    include_usage: bool,
}

/// POST {base}/images/generations — the OpenAI image API shape.
#[derive(Debug, Serialize)]
struct ImageGenRequest {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageGenResponse {
    data: Option<Vec<ImageGenItem>>,
}

#[derive(Debug, Deserialize)]
struct ImageGenItem {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    /// DeepSeek-R1 style reasoning trace, on response messages only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

/// One SSE frame of a streaming chat completion.
#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    choices: Option<Vec<ChatStreamChoice>>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChoice {
    delta: Option<ChatStreamDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
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

#[derive(Debug, Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![BackendFeature::Discovery]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        let cards = self
            .models
            .iter()
            .filter_map(|m| {
                let cap = Capability::from_str_loose(&m.capability)?;
                let mut card =
                    ModelCard::new(self.name.clone(), m.name.clone(), cap, self.cost_tier);
                card.id = canonical_model_id(&self.name, &m.name);
                card.availability = ModelAvailability::Configured;
                card.residency = ModelResidency::Remote;
                card.context_window = m.context_window.unwrap_or(4096);
                card.memory_estimate_bytes = m.memory_mb.unwrap_or(0) * 1024 * 1024;
                card.execution.max_concurrency = 32;
                card.refresh_status();
                Some(card)
            })
            .collect();
        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        if self.api_key.is_empty() {
            return Ok(BackendHealth::Unavailable);
        }

        let models_url = format!("{}/models", self.base_url);
        let resp = self
            .client
            .get(&models_url)
            .bearer_auth(&self.api_key)
            .send()
            .await;

        if let Ok(r) = resp {
            if r.status().is_success() {
                return Ok(BackendHealth::Healthy);
            }
            if r.status().as_u16() == 404 && self.health_via_chat().await {
                return Ok(BackendHealth::Healthy);
            }
            if r.status().is_server_error() {
                return Ok(BackendHealth::Degraded);
            }
            return Ok(BackendHealth::Unavailable);
        }

        if self.health_via_chat().await {
            Ok(BackendHealth::Healthy)
        } else {
            Ok(BackendHealth::Unavailable)
        }
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: canonical_model_id(&self.name, model_id_name(model_id)),
            residency: ModelResidency::Remote,
            memory_bytes: Some(0),
            changed: false,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        Ok(LifecycleResult {
            model_id: canonical_model_id(&self.name, model_id_name(model_id)),
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
        let model_name = model_id_name(model_id);
        let capability = request.capability.unwrap_or(Capability::Chat);
        let start = std::time::Instant::now();

        let supports = self.models.iter().any(|m| {
            m.name == model_name && Capability::from_str_loose(&m.capability) == Some(capability)
        });
        if !supports {
            return Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' model '{}' does not support {capability}",
                self.name, model_name
            )));
        }

        match capability {
            Capability::Chat => self.invoke_chat(model_name, request, start).await,
            Capability::Tts => self.invoke_tts(model_name, request, start).await,
            Capability::Asr => self.invoke_asr(model_name, request, start).await,
            Capability::ImageGen => self.invoke_image_gen(model_name, request, start).await,
            other => Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' does not support {other}",
                self.name
            ))),
        }
    }

    /// Stream chat completions token-by-token. Reasoning models surface their
    /// thinking trace as `Thinking` deltas ahead of the answer text. Other
    /// capabilities keep the default single-delta compatibility path.
    async fn stream(
        &self,
        model_id: &str,
        request: &InferenceRequest,
        sink: StreamSink,
    ) -> Result<InferenceResponse, EngineError> {
        let capability = request.capability.unwrap_or(Capability::Chat);
        if capability != Capability::Chat {
            let response = self.invoke(model_id, request).await?;
            if let Some(text) = &response.text
                && !text.is_empty()
            {
                let _ = sink.send(StreamDelta::Text(text.clone())).await;
            }
            return Ok(response);
        }
        let model_name = model_id_name(model_id);
        self.stream_chat(model_name, request, sink).await
    }
}

impl OpenAiCompatProvider {
    async fn health_via_chat(&self) -> bool {
        let Some(first_chat_model) = self
            .models
            .iter()
            .find(|m| Capability::from_str_loose(&m.capability) == Some(Capability::Chat))
            .map(|m| m.name.clone())
        else {
            return false;
        };

        let body = ChatCompletionRequest {
            model: first_chat_model,
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
                reasoning_content: None,
            }],
            max_tokens: Some(1),
            temperature: None,
            stream: None,
            stream_options: None,
            enable_thinking: None,
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
                reasoning_content: None,
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

        let temperature = request.params.get("temperature").and_then(|v| v.as_f64());
        let enable_thinking = request
            .params
            .get("enable_thinking")
            .and_then(|v| v.as_bool());

        let body = ChatCompletionRequest {
            model: model_name.to_string(),
            messages,
            max_tokens,
            temperature,
            stream: None,
            stream_options: None,
            enable_thinking,
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

        let answer = chat
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.message.as_ref());
        let text = answer.map(|m| m.content.clone());
        let reasoning = answer
            .and_then(|m| m.reasoning_content.clone())
            .filter(|r| !r.is_empty());

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
            fallback_used: false,
            routing_reason: None,
            reasoning,
            files: Vec::new(),
        })
    }

    /// Generate images via the OpenAI images API. b64 payloads are written
    /// straight to the artifacts dir; url payloads are downloaded first.
    /// Every artifact path lands in `files` (the first also mirrors into
    /// `file` for single-image callers).
    async fn invoke_image_gen(
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
            .ok_or_else(|| {
                EngineError::InvalidRequest("image generation requires a prompt".into())
            })?;

        let n = request
            .params
            .get("n")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let size = request
            .params
            .get("size")
            .and_then(|v| v.as_str())
            .map(String::from);

        let body = ImageGenRequest {
            model: model_name.to_string(),
            prompt,
            n,
            size,
            response_format: Some("b64_json".into()),
        };

        let url = format!("{}/images/generations", self.base_url);
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
                detail: format!("image gen HTTP {status}: {text}"),
            });
        }

        let payload: ImageGenResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("image gen parse error: {e}"),
            })?;

        let items = payload.data.unwrap_or_default();
        if items.is_empty() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image gen returned no artifacts".into(),
            });
        }

        let mut files: Vec<String> = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let bytes = if let Some(b64) = item.b64_json {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(b64.as_bytes())
                    .map_err(|e| EngineError::ProviderError {
                        provider: self.name.clone(),
                        detail: format!("invalid b64 image: {e}"),
                    })?
            } else if let Some(image_url) = item.url {
                self.client
                    .get(&image_url)
                    .send()
                    .await
                    .map_err(|e| EngineError::ProviderError {
                        provider: self.name.clone(),
                        detail: format!("image download failed: {e}"),
                    })?
                    .bytes()
                    .await
                    .map_err(|e| EngineError::ProviderError {
                        provider: self.name.clone(),
                        detail: format!("image download read failed: {e}"),
                    })?
                    .to_vec()
            } else {
                continue;
            };
            let path = self.output_dir.join(format!(
                "mofa_img_{:02}_{}.png",
                index,
                uuid::Uuid::new_v4()
            ));
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| EngineError::Internal(format!("write error: {e}")))?;
            files.push(path.to_string_lossy().to_string());
        }

        if files.is_empty() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image gen artifacts had neither b64_json nor url".into(),
            });
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(InferenceResponse {
            text: None,
            file: Some(files[0].clone()),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: None,
            fallback_used: false,
            routing_reason: None,
            reasoning: None,
            files,
        })
    }

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
                request
                    .params
                    .get("input")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .ok_or_else(|| EngineError::InvalidRequest("TTS requires text input".into()))?;

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

        let path = self
            .output_dir
            .join(format!("mofa_tts_{}.mp3", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("write error: {e}")))?;
        let path = path.to_string_lossy().to_string();

        let duration_ms = start.elapsed().as_millis() as u64;
        Ok(InferenceResponse {
            text: None,
            file: Some(path),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: None,
            fallback_used: false,
            routing_reason: None,
            reasoning: None,
            files: Vec::new(),
        })
    }

    async fn invoke_asr(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let file_path = request
            .input_file
            .as_deref()
            .ok_or_else(|| EngineError::InvalidRequest("ASR requires input_file".into()))?;

        let file_bytes = tokio::fs::read(file_path).await.map_err(|e| {
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

        let asr: AsrResponse = resp.json().await.map_err(|e| EngineError::ProviderError {
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
            fallback_used: false,
            routing_reason: None,
            reasoning: None,
            files: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Image generation against a mock images endpoint: b64 artifacts are
    /// written to the output dir, `files` lists them all, and the request
    /// body carries prompt/n/size.
    #[tokio::test]
    async fn image_gen_writes_artifacts_and_forwards_params() {
        use base64::Engine as _;
        let png_1px_red = base64::engine::general_purpose::STANDARD.encode([
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE,
        ]);

        let (seen_tx, mut seen_rx) = tokio::sync::mpsc::channel::<serde_json::Value>(1);
        let seen_for_task = std::sync::Arc::new(seen_tx);

        let app = axum::Router::new().route(
            "/v1/images/generations",
            axum::routing::post(
                move |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    let _ = seen_for_task.send(body).await;
                    axum::Json(serde_json::json!({
                        "created": 1,
                        "data": [
                            { "b64_json": png_1px_red },
                            { "b64_json": png_1px_red },
                        ]
                    }))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let output_dir = tempfile::tempdir().unwrap();
        let provider = OpenAiCompatProvider::with_output_dir(
            "mock-openai",
            format!("http://{addr}/v1"),
            "sk-test",
            vec![ModelDef {
                name: "gpt-image".into(),
                capability: "image_gen".into(),
                context_window: None,
                memory_mb: None,
            }],
            CostTier::High,
            Some(output_dir.path().to_string_lossy().to_string()),
        )
        .with_http_client(reqwest::Client::builder().no_proxy().build().unwrap());

        let request = InferenceRequest {
            capability: Some(Capability::ImageGen),
            model: Some("mock-openai/gpt-image".into()),
            app_id: None,
            session_id: None,
            fallback_policy: Default::default(),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "一只橘猫".into(),
            }],
            input_file: None,
            params: serde_json::json!({"n": 2, "size": "1024x1024"}),
            hint_next: None,
            request_id: "req-img".into(),
        };

        let resp = provider
            .invoke("mock-openai/gpt-image", &request)
            .await
            .expect("image gen succeeds");

        assert_eq!(resp.files.len(), 2);
        assert_eq!(resp.file.as_deref(), Some(resp.files[0].as_str()));
        for path in &resp.files {
            let bytes = std::fs::read(path).expect("artifact on disk");
            // PNG magic survived the round-trip.
            assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
        }

        let sent = seen_rx.recv().await.expect("provider saw request body");
        assert_eq!(sent["prompt"], "一只橘猫");
        assert_eq!(sent["n"], 2);
        assert_eq!(sent["size"], "1024x1024");
        assert_eq!(sent["response_format"], "b64_json");
        assert_eq!(resp.model_used, "gpt-image");
    }
    /// Stream an OpenAI-style SSE chat completion (reasoning model) against a
    /// mock provider endpoint, and verify delta kinds, order, aggregation,
    /// and request passthrough (stream flag, temperature, max_tokens).
    #[tokio::test]
    async fn stream_chat_parses_reasoning_and_text_deltas() {
        use axum::body::Body;
        use axum::http::header;
        use axum::response::Response;
        use tokio_stream::wrappers::ReceiverStream;

        let (seen_tx, mut seen_rx) = tokio::sync::mpsc::channel::<serde_json::Value>(1);
        let seen = std::sync::Arc::new(seen_tx);
        let seen_for_task = seen.clone();

        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(
                move |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    let _ = seen_for_task.send(body).await;
                    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(8);
                    tokio::spawn(async move {
                        // Deliberately split the first frame across two
                        // channel items: the parser must buffer partial lines.
                        let frames: Vec<String> = vec![
                            r#"data: {"choices":[{"delta":{"reasoning_content":"思考"}}]}"#.into(),
                            "\n\n".into(),
                            r#"data: {"choices":[{"delta":{"content":"答"}}]}"#.into(),
                            "\n\n".into(),
                            r#"data: {"choices":[{"delta":{"content":"案"}}]}"#.into(),
                            "\n\n".into(),
                            r#"data: {"choices":[],"usage":{"total_tokens":7}}"#.into(),
                            "\n\n".into(),
                            "data: [DONE]\n\n".into(),
                        ];
                        for frame in frames {
                            let mid = frame.len() / 2;
                            let (a, b) = frame.split_at(mid);
                            let _ = tx.send(Ok(a.as_bytes().to_vec())).await;
                            let _ = tx.send(Ok(b.as_bytes().to_vec())).await;
                        }
                    });
                    let body = Body::from_stream(ReceiverStream::new(rx));
                    Response::builder()
                        .status(200)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .body(body)
                        .unwrap()
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // The default client follows system proxy settings, which cannot
        // reach the loopback mock; talk to it directly.
        let direct_client = Client::builder().no_proxy().build().expect("test client");
        let provider = OpenAiCompatProvider::new(
            "mock-openai",
            &format!("http://{addr}/v1"),
            "sk-test",
            vec![ModelDef {
                name: "reasoner".into(),
                capability: "chat".into(),
                context_window: Some(8192),
                memory_mb: None,
            }],
            CostTier::Low,
        )
        .with_http_client(direct_client);

        let request = InferenceRequest {
            capability: Some(Capability::Chat),
            model: Some("mock-openai/reasoner".into()),
            app_id: None,
            session_id: None,
            fallback_policy: Default::default(),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "hi".into(),
            }],
            input_file: None,
            params: serde_json::json!({"temperature": 0.3, "max_tokens": 64, "enable_thinking": true}),
            hint_next: None,
            request_id: "req-t".into(),
        };

        let (sink_tx, mut sink_rx) = tokio::sync::mpsc::channel::<StreamDelta>(16);
        let resp = provider
            .stream("mock-openai/reasoner", &request, sink_tx)
            .await
            .expect("stream succeeds");

        let mut kinds = Vec::new();
        while let Some(delta) = sink_rx.recv().await {
            kinds.push(if delta.is_thinking() {
                "thinking"
            } else {
                "text"
            });
        }
        assert_eq!(kinds, vec!["thinking", "text", "text"]);
        assert_eq!(resp.text.as_deref(), Some("答案"));
        assert_eq!(resp.reasoning.as_deref(), Some("思考"));
        assert_eq!(resp.tokens_used, Some(7));

        let sent = seen_rx.recv().await.expect("provider saw request body");
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["stream_options"]["include_usage"], true);
        assert_eq!(sent["temperature"], 0.3);
        assert_eq!(sent["max_tokens"], 64);
        assert_eq!(sent["enable_thinking"], true);
        assert_eq!(resp.model_used, "reasoner");
    }
    #[tokio::test]
    async fn discover_returns_configured_models() {
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

        let cards = provider.discover().await.unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].id, "test/model-a");
        assert_eq!(cards[0].capability, Capability::Chat);
        assert_eq!(cards[0].residency, ModelResidency::Remote);
        assert_eq!(cards[1].capability, Capability::Tts);
        assert_eq!(cards[1].memory_estimate_bytes, 512 * 1024 * 1024);
    }

    #[test]
    fn kind_is_openai_compat() {
        let p = OpenAiCompatProvider::new("x", "https://example.com", "key", vec![], CostTier::Low);
        assert_eq!(p.kind(), ProviderKind::OpenAiCompatible);
        assert_eq!(p.name(), "x");
    }
}

// ==================== Streaming ====================

impl OpenAiCompatProvider {
    /// Stream a chat completion: POST with `stream: true`, parse the SSE
    /// frames, forward `reasoning_content` deltas as `Thinking` and `content`
    /// deltas as `Text`, and return the aggregate response.
    async fn stream_chat(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        sink: StreamSink,
    ) -> Result<InferenceResponse, EngineError> {
        let start = std::time::Instant::now();
        let messages: Vec<ChatMessage> = request
            .messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                reasoning_content: None,
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
        let temperature = request.params.get("temperature").and_then(|v| v.as_f64());
        let enable_thinking = request
            .params
            .get("enable_thinking")
            .and_then(|v| v.as_bool());

        let body = ChatCompletionRequest {
            model: model_name.to_string(),
            messages,
            max_tokens,
            temperature,
            stream: Some(true),
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            enable_thinking,
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

        let mut text_out = String::new();
        let mut reasoning_out = String::new();
        let mut tokens_used: Option<u32> = None;
        // SSE frames can split across network chunks; buffer partial lines
        // (splitting at `\n` is UTF-8 safe).
        let mut line_buf: Vec<u8> = Vec::new();
        let mut upstream = resp.bytes_stream();
        while let Some(item) = tokio_stream::StreamExt::next(&mut upstream).await {
            let bytes = item.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: e.to_string(),
            })?;
            line_buf.extend_from_slice(&bytes);
            while let Some(pos) = line_buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = line_buf.drain(..=pos).collect();
                consume_sse_line(
                    &String::from_utf8_lossy(&line),
                    &sink,
                    &mut text_out,
                    &mut reasoning_out,
                    &mut tokens_used,
                )
                .await;
            }
        }

        Ok(InferenceResponse {
            text: (!text_out.is_empty()).then_some(text_out),
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used,
            fallback_used: false,
            routing_reason: None,
            reasoning: (!reasoning_out.is_empty()).then_some(reasoning_out),
            files: Vec::new(),
        })
    }
}

/// Parse one SSE line from an OpenAI-compatible stream and push its deltas.
async fn consume_sse_line(
    line: &str,
    sink: &StreamSink,
    text_out: &mut String,
    reasoning_out: &mut String,
    tokens_used: &mut Option<u32>,
) {
    let line = line.trim_end_matches(['\n', '\r']);
    let Some(data) = line.strip_prefix("data: ") else {
        return; // comments, keep-alive pings, `event:` lines
    };
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return;
    }
    let Ok(chunk) = serde_json::from_str::<ChatStreamChunk>(data) else {
        return; // non-JSON keep-alive payload
    };
    if let Some(choice) = chunk.choices.as_ref().and_then(|c| c.first()) {
        if let Some(delta) = &choice.delta {
            if let Some(reasoning) = delta.reasoning_content.as_ref().filter(|r| !r.is_empty()) {
                reasoning_out.push_str(reasoning);
                let _ = sink.send(StreamDelta::Thinking(reasoning.clone())).await;
            }
            if let Some(content) = delta.content.as_ref().filter(|c| !c.is_empty()) {
                text_out.push_str(content);
                let _ = sink.send(StreamDelta::Text(content.clone())).await;
            }
        }
    }
    if let Some(usage) = chunk.usage {
        *tokens_used = usage.total_tokens;
    }
}
