//! The main engine orchestrator.
//!
//! `Engine` ties together providers, routing, discovery, health, lifecycle,
//! circuit breaking, memory accounting, and observability.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use mofa_kernel::{
    BackendHealth, BackendStatus, CostTier, EngineError, EngineEvent, EngineStatus, FallbackPolicy,
    InferenceRequest, InferenceResponse, ModelCard, ModelResidency, Provider, ProviderHealth,
    ProviderKind,
};
use tokio::sync::broadcast;

use crate::backends::{OllamaProvider, OpenAiCompatProvider};
use crate::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerRegistry, CircuitState};
use crate::config::EngineConfig;
use crate::memory::MemoryManager;
use crate::preflight::PreflightPredictor;
use crate::router::{Router, RoutingProvider};

#[derive(Clone)]
struct RegisteredProvider {
    name: String,
    kind: ProviderKind,
    priority: u8,
    provider: Arc<dyn Provider>,
}

/// The main MoFA Engine orchestrator.
pub struct Engine {
    /// Named providers.
    providers: Vec<RegisteredProvider>,
    /// Cached model cards.
    models: DashMap<String, ModelCard>,
    /// Latest backend health by provider.
    backend_health: DashMap<String, BackendHealth>,
    /// Memory manager.
    memory: MemoryManager,
    /// Circuit breaker registry.
    circuit_breakers: CircuitBreakerRegistry,
    /// Preflight predictor.
    preflight: PreflightPredictor,
    /// Event broadcast channel.
    event_tx: broadcast::Sender<EngineEvent>,
    /// Engine start time.
    started_at: Instant,
}

impl Engine {
    /// Create and initialize a new engine from configuration.
    ///
    /// Panics if configuration is invalid. Application entrypoints should prefer
    /// `try_new`; tests keep using this helper for concise setup.
    pub async fn new(config: EngineConfig) -> Arc<Self> {
        Self::try_new(config)
            .await
            .expect("engine configuration should be valid")
    }

    /// Create and initialize a new engine from validated configuration.
    pub async fn try_new(config: EngineConfig) -> Result<Arc<Self>, EngineError> {
        config.validate()?;

        let (event_tx, _) = broadcast::channel(256);
        let memory = MemoryManager::new(config.memory.budget_mb);
        let circuit_breakers = CircuitBreakerRegistry::new(CircuitBreakerConfig::default());
        let preflight = PreflightPredictor::new();

        let mut providers = Vec::new();
        for pc in &config.providers {
            if !pc.enabled {
                tracing::info!("provider '{}' is disabled, skipping", pc.name);
                continue;
            }

            let kind = pc.provider_kind()?;
            let cost_tier = CostTier::from_str_loose(&pc.cost_tier);
            let provider: Arc<dyn Provider> = match kind {
                ProviderKind::Ollama => Arc::new(OllamaProvider::new(&pc.name, &pc.base_url)),
                ProviderKind::OpenAiCompatible => Arc::new(OpenAiCompatProvider::new(
                    &pc.name,
                    &pc.base_url,
                    pc.api_key.clone().unwrap_or_default(),
                    pc.models.clone(),
                    cost_tier,
                )),
                _ => {
                    return Err(EngineError::Config(format!(
                        "provider '{}' uses unsupported provider kind",
                        pc.name
                    )));
                }
            };

            providers.push(RegisteredProvider {
                name: pc.name.clone(),
                kind,
                priority: pc.priority,
                provider,
            });
        }

        let engine = Arc::new(Self {
            providers,
            models: DashMap::new(),
            backend_health: DashMap::new(),
            memory,
            circuit_breakers,
            preflight,
            event_tx,
            started_at: Instant::now(),
        });

        engine.refresh_resources().await;
        Ok(engine)
    }

    /// Refresh provider health and model discovery.
    pub async fn refresh_resources(&self) {
        self.refresh_health().await;
        self.discover_all().await;
    }

    /// Discover models from all providers with bounded per-provider timeouts.
    async fn discover_all(&self) {
        let handles = self
            .providers
            .iter()
            .map(|registered| {
                let name = registered.name.clone();
                let provider = Arc::clone(&registered.provider);
                tokio::spawn(async move {
                    let result = tokio::time::timeout(Duration::from_secs(8), provider.discover())
                        .await
                        .map_err(|_| EngineError::Timeout(format!("discovery timeout for {name}")))
                        .and_then(|inner| inner);
                    (name, result)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let Ok((name, result)) = handle.await else {
                continue;
            };

            match result {
                Ok(cards) => {
                    let discovered_ids = cards.iter().map(|c| c.id.clone()).collect::<HashSet<_>>();
                    let stale = self
                        .models
                        .iter()
                        .filter(|entry| {
                            entry.provider == name && !discovered_ids.contains(entry.key())
                        })
                        .map(|entry| entry.key().clone())
                        .collect::<Vec<_>>();
                    for id in stale {
                        self.models.remove(&id);
                    }

                    let count = cards.len();
                    for mut card in cards {
                        card.refresh_status();
                        self.models.insert(card.id.clone(), card);
                    }
                    let _ = self.event_tx.send(EngineEvent::DiscoveryCompleted {
                        provider: name.clone(),
                        models: count,
                        success: true,
                    });
                    tracing::info!("discovered {count} models from '{name}'");
                }
                Err(e) => {
                    tracing::warn!("discovery from '{name}' failed: {e}");
                    let _ = self.event_tx.send(EngineEvent::DiscoveryCompleted {
                        provider: name,
                        models: 0,
                        success: false,
                    });
                }
            }
        }
    }

    async fn refresh_health(&self) {
        let handles = self
            .providers
            .iter()
            .map(|registered| {
                let name = registered.name.clone();
                let provider = Arc::clone(&registered.provider);
                tokio::spawn(async move {
                    let health = tokio::time::timeout(Duration::from_secs(5), provider.health())
                        .await
                        .map_err(|_| EngineError::Timeout(format!("health timeout for {name}")))
                        .and_then(|inner| inner)
                        .unwrap_or(BackendHealth::Unavailable);
                    (name, health)
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            let Ok((name, health)) = handle.await else {
                continue;
            };
            let previous = self.backend_health.insert(name.clone(), health);
            if previous != Some(health) {
                let _ = self.event_tx.send(EngineEvent::ProviderHealthChanged {
                    provider: name,
                    health,
                });
            }
        }
    }

    /// Return all known model cards.
    pub async fn capabilities(&self) -> Vec<ModelCard> {
        let mut models = self
            .models
            .iter()
            .map(|e| e.value().clone())
            .collect::<Vec<_>>();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    /// Run inference: route → circuit-check → load → invoke → optional fallback.
    pub async fn invoke(&self, req: InferenceRequest) -> Result<InferenceResponse, EngineError> {
        self.reject_ambiguous_short_name(&req)?;

        let all_models = self.capabilities().await;
        let providers = self.routing_providers();
        let selected = Router::route(&all_models, &req, &providers).ok_or_else(|| {
            let cap_str = req
                .capability
                .map(|c| c.to_string())
                .unwrap_or_else(|| "any".into());
            EngineError::NoCapableModel(cap_str)
        })?;

        let model_id = selected.model.id.clone();
        let provider_name = selected.model.provider.clone();
        let routing_reason = selected.reason.clone();

        match self
            .try_invoke(&model_id, &provider_name, &req, Some(routing_reason))
            .await
        {
            Ok(resp) => Ok(resp),
            Err(primary_err) if self.can_fallback(&req) => {
                tracing::warn!("primary model '{model_id}' failed: {primary_err}, trying fallback");
                let mut fallback_req = req.clone();
                fallback_req.model = None;
                let fallback_models = self
                    .capabilities()
                    .await
                    .into_iter()
                    .filter(|m| m.id != model_id && m.provider != provider_name)
                    .collect::<Vec<_>>();
                let Some(fallback) = Router::route(&fallback_models, &fallback_req, &providers)
                else {
                    return Err(primary_err);
                };
                let fb_id = fallback.model.id.clone();
                let fb_provider = fallback.model.provider.clone();
                let mut resp = self
                    .try_invoke(&fb_id, &fb_provider, &fallback_req, Some(fallback.reason))
                    .await
                    .map_err(|_| primary_err)?;
                resp.fallback_used = true;
                Ok(resp)
            }
            Err(primary_err) => Err(primary_err),
        }
    }

    fn reject_ambiguous_short_name(&self, req: &InferenceRequest) -> Result<(), EngineError> {
        let Some(target) = req.model.as_deref() else {
            return Ok(());
        };
        // The `::` form is always an unambiguous qualifier.
        if target.contains("::") {
            return Ok(());
        }
        // Treat `provider/model` as qualified only when the prefix is a registered provider.
        if let Some((maybe_provider, _)) = target.split_once('/')
            && self.providers.iter().any(|p| p.name == maybe_provider)
        {
            return Ok(());
        }
        let matches = self
            .models
            .iter()
            .filter(|entry| entry.name == target)
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(EngineError::InvalidRequest(format!(
                "model name '{target}' is ambiguous; use one of: {}",
                matches.join(", ")
            )));
        }
        Ok(())
    }

    fn can_fallback(&self, req: &InferenceRequest) -> bool {
        match (req.model.is_some(), req.fallback_policy) {
            (_, FallbackPolicy::Disabled) => false,
            (true, FallbackPolicy::CapabilityOnly) => false,
            (true, FallbackPolicy::AllowNamed) => true,
            (false, _) => true,
        }
    }

    /// Attempt to invoke a specific model, handling circuit breaker and loading.
    async fn try_invoke(
        &self,
        model_id: &str,
        provider_name: &str,
        req: &InferenceRequest,
        routing_reason: Option<String>,
    ) -> Result<InferenceResponse, EngineError> {
        if !self.circuit_breakers.allow_request(provider_name) {
            return Err(EngineError::CircuitOpen(provider_name.into()));
        }

        let provider = self.find_provider(provider_name).ok_or_else(|| {
            EngineError::Internal(format!("provider '{provider_name}' not found"))
        })?;

        let _ = self.event_tx.send(EngineEvent::RequestStarted {
            request_id: req.request_id.clone(),
            capability: req.capability,
            model_id: model_id.to_string(),
        });

        if let Err(e) = self.ensure_loaded(model_id, provider_name, &provider).await {
            let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                request_id: req.request_id.clone(),
                duration_ms: 0,
                success: false,
            });
            return Err(e);
        }
        self.begin_execution(model_id);

        let start = Instant::now();
        let invoke_timeout = Duration::from_secs(180);
        let result = tokio::time::timeout(invoke_timeout, provider.invoke(model_id, req)).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        self.end_execution(model_id);

        match result {
            Ok(Ok(mut resp)) => {
                resp.routing_reason = routing_reason;
                self.circuit_breakers.record_success(provider_name);
                if let Some(cap) = req.capability {
                    self.preflight.record(cap);
                }
                let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                    request_id: req.request_id.clone(),
                    duration_ms,
                    success: true,
                });
                Ok(resp)
            }
            Ok(Err(e)) => {
                self.circuit_breakers.record_failure(provider_name);
                let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                    request_id: req.request_id.clone(),
                    duration_ms,
                    success: false,
                });
                Err(e)
            }
            Err(_) => {
                self.circuit_breakers.record_failure(provider_name);
                let _ = self.event_tx.send(EngineEvent::RequestCompleted {
                    request_id: req.request_id.clone(),
                    duration_ms: invoke_timeout.as_millis() as u64,
                    success: false,
                });
                Err(EngineError::Timeout(format!(
                    "provider '{}' did not respond within {}s",
                    provider_name,
                    invoke_timeout.as_secs()
                )))
            }
        }
    }

    async fn ensure_loaded(
        &self,
        model_id: &str,
        provider_name: &str,
        provider: &Arc<dyn Provider>,
    ) -> Result<(), EngineError> {
        let residency = self
            .models
            .get(model_id)
            .map(|m| m.residency)
            .unwrap_or(ModelResidency::Unknown);
        if matches!(residency, ModelResidency::Loaded | ModelResidency::Remote) {
            return Ok(());
        }

        self.set_model_residency(model_id, ModelResidency::Loading);
        let load_result = tokio::time::timeout(Duration::from_secs(30), provider.load(model_id))
            .await
            .map_err(|_| {
                self.circuit_breakers.record_failure(provider_name);
                self.set_model_residency(model_id, ModelResidency::Unloaded);
                EngineError::Timeout(format!("load timeout for {provider_name}/{model_id}"))
            })?
            .inspect_err(|_| {
                self.circuit_breakers.record_failure(provider_name);
                self.set_model_residency(model_id, ModelResidency::Unloaded);
            })?;

        self.set_model_residency(model_id, load_result.residency);
        if matches!(load_result.residency, ModelResidency::Loaded)
            && let Some(card) = self.models.get(model_id)
        {
            let bytes = load_result
                .memory_bytes
                .unwrap_or(card.memory_estimate_bytes);
            if bytes > 0 {
                if !self.memory.can_fit(bytes) {
                    let _ = provider.unload(model_id).await;
                    self.set_model_residency(model_id, ModelResidency::Unloaded);
                    return Err(EngineError::MemoryPressure {
                        need: bytes,
                        available: self.memory.available_bytes(),
                    });
                }
                self.memory.allocate(model_id, bytes);
                let _ = self.event_tx.send(EngineEvent::MemoryChanged {
                    used_bytes: self.memory.used_bytes(),
                    total_bytes: self.memory.budget_bytes(),
                });
            }
        }
        Ok(())
    }

    fn find_provider(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .iter()
            .find(|registered| registered.name == name)
            .map(|registered| Arc::clone(&registered.provider))
    }

    fn routing_providers(&self) -> Vec<RoutingProvider> {
        self.providers
            .iter()
            .map(|registered| {
                let health = self
                    .backend_health
                    .get(&registered.name)
                    .map(|h| *h)
                    .unwrap_or(BackendHealth::Unknown);
                RoutingProvider {
                    name: registered.name.clone(),
                    kind: registered.kind,
                    priority: registered.priority,
                    health,
                    circuit_open: self.circuit_breakers.state(&registered.name)
                        == CircuitState::Open,
                }
            })
            .collect()
    }

    fn begin_execution(&self, model_id: &str) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            card.execution.active_requests = card.execution.active_requests.saturating_add(1);
            card.refresh_status();
        }
        self.memory.touch(model_id);
    }

    fn end_execution(&self, model_id: &str) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            card.execution.active_requests = card.execution.active_requests.saturating_sub(1);
            card.refresh_status();
        }
        self.memory.touch(model_id);
    }

    fn set_model_residency(&self, model_id: &str, new_residency: ModelResidency) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            let old_residency = card.residency;
            let old_status = card.status;
            if old_residency != new_residency {
                card.residency = new_residency;
                card.refresh_status();
                let _ = self.event_tx.send(EngineEvent::ModelResidencyChanged {
                    model_id: model_id.to_string(),
                    old: old_residency,
                    new: new_residency,
                });
                if old_status != card.status {
                    let _ = self.event_tx.send(EngineEvent::ModelStatusChanged {
                        model_id: model_id.to_string(),
                        old: old_status,
                        new: card.status,
                    });
                }
            }
        }
    }

    /// Get a snapshot of the engine status.
    pub async fn status(&self) -> EngineStatus {
        let all_models = self.capabilities().await;
        let total_models = all_models.len();
        let loaded_models = all_models
            .iter()
            .filter(|m| matches!(m.residency, ModelResidency::Loaded | ModelResidency::Remote))
            .count();

        let backends = self.backend_statuses();
        let provider_health = backends
            .iter()
            .map(|backend| ProviderHealth {
                name: backend.name.clone(),
                healthy: backend.health.is_routable()
                    && backend.circuit_state != CircuitState::Open.to_string(),
                circuit_state: backend.circuit_state.clone(),
            })
            .collect();

        EngineStatus {
            total_models,
            loaded_models,
            providers: self.providers.len(),
            memory_used_bytes: self.memory.used_bytes(),
            memory_budget_bytes: self.memory.budget_bytes(),
            uptime_secs: self.started_at.elapsed().as_secs(),
            provider_health,
            backends,
        }
    }

    /// Get backend status snapshots.
    pub fn backend_statuses(&self) -> Vec<BackendStatus> {
        self.providers
            .iter()
            .map(|registered| BackendStatus {
                name: registered.name.clone(),
                kind: registered.kind,
                health: self
                    .backend_health
                    .get(&registered.name)
                    .map(|h| *h)
                    .unwrap_or(BackendHealth::Unknown),
                circuit_state: self.circuit_breakers.state(&registered.name).to_string(),
                features: registered.provider.features(),
            })
            .collect()
    }

    /// Subscribe to engine events.
    pub fn subscribe_events(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    /// Engine uptime in seconds.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mofa_kernel::{
        BackendFeature, Capability, ExecutionState, LifecycleResult, Message, ModelAvailability,
        canonical_model_id,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[derive(Default)]
    struct MockProvider {
        name: String,
        discover_calls: AtomicUsize,
    }

    impl MockProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
                discover_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn kind(&self) -> ProviderKind {
            ProviderKind::OpenAiCompatible
        }

        fn features(&self) -> Vec<BackendFeature> {
            vec![BackendFeature::Discovery]
        }

        async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
            self.discover_calls.fetch_add(1, Ordering::SeqCst);
            let mut card = ModelCard::new(&self.name, "mock-chat", Capability::Chat, CostTier::Low);
            card.id = canonical_model_id(&self.name, "mock-chat");
            card.availability = ModelAvailability::Configured;
            card.residency = ModelResidency::Remote;
            card.execution = ExecutionState {
                active_requests: 0,
                max_concurrency: 4,
            };
            card.refresh_status();
            Ok(vec![card])
        }

        async fn health(&self) -> Result<BackendHealth, EngineError> {
            Ok(BackendHealth::Healthy)
        }

        async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
            Ok(LifecycleResult {
                model_id: model_id.into(),
                residency: ModelResidency::Remote,
                memory_bytes: Some(0),
                changed: false,
            })
        }

        async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
            Ok(LifecycleResult {
                model_id: model_id.into(),
                residency: ModelResidency::Remote,
                memory_bytes: Some(0),
                changed: false,
            })
        }

        async fn invoke(
            &self,
            model_id: &str,
            request: &InferenceRequest,
        ) -> Result<InferenceResponse, EngineError> {
            Ok(InferenceResponse {
                text: Some("ok".into()),
                file: None,
                model_used: mofa_kernel::model_id_name(model_id).into(),
                provider: self.name.clone(),
                duration_ms: 1,
                request_id: request.request_id.clone(),
                tokens_used: Some(1),
                fallback_used: false,
                routing_reason: None,
            })
        }
    }

    fn engine_with_provider(provider: Arc<dyn Provider>) -> Arc<Engine> {
        let (event_tx, _) = broadcast::channel(256);
        Arc::new(Engine {
            providers: vec![RegisteredProvider {
                name: provider.name().into(),
                kind: provider.kind(),
                priority: 1,
                provider,
            }],
            models: DashMap::new(),
            backend_health: DashMap::new(),
            memory: MemoryManager::new(Some(100)),
            circuit_breakers: CircuitBreakerRegistry::new(CircuitBreakerConfig::default()),
            preflight: PreflightPredictor::new(),
            event_tx,
            started_at: Instant::now(),
        })
    }

    fn chat_request(model: Option<&str>) -> InferenceRequest {
        InferenceRequest {
            capability: Some(Capability::Chat),
            model: model.map(str::to_owned),
            app_id: None,
            session_id: None,
            fallback_policy: FallbackPolicy::default(),
            messages: vec![Message {
                role: "user".into(),
                content: "hello".into(),
            }],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
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
        let result = engine.invoke(chat_request(None)).await;
        assert!(matches!(result, Err(EngineError::NoCapableModel(_))));
    }

    #[tokio::test]
    async fn event_subscription_works() {
        let engine = Engine::new(minimal_config()).await;
        let mut rx = engine.subscribe_events();

        let _ = engine.event_tx.send(EngineEvent::ProviderHealthChanged {
            provider: "test".into(),
            health: BackendHealth::Healthy,
        });

        let event = rx.recv().await.unwrap();
        match event {
            EngineEvent::ProviderHealthChanged { provider, health } => {
                assert_eq!(provider, "test");
                assert_eq!(health, BackendHealth::Healthy);
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

    #[tokio::test]
    async fn mock_provider_refresh_and_invoke_are_deterministic() {
        let provider = Arc::new(MockProvider::new("mock"));
        let engine = engine_with_provider(provider);
        engine.refresh_resources().await;

        let caps = engine.capabilities().await;
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].id, "mock/mock-chat");

        let resp = engine.invoke(chat_request(None)).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
        assert_eq!(resp.provider, "mock");
        assert!(resp.routing_reason.is_some());
    }

    #[tokio::test]
    async fn ambiguous_short_model_name_is_rejected() {
        let provider = Arc::new(MockProvider::new("mock"));
        let engine = engine_with_provider(provider);
        let mut a = ModelCard::new("a", "same", Capability::Chat, CostTier::Free);
        a.residency = ModelResidency::Remote;
        a.refresh_status();
        let mut b = ModelCard::new("b", "same", Capability::Chat, CostTier::Free);
        b.residency = ModelResidency::Remote;
        b.refresh_status();
        engine.models.insert(a.id.clone(), a);
        engine.models.insert(b.id.clone(), b);

        let err = engine.invoke(chat_request(Some("same"))).await.unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }
}
