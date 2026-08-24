//! Ollama provider backend.
//!
//! Communicates with a local Ollama instance via its HTTP API.

use async_trait::async_trait;
use mofa_kernel::{
    BackendFeature, BackendHealth, Capability, CostTier, EngineError, InferenceRequest,
    InferenceResponse, LifecycleResult, ModelAvailability, ModelCard, ModelResidency, Provider,
    ProviderKind, canonical_model_id, model_id_name,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Provider for a local Ollama instance.
pub struct OllamaProvider {
    /// Display name.
    name: String,
    /// Base URL.
    base_url: String,
    /// HTTP client.
    client: Client,
}

impl OllamaProvider {
    /// Create a new Ollama provider.
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        // A build failure means the system TLS/HTTP stack is unusable; fail
        // loudly rather than silently dropping the configured timeouts/no_proxy.
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build()
            .expect("failed to build HTTP client");

        Self {
            name: name.into(),
            base_url: base_url.into(),
            client,
        }
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
    keep_alive: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    content: String,
    /// Bare base64 image payloads (Ollama chat API format).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
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
                if lower.contains("embed") || lower.contains(":cloud") || lower.contains("-cloud") {
                    return None;
                }

                let mut card = ModelCard::new(
                    self.name.clone(),
                    model_name.clone(),
                    Capability::Chat,
                    CostTier::Free,
                );
                card.id = canonical_model_id(&self.name, &model_name);
                card.availability = ModelAvailability::Discovered;
                card.residency = if loaded.contains(&model_name) {
                    ModelResidency::Loaded
                } else {
                    ModelResidency::Unloaded
                };
                card.context_window = 4096;
                card.memory_estimate_bytes = m.size.unwrap_or(0);
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
        let model_name = model_id_name(model_id);
        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: " ".into(),
                images: Vec::new(),
            }],
            stream: false,
            keep_alive: Some("5m".into()),
        };
        // (health probe: no images)
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
            model_id: canonical_model_id(&self.name, model_name),
            residency: ModelResidency::Loaded,
            memory_bytes: None,
            changed: true,
        })
    }

    async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
        let model_name = model_id_name(model_id);
        let body = OllamaChatRequest {
            model: model_name.to_string(),
            messages: vec![OllamaMessage {
                role: "user".into(),
                content: " ".into(),
                images: Vec::new(),
            }],
            stream: false,
            keep_alive: Some("0".into()),
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
            model_id: canonical_model_id(&self.name, model_name),
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

        let messages: Vec<OllamaMessage> = request
            .messages
            .iter()
            .map(|m| OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                // Ollama wants bare base64; kernel messages carry data URLs.
                images: m
                    .images
                    .iter()
                    .map(|url| {
                        url.split_once(",")
                            .map(|(_, payload)| payload.to_string())
                            .unwrap_or_else(|| url.clone())
                    })
                    .collect(),
            })
            .collect();

        if messages.is_empty() {
            return Err(EngineError::InvalidRequest("no messages provided".into()));
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
        let text = chat_resp.message.map(|m| m.content).unwrap_or_default();

        Ok(InferenceResponse {
            text: Some(text),
            file: None,
            model_used: model_name.to_string(),
            provider: self.name.clone(),
            duration_ms,
            request_id: request.request_id.clone(),
            tokens_used: chat_resp.eval_count,
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

    #[test]
    fn provider_metadata() {
        let p = OllamaProvider::new("test-ollama", "http://localhost:11434");
        assert_eq!(p.kind(), ProviderKind::Ollama);
        assert_eq!(p.name(), "test-ollama");
        assert!(p.features().contains(&BackendFeature::Discovery));
    }

    #[test]
    fn model_id_parse_accepts_old_and_new_format() {
        assert_eq!(model_id_name("ollama/llama3:8b"), "llama3:8b");
        assert_eq!(model_id_name("ollama::llama3:8b"), "llama3:8b");
    }
}
