//! Resource discovery — auto-detect available backends and models.

use crate::config::EngineConfig;
use crate::backends::ollama::OllamaBackend;
use crate::backends::openai::OpenAiBackend;
use mofa_kernel::{ModelBackend, ModelInfo};
use tracing::{info, warn};

pub struct Discovery;

impl Discovery {
    pub async fn discover_all(config: &EngineConfig) -> Vec<(Box<dyn ModelBackend>, Vec<ModelInfo>)> {
        let mut results = Vec::new();

        // Try Ollama
        let ollama_url = config.backends.ollama.as_ref()
            .map(|c| c.url.clone())
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let ollama = OllamaBackend::new(&ollama_url);
        match ollama.discover().await {
            Ok(models) if !models.is_empty() => {
                info!(count = models.len(), "discovered Ollama models");
                results.push((Box::new(ollama) as Box<dyn ModelBackend>, models));
            }
            Ok(_) => {
                info!("Ollama running but no models found");
                results.push((Box::new(ollama) as Box<dyn ModelBackend>, vec![]));
            }
            Err(e) => {
                warn!("Ollama not available: {e}");
            }
        }

        // Try OpenAI
        let api_key = config.backends.openai.as_ref()
            .map(|c| c.api_key.clone())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok());

        if let Some(key) = api_key {
            let base_url = config.backends.openai.as_ref()
                .map(|c| c.base_url.clone())
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

            let openai = OpenAiBackend::new(&base_url, &key);
            match openai.discover().await {
                Ok(models) => {
                    info!(count = models.len(), "registered OpenAI models");
                    results.push((Box::new(openai) as Box<dyn ModelBackend>, models));
                }
                Err(e) => {
                    warn!("OpenAI setup failed: {e}");
                }
            }
        }

        results
    }
}
