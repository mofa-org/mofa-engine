//! Ollama provider backend.
//!
//! Communicates with a local Ollama instance via its HTTP API.

use async_trait::async_trait;
use base64::Engine as _;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind, ReasoningEffort, StreamDelta, StreamSink,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::config::ModelDef;

/// Provider for a local Ollama instance.
pub(crate) struct OllamaProvider {
    /// Display name.
    name: String,
    /// Base URL.
    base_url: String,
    /// HTTP client.
    client: Client,
    /// Config-supplied reasoning-tier overrides, keyed by Ollama model name
    /// (e.g. `"deepseek-r1:8b"`). Ollama auto-discovers its models, so these are
    /// applied as annotations on the discovered card rather than a replacement
    /// enumeration — they let a *local* reasoning model participate in
    /// `reasoning.effort` → tier routing (S2), which cloud-configured backends
    /// already get from their explicit model list.
    reasoning_tiers: HashMap<String, ReasoningEffort>,
}

impl OllamaProvider {
    /// Create a new Ollama provider.
    ///
    /// Fails (rather than panicking or silently dropping the configured
    /// timeouts/`no_proxy`) if the system TLS/HTTP stack cannot build a client,
    /// so the engine surfaces a clean startup error instead of crashing.
    ///
    /// `models` are optional per-model annotations; only entries carrying a
    /// `reasoning_tier` are retained (as tier-routing overrides). All other model
    /// metadata is auto-discovered from Ollama, so the list may be empty.
    pub(crate) fn new(
        name: impl Into<String>,
        base_url: impl Into<String>,
        models: &[ModelDef],
    ) -> Result<Self, EngineError> {
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| EngineError::Config(format!("failed to build Ollama HTTP client: {e}")))?;

        let reasoning_tiers = models
            .iter()
            .filter_map(|m| {
                let tier = m.reasoning_tier.as_deref()?;
                let tier = ReasoningEffort::from_str_loose(tier)?;
                Some((m.name.clone(), tier))
            })
            .collect();

        Ok(Self {
            name: name.into(),
            base_url: base_url.into(),
            client,
            reasoning_tiers,
        })
    }

    async fn loaded_models(&self) -> HashSet<String> {
        let url = format!("{}/api/ps", self.base_url);
        let Ok(resp) = self.client.get(&url).send().await else {
            return HashSet::new();
        };
        let Ok(ps) = resp.json::<OllamaPsResponse>().await else {
            return HashSet::new();
        };
        ps.models
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| m.name.or(m.model))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Option<Vec<OllamaModel>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaPsResponse {
    models: Option<Vec<OllamaModel>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaModel {
    name: Option<String>,
    model: Option<String>,
    size: Option<u64>,
    #[serde(default)]
    details: OllamaModelDetails,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
struct OllamaModelDetails {
    family: Option<String>,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_alive: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Default, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    /// Base64-encoded image data for multimodal (vision) models such as `llava`.
    /// Ollama expects raw base64 (no `data:` prefix); empty for text-only turns,
    /// in which case the field is omitted entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaChatResponse {
    message: Option<OllamaMessage>,
    /// Completion/output tokens.
    #[serde(default)]
    eval_count: Option<u32>,
    /// Prompt/input tokens.
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    total_duration: Option<u64>,
}

/// Request to Ollama's `/api/embed` endpoint (batch-capable).
#[derive(Debug, Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

/// Response from Ollama's `/api/embed` endpoint.
#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    /// One vector per input, in input order.
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
    /// Prompt/input tokens, when reported.
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

/// One NDJSON object from Ollama's `stream: true` response.
#[derive(Debug, Deserialize)]
struct OllamaStreamChunk {
    /// Incremental message with the next token(s) of content.
    message: Option<OllamaMessage>,
    /// Set on the final object.
    #[serde(default)]
    done: bool,
    /// Completion/output tokens (final object only).
    #[serde(default)]
    eval_count: Option<u32>,
    /// Prompt/input tokens (final object only).
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

impl OllamaProvider {
    /// Parse one NDJSON line from an Ollama stream. Blank/whitespace lines and
    /// unparseable fragments yield `None`.
    fn parse_stream_line(line: &[u8]) -> Option<OllamaStreamChunk> {
        if line.iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        serde_json::from_slice(line).ok()
    }

    /// Convert MoFA conversation messages to Ollama's wire form, resolving any
    /// attached image references to the raw base64 a vision model (e.g. `llava`)
    /// expects. A message with no images serializes as a plain text turn.
    ///
    /// Note: Ollama models are discovered as [`Capability::Chat`], so the router
    /// only sends chat requests here; attaching images lets a locally-pulled
    /// vision model still answer an image-bearing chat turn on-device.
    async fn to_ollama_messages(&self, request: &InferenceRequest) -> Vec<OllamaMessage> {
        let mut out = Vec::with_capacity(request.messages.len());
        for m in &request.messages {
            let mut images = Vec::with_capacity(m.images.len());
            for image in &m.images {
                if let Some(b64) = Self::resolve_image_b64(image).await {
                    images.push(b64);
                }
            }
            out.push(OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                images,
            });
        }
        out
    }

    /// Embed one or more inputs via Ollama's `/api/embed`. Inputs are resolved
    /// the same way as the cloud backends (`params.input` string/array, else the
    /// message contents), so embedding requests are portable across providers.
    async fn embed(
        &self,
        model_name: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let inputs =
            crate::backends::openai_compat::OpenAiCompatProvider::embedding_inputs(request);
        if inputs.is_empty() {
            return Err(EngineError::InvalidRequest(
                "embedding requires text input (params.input or messages)".into(),
            ));
        }

        let body = OllamaEmbedRequest {
            model: model_name.to_string(),
            input: inputs,
        };
        let url = format!("{}/api/embed", self.base_url);
        let start = std::time::Instant::now();
        let resp = self
            .client
            .post(&url)
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
                detail: format!("embed HTTP {status}: {text}"),
            });
        }

        let embed: OllamaEmbedResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("embed parse error: {e}"),
            })?;
        if embed.embeddings.is_empty() {
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "embed response contained no vectors".into(),
            });
        }

        Ok(InferenceResponse {
            text: None,
            file: None,
            embedding: Some(embed.embeddings),
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            request_id: request.request_id.clone(),
            tokens_used: embed.prompt_eval_count,
            prompt_tokens: embed.prompt_eval_count,
            completion_tokens: Some(0),
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }

    /// Resolve one image reference to the raw base64 payload Ollama accepts.
    /// A `data:` URL is stripped to its payload; a local file is read and
    /// encoded; an `http(s)` URL is skipped (Ollama cannot fetch remote images)
    /// with a warning. Unreadable references are skipped rather than failing the
    /// whole request.
    async fn resolve_image_b64(image: &str) -> Option<String> {
        if image.starts_with("data:") {
            // `data:<mime>;base64,<payload>` → keep only the base64 payload.
            return image
                .split_once(',')
                .map(|(_, payload)| payload.to_string());
        }
        if image.starts_with("http://") || image.starts_with("https://") {
            tracing::warn!("Ollama cannot fetch remote image URL '{image}'; skipping");
            return None;
        }
        match tokio::fs::read(image).await {
            Ok(bytes) => Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            Err(e) => {
                tracing::warn!("cannot read local image '{image}': {e}; skipping");
                None
            }
        }
    }

    /// Extract or default Ollama options (num_ctx, num_predict, temperature)
    /// to support long documents and full meeting transcripts up to 32K tokens.
    fn resolve_options(request: &InferenceRequest) -> Option<OllamaOptions> {
        let num_ctx = request
            .params
            .get("num_ctx")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .or(Some(32768));

        let num_predict = request
            .params
            .get("max_tokens")
            .or_else(|| request.params.get("num_predict"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .or(Some(4096));

        let temperature = request
            .params
            .get("temperature")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32);

        Some(OllamaOptions {
            num_ctx,
            num_predict,
            temperature,
        })
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    fn features(&self) -> Vec<BackendFeature> {
        vec![
            BackendFeature::Discovery,
            BackendFeature::Load,
            BackendFeature::Unload,
            BackendFeature::ResidencyInspection,
            BackendFeature::MemoryReporting,
        ]
    }

    async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("discover failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("discover HTTP {status}"),
            });
        }

        let tags: OllamaTagsResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("tags parse failed: {e}"),
            })?;
        let loaded = self.loaded_models().await;

        let cards = tags
            .models
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| {
                let model_name = m.name.or(m.model)?;
                let lower = model_name.to_lowercase();
                // Ollama's cloud-hosted tags are not local models; skip them.
                if lower.contains(":cloud") || lower.contains("-cloud") {
                    return None;
                }

                // The tags endpoint reports no modality, so we infer capability
                // from the name: models named `*embed*` serve `Embedding`
                // (e.g. `nomic-embed-text`, `mxbai-embed-large`) and everything
                // else is typed `Chat`.
                //
                // TODO: this same lack of modality metadata means a locally-pulled
                // *vision* model is still typed `Chat`, so it won't be routed for an
                // explicit `Vlm` request (though it can answer an image-bearing chat
                // turn — see `to_ollama_messages`). Family-based modality inference
                // is deferred.
                let capability = if lower.contains("embed") {
                    Capability::Embedding
                } else if lower.contains("llava")
                    || lower.contains("qwen2-vl")
                    || lower.contains("bakllava")
                    || lower.contains("moondream")
                    || lower.contains("vision")
                {
                    Capability::Vlm
                } else if lower.contains("flux")
                    || lower.contains("diffusion")
                    || lower.contains("sdxl")
                    || lower.contains("image")
                {
                    Capability::ImageGen
                } else {
                    Capability::Chat
                };
                let mut card = ModelCard::new(
                    self.name.clone(),
                    model_name.clone(),
                    capability,
                    CostTier::Free,
                );
                if capability == Capability::Vlm {
                    card.capabilities = vec![Capability::Vlm, Capability::Chat];
                }
                card.id = ModelId::canonical(&self.name, &model_name);
                card.availability = ModelAvailability::Discovered;
                card.residency = if loaded.contains(&model_name) {
                    ModelResidency::Loaded
                } else {
                    ModelResidency::Unloaded
                };
                card.context_window = 4096;
                card.memory_estimate_bytes = m.size.unwrap_or(0);
                // Config-supplied tier override lets this local model take part in
                // `reasoning.effort` → tier routing (S2).
                card.reasoning_tier = self.reasoning_tiers.get(&model_name).copied();
                card.refresh_status();
                Some(card)
            })
            .collect();

        Ok(cards)
    }

    async fn health(&self) -> Result<BackendHealth, EngineError> {
        let url = format!("{}/", self.base_url);
        match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => Ok(BackendHealth::Healthy),
            Ok(r) if r.status().is_server_error() => Ok(BackendHealth::Degraded),
            Ok(_) => Ok(BackendHealth::Unavailable),
            Err(e) => Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: e.to_string(),
            }),
        }
    }

    async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        let model_name = ModelId::name(model_id);
        let lower = model_name.to_lowercase();

        // Non-chat models (e.g. diffusion / flux) in Ollama do not support `/api/chat`.
        if lower.contains("flux") || lower.contains("diffusion") || lower.contains("sdxl") {
            return Ok(LifecycleResult {
                model_id: ModelId::canonical(&self.name, model_name),
                residency: ModelResidency::Loaded,
                memory_bytes: None,
                changed: false,
            });
        }

        if lower.contains("embed") {
            let body = serde_json::json!({
                "model": model_name,
                "input": " ",
                "keep_alive": -1,
            });
            let url = format!("{}/api/embed", self.base_url);
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("load embed failed: {e}"),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("load embed HTTP {status}: {text}"),
                });
            }
            return Ok(LifecycleResult {
                model_id: ModelId::canonical(&self.name, model_name),
                residency: ModelResidency::Loaded,
                memory_bytes: None,
                changed: true,
            });
        }

        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: " ".into(),
                images: Vec::new(),
            }],
            stream: false,
            keep_alive: Some(serde_json::json!(-1)),
            options: None,
        };
        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("load failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("load HTTP {status}: {text}"),
            });
        }

        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, model_name),
            residency: ModelResidency::Loaded,
            memory_bytes: None,
            changed: true,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        let model_name = ModelId::name(model_id);
        let lower = model_name.to_lowercase();

        if lower.contains("flux") || lower.contains("diffusion") || lower.contains("sdxl") {
            return Ok(LifecycleResult {
                model_id: ModelId::canonical(&self.name, model_name),
                residency: ModelResidency::Unloaded,
                memory_bytes: None,
                changed: false,
            });
        }

        if lower.contains("embed") {
            let body = serde_json::json!({
                "model": model_name,
                "input": " ",
                "keep_alive": 0,
            });
            let url = format!("{}/api/embed", self.base_url);
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("unload embed failed: {e}"),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: format!("unload embed HTTP {status}: {text}"),
                });
            }
            return Ok(LifecycleResult {
                model_id: ModelId::canonical(&self.name, model_name),
                residency: ModelResidency::Unloaded,
                memory_bytes: None,
                changed: true,
            });
        }

        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: " ".into(),
                images: Vec::new(),
            }],
            stream: false,
            keep_alive: Some(serde_json::json!(0)),
            options: None,
        };

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("unload failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("unload HTTP {status}: {text}"),
            });
        }

        Ok(LifecycleResult {
            model_id: ModelId::canonical(&self.name, model_name),
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

        // Embedding is a distinct endpoint (`/api/embed`), so dispatch it before
        // building a chat body.
        if request.capability == Some(Capability::Embedding) {
            return self.embed(model_name, request).await;
        }

        let messages = self.to_ollama_messages(request).await;
        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
        }

        let options = Self::resolve_options(request);
        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages,
            stream: false,
            keep_alive: None,
            options,
        };

        let url = format!("{}/api/chat", self.base_url);
        let start = std::time::Instant::now();

        let resp = self
            .client
            .post(&url)
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

        let chat_resp: OllamaChatResponse =
            resp.json().await.map_err(|e| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: format!("response parse error: {e}"),
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let text = chat_resp.message.map(|m| m.content).unwrap_or_default();
        let prompt_tokens = chat_resp.prompt_eval_count;
        let completion_tokens = chat_resp.eval_count;
        let tokens_used = match (prompt_tokens, completion_tokens) {
            (None, None) => None,
            (p, c) => Some(p.unwrap_or(0) + c.unwrap_or(0)),
        };

        Ok(InferenceResponse {
            text: Some(text),
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used,
            prompt_tokens,
            completion_tokens,
            fallback_used: false,
            routing_reason: None,
            ..Default::default()
        })
    }

    async fn stream(
        &self,
        model_id: &str,
        request: &InferenceRequest,
        sink: StreamSink,
    ) -> Result<InferenceResponse, EngineError> {
        let model_name = ModelId::name(model_id);

        // Embedding is a distinct, non-incremental endpoint (`/api/embed`): there
        // are no token deltas to stream, so fall back to a single-shot invoke and
        // return its vectors — mirroring the cloud backends, whose `stream` also
        // delegates non-chat capabilities to `invoke`.
        if request.capability == Some(Capability::Embedding) {
            return self.embed(model_name, request).await;
        }

        let messages = self.to_ollama_messages(request).await;
        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
        }

        let options = Self::resolve_options(request);
        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages,
            stream: true,
            keep_alive: None,
            options,
        };
        let url = format!("{}/api/chat", self.base_url);
        let start = Instant::now();

        let mut resp = self
            .client
            .post(&url)
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

        // Ollama streams newline-delimited JSON. Buffer bytes and emit each
        // token's content as it completes a line, accumulating the full text.
        let mut buf: Vec<u8> = Vec::new();
        let mut full = String::new();
        let mut prompt_tokens = None;
        let mut completion_tokens = None;

        let mut apply = |chunk: OllamaStreamChunk, full: &mut String| -> Option<String> {
            if chunk.done {
                prompt_tokens = chunk.prompt_eval_count;
                completion_tokens = chunk.eval_count;
            }
            let content = chunk.message.map(|m| m.content)?;
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
            while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                if let Some(chunk) = Self::parse_stream_line(&line)
                    && let Some(delta) = apply(chunk, &mut full)
                    && sink.send(StreamDelta::Text(delta)).await.is_err()
                {
                    // The receiver was dropped (client disconnected): stop draining
                    // the upstream generation rather than paying for tokens no one
                    // will read.
                    break 'read;
                }
            }
        }
        // Any trailing bytes without a final newline.
        if let Some(chunk) = Self::parse_stream_line(&buf)
            && let Some(delta) = apply(chunk, &mut full)
        {
            let _ = sink.send(StreamDelta::Text(delta)).await;
        }

        let tokens_used = match (prompt_tokens, completion_tokens) {
            (None, None) => None,
            (p, c) => Some(p.unwrap_or(0) + c.unwrap_or(0)),
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_metadata() {
        let p = OllamaProvider::new("test-ollama", "http://localhost:11434", &[]).unwrap();
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.name(), "test-ollama");
        assert!(p.features().contains(&BackendFeature::Discovery));
    }

    #[test]
    fn config_reasoning_tiers_are_parsed_and_filtered() {
        // Only models with a recognizable reasoning_tier become overrides; a
        // plain chat model and an unparseable tier are dropped, so a card without
        // an override simply keeps `reasoning_tier = None`.
        let models = vec![
            ModelDef {
                name: "deepseek-r1:8b".into(),
                capability: "chat".into(),
                reasoning_tier: Some("high".into()),
                ..Default::default()
            },
            ModelDef {
                name: "qwen3:4b".into(),
                capability: "chat".into(),
                reasoning_tier: Some("low".into()),
                ..Default::default()
            },
            ModelDef {
                name: "llama3:8b".into(),
                capability: "chat".into(),
                reasoning_tier: None,
                ..Default::default()
            },
            ModelDef {
                name: "mystery:1b".into(),
                capability: "chat".into(),
                reasoning_tier: Some("not-a-tier".into()),
                ..Default::default()
            },
        ];
        let p = OllamaProvider::new("ollama", "http://localhost:11434", &models).unwrap();

        assert_eq!(
            p.reasoning_tiers.get("deepseek-r1:8b"),
            Some(&ReasoningEffort::High)
        );
        assert_eq!(
            p.reasoning_tiers.get("qwen3:4b"),
            Some(&ReasoningEffort::Low)
        );
        assert!(!p.reasoning_tiers.contains_key("llama3:8b"));
        assert!(!p.reasoning_tiers.contains_key("mystery:1b"));
        assert_eq!(p.reasoning_tiers.len(), 2);
    }

    #[test]
    fn model_id_parse_accepts_old_and_new_format() {
        assert_eq!(ModelId::name("ollama/llama3:8b"), "llama3:8b");
        assert_eq!(ModelId::name("ollama::llama3:8b"), "llama3:8b");
    }

    #[test]
    fn ndjson_stream_lines_accumulate_text_and_tokens() {
        // A realistic Ollama `stream: true` sequence, including a blank line that
        // must be ignored and a final `done` object carrying token counts.
        let lines: Vec<&[u8]> = vec![
            br#"{"message":{"role":"assistant","content":"He"},"done":false}"#,
            b"",
            br#"{"message":{"role":"assistant","content":"llo"},"done":false}"#,
            br#"{"message":{"role":"assistant","content":""},"done":false}"#,
            br#"{"message":{"role":"assistant","content":"!"},"done":false}"#,
            br#"{"done":true,"eval_count":3,"prompt_eval_count":5}"#,
        ];

        let mut full = String::new();
        let mut deltas = Vec::new();
        let mut prompt = None;
        let mut completion = None;
        for line in lines {
            if let Some(chunk) = OllamaProvider::parse_stream_line(line) {
                if chunk.done {
                    prompt = chunk.prompt_eval_count;
                    completion = chunk.eval_count;
                }
                if let Some(content) = chunk.message.map(|m| m.content)
                    && !content.is_empty()
                {
                    full.push_str(&content);
                    deltas.push(content);
                }
            }
        }

        // Multiple real deltas (not one lump), empty content skipped.
        assert_eq!(deltas, vec!["He", "llo", "!"]);
        assert_eq!(full, "Hello!");
        assert_eq!(prompt, Some(5));
        assert_eq!(completion, Some(3));
    }

    #[test]
    fn parse_stream_line_ignores_blank_and_garbage() {
        assert!(OllamaProvider::parse_stream_line(b"").is_none());
        assert!(OllamaProvider::parse_stream_line(b"   \n").is_none());
        assert!(OllamaProvider::parse_stream_line(b"not json").is_none());
    }
}
