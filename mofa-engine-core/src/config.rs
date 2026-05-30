//! Configuration loading for the MoFA Engine.
//!
//! Tries, in order:
//! 1. Explicit path (if provided)
//! 2. `config.toml` in the current working directory
//! 3. `~/.config/mofa-engine/config.toml`
//! 4. Auto-detection from environment variables

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Network listen settings
    #[serde(default)]
    pub listen: ListenConfig,
    /// Memory management settings
    #[serde(default)]
    pub memory: MemoryConfig,
    /// Provider definitions
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

/// Network listen configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenConfig {
    /// Bind address
    #[serde(default = "default_host")]
    pub host: String,
    /// Bind port
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "0.0.0.0".into()
}

fn default_port() -> u16 {
    8420
}

/// Memory management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Memory budget in megabytes (None = auto-detect from system)
    pub budget_mb: Option<u64>,
    /// Seconds of idle time before evicting a model
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            budget_mb: None,
            idle_timeout_secs: default_idle_timeout(),
        }
    }
}

fn default_idle_timeout() -> u64 {
    120
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider display name
    pub name: String,
    /// Provider kind: "ollama" or "openai_compatible"
    pub kind: String,
    /// Base URL for the API
    pub base_url: String,
    /// API key (supports `${ENV_VAR}` syntax)
    pub api_key: Option<String>,
    /// Routing priority (lower = preferred; 1 = local, 10 = cloud)
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Cost tier string
    #[serde(default = "default_cost_tier")]
    pub cost_tier: String,
    /// Explicit model definitions
    #[serde(default)]
    pub models: Vec<ModelDef>,
    /// Whether this provider is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_priority() -> u8 {
    5
}

fn default_cost_tier() -> String {
    "medium".into()
}

fn default_enabled() -> bool {
    true
}

/// A model definition within a provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDef {
    /// Model name / identifier
    pub name: String,
    /// Capability string (e.g. "chat", "tts")
    pub capability: String,
    /// Context window size in tokens
    pub context_window: Option<u32>,
    /// Estimated memory in megabytes
    pub memory_mb: Option<u64>,
}

impl EngineConfig {
    /// Load configuration from the first available source.
    ///
    /// Priority: explicit path > `./config.toml` > `~/.config/mofa-engine/config.toml` > env auto-detect.
    pub fn load(explicit_path: Option<&Path>) -> Self {
        // 1. Explicit path
        if let Some(path) = explicit_path
            && let Some(cfg) = Self::from_toml_file(path) {
                tracing::info!("loaded config from {}", path.display());
                return cfg;
            }

        // 2. CWD config.toml
        let cwd_path = PathBuf::from("config.toml");
        if let Some(cfg) = Self::from_toml_file(&cwd_path) {
            tracing::info!("loaded config from ./config.toml");
            return cfg;
        }

        // 3. ~/.config/mofa-engine/config.toml
        if let Some(config_dir) = dirs::config_dir() {
            let home_path = config_dir.join("mofa-engine").join("config.toml");
            if let Some(cfg) = Self::from_toml_file(&home_path) {
                tracing::info!("loaded config from {}", home_path.display());
                return cfg;
            }
        }

        // 4. Auto-detect from env
        tracing::info!("no config file found, auto-detecting from environment");
        Self::from_env()
    }

    /// Try to parse a TOML config file, resolving `${ENV_VAR}` in api_key fields.
    fn from_toml_file(path: &Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut config: Self = toml::from_str(&content).ok()?;

        // Resolve env vars in api_key fields
        for provider in &mut config.providers {
            if let Some(ref key) = provider.api_key {
                provider.api_key = Some(resolve_env_var(key));
            }
        }

        Some(config)
    }

    /// Auto-detect providers from well-known environment variables.
    fn from_env() -> Self {
        let mut providers = Vec::new();

        // Always try local Ollama
        providers.push(ProviderConfig {
            name: "ollama".into(),
            kind: "ollama".into(),
            base_url: "http://127.0.0.1:11434".into(),
            api_key: None,
            priority: 1,
            cost_tier: "free".into(),
            models: vec![],
            enabled: true,
        });

        // OpenAI
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            providers.push(ProviderConfig {
                name: "openai".into(),
                kind: "openai_compatible".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key: Some(key),
                priority: 10,
                cost_tier: "high".into(),
                models: vec![
                    ModelDef { name: "gpt-4o".into(), capability: "chat".into(), context_window: Some(128000), memory_mb: None },
                    ModelDef { name: "gpt-4o-mini".into(), capability: "chat".into(), context_window: Some(128000), memory_mb: None },
                    ModelDef { name: "tts-1".into(), capability: "tts".into(), context_window: None, memory_mb: None },
                    ModelDef { name: "tts-1-hd".into(), capability: "tts".into(), context_window: None, memory_mb: None },
                    ModelDef { name: "whisper-1".into(), capability: "asr".into(), context_window: None, memory_mb: None },
                ],
                enabled: true,
            });
        }

        // DeepSeek
        if let Ok(key) = std::env::var("DEEPSEEK_API_KEY") {
            providers.push(ProviderConfig {
                name: "deepseek".into(),
                kind: "openai_compatible".into(),
                base_url: "https://api.deepseek.com".into(),
                api_key: Some(key),
                priority: 8,
                cost_tier: "low".into(),
                models: vec![
                    ModelDef { name: "deepseek-chat".into(), capability: "chat".into(), context_window: Some(64000), memory_mb: None },
                    ModelDef { name: "deepseek-reasoner".into(), capability: "chat".into(), context_window: Some(64000), memory_mb: None },
                ],
                enabled: true,
            });
        }

        // DashScope
        if let Ok(key) = std::env::var("DASHSCOPE_API_KEY") {
            providers.push(ProviderConfig {
                name: "dashscope".into(),
                kind: "openai_compatible".into(),
                base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
                api_key: Some(key),
                priority: 8,
                cost_tier: "medium".into(),
                models: vec![
                    ModelDef { name: "qwen-plus".into(), capability: "chat".into(), context_window: Some(131072), memory_mb: None },
                    ModelDef { name: "qwen-turbo".into(), capability: "chat".into(), context_window: Some(131072), memory_mb: None },
                    ModelDef { name: "qwen-max".into(), capability: "chat".into(), context_window: Some(32768), memory_mb: None },
                ],
                enabled: true,
            });
        }

        // NVIDIA
        if let Ok(key) = std::env::var("NVIDIA_API_KEY") {
            providers.push(ProviderConfig {
                name: "nvidia".into(),
                kind: "openai_compatible".into(),
                base_url: "https://integrate.api.nvidia.com/v1".into(),
                api_key: Some(key),
                priority: 8,
                cost_tier: "medium".into(),
                models: vec![
                    ModelDef { name: "meta/llama-3.1-8b-instruct".into(), capability: "chat".into(), context_window: Some(128000), memory_mb: None },
                ],
                enabled: true,
            });
        }

        // Perplexity
        if let Ok(key) = std::env::var("PERPLEXITY_API_KEY") {
            providers.push(ProviderConfig {
                name: "perplexity".into(),
                kind: "openai_compatible".into(),
                base_url: "https://api.perplexity.ai".into(),
                api_key: Some(key),
                priority: 9,
                cost_tier: "medium".into(),
                models: vec![
                    ModelDef { name: "sonar".into(), capability: "chat".into(), context_window: Some(128000), memory_mb: None },
                ],
                enabled: true,
            });
        }

        // ZAI (Zhipu BigModel)
        if let Ok(key) = std::env::var("ZAI_API_KEY") {
            providers.push(ProviderConfig {
                name: "zhipu".into(),
                kind: "openai_compatible".into(),
                base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
                api_key: Some(key),
                priority: 8,
                cost_tier: "low".into(),
                models: vec![
                    ModelDef { name: "glm-4-flash".into(), capability: "chat".into(), context_window: Some(128000), memory_mb: None },
                ],
                enabled: true,
            });
        }

        EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            providers,
        }
    }
}

/// Resolve `${ENV_VAR}` patterns in a string.
fn resolve_env_var(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("${")
        && let Some(var_name) = rest.strip_suffix('}') {
            return std::env::var(var_name).unwrap_or_default();
        }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_env_var_plain() {
        assert_eq!(resolve_env_var("plain-key"), "plain-key");
    }

    #[test]
    fn resolve_env_var_syntax() {
        // SAFETY: this test runs single-threaded and restores the env var.
        unsafe { std::env::set_var("TEST_MOFA_KEY", "resolved-value") };
        assert_eq!(resolve_env_var("${TEST_MOFA_KEY}"), "resolved-value");
        unsafe { std::env::remove_var("TEST_MOFA_KEY") };
    }

    #[test]
    fn default_config_has_ollama() {
        let cfg = EngineConfig::from_env();
        assert!(cfg.providers.iter().any(|p| p.name == "ollama"));
    }

    #[test]
    fn default_listen_port() {
        let cfg = EngineConfig::from_env();
        assert_eq!(cfg.listen.port, 8420);
    }

    #[test]
    fn toml_roundtrip() {
        let cfg = EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            providers: vec![ProviderConfig {
                name: "test".into(),
                kind: "ollama".into(),
                base_url: "http://localhost:11434".into(),
                api_key: None,
                priority: 1,
                cost_tier: "free".into(),
                models: vec![],
                enabled: true,
            }],
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: EngineConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].name, "test");
    }
}
