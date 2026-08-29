//! Configuration loading for the MoFA Engine.
//!
//! Tries, in order:
//! 1. Explicit path (if provided)
//! 2. `config.toml` in the current working directory
//! 3. `~/.config/mofa-engine/config.toml`
//! 4. Auto-detection from environment variables

use mofa_kernel::{EngineError, ProviderKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level engine configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineConfig {
    #[serde(default)]
    pub listen: ListenConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub timeouts: TimeoutConfig,
    /// Preflight (predictive warming) settings.
    #[serde(default)]
    pub preflight: PreflightConfig,
    #[serde(default)]
    pub artifacts: ArtifactConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

/// Retention policy for engine-generated artifacts (e.g. TTS audio files).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactConfig {
    /// Directory scanned for cleanup (default: the mofa-owned `mofa_artifacts`
    /// subdirectory of the system temp dir).
    #[serde(default)]
    pub dir: Option<String>,
    /// Delete engine artifacts older than this many seconds. `0` disables the
    /// background sweep (artifacts are then the caller's responsibility).
    #[serde(default = "ArtifactConfig::default_artifact_retention")]
    pub retention_secs: u64,
}

impl Default for ArtifactConfig {
    fn default() -> Self {
        Self {
            dir: None,
            retention_secs: Self::default_artifact_retention(),
        }
    }
}

impl ArtifactConfig {
    /// Retain generated artifacts for one hour by default.
    fn default_artifact_retention() -> u64 {
        3600
    }
}

/// Security and file-access constraints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Allowlist of directory roots that request `input_file` paths must fall
    /// under. Empty (the default) allows any local path — appropriate only for a
    /// trusted single-user machine.
    #[serde(default)]
    pub input_roots: Vec<String>,
}

/// Network listen configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListenConfig {
    #[serde(default = "ListenConfig::default_host")]
    pub host: String,
    #[serde(default = "ListenConfig::default_port")]
    pub port: u16,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            host: Self::default_host(),
            port: Self::default_port(),
        }
    }
}

impl ListenConfig {
    /// Bind loopback by default — the API is exposed to the local machine only
    /// unless the operator opts into a wider host.
    fn default_host() -> String {
        "127.0.0.1".into()
    }

    fn default_port() -> u16 {
        8420
    }
}

/// Memory management configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Memory budget in megabytes (None = auto-detect from system).
    pub budget_mb: Option<u64>,
    /// Seconds of idle time before evicting a model.
    #[serde(default = "MemoryConfig::default_idle_timeout")]
    pub idle_timeout_secs: u64,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            budget_mb: None,
            idle_timeout_secs: Self::default_idle_timeout(),
        }
    }
}

impl MemoryConfig {
    /// Evict a model after two minutes idle by default.
    fn default_idle_timeout() -> u64 {
        120
    }
}

/// Timeouts for the distinct phases of a request, all in seconds.
///
/// These bound the time spent waiting in the admission queue, loading a model,
/// running inference, and the request as a whole, plus the per-provider
/// discovery and health probes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Overall budget for a single `invoke`, spanning queueing, loading, and
    /// inference across every candidate attempted.
    #[serde(default = "TimeoutConfig::default_request_timeout")]
    pub request_secs: u64,
    /// Maximum time a request may wait for a concurrency permit before failing.
    #[serde(default = "TimeoutConfig::default_queue_timeout")]
    pub queue_secs: u64,
    /// Maximum time to load/warm a single model.
    #[serde(default = "TimeoutConfig::default_load_timeout")]
    pub load_secs: u64,
    /// Maximum time for a single inference call to a provider.
    #[serde(default = "TimeoutConfig::default_inference_timeout")]
    pub inference_secs: u64,
    /// Maximum time for a single provider discovery probe.
    #[serde(default = "TimeoutConfig::default_discovery_timeout")]
    pub discovery_secs: u64,
    /// Maximum time for a single provider health probe.
    #[serde(default = "TimeoutConfig::default_health_timeout")]
    pub health_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            request_secs: Self::default_request_timeout(),
            queue_secs: Self::default_queue_timeout(),
            load_secs: Self::default_load_timeout(),
            inference_secs: Self::default_inference_timeout(),
            discovery_secs: Self::default_discovery_timeout(),
            health_secs: Self::default_health_timeout(),
        }
    }
}

// These `Duration` accessors are consumed only by the engine (in-crate), so they
// are `pub(crate)`; the struct itself and its `_secs` fields stay `pub` for
// external configuration/serialization.
impl TimeoutConfig {
    /// Overall request budget as a `Duration`.
    pub(crate) fn request(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.request_secs)
    }
    /// Queue-wait budget as a `Duration`.
    pub(crate) fn queue(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.queue_secs)
    }
    /// Load budget as a `Duration`.
    pub(crate) fn load(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.load_secs)
    }
    /// Inference budget as a `Duration`.
    pub(crate) fn inference(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.inference_secs)
    }
    /// Discovery probe budget as a `Duration`.
    pub(crate) fn discovery(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.discovery_secs)
    }
    /// Health probe budget as a `Duration`.
    pub(crate) fn health(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.health_secs)
    }
}

impl TimeoutConfig {
    fn default_request_timeout() -> u64 {
        240
    }
    fn default_queue_timeout() -> u64 {
        30
    }
    fn default_load_timeout() -> u64 {
        60
    }
    fn default_inference_timeout() -> u64 {
        180
    }
    fn default_discovery_timeout() -> u64 {
        8
    }
    fn default_health_timeout() -> u64 {
        5
    }
}

/// Preflight (predictive warming) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightConfig {
    /// Master switch for all predictive warming.
    #[serde(default = "PreflightConfig::default_true")]
    pub enabled: bool,
    /// Whether the engine learns capability transition history.
    #[serde(default = "PreflightConfig::default_true")]
    pub history_learning: bool,
    /// Whether the engine speculatively warms models from hints/history/subscriptions.
    #[serde(default = "PreflightConfig::default_true")]
    pub speculative_warming: bool,
    /// Minimum confidence (0.0–1.0) a history prediction needs before it warms a model.
    #[serde(default = "PreflightConfig::default_confidence_threshold")]
    pub confidence_threshold: f64,
    /// Minimum number of observed transitions before history is trusted.
    #[serde(default = "PreflightConfig::default_min_samples")]
    pub min_samples: u64,
    /// Default lifetime of a capability subscription, in seconds.
    #[serde(default = "PreflightConfig::default_subscription_ttl")]
    pub subscription_ttl_secs: u64,
}

impl Default for PreflightConfig {
    fn default() -> Self {
        Self {
            enabled: Self::default_true(),
            history_learning: Self::default_true(),
            speculative_warming: Self::default_true(),
            confidence_threshold: Self::default_confidence_threshold(),
            min_samples: Self::default_min_samples(),
            subscription_ttl_secs: Self::default_subscription_ttl(),
        }
    }
}

impl PreflightConfig {
    /// Preflight and its sub-switches are on by default; the operator opts out.
    fn default_true() -> bool {
        true
    }
    /// Require moderately strong evidence before a learned prediction warms a model.
    fn default_confidence_threshold() -> f64 {
        0.6
    }
    /// Ignore a transition until it has been seen a few times, to avoid warming on noise.
    fn default_min_samples() -> u64 {
        3
    }
    /// Keep a capability subscription warm for an hour by default.
    fn default_subscription_ttl() -> u64 {
        3600
    }
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    /// Provider kind, e.g. `"ollama"` or `"openai_compatible"`.
    pub kind: String,
    pub base_url: String,
    /// API key (supports `${ENV_VAR}` syntax).
    pub api_key: Option<String>,
    /// Routing priority (lower = preferred; 1 = local, 10 = cloud).
    #[serde(default = "ProviderConfig::default_priority")]
    pub priority: u8,
    #[serde(default = "ProviderConfig::default_cost_tier")]
    pub cost_tier: String,
    #[serde(default)]
    pub models: Vec<ModelDef>,
    #[serde(default = "ProviderConfig::default_enabled")]
    pub enabled: bool,
    /// Program to execute for a `local_tts` process-adapter backend.
    ///
    /// Only used when `kind = "local_tts"`. The engine runs this command once
    /// per synthesis, substituting placeholders in `args`.
    #[serde(default)]
    pub command: Option<String>,
    /// Argument template for a `local_tts` command. Each element may contain the
    /// placeholders `{text}` (input text), `{text_file}` (path to a temp file
    /// holding the input text), and `{output}` (path the command must write
    /// audio to).
    #[serde(default)]
    pub args: Vec<String>,
    /// Output audio container/extension for `local_tts` artifacts (default `wav`).
    #[serde(default)]
    pub output_format: Option<String>,
    /// Vendor dialect for a `cloud_video_gen` backend: `"ark"` (default, the
    /// Volcengine Ark / Seedance task API) or `"agnes"` (the Agnes AI gateway's
    /// `/videos` task API). Ignored by other provider kinds.
    #[serde(default)]
    pub dialect: String,
    /// Directory for `local_tts` audio artifacts (default: the mofa-owned
    /// `mofa_artifacts` subdirectory of the system temp dir).
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Extra arguments a `local_asr` command receives when a request enables
    /// speaker diarization (`params.diarize = true`). Placeholder-substituted like
    /// `args`. Leave empty when the CLI cannot separate speakers.
    #[serde(default)]
    pub diarize_args: Vec<String>,
    /// USD price per 1000 prompt (input) tokens. Used for cost tracking; `0`
    /// (the default, e.g. local models) means no cost is attributed.
    #[serde(default)]
    pub price_input_per_1k: f64,
    /// USD price per 1000 completion (output) tokens.
    #[serde(default)]
    pub price_output_per_1k: f64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: String::new(),
            base_url: String::new(),
            api_key: None,
            priority: Self::default_priority(),
            cost_tier: Self::default_cost_tier(),
            models: Vec::new(),
            enabled: Self::default_enabled(),
            command: None,
            args: Vec::new(),
            output_format: None,
            dialect: String::new(),
            output_dir: None,
            diarize_args: Vec::new(),
            price_input_per_1k: 0.0,
            price_output_per_1k: 0.0,
        }
    }
}

impl ProviderConfig {
    /// A middling routing priority, so an unprioritized provider sits between the
    /// conventional local (1) and cloud (10) bands.
    fn default_priority() -> u8 {
        5
    }
    fn default_cost_tier() -> String {
        "medium".into()
    }
    /// Providers are enabled unless explicitly disabled.
    fn default_enabled() -> bool {
        true
    }
}

/// A model definition within a provider config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelDef {
    pub name: String,
    /// Capability string (e.g. `"chat"`, `"tts"`).
    pub capability: String,
    /// Context window size in tokens.
    pub context_window: Option<u32>,
    /// Estimated memory in megabytes.
    pub memory_mb: Option<u64>,
    /// Reasoning tier (`low`/`medium`/`high`) this model serves, for
    /// `reasoning.effort` tier routing (S2). Omit for non-reasoning models.
    #[serde(default)]
    pub reasoning_tier: Option<String>,
}

impl EngineConfig {
    /// Load configuration from the first available source.
    ///
    /// Priority: explicit path > `./config.toml` > `~/.config/mofa-engine/config.toml` > env auto-detect.
    pub fn load(explicit_path: Option<&Path>) -> Self {
        Self::load_checked(explicit_path).unwrap_or_else(|e| {
            tracing::warn!("falling back to environment auto-detect after config error: {e}");
            Self::from_env()
        })
    }

    /// Load and validate configuration from the first available source.
    pub fn load_checked(explicit_path: Option<&Path>) -> Result<Self, EngineError> {
        // 1. Explicit path
        if let Some(path) = explicit_path {
            let cfg = Self::from_toml_file(path)?;
            tracing::info!("loaded config from {}", path.display());
            cfg.validate()?;
            return Ok(cfg);
        }

        // 2. CWD config.toml
        let cwd_path = PathBuf::from("config.toml");
        if cwd_path.exists() {
            let cfg = Self::from_toml_file(&cwd_path)?;
            tracing::info!("loaded config from ./config.toml");
            cfg.validate()?;
            return Ok(cfg);
        }

        // 3. ~/.config/mofa-engine/config.toml
        if let Some(config_dir) = dirs::config_dir() {
            let home_path = config_dir.join("mofa-engine").join("config.toml");
            if home_path.exists() {
                let cfg = Self::from_toml_file(&home_path)?;
                tracing::info!("loaded config from {}", home_path.display());
                cfg.validate()?;
                return Ok(cfg);
            }
        }

        // 4. Auto-detect from env
        tracing::info!("no config file found, auto-detecting from environment");
        let cfg = Self::from_env();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Try to parse a TOML config file, resolving `${ENV_VAR}` in api_key fields.
    fn from_toml_file(path: &Path) -> Result<Self, EngineError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| EngineError::Config(format!("cannot read {}: {e}", path.display())))?;
        let mut config: Self = toml::from_str(&content)
            .map_err(|e| EngineError::Config(format!("invalid TOML in {}: {e}", path.display())))?;

        // Resolve env vars in api_key fields
        for provider in &mut config.providers {
            if let Some(ref key) = provider.api_key {
                provider.api_key = Some(ProviderConfig::resolve_env_var(key)?);
            }
        }

        Ok(config)
    }

    /// Validate configuration before constructing the engine.
    pub(crate) fn validate(&self) -> Result<(), EngineError> {
        if !(0.0..=1.0).contains(&self.preflight.confidence_threshold) {
            return Err(EngineError::Config(format!(
                "preflight.confidence_threshold must be within 0.0..=1.0, got {}",
                self.preflight.confidence_threshold
            )));
        }
        // Each configured allowlist root must resolve. Otherwise a typo would be
        // silently dropped, leaving an *empty* allowlist that permits every path
        // — a security downgrade rather than the intended restriction.
        for root in &self.security.input_roots {
            if std::fs::canonicalize(root).is_err() {
                return Err(EngineError::Config(format!(
                    "security.input_roots entry '{root}' does not exist or cannot be resolved"
                )));
            }
        }
        for provider in &self.providers {
            if !provider.enabled {
                continue;
            }
            provider.provider_kind()?;
            if provider.kind == "openai_compatible"
                && provider.api_key.as_deref().unwrap_or_default().is_empty()
            {
                return Err(EngineError::Config(format!(
                    "provider '{}' requires a non-empty api_key",
                    provider.name
                )));
            }
            // Every local process-adapter kind is spawned from a configured
            // `command` and serves an explicit model list, so validate all three
            // uniformly: catch a missing command or empty model list here (at
            // `validate-config` time) rather than later at engine construction or
            // silently discovering nothing.
            if matches!(
                provider.kind.as_str(),
                "local_tts" | "local_asr" | "local_image_gen" | "local_video_gen"
            ) {
                if provider.command.as_deref().unwrap_or_default().is_empty() {
                    return Err(EngineError::Config(format!(
                        "provider '{}' ({}) requires a non-empty command",
                        provider.name, provider.kind
                    )));
                }
                if provider.models.is_empty() {
                    return Err(EngineError::Config(format!(
                        "provider '{}' ({}) must declare at least one model",
                        provider.name, provider.kind
                    )));
                }
            }
            for model in &provider.models {
                if mofa_kernel::Capability::from_str_loose(&model.capability).is_none() {
                    return Err(EngineError::Config(format!(
                        "provider '{}' model '{}' has unknown capability '{}'",
                        provider.name, model.name, model.capability
                    )));
                }
            }
        }
        Ok(())
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
            ..Default::default()
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
                    ModelDef {
                        name: "gpt-4o".into(),
                        capability: "chat".into(),
                        context_window: Some(128000),
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "gpt-4o-mini".into(),
                        capability: "chat".into(),
                        context_window: Some(128000),
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "tts-1".into(),
                        capability: "tts".into(),
                        context_window: None,
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "tts-1-hd".into(),
                        capability: "tts".into(),
                        context_window: None,
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "whisper-1".into(),
                        capability: "asr".into(),
                        context_window: None,
                        memory_mb: None,
                        ..Default::default()
                    },
                ],
                enabled: true,
                ..Default::default()
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
                    ModelDef {
                        name: "deepseek-chat".into(),
                        capability: "chat".into(),
                        context_window: Some(64000),
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "deepseek-reasoner".into(),
                        capability: "chat".into(),
                        context_window: Some(64000),
                        memory_mb: None,
                        ..Default::default()
                    },
                ],
                enabled: true,
                ..Default::default()
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
                    ModelDef {
                        name: "qwen-plus".into(),
                        capability: "chat".into(),
                        context_window: Some(131072),
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "qwen-turbo".into(),
                        capability: "chat".into(),
                        context_window: Some(131072),
                        memory_mb: None,
                        ..Default::default()
                    },
                    ModelDef {
                        name: "qwen-max".into(),
                        capability: "chat".into(),
                        context_window: Some(32768),
                        memory_mb: None,
                        ..Default::default()
                    },
                ],
                enabled: true,
                ..Default::default()
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
                models: vec![ModelDef {
                    name: "meta/llama-3.1-8b-instruct".into(),
                    capability: "chat".into(),
                    context_window: Some(128000),
                    memory_mb: None,
                    ..Default::default()
                }],
                enabled: true,
                ..Default::default()
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
                models: vec![ModelDef {
                    name: "sonar".into(),
                    capability: "chat".into(),
                    context_window: Some(128000),
                    memory_mb: None,
                    ..Default::default()
                }],
                enabled: true,
                ..Default::default()
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
                models: vec![ModelDef {
                    name: "glm-4-flash".into(),
                    capability: "chat".into(),
                    context_window: Some(128000),
                    memory_mb: None,
                    ..Default::default()
                }],
                enabled: true,
                ..Default::default()
            });
        }

        EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            artifacts: ArtifactConfig::default(),
            security: SecurityConfig::default(),
            providers,
        }
    }
}

impl ProviderConfig {
    /// Resolve a `${ENV_VAR}` reference to the environment variable's value,
    /// passing any other string through unchanged. Lets a config keep secrets
    /// (e.g. `api_key = "${OPENAI_API_KEY}"`) out of the file itself.
    fn resolve_env_var(s: &str) -> Result<String, EngineError> {
        // Two accepted indirection forms resolve to an environment variable:
        //   - `${VAR}` (shell-style), and
        //   - `env:VAR` (the terser form used throughout our example configs).
        // Anything else is treated as a literal secret. A real API key never
        // matches these prefixes, so this cannot accidentally shadow one.
        let var_name = s
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
            .or_else(|| s.strip_prefix("env:"));
        if let Some(var_name) = var_name {
            return std::env::var(var_name).map_err(|_| {
                EngineError::Config(format!("environment variable '{var_name}' is not set"))
            });
        }
        Ok(s.to_string())
    }

    /// Parse the configured provider kind.
    pub(crate) fn provider_kind(&self) -> Result<ProviderKind, EngineError> {
        match self.kind.as_str() {
            "ollama" => Ok(ProviderKind::Ollama),
            "openai_compatible" => Ok(ProviderKind::OpenAiCompatible),
            "local_tts" => Ok(ProviderKind::LocalTts),
            "local_asr" => Ok(ProviderKind::LocalAsr),
            "local_image_gen" => Ok(ProviderKind::LocalImageGen),
            "local_video_gen" => Ok(ProviderKind::LocalVideoGen),
            "cloud_video_gen" => Ok(ProviderKind::CloudVideoGen),
            "liter_llm" => Ok(ProviderKind::LiterLlm),
            other => Err(EngineError::Config(format!(
                "unknown provider kind '{}' for provider '{}'",
                other, self.name
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_env_var_plain() {
        assert_eq!(
            ProviderConfig::resolve_env_var("plain-key").unwrap(),
            "plain-key"
        );
    }

    #[test]
    fn resolve_env_var_syntax() {
        // SAFETY: this test runs single-threaded and restores the env var.
        unsafe { std::env::set_var("TEST_MOFA_KEY", "resolved-value") };
        assert_eq!(
            ProviderConfig::resolve_env_var("${TEST_MOFA_KEY}").unwrap(),
            "resolved-value"
        );
        unsafe { std::env::remove_var("TEST_MOFA_KEY") };
    }

    #[test]
    fn resolve_env_var_colon_syntax() {
        // The terser `env:VAR` form (used across our example configs) resolves the
        // same way as `${VAR}`.
        // SAFETY: this test runs single-threaded and restores the env var.
        unsafe { std::env::set_var("TEST_MOFA_KEY2", "resolved-colon") };
        assert_eq!(
            ProviderConfig::resolve_env_var("env:TEST_MOFA_KEY2").unwrap(),
            "resolved-colon"
        );
        unsafe { std::env::remove_var("TEST_MOFA_KEY2") };
    }

    #[test]
    fn unresolved_env_var_errors() {
        unsafe { std::env::remove_var("MISSING_MOFA_KEY") };
        assert!(ProviderConfig::resolve_env_var("${MISSING_MOFA_KEY}").is_err());
        assert!(ProviderConfig::resolve_env_var("env:MISSING_MOFA_KEY").is_err());
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
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            artifacts: ArtifactConfig::default(),
            security: SecurityConfig::default(),
            providers: vec![ProviderConfig {
                name: "test".into(),
                kind: "ollama".into(),
                base_url: "http://localhost:11434".into(),
                api_key: None,
                priority: 1,
                cost_tier: "free".into(),
                models: vec![],
                enabled: true,
                ..Default::default()
            }],
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: EngineConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.providers.len(), 1);
        assert_eq!(parsed.providers[0].name, "test");
    }

    #[test]
    fn example_config_is_valid() {
        // The shipped example must always parse and validate so docs cannot drift.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../config.example.toml");
        let content = std::fs::read_to_string(path).expect("config.example.toml should exist");
        let cfg: EngineConfig = toml::from_str(&content).expect("example config should parse");
        cfg.validate().expect("example config should validate");
        assert!(cfg.providers.iter().any(|p| p.name == "ollama"));
        assert_eq!(cfg.timeouts.queue_secs, 30);
        assert!(cfg.preflight.enabled);
    }

    #[test]
    fn timeout_and_preflight_defaults_apply_from_partial_toml() {
        // A config that omits [timeouts] and [preflight] must still parse with defaults.
        let cfg: EngineConfig = toml::from_str("[memory]\nbudget_mb = 1024\n").unwrap();
        assert_eq!(cfg.timeouts.queue_secs, 30);
        assert_eq!(cfg.timeouts.inference_secs, 180);
        assert!(cfg.preflight.enabled);
        assert!(cfg.preflight.speculative_warming);
        assert_eq!(cfg.preflight.min_samples, 3);
        cfg.validate().unwrap();
    }

    #[test]
    fn validate_rejects_out_of_range_confidence() {
        let mut cfg = EngineConfig::from_env();
        cfg.preflight.confidence_threshold = 1.5;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_requires_command_and_models_for_all_local_kinds() {
        // Every local process-adapter kind must be validated uniformly: a missing
        // command, or an empty model list, is rejected at validate-config time.
        for kind in [
            "local_tts",
            "local_asr",
            "local_image_gen",
            "local_video_gen",
        ] {
            let base = |command: Option<&str>, models: Vec<ModelDef>| EngineConfig {
                providers: vec![ProviderConfig {
                    name: format!("{kind}-provider"),
                    kind: kind.into(),
                    base_url: String::new(),
                    command: command.map(String::from),
                    models,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let a_model = || {
                vec![ModelDef {
                    name: "fixture".into(),
                    capability: match kind {
                        "local_tts" => "tts",
                        "local_asr" => "asr",
                        "local_video_gen" => "video_gen",
                        _ => "image_gen",
                    }
                    .into(),
                    ..Default::default()
                }]
            };

            // Missing command → rejected.
            assert!(
                base(None, a_model()).validate().is_err(),
                "{kind} without a command should fail validation"
            );
            // Empty model list → rejected.
            assert!(
                base(Some("some-cli"), vec![]).validate().is_err(),
                "{kind} with no models should fail validation"
            );
            // Command + at least one model → valid.
            base(Some("some-cli"), a_model())
                .validate()
                .unwrap_or_else(|e| panic!("{kind} with command and model should validate: {e}"));
        }
    }

    #[test]
    fn validate_rejects_unknown_provider_kind() {
        let cfg = EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig::default(),
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            artifacts: ArtifactConfig::default(),
            security: SecurityConfig::default(),
            providers: vec![ProviderConfig {
                name: "bad".into(),
                kind: "bad_kind".into(),
                base_url: "http://localhost".into(),
                api_key: None,
                priority: 1,
                cost_tier: "free".into(),
                models: vec![],
                enabled: true,
                ..Default::default()
            }],
        };
        assert!(cfg.validate().is_err());
    }
}
