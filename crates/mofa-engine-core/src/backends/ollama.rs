//! Ollama backend — talks to a local Ollama instance.

use async_trait::async_trait;
use mofa_kernel::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use dashmap::DashMap;

pub struct OllamaBackend {
    client: Client,
    base_url: String,
    models: DashMap<String, ModelInfo>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
    size: u64,
}

#[derive(Deserialize)]
struct OllamaListResponse {
    models: Vec<OllamaModel>,
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    stream: bool,
}

#[derive(Serialize, Deserialize)]
struct OllamaChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaChatMessage,
}

impl OllamaBackend {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            models: DashMap::new(),
        }
    }
}

#[async_trait]
impl ModelBackend for OllamaBackend {
    fn backend_type(&self) -> BackendType {
        BackendType::Ollama
    }

    async fn discover(&self) -> Result<Vec<ModelInfo>, EngineError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self.client.get(&url).send().await
            .map_err(|e| EngineError::BackendError(format!("ollama discover: {e}")))?;

        if !resp.status().is_success() {
            return Err(EngineError::BackendError(format!("ollama returned {}", resp.status())));
        }

        let list: OllamaListResponse = resp.json().await
            .map_err(|e| EngineError::BackendError(format!("ollama parse: {e}")))?;

        let mut infos = Vec::new();
        for m in list.models {
            let clean_name = m.name.split(':').next().unwrap_or(&m.name).to_string();
            let id = format!("ollama:{clean_name}");
            let info = ModelInfo {
                id: id.clone(),
                name: clean_name,
                model_type: ModelType::Llm,
                backend: BackendType::Ollama,
                status: ModelStatus::Available,
                memory_bytes: m.size,
                priority: ModelPriority::Normal,
            };
            self.models.insert(id, info.clone());
            infos.push(info);
        }
        Ok(infos)
    }

    async fn health_check(&self) -> Result<bool, EngineError> {
        let resp = self.client.get(&self.base_url).send().await;
        Ok(resp.is_ok_and(|r| r.status().is_success()))
    }

    async fn load_model(&self, model_id: &str) -> Result<(), EngineError> {
        if let Some(mut info) = self.models.get_mut(model_id) {
            info.status = ModelStatus::Loaded;
        }
        Ok(())
    }

    async fn unload_model(&self, model_id: &str) -> Result<(), EngineError> {
        if let Some(mut info) = self.models.get_mut(model_id) {
            info.status = ModelStatus::Available;
        }
        Ok(())
    }

    async fn run(&self, model_id: &str, input: &ModelInput) -> Result<ModelOutput, EngineError> {
        let model_name = self.models.get(model_id)
            .map(|m| m.name.clone())
            .ok_or_else(|| EngineError::ModelNotFound(model_id.to_string()))?;

        let text = input.text.as_deref()
            .ok_or_else(|| EngineError::InvalidInput("text input required for LLM".to_string()))?;

        let mut messages = Vec::new();
        if let Some(ref prompt) = input.prompt {
            messages.push(OllamaChatMessage {
                role: "system".to_string(),
                content: prompt.clone(),
            });
        }
        messages.push(OllamaChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        let req = OllamaChatRequest {
            model: model_name,
            messages,
            stream: false,
        };

        let url = format!("{}/api/chat", self.base_url);
        let resp = self.client.post(&url).json(&req).send().await
            .map_err(|e| EngineError::InferenceFailed(format!("ollama: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EngineError::InferenceFailed(format!("ollama error: {body}")));
        }

        let chat_resp: OllamaChatResponse = resp.json().await
            .map_err(|e| EngineError::InferenceFailed(format!("ollama parse: {e}")))?;

        Ok(ModelOutput {
            text: Some(chat_resp.message.content),
            file: None,
            base64: None,
        })
    }

    fn supports_type(&self, model_type: ModelType) -> bool {
        matches!(model_type, ModelType::Llm | ModelType::Vlm)
    }
}
