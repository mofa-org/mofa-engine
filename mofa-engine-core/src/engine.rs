//! The main engine orchestrator.
//!
//! `Engine` ties together providers, routing, memory management,
//! circuit breaking, and preflight prediction into a single
//! `Arc<Engine>` that is shared across the HTTP server.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use mofa_kernel::{
    Capability, CostTier, EngineError, EngineEvent, EngineStatus, InferenceRequest,
    InferenceResponse, ModelCard, ModelStatus, Provider, ProviderHealth, ProviderKind,
};
use tokio::sync::broadcast;

use crate::backends::{OllamaProvider, OpenAiCompatProvider};
use crate::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerRegistry};
use crate::config::EngineConfig;
use crate::memory::MemoryManager;
use crate::preflight::PreflightPredictor;
use crate::router::Router;

/// The main MoFA Engine orchestrator.
pub struct Engine {
    /// Named providers
    providers: Vec<(String, Arc<dyn Provider>)>,
    /// Cached model cards (refreshed on discover)
    models: DashMap<String, ModelCard>,
    /// Provider kind lookup
    provider_kinds: Vec<(String, ProviderKind)>,
    /// Memory manager
    memory: MemoryManager,
    /// Circuit breaker registry
    circuit_breakers: CircuitBreakerRegistry,
    /// Preflight predictor
    preflight: PreflightPredictor,
    /// Event broadcast channel
    event_tx: broadcast::Sender<EngineEvent>,
    /// Engine start time
    started_at: Instant,
}

impl Engine {
    /// Create and initialize a new engine from configuration.
    pub async fn new(config: EngineConfig) -> Arc<Self> {
        let (event_tx, _) = broadcast::channel(256);
        let memory = MemoryManager::new(config.memory.budget_mb);
        let circuit_breakers =
            CircuitBreakerRegistry::new(CircuitBreakerConfig::default());
        let preflight = PreflightPredictor::new();

        let mut providers: Vec<(String, Arc<dyn Provider>)> = Vec::new();
        let mut provider_kinds: Vec<(String, ProviderKind)> = Vec::new();

        for pc in &config.providers {
            if !pc.enabled {
                tracing::info!("provider '{}' is disabled, skipping", pc.name);
                continue;
            }

            match pc.kind.as_str() {
                "ollama" => {
                    let p = OllamaProvider::new(&pc.name, &pc.base_url);
                    provider_kinds.push((pc.name.clone(), ProviderKind::Ollama));
                    providers.push((pc.name.clone(), Arc::new(p)));
                }
                "openai_compatible" => {
                    let key = pc.api_key.clone().unwrap_or_default();
                    let cost = CostTier::from_str_loose(&pc.cost_tier);
                    let p = OpenAiCompatProvider::new(
                        &pc.name,
                        &pc.base_url,
                        key,
                        pc.models.clone(),
                        cost,
                    );
                    provider_kinds
                        .push((pc.name.clone(), ProviderKind::OpenAiCompatible));
                    providers.push((pc.name.clone(), Arc::new(p)));
                }
                other => {
                    tracing::warn!("unknown provider kind '{other}', skipping");
                }
            }
        }

        let engine = Arc::new(Self {
            providers,
            models: DashMap::new(),
            provider_kinds,
            memory,
            circuit_breakers,
            preflight,
            event_tx,
            started_at: Instant::now(),
        });

        // Initial discovery
        engine.discover_all().await;

        engine
    }

    /// Discover models from all providers.
    async fn discover_all(&self) {
        for (name, provider) in &self.providers {
            tracing::info!("discovering models from provider '{name}'");
            let cards = provider.discover().await;
            let count = cards.len();
            for card in cards {
                self.models.insert(card.id.clone(), card);
            }
            tracing::info!("discovered {count} models from '{name}'");
        }
    }

    /// Return all known model cards.
    pub async fn capabilities(&self) -> Vec<ModelCard> {
        self.models.iter().map(|e| e.value().clone()).collect()
    }

    /// Run inference: route → circuit-check → warm → invoke → fallback.
    pub async fn invoke(
        &self,
        req: InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        let all_models: Vec<ModelCard> =
            self.models.iter().map(|e| e.value().clone()).collect();

        // Primary routing
        let selected = Router::select(&all_models, &req, &self.provider_kinds)
            .ok_or_else(|| {
                let cap_str = req
                    .capability
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "any".into());
                EngineError::NoCapableModel(cap_str)
            })?;

        let model_id = selected.id.clone();
        let provider_name = selected.provider.clone();

        // Try primary, then fallback
        match self.try_invoke(&model_id, &provider_name, &req).await {
            Ok(resp) => Ok(resp),
            Err(primary_err) => {
                tracing::warn!(
                    "primary model '{model_id}' failed: {primary_err}, trying fallback"
                );

                // Find a fallback from a different provider
                let fallback = all_models
                    .iter()
                    .filter(|m| m.provider != provider_name && m.id != model_id)
                    .filter(|m| {
                        req.capability
                            .is_none_or(|cap| m.capability == cap)
                    })
                    .filter(|m| {
                        m.status != ModelStatus::Failed && m.status != ModelStatus::Busy
                    })
                    .max_by_key(|m| match m.status {
                        ModelStatus::Hot => 3,
                        ModelStatus::Warming => 2,
                        ModelStatus::Cold => 1,
                        _ => 0,
                    });

                if let Some(fb) = fallback {
                    let fb_id = fb.id.clone();
                    let fb_provider = fb.provider.clone();
                    self.try_invoke(&fb_id, &fb_provider, &req)
                        .await
                        .map_err(|_| primary_err)
                } else {
                    Err(primary_err)
                }
            }
        }
    }

    /// Attempt to invoke a specific model, handling circuit breaker and warming.
    async fn try_invoke(
        &self,
        model_id: &str,
        provider_name: &str,
        req: &InferenceRequest,
    ) -> Result<InferenceResponse, EngineError> {
        // Circuit breaker check
        if !self.circuit_breakers.allow_request(provider_name) {
            return Err(EngineError::CircuitOpen(provider_name.into()));
        }

        let provider = self.find_provider(provider_name).ok_or_else(|| {
            EngineError::Internal(format!("provider '{provider_name}' not found"))
        })?;

        // Emit RequestStarted
        let _ = self.event_tx.send(EngineEvent::RequestStarted {
            request_id: req.request_id.clone(),
            capability: req.capability,
            model_id: model_id.to_string(),
        });

        // Ensure model is warm
        let status = self
            .models
            .get(model_id)
            .map(|m| m.status)
            .unwrap_or(ModelStatus::Cold);

        if status == ModelStatus::Cold {
            self.set_model_status(model_id, ModelStatus::Warming);
            provider.warm(model_id).await;
            self.set_model_status(model_id, ModelStatus::Hot);
        }

        // Mark busy
        self.set_model_status(model_id, ModelStatus::Busy);
        self.memory.touch(model_id);

        let start = Instant::now();
        let result = provider.invoke(model_id, req).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(_) => {
                self.circuit_breakers.record_success(provider_name);
                self.set_model_status(model_id, ModelStatus::Hot);

                // Record preflight transition
                if let Some(cap) = req.capability {
                    self.preflight.record(cap);
                }

                let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                    request_id: req.request_id.clone(),
                    duration_ms,
                    success: true,
                });

                // Process hint — warm predicted model in background
                if let Some(ref hint) = req.hint_next
                    && let Some(next_cap) = Capability::from_str_loose(hint) {
                        self.warm_for_capability(next_cap).await;
                    }
            }
            Err(_) => {
                self.circuit_breakers.record_failure(provider_name);
                self.set_model_status(model_id, ModelStatus::Hot); // back to Hot, not Failed (transient)

                let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                    request_id: req.request_id.clone(),
                    duration_ms,
                    success: false,
                });
            }
        }

        result
    }

    /// Find a provider by name.
    fn find_provider(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| Arc::clone(p))
    }

    /// Update a model's status and emit an event.
    fn set_model_status(&self, model_id: &str, new_status: ModelStatus) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            let old = card.status;
            if old != new_status {
                card.status = new_status;
                let _ = self.event_tx.send(EngineEvent::ModelStatusChanged {
                    model_id: model_id.to_string(),
                    old,
                    new: new_status,
                });
            }
        }
    }

    /// Pre-warm the best model for a given capability.
    async fn warm_for_capability(&self, capability: Capability) {
        let all_models: Vec<ModelCard> =
            self.models.iter().map(|e| e.value().clone()).collect();

        let candidate = all_models
            .iter().find(|m| m.capability == capability && m.status == ModelStatus::Cold);

        if let Some(model) = candidate
            && let Some(provider) = self.find_provider(&model.provider) {
                tracing::info!(
                    "preflight warming '{}' for {capability}",
                    model.id
                );
                let model_id = model.id.clone();
                self.set_model_status(&model_id, ModelStatus::Warming);
                provider.warm(&model_id).await;
                self.set_model_status(&model_id, ModelStatus::Hot);
            }
    }

    /// Get a snapshot of the engine status.
    pub async fn status(&self) -> EngineStatus {
        let all_models: Vec<ModelCard> =
            self.models.iter().map(|e| e.value().clone()).collect();

        let total_models = all_models.len();
        let loaded_models = all_models
            .iter()
            .filter(|m| m.status == ModelStatus::Hot || m.status == ModelStatus::Busy)
            .count();

        let providers_count = self.providers.len();
        let uptime_secs = self.started_at.elapsed().as_secs();

        let mut provider_health = Vec::new();
        for (name, _) in &self.providers {
            let state = self.circuit_breakers.state(name);
            provider_health.push(ProviderHealth {
                name: name.clone(),
                healthy: state == crate::circuit_breaker::CircuitState::Closed,
                circuit_state: state.to_string(),
            });
        }

        EngineStatus {
            total_models,
            loaded_models,
            providers: providers_count,
            memory_used_bytes: self.memory.used_bytes(),
            memory_budget_bytes: self.memory.budget_bytes(),
            uptime_secs,
            provider_health,
        }
    }

    /// Subscribe to engine events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    /// Get the listen configuration from the original config.
    /// (This is a convenience — the caller typically has the config already.)
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EngineConfig, ListenConfig, MemoryConfig, ProviderConfig};

    fn minimal_config() -> EngineConfig {
        EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig {
                budget_mb: Some(100),
                idle_timeout_secs: 60,
            },
            providers: vec![],
        }
    }

    #[tokio::test]
    async fn engine_starts_with_empty_config() {
        let engine = Engine::new(minimal_config()).await;
        let caps = engine.capabilities().await;
        assert!(caps.is_empty());
    }

    #[tokio::test]
    async fn status_reports_uptime() {
        let engine = Engine::new(minimal_config()).await;
        let status = engine.status().await;
        assert_eq!(status.total_models, 0);
        assert!(status.uptime_secs < 2);
    }

    #[tokio::test]
    async fn invoke_returns_no_capable_model() {
        let engine = Engine::new(minimal_config()).await;
        let req = InferenceRequest {
            capability: Some(Capability::Chat),
            model: None,
            messages: vec![],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
        };
        let result = engine.invoke(req).await;
        assert!(matches!(result, Err(EngineError::NoCapableModel(_))));
    }

    #[tokio::test]
    async fn event_subscription_works() {
        let engine = Engine::new(minimal_config()).await;
        let mut rx = engine.subscribe_events();

        // Send a test event manually
        let _ = engine.event_tx.send(EngineEvent::ProviderHealthChanged {
            provider: "test".into(),
            healthy: true,
        });

        let event = rx.recv().await.unwrap();
        match event {
            EngineEvent::ProviderHealthChanged { provider, healthy } => {
                assert_eq!(provider, "test");
                assert!(healthy);
            }
            _ => panic!("unexpected event"),
        }
    }

    #[tokio::test]
    async fn disabled_provider_is_skipped() {
        let config = EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig {
                budget_mb: Some(100),
                idle_timeout_secs: 60,
            },
            providers: vec![ProviderConfig {
                name: "disabled-ollama".into(),
                kind: "ollama".into(),
                base_url: "http://localhost:99999".into(),
                api_key: None,
                priority: 1,
                cost_tier: "free".into(),
                models: vec![],
                enabled: false,
            }],
        };
        let engine = Engine::new(config).await;
        let status = engine.status().await;
        assert_eq!(status.providers, 0);
    }
}
