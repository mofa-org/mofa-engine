//! Core types shared across the engine.

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelType {
    Llm,
    Tts,
    Asr,
    ImageGen,
    VideoGen,
    Vlm,
    VoiceClone,
}

impl std::fmt::Display for ModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llm => write!(f, "llm"),
            Self::Tts => write!(f, "tts"),
            Self::Asr => write!(f, "asr"),
            Self::ImageGen => write!(f, "image_gen"),
            Self::VideoGen => write!(f, "video_gen"),
            Self::Vlm => write!(f, "vlm"),
            Self::VoiceClone => write!(f, "voice_clone"),
        }
    }
}

impl std::str::FromStr for ModelType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "llm" => Ok(Self::Llm),
            "tts" => Ok(Self::Tts),
            "asr" => Ok(Self::Asr),
            "image_gen" | "imagegen" | "image-gen" => Ok(Self::ImageGen),
            "video_gen" | "videogen" | "video-gen" => Ok(Self::VideoGen),
            "vlm" => Ok(Self::Vlm),
            "voice_clone" | "voiceclone" | "voice-clone" => Ok(Self::VoiceClone),
            _ => Err(format!("unknown model type: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BackendType {
    Ollama,
    OpenAi,
    Mlx,
    Cuda,
    CpuOnnx,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ollama => write!(f, "ollama"),
            Self::OpenAi => write!(f, "openai"),
            Self::Mlx => write!(f, "mlx"),
            Self::Cuda => write!(f, "cuda"),
            Self::CpuOnnx => write!(f, "cpu_onnx"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelStatus {
    Available,
    Loading,
    Loaded,
    Running,
    Unloading,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub model_type: ModelType,
    pub backend: BackendType,
    pub status: ModelStatus,
    pub memory_bytes: u64,
    pub priority: ModelPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ModelPriority {
    Resident,
    Normal,
    Low,
}

impl Default for ModelPriority {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    #[serde(rename = "type")]
    pub model_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input: ModelInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<PrefetchHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResponse {
    pub output: ModelOutput,
    pub model_used: String,
    pub backend: String,
    pub duration_ms: u64,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesResponse {
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineStatus {
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub loaded_models: Vec<ModelInfo>,
    pub backends: Vec<BackendStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStatus {
    pub backend_type: BackendType,
    pub healthy: bool,
    pub model_count: usize,
}
