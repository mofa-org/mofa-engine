//! OpenAI-compatible backend — supports LLM, TTS, ASR.

use async_trait::async_trait;
use mofa_kernel::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use dashmap::DashMap;
use std::path::Path;

pub struct OpenAiBackend {
    client: Client,
    base_url: String,
    api_key: String,
    models: DashMap<String, ModelInfo>,
}

impl OpenAiBackend {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            models: DashMap::new(),
        }
    }

    fn register_defaults(&self) -> Vec<ModelInfo> {
        let defaults = vec![
            ("openai:gpt-4o", "gpt-4o", ModelType::Llm, 0u64),
            ("openai:gpt-4o-mini", "gpt-4o-mini", ModelType::Llm, 0),
            ("openai:tts-1", "tts-1", ModelType::Tts, 0),
            ("openai:tts-1-hd", "tts-1-hd", ModelType::Tts, 0),
            ("openai:whisper-1", "whisper-1", ModelType::Asr, 0),
        ];

        let mut infos = Vec::new();
        for (id, name, mt, mem) in defaults {
            let info = ModelInfo {
                id: id.to_string(),
                name: name.to_string(),
                model_type: mt,
                backend: BackendType::OpenAi,
                status: ModelStatus::Available,
                memory_bytes: mem,
                priority: ModelPriority::Normal,
            };
            self.models.insert(id.to_string(), info.clone());
            infos.push(info);
        }
        infos
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Serialize)]
struct TtsRequest {
    model: String,
    input: String,
    voice: String,
    response_format: String,
}

#[async_trait]
impl ModelBackend for OpenAiBackend {
    fn backend_type(&self) -> BackendType {
        BackendType::OpenAi
    }

    async fn discover(&self) -> Result<Vec<ModelInfo>, EngineError> {
        Ok(self.register_defaults())
    }

    async fn health_check(&self) -> Result<bool, EngineError> {
        let resp = self.client.get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send().await;
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
        let info = self.models.get(model_id)
            .ok_or_else(|| EngineError::ModelNotFound(model_id.to_string()))?;

        match info.model_type {
            ModelType::Llm => self.run_llm(&info.name, input).await,
            ModelType::Tts => self.run_tts(&info.name, input).await,
            ModelType::Asr => self.run_asr(&info.name, input).await,
            _ => Err(EngineError::BackendError(format!("openai does not support {:?}", info.model_type))),
        }
    }

    fn supports_type(&self, model_type: ModelType) -> bool {
        matches!(model_type, ModelType::Llm | ModelType::Tts | ModelType::Asr)
    }
}

impl OpenAiBackend {
    async fn run_llm(&self, model: &str, input: &ModelInput) -> Result<ModelOutput, EngineError> {
        let text = input.text.as_deref()
            .ok_or_else(|| EngineError::InvalidInput("text required".to_string()))?;

        let mut messages = Vec::new();
        if let Some(ref prompt) = input.prompt {
            messages.push(ChatMessage { role: "system".to_string(), content: prompt.clone() });
        }
        messages.push(ChatMessage { role: "user".to_string(), content: text.to_string() });

        let req = ChatRequest { model: model.to_string(), messages };

        let resp = self.client.post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&req)
            .send().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai llm: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EngineError::InferenceFailed(format!("openai error: {body}")));
        }

        let chat: ChatResponse = resp.json().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai parse: {e}")))?;

        let content = chat.choices.first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();

        Ok(ModelOutput { text: Some(content), file: None, base64: None })
    }

    async fn run_tts(&self, model: &str, input: &ModelInput) -> Result<ModelOutput, EngineError> {
        let text = input.text.as_deref()
            .ok_or_else(|| EngineError::InvalidInput("text required for TTS".to_string()))?;

        let voice = input.params.as_ref()
            .and_then(|p| p.get("voice"))
            .and_then(|v| v.as_str())
            .unwrap_or("alloy");

        let req = TtsRequest {
            model: model.to_string(),
            input: text.to_string(),
            voice: voice.to_string(),
            response_format: "mp3".to_string(),
        };

        let resp = self.client.post(format!("{}/audio/speech", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&req)
            .send().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai tts: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EngineError::InferenceFailed(format!("openai tts error: {body}")));
        }

        let audio_bytes = resp.bytes().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai tts read: {e}")))?;

        let out_path = std::env::temp_dir().join(format!("mofa_tts_{}.mp3", uuid::Uuid::new_v4()));
        tokio::fs::write(&out_path, &audio_bytes).await
            .map_err(|e| EngineError::InferenceFailed(format!("write audio: {e}")))?;

        Ok(ModelOutput {
            text: None,
            file: Some(out_path.to_string_lossy().to_string()),
            base64: None,
        })
    }

    async fn run_asr(&self, model: &str, input: &ModelInput) -> Result<ModelOutput, EngineError> {
        let file_path = input.file.as_deref()
            .ok_or_else(|| EngineError::InvalidInput("file path required for ASR".to_string()))?;

        let file_bytes = tokio::fs::read(file_path).await
            .map_err(|e| EngineError::InvalidInput(format!("read audio file: {e}")))?;

        let file_name = Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("audio.wav")
            .to_string();

        let part = reqwest::multipart::Part::bytes(file_bytes.to_vec())
            .file_name(file_name)
            .mime_str("audio/wav")
            .map_err(|e| EngineError::InferenceFailed(format!("mime: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .text("model", model.to_string())
            .part("file", part);

        let resp = self.client.post(format!("{}/audio/transcriptions", self.base_url))
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai asr: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EngineError::InferenceFailed(format!("openai asr error: {body}")));
        }

        #[derive(Deserialize)]
        struct TranscriptionResponse { text: String }

        let result: TranscriptionResponse = resp.json().await
            .map_err(|e| EngineError::InferenceFailed(format!("openai asr parse: {e}")))?;

        Ok(ModelOutput { text: Some(result.text), file: None, base64: None })
    }
}
