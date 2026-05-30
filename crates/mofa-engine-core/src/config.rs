//! Engine configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub memory_budget_mb: Option<u64>,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default)]
    pub backends: BackendsConfig,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

fn default_port() -> u16 { 8420 }
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_idle_timeout_secs() -> u64 { 60 }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendsConfig {
    #[serde(default)]
    pub ollama: Option<OllamaConfig>,
    #[serde(default)]
    pub openai: Option<OpenAiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_url")]
    pub url: String,
}

fn default_ollama_url() -> String { "http://localhost:11434".to_string() }

impl Default for OllamaConfig {
    fn default() -> Self {
        Self { url: default_ollama_url() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
    pub api_key: String,
    #[serde(default = "default_openai_url")]
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
}

fn default_openai_url() -> String { "https://api.openai.com/v1".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub model_type: String,
    pub backend: String,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub priority: Option<String>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            memory_budget_mb: None,
            idle_timeout_secs: default_idle_timeout_secs(),
            backends: BackendsConfig::default(),
            models: Vec::new(),
        }
    }
}

impl EngineConfig {
    pub fn load() -> Self {
        let config_path = Self::config_path();
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str::<EngineConfig>(&content) {
                    return config;
                }
            }
        }

        let mut config = Self::default();

        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            config.backends.openai = Some(OpenAiConfig {
                api_key: key,
                base_url: default_openai_url(),
                models: vec![
                    "gpt-4o".to_string(),
                    "gpt-4o-mini".to_string(),
                    "tts-1".to_string(),
                    "whisper-1".to_string(),
                ],
            });
        }

        config
    }

    pub fn config_path() -> PathBuf {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mofa-engine");
        dir.join("config.toml")
    }

    pub fn memory_budget_bytes(&self) -> u64 {
        self.memory_budget_mb
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or_else(|| {
                let sys = sysinfo::System::new_all();
                sys.total_memory()
            })
    }
}
