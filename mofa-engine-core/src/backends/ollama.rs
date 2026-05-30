//! Ollama provider backend.
//!
//! Communicates with a local Ollama instance via its HTTP API.

use async_trait::async_trait;
use mofa_kernel::{
    Capability, CostTier, EngineError, InferenceRequest, InferenceResponse, ModelCard,
    ModelStatus, Provider, ProviderKind,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Provider for a local Ollama instance.
pub struct OllamaProvider {
    /// Display name
    name: String,
    /// Base URL (e.g. `http://127.0.0.1:11434`)
    base_url: String,
    /// HTTP client
    client: Client,
}

impl OllamaProvider {
    /// Create a new Ollama provider.
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300))
            .build()
            .unwrap_or_default();

        Self {
            name: name.into(),
            base_url: base_url.into(),
            client,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
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
    keep_alive: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OllamaChatResponse {
    message: Option<OllamaMessage>,
    #[serde(default)]
    eval_count: Option<u32>,
    #[serde(default)]
    total_duration: Option<u64>,
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn discover(&self) -> Vec<ModelCard> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("ollama discover failed: {e}");
                return vec![];
            }
        };

        let tags: OllamaTagsResponse = match resp.json().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("ollama tags parse failed: {e}");
                return vec![];
            }
        };

        let models = tags.models.unwrap_or_default();
        models
            .into_iter()
            .filter_map(|m| {
                let model_name = m.name.or(m.model)?;

                let lower = model_name.to_lowercase();
                if lower.contains("embed") {
                    return None;
                }

                // Skip Ollama cloud-proxy models — they require a paid subscription
                if lower.contains(":cloud") || lower.contains("-cloud") {
                    return None;
                }

                let id = format!("{}::{}", self.name, model_name);
                Some(ModelCard {
                    id,
                    name: model_name,
                    provider: self.name.clone(),
                    capability: Capability::Chat,
                    status: ModelStatus::Cold,
                    cost_tier: CostTier::Free,
                    context_window: 4096,
                    memory_estimate_bytes: m.size.unwrap_or(0),
                })
            })
            .collect()
    }

    async fn health(&self) -> bool {
        let url = format!("{}/", self.base_url);
        matches!(self.client.get(&url).send().await, Ok(r) if r.status().is_success())
    }

    async fn invoke(
        &self,
        model_id: &str,
        request: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        // Extract model name from "provider::model" format
        let model_name = model_id
            .split("::")
            .nth(1)
            .unwrap_or(model_id);

        let messages: Vec<OllamaMessage> = request
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        if messages.is_empty() {
            return Err(EngineError::InvalidRequest(
                "no messages provided".into(),
            ));
        }

        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages,
            stream: false,
            keep_alive: None,
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
        let text = chat_resp
            .message
            .map(|m| m.content)
            .unwrap_or_default();

        Ok(InferenceResponse {
            text: Some(text),
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: chat_resp.eval_count,
        })
    }

    async fn warm(&self, _model_id: &str) {
        // Ollama loads models on-demand during inference.
        // Explicit warming via /api/chat would block for the full
        // model load time, so we skip it and let invoke() handle loading.
    }

    async fn evict(&self, model_id: &str) {
        let model_name = model_id.split("::").nth(1).unwrap_or(model_id);
        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            keep_alive: Some("0".into()),
        };

        let url = format!("{}/api/chat", self.base_url);
        let _ = self.client.post(&url).json(&body).send().await;
    }

    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_name() {
        let p = OllamaProvider::new("test-ollama", "http://localhost:11434");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.name, "test-ollama");
    }

    #[test]
    fn model_id_parse() {
        let id = "ollama::llama3:8b";
        let name = id.split("::").nth(1).unwrap_or(id);
        assert_eq!(name, "llama3:8b");
    }
}
