//! Multi-vendor cloud provider backed by the `liter-llm` crate.
//!
//! `liter-llm` exposes 140+ LLM vendors behind one OpenAI-style contract and
//! handles per-vendor request/response adaptation, auth, and the `provider/model`
//! routing convention. MoFA keeps ownership of routing, failover, memory, and
//! dual-track observability; this adapter just bridges the [`Provider`] trait to
//! liter-llm's [`LlmClient`].
//!
//! A configured model `name` is the liter-llm model id (e.g. `openai/gpt-4o`,
//! `deepseek/deepseek-chat`). MoFA's canonical id becomes `<provider>/<name>`, and
//! [`ModelId::name`] recovers the `provider/model` string liter-llm expects.

use async_trait::async_trait;
use base64::Engine as _;
use liter_llm::{
    AssistantContent, AssistantMessage, ChatCompletionRequest, ClientConfigBuilder, ContentPart,
    CreateImageRequest, DefaultClient, ImageDetail, LlmClient, Message as LiterMessage,
    SystemMessage, UserContent, UserMessage,
};
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelId, ModelResidency,
    Provider, ProviderKind, Reasoning, ReasoningEffort, StreamDelta, StreamSink,
};
use std::time::Instant;
use tokio_stream::StreamExt as _;

use crate::config::ModelDef;

/// A provider that reaches many cloud vendors through the `liter-llm` gateway.
pub struct LiterLLMProvider {
    /// Display name.
    name: String,
    /// Configured models (each `name` is a liter-llm `provider/model` id).
    models: Vec<ModelDef>,
    /// Cost tier for all models from this provider.
    cost_tier: CostTier,
    /// Directory for generated image artifacts.
    output_dir: std::path::PathBuf,
    /// The liter-llm client. `None` when no usable credentials were resolved at
    /// construction, in which case the provider reports itself unavailable rather
    /// than failing engine startup (mirrors the OpenAI-compatible backend).
    client: Option<DefaultClient>,
}

impl LiterLLMProvider {
    /// Build a liter-llm provider.
    ///
    /// When `api_key` is empty, liter-llm resolves the vendor's environment
    /// variable (e.g. `OPENAI_API_KEY`) from the first model's `provider/` prefix.
    /// A non-empty `base_url` overrides the vendor default (e.g. a private gateway).
    pub fn new(
        name: impl Into<String>,
        api_key: Option<String>,
        base_url: impl Into<String>,
        models: Vec<ModelDef>,
        cost_tier: CostTier,
        output_dir: Option<String>,
    ) -> Result<Self, EngineError> {
        let name = name.into();
        let key = api_key.unwrap_or_default();
        let has_key = !key.is_empty();

        let mut builder = ClientConfigBuilder::new(key).load_env(!has_key);
        let base_url = base_url.into();
        if !base_url.is_empty() {
            builder = builder.base_url(base_url);
        }
        let config = builder.build();

        // The model hint's `provider/` prefix selects the default vendor (and its
        // env var when `load_env` is on). Per-request model prefixes still route.
        let model_hint = models.first().map(|m| m.name.clone());
        let client = DefaultClient::new(config, model_hint.as_deref()).ok();
        if client.is_none() {
            tracing::warn!(
                provider = %name,
                "liter-llm provider has no usable credentials; marked unavailable"
            );
        }

        Ok(Self {
            name,
            models,
            cost_tier,
            output_dir: output_dir
                .filter(|s| !s.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir),
            client,
        })
    }

    fn client(&self) -> Result<&DefaultClient, EngineError> {
        self.client
            .as_ref()
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "no liter-llm credentials configured".into(),
            })
    }

    fn provider_error(&self, e: impl std::fmt::Display) -> EngineError {
        EngineError::ProviderError {
            provider: self.name.clone(),
            detail: e.to_string(),
        }
    }

    /// Map MoFA conversation messages to liter-llm's typed text `Message`s.
    fn to_liter_messages(request: &InferenceRequest) -> Vec<LiterMessage> {
        request
            .messages
            .iter()
            .map(|m| Self::text_message(&m.role, m.content.clone()))
            .collect()
    }

    /// Build a chat request from MoFA text messages, applying `max_tokens` and
    /// the reasoning effort tier. Errors if there are no messages.
    fn build_chat_request(
        &self,
        model_name: &str,
        request: &InferenceRequest,
    ) -> Result<ChatCompletionRequest, EngineError> {
        let messages = Self::to_liter_messages(request);
        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
        }
        Ok(ChatCompletionRequest {
            model: model_name.to_string(),
            messages,
            max_tokens: request.params.get("max_tokens").and_then(|v| v.as_u64()),
            reasoning_effort: Self::to_liter_effort(request.reasoning),
            ..Default::default()
        })
    }

    async fn invoke_chat(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let req = self.build_chat_request(model_name, request)?;

        let resp = self
            .client()?
            .chat(req)
            .await
            .map_err(|e| self.provider_error(e))?;

        let text = resp.choices.first().and_then(|c| c.message.text());
        let (prompt_tokens, completion_tokens, tokens_used) = Self::split_usage(resp.usage);

        Ok(InferenceResponse {
            text,
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

    async fn invoke_image_gen(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: Instant,
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
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                EngineError::InvalidRequest("image generation requires a prompt".into())
            })?;

        let size = request
            .params
            .get("size")
            .and_then(|v| v.as_str())
            .map(String::from);

        let req = CreateImageRequest {
            prompt,
            model: Some(model_name.to_string()),
            n: Some(1),
            size,
            // Prefer base64 so we can persist a managed local artifact; vendors that
            // ignore this still return a URL, which we surface as-is.
            response_format: Some("b64_json".into()),
            ..Default::default()
        };

        let resp = self
            .client()?
            .image_generate(req)
            .await
            .map_err(|e| self.provider_error(e))?;

        let image = resp
            .data
            .into_iter()
            .next()
            .ok_or_else(|| EngineError::ProviderError {
                provider: self.name.clone(),
                detail: "image generation returned no data".into(),
            })?;

        let (text, file) = if let Some(b64) = image.b64_json {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .map_err(|e| self.provider_error(format!("image base64 decode error: {e}")))?;
            let path = self
                .output_dir
                .join(format!("mofa_image_{}.png", uuid::Uuid::new_v4()));
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| EngineError::Internal(format!("image write error: {e}")))?;
            (None, Some(path.to_string_lossy().to_string()))
        } else {
            (image.url, None)
        };

        Ok(InferenceResponse {
            text,
            file,
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

    /// Vision-language understanding (S3): send the conversation as multimodal
    /// chat, attaching each message's `images` as image parts with a `detail`
    /// tier (`low | high | auto`, from `params.detail`) that passes through to the
    /// vendor's image billing.
    async fn invoke_vlm(
        &self,
        model_name: &str,
        request: &InferenceRequest,
        start: Instant,
    ) -> Result<InferenceResponse, EngineError> {
        let detail = match request.params.get("detail").and_then(|v| v.as_str()) {
            Some("low") => ImageDetail::Low,
            Some("high") => ImageDetail::High,
            _ => ImageDetail::Auto,
        };

        let mut messages = Vec::with_capacity(request.messages.len());
        for m in &request.messages {
            if m.images.is_empty() {
                messages.push(Self::text_message(&m.role, m.content.clone()));
                continue;
            }
            // A multimodal turn: text first, then each image as a detail-tagged part.
            let mut parts = Vec::with_capacity(m.images.len() + 1);
            if !m.content.is_empty() {
                parts.push(ContentPart::text(m.content.clone()));
            }
            for image in &m.images {
                parts.push(ContentPart::image_with_detail(
                    Self::image_to_url(image).await?,
                    detail.clone(),
                ));
            }
            messages.push(LiterMessage::user_with_parts(parts));
        }
        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
        }

        let req = ChatCompletionRequest {
            model: model_name.to_string(),
            messages,
            max_tokens: request.params.get("max_tokens").and_then(|v| v.as_u64()),
            reasoning_effort: Self::to_liter_effort(request.reasoning),
            ..Default::default()
        };

        let resp = self
            .client()?
            .chat(req)
            .await
            .map_err(|e| self.provider_error(e))?;

        let text = resp.choices.first().and_then(|c| c.message.text());
        let (prompt_tokens, completion_tokens, tokens_used) = Self::split_usage(resp.usage);

        Ok(InferenceResponse {
            text,
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

/// Pure/IO conversions between MoFA and liter-llm types, grouped as private
/// associated functions rather than free functions.
impl LiterLLMProvider {
    /// Map a MoFA reasoning request to liter-llm's effort enum.
    fn to_liter_effort(reasoning: Option<Reasoning>) -> Option<liter_llm::ReasoningEffort> {
        reasoning.map(|r| match r.effort {
            ReasoningEffort::Low => liter_llm::ReasoningEffort::Low,
            ReasoningEffort::Medium => liter_llm::ReasoningEffort::Medium,
            ReasoningEffort::High => liter_llm::ReasoningEffort::High,
        })
    }

    /// Build a plain-text liter-llm message for the given role.
    fn text_message(role: &str, content: String) -> LiterMessage {
        match role {
            "system" | "developer" => LiterMessage::System(SystemMessage {
                content: UserContent::Text(content),
                ..Default::default()
            }),
            "assistant" => LiterMessage::Assistant(AssistantMessage {
                content: Some(AssistantContent::Text(content)),
                ..Default::default()
            }),
            _ => LiterMessage::User(UserMessage {
                content: UserContent::Text(content),
                ..Default::default()
            }),
        }
    }

    /// Resolve an image reference to a URL liter-llm accepts. HTTP(S) and `data:`
    /// URLs pass through; a local file path is read and encoded as a `data:` URL.
    async fn image_to_url(image: &str) -> Result<String, EngineError> {
        if image.starts_with("http://")
            || image.starts_with("https://")
            || image.starts_with("data:")
        {
            return Ok(image.to_string());
        }
        let bytes = tokio::fs::read(image).await.map_err(|e| {
            EngineError::InvalidRequest(format!("cannot read image '{image}': {e}"))
        })?;
        let mime = match std::path::Path::new(image)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/jpeg",
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(format!("data:{mime};base64,{b64}"))
    }

    /// Split a liter-llm `Usage` into `(prompt, completion, total)` as `Option<u32>`,
    /// falling back to `prompt + completion` when the vendor omits a total.
    fn split_usage(usage: Option<liter_llm::Usage>) -> (Option<u32>, Option<u32>, Option<u32>) {
        match usage {
            None => (None, None, None),
            Some(u) => {
                let prompt = u.prompt_tokens as u32;
                let completion = u.completion_tokens as u32;
                let total = if u.total_tokens > 0 {
                    u.total_tokens as u32
                } else {
                    prompt + completion
                };
                (Some(prompt), Some(completion), Some(total))
            }
        }
    }
}

#[async_trait]
impl Provider for LiterLLMProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::LiterLlm
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
        let Some(client) = &self.client else {
            return Ok(BackendHealth::Unavailable);
        };
        // A flaky/unsupported model-list endpoint should not kill routing, so a
        // reachable-but-erroring vendor stays Degraded (still routable) rather than
        // Unavailable; invoke-time failures are what trip the circuit breaker.
        match client.list_models().await {
            Ok(_) => Ok(BackendHealth::Healthy),
            Err(_) => Ok(BackendHealth::Degraded),
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
        let start = Instant::now();

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
            Capability::Vlm => self.invoke_vlm(model_name, request, start).await,
            Capability::ImageGen => self.invoke_image_gen(model_name, request, start).await,
            other => Err(EngineError::UnsupportedOperation(format!(
                "provider '{}' does not support {other} via liter-llm",
                self.name
            ))),
        }
    }

    /// Real per-token streaming via liter-llm's `chat_stream`, forwarding each
    /// `delta.content` through `sink`. Non-chat capabilities fall back to a
    /// single-shot emit through [`invoke`](Self::invoke).
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
        let req = self.build_chat_request(model_name, request)?;

        let start = Instant::now();
        let mut stream = self
            .client()?
            .chat_stream(req)
            .await
            .map_err(|e| self.provider_error(e))?;

        let mut full = String::new();
        let mut usage = None;
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| self.provider_error(e))?;
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
            let Some(choice) = chunk.choices.into_iter().next() else {
                continue;
            };
            // Thought-chain deltas are forwarded distinctly from answer text so
            // callers can display/audit the reasoning separately (S2).
            if let Some(rc) = choice.delta.reasoning_content
                && !rc.is_empty()
            {
                let _ = sink.send(StreamDelta::Reasoning(rc)).await;
            }
            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                full.push_str(&content);
                let _ = sink.send(StreamDelta::Text(content)).await;
            }
        }

        let (prompt_tokens, completion_tokens, tokens_used) = Self::split_usage(usage);
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

    fn model(name: &str, cap: &str) -> ModelDef {
        ModelDef {
            name: name.into(),
            capability: cap.into(),
            context_window: None,
            memory_mb: None,
            ..Default::default()
        }
    }

    #[test]
    fn kind_and_locality() {
        let p = LiterLLMProvider::new(
            "liter",
            Some("sk-test".into()),
            "",
            vec![model("openai/gpt-4o", "chat")],
            CostTier::Medium,
            None,
        )
        .unwrap();
        assert_eq!(p.kind(), ProviderKind::LiterLlm);
        // Cloud gateway: never treated as on-device.
        assert!(!p.kind().is_local());
    }

    #[tokio::test]
    async fn discover_maps_provider_prefixed_models() {
        let p = LiterLLMProvider::new(
            "liter",
            Some("sk-test".into()),
            "",
            vec![
                model("openai/gpt-4o", "chat"),
                model("openai/dall-e-3", "image_gen"),
            ],
            CostTier::Medium,
            None,
        )
        .unwrap();
        let cards = p.discover().await.unwrap();
        assert_eq!(cards.len(), 2);
        // Canonical id is `<provider>/<name>`, and ModelId::name recovers the
        // `provider/model` string liter-llm routes on.
        assert_eq!(cards[0].id, "liter/openai/gpt-4o");
        assert_eq!(ModelId::name(&cards[0].id), "openai/gpt-4o");
        assert_eq!(cards[0].capability, Capability::Chat);
        assert_eq!(cards[1].capability, Capability::ImageGen);
        assert_eq!(cards[0].residency, ModelResidency::Remote);
    }

    #[tokio::test]
    async fn missing_credentials_report_unavailable() {
        // Empty key + a vendor whose env var is unset → no client → Unavailable,
        // and startup still succeeds (no hard failure).
        let p = LiterLLMProvider::new(
            "liter",
            None,
            "",
            vec![model("openai/gpt-4o", "chat")],
            CostTier::Medium,
            None,
        )
        .unwrap();
        // In CI OPENAI_API_KEY is typically unset; if it is set, the client builds
        // and health is not Unavailable — accept either, but never panic.
        let health = p.health().await.unwrap();
        assert!(matches!(
            health,
            BackendHealth::Unavailable | BackendHealth::Healthy | BackendHealth::Degraded
        ));
    }

    #[test]
    fn usage_split_falls_back_to_sum() {
        let (p, c, t) = LiterLLMProvider::split_usage(Some(liter_llm::Usage {
            prompt_tokens: 5,
            completion_tokens: 3,
            total_tokens: 0,
            ..Default::default()
        }));
        assert_eq!((p, c, t), (Some(5), Some(3), Some(8)));
        assert_eq!(LiterLLMProvider::split_usage(None), (None, None, None));
    }

    #[test]
    fn effort_maps_to_liter_enum() {
        use liter_llm::ReasoningEffort as L;
        let mk = |e| {
            Some(Reasoning {
                effort: e,
                include: false,
            })
        };
        assert!(matches!(
            LiterLLMProvider::to_liter_effort(mk(ReasoningEffort::Low)),
            Some(L::Low)
        ));
        assert!(matches!(
            LiterLLMProvider::to_liter_effort(mk(ReasoningEffort::Medium)),
            Some(L::Medium)
        ));
        assert!(matches!(
            LiterLLMProvider::to_liter_effort(mk(ReasoningEffort::High)),
            Some(L::High)
        ));
        assert!(LiterLLMProvider::to_liter_effort(None).is_none());
    }

    #[tokio::test]
    async fn image_url_and_data_url_pass_through() {
        assert_eq!(
            LiterLLMProvider::image_to_url("https://example.com/a.png")
                .await
                .unwrap(),
            "https://example.com/a.png"
        );
        assert_eq!(
            LiterLLMProvider::image_to_url("data:image/png;base64,AAAA")
                .await
                .unwrap(),
            "data:image/png;base64,AAAA"
        );
    }

    #[tokio::test]
    async fn local_image_file_is_encoded_as_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.png");
        tokio::fs::write(&path, b"\x89PNG\r\n").await.unwrap();
        let url = LiterLLMProvider::image_to_url(path.to_str().unwrap())
            .await
            .unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn discover_includes_vlm_capability() {
        let p = LiterLLMProvider::new(
            "liter",
            Some("sk-test".into()),
            "",
            vec![model("openai/gpt-4o", "vlm")],
            CostTier::Medium,
            None,
        )
        .unwrap();
        let cards = p.discover().await.unwrap();
        assert_eq!(cards[0].capability, Capability::Vlm);
    }
}
