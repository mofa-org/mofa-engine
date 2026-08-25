//! Generic OpenAI-compatible provider backend.
//!
//! Works with APIs that follow OpenAI-style chat, TTS, ASR, embedding, and
//! image-generation (`/images/generations`) contracts — e.g. the Agnes AI
//! gateway used to validate the media scenarios end-to-end.

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind, ReasoningEffort, StreamDelta, StreamSink,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::ModelDef;

/// A provider for any OpenAI-compatible API.
pub(crate) struct OpenAiCompatProvider {
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
    /// Create a new OpenAI-compatible provider with default (temp-dir) artifact
    /// output. Test-support: the engine factory uses [`Self::with_output_dir`].
    #[cfg(test)]
    pub fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
    ) -> Result<Self, EngineError> {
        Self::with_output_dir(name, base_url, api_key, models, cost_tier, None)
    }

    /// Create a provider, writing TTS artifacts into `output_dir` (or the
    /// mofa-owned default artifact directory when `None`) so they land where
    /// the artifact sweeper looks.
    ///
    /// Fails (rather than panicking or silently dropping the configured
    /// timeouts) if the system TLS/HTTP stack cannot build a client.
    pub(crate) fn with_output_dir(
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Result<Self, EngineError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| {
                EngineError::Config(format!(
                    "failed to build OpenAI-compatible HTTP client: {e}"
                ))
            })?;

        Ok(Self {
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            models,
            cost_tier,
            output_dir: crate::artifacts::ensure_artifact_dir(output_dir),
            client,
        })
    }
}

#[derive(Debug, Serialize, Default)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

/// Ask the server to emit a final usage-only chunk during streaming (OpenAI
/// only reports token counts in a stream when this is set).
#[derive(Debug, Serialize, Default)]
struct StreamOptions {
    include_usage: bool,
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
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
}

/// One `chat.completion.chunk` object from an OpenAI-style SSE stream.
#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    #[serde(default)]
    choices: Vec<ChatChunkChoice>,
    /// Present only on the terminal usage chunk (requires `stream_options`).
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChunkChoice {
    #[serde(default)]
    delta: ChatDelta,
}

#[derive(Debug, Deserialize, Default)]
struct ChatDelta {
    #[serde(default)]
    content: Option<String>,
}

/// A parsed SSE event from the chat stream.
enum SseEvent {
    /// A `chat.completion.chunk` payload.
    Chunk(ChatCompletionChunk),
    /// The terminal `data: [DONE]` sentinel.
    Done,
}

impl OpenAiCompatProvider {
    /// Parse one SSE event block (the bytes preceding a blank-line delimiter).
    ///
    /// Joins the event's `data:` field lines with newlines per the SSE spec
    /// (a single leading space after `data:` is stripped), ignoring
    /// comments/`event:` lines and tolerating `\r\n` (`str::lines` drops the
    /// terminator). Blank blocks and unparseable payloads yield `None`.
    fn parse_sse_event(block: &[u8]) -> Option<SseEvent> {
        let text = std::str::from_utf8(block).ok()?;
        let mut data = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                // Per the spec a single space immediately after the colon is part
                // of the delimiter, not the value; any further whitespace is content.
                data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        if data.is_empty() {
            return None;
        }
        if data == "[DONE]" {
            return Some(SseEvent::Done);
        }
        serde_json::from_str::<ChatCompletionChunk>(&data)
            .ok()
            .map(SseEvent::Chunk)
    }

    /// Length (in bytes) of the first complete SSE event frame in `buf`,
    /// including its blank-line delimiter, or `None` if no complete frame is
    /// buffered yet. Recognizes both `\n\n` and the spec-permitted `\r\n\r\n`
    /// (as `\n` followed by `\r\n`), so a CRLF-emitting server or proxy streams
    /// correctly rather than buffering the whole body.
    fn sse_frame_len(buf: &[u8]) -> Option<usize> {
        for i in 0..buf.len() {
            if buf[i] != b'\n' {
                continue;
            }
            match buf.get(i + 1) {
                Some(b'\n') => return Some(i + 2),
                Some(b'\r') if buf.get(i + 2) == Some(&b'\n') => return Some(i + 3),
                _ => {}
            }
        }
        None
    }

    /// Collect the input strings to embed from a request: an explicit
    /// `params.input` (a string or an array of strings) takes precedence,
    /// otherwise every non-empty message content is embedded in order. Empty
    /// input is rejected by the caller.
    ///
    /// Shared by the Ollama and liter-llm backends so embedding requests resolve
    /// their inputs identically across providers.
    pub(crate) fn embedding_inputs(request: &InferenceRequest) -> Vec<String> {
        match request.params.get("input") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => return vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => {
                let inputs: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if !inputs.is_empty() {
                    return inputs;
                }
            }
            _ => {}
        }
        request
            .messages
            .iter()
            .map(|m| m.content.clone())
            .filter(|c| !c.trim().is_empty())
            .collect()
    }
}

#[derive(Debug, Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

/// OpenAI-style `/images/generations` request.
#[derive(Debug, Serialize)]
struct ImageGenRequest {
    model: String,
    prompt: String,
    n: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<String>,
    /// Optional and omitted by default: some gateways (e.g. Agnes) reject
    /// `response_format` with a 400, and when it is absent they return an image
    /// `url` — which we download into a managed artifact anyway. Set it only when
    /// the caller explicitly asks (`params.response_format`, e.g. `"b64_json"`)
    /// for a server known to honor it.
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageGenResponse {
    data: Vec<ImageGenData>,
}

#[derive(Debug, Deserialize)]
struct ImageGenData {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// OpenAI-style `/embeddings` request: one or more input strings embedded in a
/// single call (the API accepts a string or an array; we always send an array).
#[derive(Debug, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    #[serde(default)]
    data: Vec<EmbeddingData>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    #[serde(default)]
    embedding: Vec<f32>,
    /// Position in the batch; the API may return rows out of order, so we sort by
    /// it. `Option` (not a `#[serde(default)]` `0`) so we can tell a real index
    /// from an absent one — collapsing every absent index to `0` would turn the
    /// sort into a silent no-op and could mismatch rows to inputs.
    index: Option<u32>,
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
                card.id = ModelId::canonical(&self.name, &m.name);
                card.availability = ModelAvailability::Configured;
                card.residency = ModelResidency::Remote;
                card.context_window = m.context_window.unwrap_or(4096);
                card.memory_estimate_bytes = m.memory_mb.unwrap_or(0) * 1024 * 1024;
                card.execution.max_concurrency = 32;
                card.reasoning_tier = m
                    .reasoning_tier
                    .as_deref()
                    .and_then(ReasoningEffort::from_str_loose);
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
            Capability::ImageEdit => self.invoke_image_edit(model_name, request, start).await,
            Capability::Embedding => self.invoke_embedding(model_name, request, start).await,
            other => Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' does not support {other}",
                self.name
            ))),
        }
    }

    /// Real per-token streaming over OpenAI-style Server-Sent Events.
    ///
    /// Overrides the pseudo-streaming default: sends `stream: true`, reads the
    /// response body incrementally, and forwards each `choices[].delta.content`
    /// token through `sink` as it arrives. Only chat streams token-by-token;
    /// other capabilities fall back to a single-shot emit via [`invoke`].
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

        let model_name = ModelId::name(model_id);
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
            stream: Some(true),
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };

        let url = format!("{}/chat/completions", self.base_url);
        let start = std::time::Instant::now();
        let mut resp = self
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

        // OpenAI SSE: events are delimited by a blank line (`\n\n`). Buffer bytes,
        // emit each token's `delta.content` as its event completes, and capture the
        // token counts from the terminal usage-only chunk.
        let mut buf: Vec<u8> = Vec::new();
        let mut full = String::new();
        let mut prompt_tokens = None;
        let mut completion_tokens = None;
        let mut total_tokens = None;

        let mut apply = |event: SseEvent, full: &mut String| -> Option<String> {
            let SseEvent::Chunk(chunk) = event else {
                return None;
            };
            if let Some(u) = chunk.usage {
                prompt_tokens = u.prompt_tokens;
                completion_tokens = u.completion_tokens;
                total_tokens = u.total_tokens;
            }
            let content = chunk.choices.into_iter().next()?.delta.content?;
            if content.is_empty() {
                return None;
            }
            full.push_str(&content);
            Some(content)
        };

        'read: while let Some(bytes) =
            resp.chunk().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("stream read error: {e}"),
            })?
        {
            buf.extend_from_slice(&bytes);
            while let Some(frame_len) = Self::sse_frame_len(&buf) {
                let block: Vec<u8> = buf.drain(..frame_len).collect();
                if let Some(event) = Self::parse_sse_event(&block)
                    && let Some(delta) = apply(event, &mut full)
                    && sink.send(StreamDelta::Text(delta)).await.is_err()
                {
                    // Receiver dropped (client disconnected): stop draining upstream.
                    break 'read;
                }
            }
        }
        // Any trailing event without a final blank line.
        if let Some(event) = Self::parse_sse_event(&buf)
            && let Some(delta) = apply(event, &mut full)
        {
            let _ = sink.send(StreamDelta::Text(delta)).await;
        }

        let tokens_used = total_tokens.or(match (prompt_tokens, completion_tokens) {
            (None, None) => None,
            (p, c) => Some(p.unwrap_or(0) + c.unwrap_or(0)),
        });
        Ok(InferenceResponse {
            text: Some(full),
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used,
            prompt_tokens,
            completion_tokens,
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }
}

impl OpenAiCompatProvider {
    /// Fallback health probe for gateways whose `GET /models` returns 404: send a
    /// minimal (`max_tokens: 1`) chat completion and treat a success as healthy.
    ///
    /// Note: this issues a *real, billable* request. It runs only when the cheap
    /// `/models` probe is unavailable (404), and only at startup and on explicit
    /// `discovery/refresh`, so the cost is a single 1-token completion per probe
    /// rather than per inference — an intentional trade to detect reachability for
    /// providers that do not expose a free model-list endpoint.
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
            }],
            max_tokens: Some(1),
            ..Default::default()
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
            ..Default::default()
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

        let (tokens, prompt_tokens, completion_tokens) = match chat.usage {
            Some(u) => (u.total_tokens, u.prompt_tokens, u.completion_tokens),
            None => (None, None, None),
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResponse {
            text,
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: tokens,
            prompt_tokens,
            completion_tokens,
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
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
            ..Default::default()
        })
    }

    /// Generate an image via the OpenAI-compatible `/images/generations` endpoint
    /// and store it as a managed artifact.
    ///
    /// We request `b64_json` so we can always write a local file; when a gateway
    /// ignores that and returns a `url` instead, we download the bytes ourselves
    /// so the caller uniformly gets a `file` path (never a bare remote URL it
    /// would have to fetch separately).
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
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| {
                EngineError::InvalidRequest("image generation requires a prompt".into())
            })?;

        let size = request
            .params
            .get("size")
            .and_then(|v| v.as_str())
            .map(String::from);

        let response_format = request
            .params
            .get("response_format")
            .and_then(|v| v.as_str())
            .map(String::from);

        let body = ImageGenRequest {
            model: model_name.to_string(),
            prompt,
            n: 1,
            size,
            response_format,
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
                detail: format!("image_gen HTTP {status}: {text}"),
            });
        }

        let parsed: ImageGenResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("image_gen decode error: {e}"),
            })?;
        let image = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image_gen returned no data".into(),
            })?;

        let bytes = self.image_bytes_from_response(image).await?;

        let path = self
            .output_dir
            .join(format!("mofa_image_{}.png", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("image write error: {e}")))?;

        Ok(InferenceResponse {
            text: None,
            file: Some(path.to_string_lossy().to_string()),
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

    /// Resolve one provider image result to bytes: decode inline base64, or
    /// download the returned URL. Shared by generation and editing.
    async fn image_bytes_from_response(&self, image: ImageGenData) -> Result<Vec<u8>, EngineError> {
        if let Some(b64) = image.b64_json {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("image base64 decode error: {e}"),
                })
        } else if let Some(remote) = image.url {
            let dl =
                self.client
                    .get(&remote)
                    .send()
                    .await
                    .map_err(|e| EngineError::ProviderError {
                        provider: self.name.clone(),
                        detail: format!("image download error: {e}"),
                    })?;
            dl.bytes()
                .await
                .map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("image download read error: {e}"),
                })
                .map(|b| b.to_vec())
        } else {
            Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image result had neither b64_json nor url".into(),
            })
        }
    }

    /// Fetch an image reference (an HTTP(S) URL, `data:` URL, or local path)
    /// to raw bytes, plus its MIME type when derivable.
    async fn fetch_image_ref(
        &self,
        reference: &str,
        role: &str,
    ) -> Result<(Vec<u8>, String), EngineError> {
        if let Some((mime, is_base64, payload)) = parse_data_url(reference) {
            if !is_base64 {
                return Err(EngineError::InvalidRequest(format!(
                    "{role} data: URL must be base64-encoded"
                )));
            }
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload.as_bytes())
                .map_err(|e| {
                    EngineError::InvalidRequest(format!(
                        "{role} data: URL payload is not valid base64: {e}"
                    ))
                })?;
            Ok((bytes, mime))
        } else if reference.starts_with("http://") || reference.starts_with("https://") {
            let resp = self.client.get(reference).send().await.map_err(|e| {
                EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("{role} download error: {e}"),
                }
            })?;
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .unwrap_or_else(|| "image/png".into());
            let bytes = resp.bytes().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("{role} download read error: {e}"),
            })?;
            Ok((bytes.to_vec(), mime))
        } else {
            // Local file path on the engine host.
            let bytes = tokio::fs::read(reference).await.map_err(|e| {
                EngineError::InvalidRequest(format!("cannot read {role} file '{reference}': {e}"))
            })?;
            let mime = match std::path::Path::new(reference)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref()
            {
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("webp") => "image/webp",
                Some("gif") => "image/gif",
                _ => "image/png",
            }
            .to_string();
            Ok((bytes, mime))
        }
    }

    /// Edit an image via the OpenAI-compatible `/images/edits` endpoint:
    /// whole-image (I2I) when no mask is supplied, or inpainting restricted
    /// to the mask's transparent areas. The input image rides on
    /// `messages[0].images[0]`; the mask on `request.input_mask`.
    async fn invoke_image_edit(
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
            .filter(|p| !p.trim().is_empty())
            .ok_or_else(|| EngineError::InvalidRequest("image editing requires a prompt".into()))?;

        let image_ref = request
            .messages
            .first()
            .and_then(|m| m.images.first())
            .ok_or_else(|| {
                EngineError::InvalidRequest(
                    "image editing requires an input image (messages[0].images)".into(),
                )
            })?;
        let (image_bytes, image_mime) = self.fetch_image_ref(image_ref, "input image").await?;

        let mut form = reqwest::multipart::Form::new()
            .text("model", model_name.to_string())
            .text("prompt", prompt)
            .text("n", "1")
            .part(
                "image",
                reqwest::multipart::Part::bytes(image_bytes)
                    .file_name("input.png")
                    .mime_str(&image_mime)
                    .map_err(|e| EngineError::Internal(format!("mime error: {e}")))?,
            );
        if let Some(mask_ref) = request.input_mask.as_deref() {
            let (mask_bytes, _) = self.fetch_image_ref(mask_ref, "mask").await?;
            form = form.part(
                "mask",
                reqwest::multipart::Part::bytes(mask_bytes)
                    .file_name("mask.png")
                    .mime_str("image/png")
                    .map_err(|e| EngineError::Internal(format!("mime error: {e}")))?,
            );
        }
        if let Some(size) = request.params.get("size").and_then(|v| v.as_str()) {
            form = form.text("size", size.to_string());
        }

        let url = format!("{}/images/edits", self.base_url);
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
                detail: format!("image_edit HTTP {status}: {text}"),
            });
        }

        let parsed: ImageGenResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("image_edit decode error: {e}"),
            })?;
        let image = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image_edit returned no data".into(),
            })?;
        let bytes = self.image_bytes_from_response(image).await?;

        let path = self
            .output_dir
            .join(format!("mofa_image_edit_{}.png", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| EngineError::Internal(format!("image write error: {e}")))?;

        Ok(InferenceResponse {
            text: None,
            file: Some(path.to_string_lossy().to_string()),
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
            ..Default::default()
        })
    }

    async fn invoke_embedding(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: std::time::Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let inputs = Self::embedding_inputs(request);
        if inputs.is_empty() {
            return Err(EngineError::InvalidRequest(
                "embedding requires text input (params.input or messages)".into(),
            ));
        }

        let body = EmbeddingRequest {
            model: model_name.to_string(),
            input: inputs,
        };

        let url = format!("{}/embeddings", self.base_url);
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
                detail: format!("embedding HTTP {status}: {text}"),
            });
        }

        let mut parsed: EmbeddingResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("embedding parse error: {e}"),
            })?;

        // The API may return rows out of batch order; restore input order — but
        // only when every row actually carries an index. If any row omits it we
        // can't reorder reliably, so we trust the provider's returned order
        // (OpenAI-compatible servers return rows in input order) rather than
        // reordering on missing data.
        if parsed.data.iter().all(|d| d.index.is_some()) {
            parsed.data.sort_by_key(|d| d.index);
        }
        let vectors: Vec<Vec<f32>> = parsed.data.into_iter().map(|d| d.embedding).collect();
        if vectors.is_empty() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "embedding response contained no vectors".into(),
            });
        }

        let (prompt_tokens, tokens_used) = match parsed.usage {
            // Embeddings bill input tokens only, so total == prompt.
            Some(u) => (u.prompt_tokens, u.total_tokens.or(u.prompt_tokens)),
            None => (None, None),
        };
        Ok(InferenceResponse {
            text: None,
            file: None,
            embedding: Some(vectors),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used,
            prompt_tokens,
            completion_tokens: Some(0),
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }
}

/// Split a `data:` URL into `(mime, is_base64, payload)`; `None` when the
/// reference is not a data URL. No percent-decoding: non-base64 payloads are
/// rejected by the caller (every image producer we support emits base64).
fn parse_data_url(reference: &str) -> Option<(String, bool, &str)> {
    let rest = reference.strip_prefix("data:")?;
    let (header, payload) = rest.split_once(',')?;
    let (mime, is_base64) = match header.split_once(';') {
        Some((m, tail)) => (m.to_string(), tail.eq_ignore_ascii_case("base64")),
        None => (header.to_string(), false),
    };
    let mime = if mime.is_empty() {
        "image/png".to_string()
    } else {
        mime
    };
    Some((mime, is_base64, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    ..Default::default()
                },
                ModelDef {
                    name: "model-b".into(),
                    capability: "tts".into(),
                    context_window: None,
                    memory_mb: Some(512),
                    ..Default::default()
                },
            ],
            CostTier::Medium,
        )
        .unwrap();

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
        let p = OpenAiCompatProvider::new("x", "https://example.com", "key", vec![], CostTier::Low)
            .unwrap();
        assert_eq!(p.kind(), ProviderKind::OpenAiCompatible);
        assert_eq!(p.name(), "x");
    }

    #[test]
    fn sse_events_accumulate_text_and_usage() {
        // A realistic OpenAI `stream: true` sequence: two content deltas, an empty
        // delta, a terminal usage-only chunk, and the `[DONE]` sentinel.
        let blocks: Vec<&[u8]> = vec![
            br#"data: {"choices":[{"delta":{"role":"assistant","content":"He"}}]}"#,
            br#"data: {"choices":[{"delta":{"content":"llo"}}]}"#,
            br#"data: {"choices":[{"delta":{"content":""}}]}"#,
            br#"data: {"choices":[{"delta":{}}],"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
            b"data: [DONE]",
        ];

        let mut full = String::new();
        let mut deltas = Vec::new();
        let mut prompt = None;
        let mut completion = None;
        let mut total = None;
        for block in blocks {
            match OpenAiCompatProvider::parse_sse_event(block) {
                Some(SseEvent::Chunk(chunk)) => {
                    if let Some(u) = chunk.usage {
                        prompt = u.prompt_tokens;
                        completion = u.completion_tokens;
                        total = u.total_tokens;
                    }
                    if let Some(content) = chunk
                        .choices
                        .into_iter()
                        .next()
                        .and_then(|c| c.delta.content)
                        && !content.is_empty()
                    {
                        full.push_str(&content);
                        deltas.push(content);
                    }
                }
                Some(SseEvent::Done) | None => {}
            }
        }

        assert_eq!(deltas, vec!["He", "llo"]);
        assert_eq!(full, "Hello");
        assert_eq!(prompt, Some(5));
        assert_eq!(completion, Some(3));
        assert_eq!(total, Some(8));
    }

    #[test]
    fn parse_sse_event_handles_done_blank_and_garbage() {
        assert!(matches!(
            OpenAiCompatProvider::parse_sse_event(b"data: [DONE]"),
            Some(SseEvent::Done)
        ));
        assert!(OpenAiCompatProvider::parse_sse_event(b"").is_none());
        assert!(OpenAiCompatProvider::parse_sse_event(b": keep-alive comment").is_none());
        assert!(OpenAiCompatProvider::parse_sse_event(b"data: not json").is_none());
    }

    #[test]
    fn sse_frame_len_detects_lf_and_crlf_delimiters() {
        // `\n\n` frame: `"data: a\n\n"` is 9 bytes (the drained frame incl. both
        // newlines), leaving `"rest"`.
        assert_eq!(
            OpenAiCompatProvider::sse_frame_len(b"data: a\n\nrest"),
            Some(9)
        );
        // `\r\n\r\n` frame (CRLF server/proxy): `"data: a\r\n\r\n"` is 11 bytes.
        assert_eq!(
            OpenAiCompatProvider::sse_frame_len(b"data: a\r\n\r\nrest"),
            Some(11)
        );
        // A lone trailing `\n` is not a complete frame yet.
        assert_eq!(OpenAiCompatProvider::sse_frame_len(b"data: a\n"), None);
    }

    #[test]
    fn embedding_inputs_prefers_params_then_messages() {
        use mofa_kernel::Message;

        // An explicit `params.input` array wins over messages.
        let mut req = InferenceRequest {
            capability: Some(Capability::Embedding),
            messages: vec![Message {
                role: "user".into(),
                content: "from message".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        req.params = serde_json::json!({ "input": ["a", "b", ""] });
        assert_eq!(OpenAiCompatProvider::embedding_inputs(&req), vec!["a", "b"]); // empties dropped

        // A single-string `params.input` embeds one row.
        req.params = serde_json::json!({ "input": "solo" });
        assert_eq!(OpenAiCompatProvider::embedding_inputs(&req), vec!["solo"]);

        // With no `params.input`, non-empty message contents are embedded in order.
        req.params = serde_json::Value::Null;
        assert_eq!(
            OpenAiCompatProvider::embedding_inputs(&req),
            vec!["from message"]
        );

        // Nothing to embed → empty (the caller turns this into InvalidRequest).
        let empty = InferenceRequest {
            capability: Some(Capability::Embedding),
            ..Default::default()
        };
        assert!(OpenAiCompatProvider::embedding_inputs(&empty).is_empty());
    }

    #[test]
    fn parse_sse_event_joins_multiple_data_lines() {
        // Two `data:` lines are joined with a newline, yielding valid JSON.
        let block = b"data: {\"choices\":[{\"delta\":\ndata: {\"content\":\"hi\"}}]}";
        let Some(SseEvent::Chunk(chunk)) = OpenAiCompatProvider::parse_sse_event(block) else {
            panic!("expected a parsed chunk from joined data lines");
        };
        assert_eq!(
            chunk
                .choices
                .into_iter()
                .next()
                .and_then(|c| c.delta.content),
            Some("hi".to_string())
        );
    }

    #[test]
    fn parse_data_url_splits_header_and_payload() {
        let (mime, is_base64, payload) = parse_data_url("data:image/png;base64,aGVsbG8=").unwrap();
        assert_eq!(mime, "image/png");
        assert!(is_base64);
        assert_eq!(payload, "aGVsbG8=");

        // No mime → image/png default; no `;base64` marker → not base64.
        let (mime, is_base64, payload) = parse_data_url("data:,abc").unwrap();
        assert_eq!(mime, "image/png");
        assert!(!is_base64);
        assert_eq!(payload, "abc");

        assert!(parse_data_url("https://example.com/x.png").is_none());
        assert!(parse_data_url("data:nocomma").is_none());
    }

    #[tokio::test]
    async fn image_edit_sends_multipart_and_writes_artifact() {
        use base64::Engine as _;
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);

        let image_data_url = format!("data:image/png;base64,{}", b64(b"fake-input-png"));
        let mask_data_url = format!("data:image/png;base64,{}", b64(b"fake-mask-png"));
        let edited = b"edited-png-bytes".to_vec();
        let edited_b64 = b64(&edited);

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let seen_handler = seen.clone();
        let app = axum::Router::new().route(
            "/images/edits",
            axum::routing::post(move |body: axum::body::Bytes| {
                let seen = seen_handler.clone();
                async move {
                    *seen.lock().unwrap() = body.to_vec();
                    axum::Json(serde_json::json!({ "data": [{ "b64_json": edited_b64 }] }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let provider = OpenAiCompatProvider::new(
            "edit-mock",
            format!("http://{addr}"),
            "sk-test",
            vec![ModelDef {
                name: "edit-model".into(),
                capability: "image_edit".into(),
                context_window: None,
                memory_mb: None,
                ..Default::default()
            }],
            CostTier::Medium,
        )
        .unwrap();

        let request = InferenceRequest {
            capability: Some(Capability::ImageEdit),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "把天空换成夜景".into(),
                images: vec![image_data_url],
            }],
            input_mask: Some(mask_data_url),
            params: serde_json::json!({ "size": "1024x1024" }),
            ..Default::default()
        };
        let resp = provider
            .invoke("edit-mock/edit-model", &request)
            .await
            .unwrap();
        assert_eq!(resp.model_used, "edit-model");
        assert_eq!(resp.provider, "edit-mock");

        let body = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
        assert!(
            body.contains("name=\"image\""),
            "image part missing: {body}"
        );
        assert!(body.contains("filename=\"input.png\""));
        assert!(body.contains("name=\"mask\""), "mask part missing: {body}");
        assert!(body.contains("filename=\"mask.png\""));
        assert!(body.contains("name=\"prompt\""));
        assert!(body.contains("把天空换成夜景"));
        assert!(body.contains("name=\"size\""));
        assert!(body.contains("1024x1024"));
        // The referenced image and mask bytes ride inline in the multipart body.
        assert!(body.contains("fake-input-png"));
        assert!(body.contains("fake-mask-png"));

        let path = resp.file.expect("artifact path");
        assert_eq!(std::fs::read(&path).unwrap(), edited);
    }

    #[tokio::test]
    async fn image_edit_without_mask_edits_whole_image() {
        use base64::Engine as _;
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);

        let image_data_url = format!("data:image/jpeg;base64,{}", b64(b"jpeg-input"));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let seen_handler = seen.clone();
        let app = axum::Router::new().route(
            "/images/edits",
            axum::routing::post(move |body: axum::body::Bytes| {
                let seen = seen_handler.clone();
                async move {
                    *seen.lock().unwrap() = body.to_vec();
                    axum::Json(serde_json::json!({ "data": [{ "b64_json": b64(b"i2i-out") }] }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let provider = OpenAiCompatProvider::new(
            "i2i-mock",
            format!("http://{addr}"),
            "sk-test",
            vec![ModelDef {
                name: "edit-model".into(),
                capability: "image_edit".into(),
                ..Default::default()
            }],
            CostTier::Medium,
        )
        .unwrap();

        let request = InferenceRequest {
            capability: Some(Capability::ImageEdit),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "整体改成水彩风格".into(),
                images: vec![image_data_url],
            }],
            ..Default::default()
        };
        let resp = provider
            .invoke("i2i-mock/edit-model", &request)
            .await
            .unwrap();

        let body = String::from_utf8_lossy(&seen.lock().unwrap().clone()).into_owned();
        // Whole-image edit: an image part with the reference's JPEG mime, and no mask.
        assert!(body.contains("name=\"image\""));
        assert!(body.contains("image/jpeg"));
        assert!(
            !body.contains("name=\"mask\""),
            "unexpected mask part: {body}"
        );
        assert_eq!(
            std::fs::read(resp.file.expect("artifact path")).unwrap(),
            b"i2i-out"
        );
    }

    #[tokio::test]
    async fn image_edit_requires_an_input_image() {
        let provider = OpenAiCompatProvider::new(
            "x",
            "https://example.com",
            "k",
            vec![ModelDef {
                name: "m".into(),
                capability: "image_edit".into(),
                ..Default::default()
            }],
            CostTier::Low,
        )
        .unwrap();
        let request = InferenceRequest {
            capability: Some(Capability::ImageEdit),
            messages: vec![mofa_kernel::Message {
                role: "user".into(),
                content: "no image attached".into(),
                images: vec![],
            }],
            ..Default::default()
        };
        let err = provider.invoke("x/m", &request).await.unwrap_err();
        assert!(err.to_string().contains("input image"), "got: {err}");
    }
}
