//! The main engine orchestrator.
//!
//! `Engine` ties together providers, routing, discovery, health, lifecycle,
//! circuit breaking, reservation-based memory admission, concurrency control,
//! idle eviction, and observability.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use mofa_kernel::{
    BackendHealth, BackendStatus, Capability, CostTier, EngineError, EngineEvent, EngineStatus,
    FallbackPolicy, InferenceRequest, InferenceResponse, ModelCard, ModelResidency, Provider,
    ProviderHealth, ProviderKind,
};
use serde::Serialize;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast};
use tokio::task::{AbortHandle, JoinHandle};

use crate::backends::{OllamaProvider, OpenAiCompatProvider};
use crate::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerRegistry, CircuitState};
use crate::config::{EngineConfig, PreflightConfig, TimeoutConfig};
use crate::memory::{AllocationSnapshot, MemoryManager};
use crate::preflight::{GLOBAL_SCOPE, PreflightMetrics, PreflightPredictor, PreflightStats};
use crate::router::{RouteDecision, Router, RoutingProvider};
use crate::subscription::{SubscriptionInfo, SubscriptionRegistry};

/// Maximum number of lifecycle records retained in the rolling history.
const LIFECYCLE_CAPACITY: usize = 256;
/// Maximum number of in-flight predictions tracked for hit/miss accounting.
/// Bounds memory against an unbounded stream of unique scope identifiers.
const MAX_PENDING_PREDICTIONS: usize = 4096;

#[derive(Clone)]
struct RegisteredProvider {
    name: String,
    kind: ProviderKind,
    priority: u8,
    provider: Arc<dyn Provider>,
}

/// A single entry in the model lifecycle history.
#[derive(Debug, Clone, Serialize)]
pub struct LifecycleRecord {
    /// Monotonic sequence number.
    pub seq: u64,
    /// Milliseconds since engine start (monotonic).
    pub at_ms: u64,
    /// Affected model.
    pub model_id: String,
    /// Event kind: `load`, `unload`, `evict`, `idle_unload`, `load_failed`, etc.
    pub event: String,
    /// Optional human-readable detail.
    pub detail: Option<String>,
}

/// A snapshot of the engine's memory accounting.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryReport {
    /// Bytes reserved across all models.
    pub used_bytes: u64,
    /// Total budget.
    pub budget_bytes: u64,
    /// Bytes still free within the budget.
    pub available_bytes: u64,
    /// Per-model allocations.
    pub allocations: Vec<AllocationSnapshot>,
}

/// The main MoFA Engine orchestrator.
pub struct Engine {
    /// Named providers.
    providers: Vec<RegisteredProvider>,
    /// Cached model cards.
    models: DashMap<String, ModelCard>,
    /// Latest backend health by provider.
    backend_health: DashMap<String, BackendHealth>,
    /// Reservation-based memory manager.
    memory: MemoryManager,
    /// Circuit breaker registry.
    circuit_breakers: CircuitBreakerRegistry,
    /// Preflight predictor (app/session-scoped transition history).
    preflight: PreflightPredictor,
    /// Preflight effectiveness counters.
    preflight_metrics: PreflightMetrics,
    /// Active capability subscriptions.
    subscriptions: SubscriptionRegistry,
    /// In-flight speculative warm tasks, keyed by model id (dedup + cancel).
    warming: DashMap<String, AbortHandle>,
    /// Per-scope pending prediction awaiting confirmation by the next request.
    pending_predictions: DashMap<String, Capability>,
    /// Operation timeouts.
    timeouts: TimeoutConfig,
    /// Preflight configuration.
    preflight_config: PreflightConfig,
    /// Idle timeout before a resident model is auto-unloaded.
    idle_timeout: Duration,
    /// Per-model concurrency admission.
    semaphores: DashMap<String, Arc<Semaphore>>,
    /// Serializes the evict-then-reserve admission critical section.
    load_gate: AsyncMutex<()>,
    /// Rolling lifecycle history.
    lifecycle: Mutex<VecDeque<LifecycleRecord>>,
    /// Lifecycle sequence counter.
    lifecycle_seq: AtomicU64,
    /// Background idle-eviction task; aborted on drop.
    idle_task: Mutex<Option<JoinHandle<()>>>,
    /// Weak self-reference for background tasks.
    weak_self: OnceLock<Weak<Engine>>,
    /// Event broadcast channel.
    event_tx: broadcast::Sender<EngineEvent>,
    /// Engine start time.
    started_at: Instant,
}

/// Remaining time until `deadline`, saturating at zero.
fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
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
        let idle_timeout = Duration::from_secs(config.memory.idle_timeout_secs);

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
            preflight_metrics: PreflightMetrics::default(),
            subscriptions: SubscriptionRegistry::new(),
            warming: DashMap::new(),
            pending_predictions: DashMap::new(),
            timeouts: config.timeouts.clone(),
            preflight_config: config.preflight.clone(),
            idle_timeout,
            semaphores: DashMap::new(),
            load_gate: AsyncMutex::new(()),
            lifecycle: Mutex::new(VecDeque::with_capacity(LIFECYCLE_CAPACITY)),
            lifecycle_seq: AtomicU64::new(0),
            idle_task: Mutex::new(None),
            weak_self: OnceLock::new(),
            event_tx,
            started_at: Instant::now(),
        });
        let _ = engine.weak_self.set(Arc::downgrade(&engine));

        engine.refresh_resources().await;
        engine.spawn_idle_eviction();
        Ok(engine)
    }

    /// Refresh provider health and model discovery.
    pub async fn refresh_resources(&self) {
        self.refresh_health().await;
        self.discover_all().await;
    }

    /// Discover models from all providers with bounded per-provider timeouts.
    async fn discover_all(&self) {
        let discovery_timeout = self.timeouts.discovery();
        let handles = self
            .providers
            .iter()
            .map(|registered| {
                let name = registered.name.clone();
                let provider = Arc::clone(&registered.provider);
                tokio::spawn(async move {
                    let result = tokio::time::timeout(discovery_timeout, provider.discover())
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
                    let had_stale = !stale.is_empty();
                    for id in stale {
                        self.models.remove(&id);
                        self.memory.deallocate(&id);
                        self.semaphores.remove(&id);
                        self.warming.remove(&id);
                    }
                    if had_stale {
                        self.emit_memory_changed();
                    }

                    let count = cards.len();
                    let mut freed = false;
                    for mut card in cards {
                        // Reconcile with the engine's current view. `.map` drops the
                        // shard guard immediately so the later insert cannot deadlock.
                        let previous = self.models.get(&card.id).map(|c| c.residency);
                        match previous {
                            // A load is in flight; don't let discovery regress it.
                            Some(ModelResidency::Loading) => {
                                card.residency = ModelResidency::Loading;
                            }
                            // The backend reports the model is no longer resident, so
                            // release the reservation we still held for it.
                            Some(ModelResidency::Loaded)
                                if !matches!(card.residency, ModelResidency::Loaded) =>
                            {
                                self.memory.deallocate(&card.id);
                                freed = true;
                            }
                            _ => {}
                        }
                        card.refresh_status();
                        self.models.insert(card.id.clone(), card);
                    }
                    if freed {
                        self.emit_memory_changed();
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
        let health_timeout = self.timeouts.health();
        let handles = self
            .providers
            .iter()
            .map(|registered| {
                let name = registered.name.clone();
                let provider = Arc::clone(&registered.provider);
                tokio::spawn(async move {
                    let health = tokio::time::timeout(health_timeout, provider.health())
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
        self.models_snapshot()
    }

    /// Synchronous snapshot of all model cards, sorted by id.
    fn models_snapshot(&self) -> Vec<ModelCard> {
        let mut models = self
            .models
            .iter()
            .map(|e| e.value().clone())
            .collect::<Vec<_>>();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    /// Run inference.
    ///
    /// Builds a single ranked candidate plan and walks it in order: the first
    /// candidate is the primary selection, and each subsequent one is a failover.
    /// Only *retryable* failures advance to the next candidate, so malformed or
    /// unsupported requests fail immediately rather than masquerading as
    /// transient errors. The candidate list itself encodes the fallback policy.
    pub async fn invoke(&self, req: InferenceRequest) -> Result<InferenceResponse, EngineError> {
        self.reject_ambiguous_short_name(&req)?;
        let overall_deadline = Instant::now() + self.timeouts.request();

        // Confirm the previous request's prediction for this scope before a new
        // one is formed during this request's admission.
        self.confirm_prediction(&req);

        let all_models = self.capabilities().await;
        let providers = self.routing_providers();
        let candidates = self.build_candidates(&all_models, &req, &providers);

        if candidates.is_empty() {
            return Err(EngineError::NoCapableModel(Self::requested_capability(
                &req,
            )));
        }

        let mut last_err: Option<EngineError> = None;
        for (idx, decision) in candidates.iter().enumerate() {
            if remaining(overall_deadline).is_zero() {
                last_err = Some(EngineError::Timeout(
                    "overall request deadline exceeded".into(),
                ));
                break;
            }

            let model_id = decision.model.id.clone();
            let provider_name = decision.model.provider.clone();
            let max_concurrency = decision.model.execution.max_concurrency;

            match self
                .try_invoke(
                    &model_id,
                    &provider_name,
                    max_concurrency,
                    &req,
                    decision.reason.clone(),
                    overall_deadline,
                )
                .await
            {
                Ok(mut resp) => {
                    resp.fallback_used = idx > 0;
                    return Ok(resp);
                }
                Err(e) => {
                    if !e.retryable() {
                        // Invalid request, unsupported operation, etc. — never fall over.
                        return Err(e);
                    }
                    tracing::warn!("candidate '{model_id}' failed (retryable): {e}");
                    last_err = Some(e);
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| EngineError::NoCapableModel(Self::requested_capability(&req))))
    }

    fn requested_capability(req: &InferenceRequest) -> String {
        req.capability
            .map(|c| c.to_string())
            .unwrap_or_else(|| "any".into())
    }

    /// Build the ordered candidate plan, applying the request's fallback policy.
    fn build_candidates<'a>(
        &self,
        all_models: &'a [ModelCard],
        req: &InferenceRequest,
        providers: &[RoutingProvider],
    ) -> Vec<RouteDecision<'a>> {
        let budget = Some(self.memory.budget_bytes());
        let primary = Router::route_ranked(all_models, req, providers, budget);

        match (req.model.is_some(), req.fallback_policy) {
            // Named request, strict (default) or fallback disabled: only the named model.
            (true, FallbackPolicy::CapabilityOnly | FallbackPolicy::Disabled) => primary,
            // Named request with explicit opt-in: the named model first, then any
            // capability-compatible candidate.
            (true, FallbackPolicy::AllowNamed) => {
                let mut cands = primary;
                let mut cap_req = req.clone();
                cap_req.model = None;
                for d in Router::route_ranked(all_models, &cap_req, providers, budget) {
                    if !cands.iter().any(|c| c.model.id == d.model.id) {
                        cands.push(d);
                    }
                }
                cands
            }
            // Capability request, fallback disabled: only the single best.
            (false, FallbackPolicy::Disabled) => primary.into_iter().take(1).collect(),
            // Capability request, default/allow: the full ranked list.
            (false, _) => primary,
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

    /// Attempt one candidate: circuit check → ensure loaded → admit concurrency
    /// → invoke, with phase-bounded timeouts that respect the overall deadline.
    async fn try_invoke(
        &self,
        model_id: &str,
        provider_name: &str,
        max_concurrency: u32,
        req: &InferenceRequest,
        routing_reason: String,
        overall_deadline: Instant,
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
            self.emit_request_completed(&req.request_id, 0, false);
            return Err(e);
        }

        // Concurrency admission with a bounded queue wait.
        let sem = self.semaphore_for(model_id, max_concurrency);
        let queue_budget = remaining(overall_deadline).min(self.timeouts.queue());
        let permit = match tokio::time::timeout(queue_budget, sem.acquire_owned()).await {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => {
                self.emit_request_completed(&req.request_id, 0, false);
                return Err(EngineError::Internal("concurrency semaphore closed".into()));
            }
            Err(_) => {
                self.emit_request_completed(&req.request_id, 0, false);
                return Err(EngineError::Timeout(format!(
                    "queue wait exceeded for '{model_id}'"
                )));
            }
        };

        self.begin_execution(model_id);

        // Kick off predictive warming of the *next* model concurrently with this
        // inference, so a hinted/predicted model is hot by the time it is needed.
        self.trigger_preflight(req);

        let inference_budget = remaining(overall_deadline).min(self.timeouts.inference());
        let start = Instant::now();
        let result = tokio::time::timeout(inference_budget, provider.invoke(model_id, req)).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        self.end_execution(model_id);
        drop(permit);

        match result {
            Ok(Ok(mut resp)) => {
                resp.routing_reason = Some(routing_reason);
                self.circuit_breakers.record_success(provider_name);
                self.record_transition(req);
                self.emit_request_completed(&req.request_id, duration_ms, true);
                Ok(resp)
            }
            Ok(Err(e)) => {
                self.circuit_breakers.record_failure(provider_name);
                self.emit_request_completed(&req.request_id, duration_ms, false);
                Err(e)
            }
            Err(_) => {
                self.circuit_breakers.record_failure(provider_name);
                self.emit_request_completed(
                    &req.request_id,
                    inference_budget.as_millis() as u64,
                    false,
                );
                Err(EngineError::Timeout(format!(
                    "provider '{provider_name}' did not respond within {}s",
                    inference_budget.as_secs()
                )))
            }
        }
    }

    /// Ensure a model is resident, reserving memory before loading.
    ///
    /// The evict-then-reserve step runs under `load_gate` so concurrent loads
    /// cannot jointly overcommit memory; the slow backend load then runs outside
    /// the gate so independent models can load in parallel.
    async fn ensure_loaded(
        &self,
        model_id: &str,
        provider_name: &str,
        provider: &Arc<dyn Provider>,
    ) -> Result<(), EngineError> {
        if self.is_resident(model_id) {
            self.memory.touch(model_id);
            return Ok(());
        }

        let estimate = self
            .models
            .get(model_id)
            .map(|m| m.memory_estimate_bytes)
            .unwrap_or(0);

        {
            let _gate = self.load_gate.lock().await;
            // Another task may have loaded this model while we waited for the gate.
            if self.is_resident(model_id) {
                self.memory.touch(model_id);
                return Ok(());
            }
            if estimate > 0 {
                self.admit_memory(model_id, estimate).await?;
            }
            self.set_model_residency(model_id, ModelResidency::Loading);
        }

        let load_result =
            match tokio::time::timeout(self.timeouts.load(), provider.load(model_id)).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    self.circuit_breakers.record_failure(provider_name);
                    self.rollback_reservation(model_id, estimate);
                    self.record_lifecycle(model_id, "load_failed", Some(&e.to_string()));
                    return Err(e);
                }
                Err(_) => {
                    self.circuit_breakers.record_failure(provider_name);
                    self.rollback_reservation(model_id, estimate);
                    self.record_lifecycle(model_id, "load_timeout", None);
                    return Err(EngineError::Timeout(format!(
                        "load timeout for {provider_name}/{model_id}"
                    )));
                }
            };

        self.set_model_residency(model_id, load_result.residency);
        if matches!(load_result.residency, ModelResidency::Loaded) {
            if let Some(observed) = load_result.memory_bytes
                && observed > 0
            {
                self.memory.reconcile(model_id, observed);
            }
            // Mark freshly loaded so it is not the immediate LRU victim before its
            // first lease is taken.
            self.memory.touch(model_id);
            self.emit_memory_changed();
            self.record_lifecycle(model_id, "load", None);
        }
        Ok(())
    }

    /// Reserve `bytes` for `model_id`, evicting idle models until it fits.
    /// Caller must hold `load_gate`.
    async fn admit_memory(&self, model_id: &str, bytes: u64) -> Result<(), EngineError> {
        if self.memory.try_reserve(model_id, bytes) {
            return Ok(());
        }
        loop {
            let mut protected = self.eviction_protected_ids();
            protected.push(model_id.to_string());
            let Some(victim) = self.memory.lru_candidate(&protected) else {
                break;
            };
            self.unload_model(&victim, "memory_pressure").await;
            if self.memory.try_reserve(model_id, bytes) {
                return Ok(());
            }
        }
        Err(EngineError::MemoryPressure {
            need: bytes,
            available: self.memory.available_bytes(),
        })
    }

    fn rollback_reservation(&self, model_id: &str, estimate: u64) {
        if estimate > 0 {
            self.memory.deallocate(model_id);
        }
        self.set_model_residency(model_id, ModelResidency::Unloaded);
        self.emit_memory_changed();
    }

    /// Unload a model from a backend and release its accounting.
    async fn unload_model(&self, model_id: &str, reason: &str) {
        let provider = self
            .models
            .get(model_id)
            .and_then(|card| self.find_provider(&card.provider));
        if let Some(provider) = provider {
            let _ = provider.unload(model_id).await;
        }
        // Frees the reservation and emits residency/memory events.
        self.set_model_residency(model_id, ModelResidency::Unloaded);
        let event = if reason == "idle_timeout" {
            "idle_unload"
        } else {
            "evict"
        };
        self.record_lifecycle(model_id, event, Some(reason));
        let _ = self.event_tx.send(EngineEvent::ModelEvicted {
            model_id: model_id.to_string(),
            reason: reason.to_string(),
        });
        tracing::info!("unloaded '{model_id}' ({reason})");
    }

    /// Model IDs that must never be evicted by memory pressure or the idle sweep.
    ///
    /// Leased (in-flight) models are already excluded inside the memory manager;
    /// this additionally protects:
    /// - models whose load is currently in flight (`Loading`), so a concurrent
    ///   admission cannot unload a model out from under an active load and leave
    ///   its reservation orphaned; and
    /// - resident models whose capability is covered by an active subscription
    ///   (the RFC "resident keep-alive" policy).
    ///
    /// If every candidate is protected, admission returns structured memory
    /// pressure rather than breaking a subscription or a load.
    fn eviction_protected_ids(&self) -> Vec<String> {
        let subscribed = self.subscriptions.active_capabilities();
        self.models
            .iter()
            .filter(|card| {
                matches!(card.residency, ModelResidency::Loading)
                    || (matches!(card.residency, ModelResidency::Loaded)
                        && !subscribed.is_empty()
                        && card.capabilities.iter().any(|cap| subscribed.contains(cap)))
            })
            .map(|card| card.key().clone())
            .collect()
    }

    // ----- Preflight / subscriptions -------------------------------------------------

    /// Scope key for history and predictions: prefer session, then app, else global.
    fn scope_key(req: &InferenceRequest) -> String {
        req.session_id
            .clone()
            .or_else(|| req.app_id.clone())
            .unwrap_or_else(|| GLOBAL_SCOPE.to_string())
    }

    fn weak(&self) -> Option<Weak<Engine>> {
        self.weak_self.get().cloned()
    }

    /// Confirm (hit/miss) the prediction made for this scope by the *previous*
    /// request, against the capability the current request actually asked for.
    /// Runs once per request, before a new prediction is made.
    fn confirm_prediction(&self, req: &InferenceRequest) {
        let Some(cap) = req.capability else {
            return;
        };
        let scope = Self::scope_key(req);
        if let Some((_, predicted)) = self.pending_predictions.remove(&scope) {
            if predicted == cap {
                self.preflight_metrics.hit();
            } else {
                self.preflight_metrics.miss();
            }
        }
    }

    /// Remember a prediction for later hit/miss confirmation, bounding the map
    /// so caller-supplied scope keys cannot grow it without limit.
    fn remember_prediction(&self, scope: String, capability: Capability) {
        if self.pending_predictions.len() >= MAX_PENDING_PREDICTIONS
            && !self.pending_predictions.contains_key(&scope)
            && let Some(victim) = self
                .pending_predictions
                .iter()
                .next()
                .map(|e| e.key().clone())
        {
            self.pending_predictions.remove(&victim);
        }
        self.pending_predictions.insert(scope, capability);
    }

    /// Record a completed capability into the scope's transition history.
    fn record_transition(&self, req: &InferenceRequest) {
        if !self.preflight_config.history_learning {
            return;
        }
        if let Some(cap) = req.capability {
            self.preflight.record(&Self::scope_key(req), cap);
        }
    }

    /// Choose which capability (if any) to speculatively warm for this request,
    /// in priority order: explicit hint, then subscription, then learned history.
    fn select_warm_capability(&self, req: &InferenceRequest) -> Option<(Capability, &'static str)> {
        if let Some(hint) = req
            .hint_next
            .as_deref()
            .and_then(Capability::from_str_loose)
        {
            return Some((hint, "hint"));
        }
        if let Some(cap) = self.first_subscribed_without_resident() {
            return Some((cap, "subscription"));
        }
        if self.preflight_config.history_learning
            && let Some(current) = req.capability
        {
            let scope = Self::scope_key(req);
            if let Some(pred) = self.preflight.predict(
                &scope,
                current,
                self.preflight_config.min_samples,
                self.preflight_config.confidence_threshold,
            ) {
                self.preflight_metrics.prediction();
                self.remember_prediction(scope, pred.capability);
                return Some((pred.capability, "history"));
            }
        }
        None
    }

    /// A subscribed capability that currently has no resident model, if any.
    fn first_subscribed_without_resident(&self) -> Option<Capability> {
        let mut subscribed: Vec<Capability> = self
            .subscriptions
            .active_capabilities()
            .into_iter()
            .collect();
        // Deterministic order so warming is reproducible.
        subscribed.sort_by_key(|c| c.to_string());
        subscribed
            .into_iter()
            .find(|cap| !self.has_resident_for(*cap))
    }

    fn has_resident_for(&self, cap: Capability) -> bool {
        self.models.iter().any(|card| {
            card.supports(cap)
                && matches!(
                    card.residency,
                    ModelResidency::Loaded | ModelResidency::Remote
                )
        })
    }

    /// Decide and launch speculative warming for the request, if enabled.
    fn trigger_preflight(&self, req: &InferenceRequest) {
        if !self.preflight_config.enabled || !self.preflight_config.speculative_warming {
            return;
        }
        if let Some((cap, source)) = self.select_warm_capability(req) {
            self.warm_capability(cap, source, req.app_id.clone(), req.session_id.clone());
        }
    }

    /// Route a capability to its best model and warm it on a background task.
    ///
    /// Speculative loads go through the same reservation/eviction admission as
    /// regular requests (via `ensure_loaded`), are deduplicated per model, and
    /// are skipped when the model is already resident.
    fn warm_capability(
        &self,
        cap: Capability,
        source: &str,
        app_id: Option<String>,
        session_id: Option<String>,
    ) {
        if !self.preflight_config.enabled || !self.preflight_config.speculative_warming {
            return;
        }

        let all = self.models_snapshot();
        let providers = self.routing_providers();
        let route_req = InferenceRequest {
            capability: Some(cap),
            model: None,
            app_id,
            session_id,
            fallback_policy: FallbackPolicy::CapabilityOnly,
            messages: Vec::new(),
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: String::new(),
        };
        let budget = Some(self.memory.budget_bytes());
        let Some(best) = Router::route_ranked(&all, &route_req, &providers, budget)
            .into_iter()
            .next()
        else {
            return;
        };

        let model_id = best.model.id.clone();
        // Already hot (loaded locally or cloud-backed) → nothing to warm.
        if self.is_resident(&model_id) {
            return;
        }
        // A load is already in flight for this model (driven by a request or an
        // earlier warm) → don't pile on a redundant concurrent load.
        if self
            .models
            .get(&model_id)
            .is_some_and(|c| c.residency == ModelResidency::Loading)
        {
            self.preflight_metrics.warm_skipped();
            return;
        }
        // Deduplicate: skip if a warm for this model is still running. We key on
        // `is_finished()` rather than mere presence so a completed task's handle
        // (left in the map intentionally) never permanently blocks future warms.
        if self
            .warming
            .get(&model_id)
            .is_some_and(|h| !h.is_finished())
        {
            self.preflight_metrics.warm_skipped();
            return;
        }
        let provider_name = best.model.provider.clone();
        let Some(provider) = self.find_provider(&provider_name) else {
            return;
        };
        let Some(weak) = self.weak() else {
            return;
        };

        self.preflight_metrics.warm_started();
        let _ = self.event_tx.send(EngineEvent::PreflightWarmStarted {
            model_id: model_id.clone(),
            source: source.to_string(),
        });

        let warm_id = model_id.clone();
        let handle = tokio::spawn(async move {
            let Some(engine) = weak.upgrade() else {
                return;
            };
            let ok = engine
                .ensure_loaded(&warm_id, &provider_name, &provider)
                .await
                .is_ok();
            // The handle is intentionally left in `warming`; dedup checks
            // `is_finished()`, and the next warm of this model overwrites it.
            if ok {
                engine.preflight_metrics.warm_completed();
            } else {
                engine.preflight_metrics.warm_failed();
            }
            let _ = engine.event_tx.send(EngineEvent::PreflightWarmCompleted {
                model_id: warm_id,
                success: ok,
            });
        });
        self.warming.insert(model_id, handle.abort_handle());
    }

    /// Register a capability subscription and immediately warm its models.
    ///
    /// Returns the new subscription id.
    pub fn subscribe(
        &self,
        app_id: Option<String>,
        session_id: Option<String>,
        capabilities: Vec<Capability>,
        ttl: Option<Duration>,
    ) -> u64 {
        let id = self.subscriptions.subscribe(
            app_id.clone(),
            session_id.clone(),
            capabilities.clone(),
            ttl,
        );
        for cap in capabilities {
            self.warm_capability(cap, "subscription", app_id.clone(), session_id.clone());
        }
        id
    }

    /// Remove a subscription by id. Returns whether one existed.
    pub fn unsubscribe(&self, id: u64) -> bool {
        self.subscriptions.unsubscribe(id)
    }

    /// List all active subscriptions.
    pub fn subscriptions(&self) -> Vec<SubscriptionInfo> {
        self.subscriptions.list()
    }

    /// Snapshot of Preflight effectiveness counters.
    pub fn preflight_stats(&self) -> PreflightStats {
        self.preflight_metrics.snapshot()
    }

    /// Background task: unload models idle beyond the configured timeout.
    fn spawn_idle_eviction(&self) {
        if self.idle_timeout.is_zero() {
            return;
        }
        let Some(weak) = self.weak_self.get().cloned() else {
            return;
        };
        // Tick often enough to act soon after the timeout without busy-looping.
        let tick = Duration::from_secs((self.idle_timeout.as_secs().max(2) / 2).clamp(1, 15));
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tick);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let Some(engine) = weak.upgrade() else {
                    break;
                };
                engine.run_idle_sweep().await;
            }
        });
        *self.idle_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    /// Unload every model that has been idle past the timeout.
    async fn run_idle_sweep(&self) {
        let protected = self.eviction_protected_ids();
        let victims = self.memory.idle_candidates(self.idle_timeout, &protected);
        for victim in victims {
            // Re-check under no lock: only unload models that are genuinely
            // resident and not leased right now.
            let resident = self
                .models
                .get(&victim)
                .map(|c| matches!(c.residency, ModelResidency::Loaded))
                .unwrap_or(false);
            if !resident || self.memory.lease_count(&victim) > 0 {
                continue;
            }
            self.unload_model(&victim, "idle_timeout").await;
        }
    }

    fn is_resident(&self, model_id: &str) -> bool {
        self.models
            .get(model_id)
            .map(|m| matches!(m.residency, ModelResidency::Loaded | ModelResidency::Remote))
            .unwrap_or(false)
    }

    fn semaphore_for(&self, model_id: &str, max_concurrency: u32) -> Arc<Semaphore> {
        let permits = max_concurrency.max(1) as usize;
        self.semaphores
            .entry(model_id.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(permits)))
            .clone()
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
            let old_status = card.status;
            card.execution.active_requests = card.execution.active_requests.saturating_add(1);
            card.refresh_status();
            if old_status != card.status {
                let _ = self.event_tx.send(EngineEvent::ModelStatusChanged {
                    model_id: model_id.to_string(),
                    old: old_status,
                    new: card.status,
                });
            }
        }
        // The memory lease protects this model from eviction for the duration.
        self.memory.lease(model_id);
    }

    fn end_execution(&self, model_id: &str) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            let old_status = card.status;
            card.execution.active_requests = card.execution.active_requests.saturating_sub(1);
            card.refresh_status();
            if old_status != card.status {
                let _ = self.event_tx.send(EngineEvent::ModelStatusChanged {
                    model_id: model_id.to_string(),
                    old: old_status,
                    new: card.status,
                });
            }
        }
        self.memory.release_lease(model_id);
    }

    fn set_model_residency(&self, model_id: &str, new_residency: ModelResidency) {
        if let Some(mut card) = self.models.get_mut(model_id) {
            let old_residency = card.residency;
            let old_status = card.status;
            if old_residency != new_residency {
                card.residency = new_residency;
                card.refresh_status();
                let new_status = card.status;
                // Release the shard guard before touching the memory manager / event bus.
                drop(card);
                let _ = self.event_tx.send(EngineEvent::ModelResidencyChanged {
                    model_id: model_id.to_string(),
                    old: old_residency,
                    new: new_residency,
                });
                if old_status != new_status {
                    let _ = self.event_tx.send(EngineEvent::ModelStatusChanged {
                        model_id: model_id.to_string(),
                        old: old_status,
                        new: new_status,
                    });
                }
                // A model that stops being resident releases its memory budget.
                if old_residency == ModelResidency::Loaded
                    && new_residency != ModelResidency::Loaded
                {
                    self.memory.deallocate(model_id);
                    self.emit_memory_changed();
                }
            }
        }
    }

    fn emit_memory_changed(&self) {
        let _ = self.event_tx.send(EngineEvent::MemoryChanged {
            used_bytes: self.memory.used_bytes(),
            total_bytes: self.memory.budget_bytes(),
        });
    }

    fn emit_request_completed(&self, request_id: &str, duration_ms: u64, success: bool) {
        let _ = self.event_tx.send(EngineEvent::RequestCompleted {
            request_id: request_id.to_string(),
            duration_ms,
            success,
        });
    }

    fn record_lifecycle(&self, model_id: &str, event: &str, detail: Option<&str>) {
        let record = LifecycleRecord {
            seq: self.lifecycle_seq.fetch_add(1, Ordering::Relaxed),
            at_ms: self.started_at.elapsed().as_millis() as u64,
            model_id: model_id.to_string(),
            event: event.to_string(),
            detail: detail.map(ToString::to_string),
        };
        let mut hist = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if hist.len() >= LIFECYCLE_CAPACITY {
            hist.pop_front();
        }
        hist.push_back(record);
    }

    /// Return the rolling lifecycle history, oldest first.
    pub fn lifecycle_history(&self) -> Vec<LifecycleRecord> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Return a snapshot of the engine's memory accounting.
    pub fn memory_report(&self) -> MemoryReport {
        MemoryReport {
            used_bytes: self.memory.used_bytes(),
            budget_bytes: self.memory.budget_bytes(),
            available_bytes: self.memory.available_bytes(),
            allocations: self.memory.snapshot(),
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

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(handle) = self
            .idle_task
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            handle.abort();
        }
        // Cancel any in-flight speculative warm tasks.
        for entry in self.warming.iter() {
            entry.value().abort();
        }
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
    use std::sync::atomic::AtomicUsize;

    use crate::config::{
        EngineConfig, ListenConfig, MemoryConfig, PreflightConfig, ProviderConfig, TimeoutConfig,
    };

    const MB: u64 = 1024 * 1024;

    fn minimal_config() -> EngineConfig {
        EngineConfig {
            listen: ListenConfig::default(),
            memory: MemoryConfig {
                budget_mb: Some(100),
                idle_timeout_secs: 60,
            },
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
            providers: vec![],
        }
    }

    /// Behavior knobs for the configurable test provider.
    #[derive(Clone, Copy)]
    enum InvokeBehavior {
        Ok,
        SlowOkMs(u64),
        FailRetryable,
        FailInvalid,
    }

    struct TestProvider {
        name: String,
        kind: ProviderKind,
        /// (model_name, capability, residency, mem_bytes, max_concurrency)
        template: Vec<(String, Capability, ModelResidency, u64, u32)>,
        behavior: InvokeBehavior,
        load_should_fail: bool,
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
        invoke_calls: Arc<AtomicUsize>,
    }

    impl TestProvider {
        fn new(name: &str, kind: ProviderKind) -> Self {
            Self {
                name: name.into(),
                kind,
                template: Vec::new(),
                behavior: InvokeBehavior::Ok,
                load_should_fail: false,
                in_flight: Arc::new(AtomicUsize::new(0)),
                max_in_flight: Arc::new(AtomicUsize::new(0)),
                invoke_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_model(
            mut self,
            name: &str,
            cap: Capability,
            residency: ModelResidency,
            mem_bytes: u64,
            max_concurrency: u32,
        ) -> Self {
            self.template
                .push((name.into(), cap, residency, mem_bytes, max_concurrency));
            self
        }

        fn behavior(mut self, behavior: InvokeBehavior) -> Self {
            self.behavior = behavior;
            self
        }

        fn failing_load(mut self) -> Self {
            self.load_should_fail = true;
            self
        }
    }

    #[async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn kind(&self) -> ProviderKind {
            self.kind
        }
        fn features(&self) -> Vec<BackendFeature> {
            vec![BackendFeature::Discovery, BackendFeature::Load]
        }

        async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
            let cards = self
                .template
                .iter()
                .map(|(name, cap, residency, mem, conc)| {
                    let mut card = ModelCard::new(&self.name, name, *cap, CostTier::Low);
                    card.id = canonical_model_id(&self.name, name);
                    card.availability = ModelAvailability::Discovered;
                    card.residency = *residency;
                    card.memory_estimate_bytes = *mem;
                    card.execution = ExecutionState {
                        active_requests: 0,
                        max_concurrency: *conc,
                    };
                    card.refresh_status();
                    card
                })
                .collect();
            Ok(cards)
        }

        async fn health(&self) -> Result<BackendHealth, EngineError> {
            Ok(BackendHealth::Healthy)
        }

        async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
            if self.load_should_fail {
                return Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: "load failed (test)".into(),
                });
            }
            let residency = if self.kind == ProviderKind::Ollama {
                ModelResidency::Loaded
            } else {
                ModelResidency::Remote
            };
            let mem = self
                .template
                .iter()
                .find(|(n, ..)| canonical_model_id(&self.name, n) == model_id)
                .map(|(.., mem, _)| *mem);
            Ok(LifecycleResult {
                model_id: model_id.into(),
                residency,
                memory_bytes: mem,
                changed: true,
            })
        }

        async fn unload(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
            Ok(LifecycleResult {
                model_id: model_id.into(),
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
            self.invoke_calls.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);

            let behavior = self.behavior;
            let result = match behavior {
                InvokeBehavior::Ok => Ok(()),
                InvokeBehavior::SlowOkMs(ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(())
                }
                InvokeBehavior::FailRetryable => Err(EngineError::ProviderError {
                    provider: self.name.clone(),
                    detail: "boom".into(),
                }),
                InvokeBehavior::FailInvalid => Err(EngineError::InvalidRequest("bad input".into())),
            };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            result?;

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

    fn build_engine(
        providers: Vec<Arc<dyn Provider>>,
        budget_mb: u64,
        idle_secs: u64,
    ) -> Arc<Engine> {
        let (event_tx, _) = broadcast::channel(256);
        let registered = providers
            .into_iter()
            .map(|provider| RegisteredProvider {
                name: provider.name().into(),
                kind: provider.kind(),
                priority: if provider.kind() == ProviderKind::Ollama {
                    1
                } else {
                    10
                },
                provider,
            })
            .collect();
        let engine = Arc::new(Engine {
            providers: registered,
            models: DashMap::new(),
            backend_health: DashMap::new(),
            memory: MemoryManager::new(Some(budget_mb)),
            circuit_breakers: CircuitBreakerRegistry::new(CircuitBreakerConfig::default()),
            preflight: PreflightPredictor::new(),
            preflight_metrics: PreflightMetrics::default(),
            subscriptions: SubscriptionRegistry::new(),
            warming: DashMap::new(),
            pending_predictions: DashMap::new(),
            timeouts: TimeoutConfig::default(),
            preflight_config: PreflightConfig::default(),
            idle_timeout: Duration::from_secs(idle_secs),
            semaphores: DashMap::new(),
            load_gate: AsyncMutex::new(()),
            lifecycle: Mutex::new(VecDeque::new()),
            lifecycle_seq: AtomicU64::new(0),
            idle_task: Mutex::new(None),
            weak_self: OnceLock::new(),
            event_tx,
            started_at: Instant::now(),
        });
        let _ = engine.weak_self.set(Arc::downgrade(&engine));
        engine.spawn_idle_eviction();
        engine
    }

    fn chat_request(model: Option<&str>) -> InferenceRequest {
        request(Capability::Chat, model, FallbackPolicy::default())
    }

    fn request(
        capability: Capability,
        model: Option<&str>,
        fallback_policy: FallbackPolicy,
    ) -> InferenceRequest {
        InferenceRequest {
            capability: Some(capability),
            model: model.map(str::to_owned),
            app_id: None,
            session_id: None,
            fallback_policy,
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
        assert!(engine.capabilities().await.is_empty());
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
        match rx.recv().await.unwrap() {
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
            timeouts: TimeoutConfig::default(),
            preflight: PreflightConfig::default(),
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
        assert_eq!(engine.status().await.providers, 0);
    }

    #[tokio::test]
    async fn discovers_and_invokes_deterministically() {
        let provider = Arc::new(
            TestProvider::new("cloud", ProviderKind::OpenAiCompatible).with_model(
                "mock-chat",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                4,
            ),
        );
        let engine = build_engine(vec![provider], 100, 0);
        engine.refresh_resources().await;

        let caps = engine.capabilities().await;
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].id, "cloud/mock-chat");

        let resp = engine.invoke(chat_request(None)).await.unwrap();
        assert_eq!(resp.text.as_deref(), Some("ok"));
        assert_eq!(resp.provider, "cloud");
        assert!(resp.routing_reason.is_some());
        assert!(!resp.fallback_used);
    }

    #[tokio::test]
    async fn ambiguous_short_model_name_is_rejected() {
        let engine = build_engine(vec![], 100, 0);
        for prov in ["a", "b"] {
            let mut card = ModelCard::new(prov, "same", Capability::Chat, CostTier::Free);
            card.residency = ModelResidency::Remote;
            card.refresh_status();
            engine.models.insert(card.id.clone(), card);
        }
        let err = engine.invoke(chat_request(Some("same"))).await.unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn capability_routing_prefers_local() {
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Unloaded,
                10 * MB,
                1,
            ),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "gpt",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let resp = engine.invoke(chat_request(None)).await.unwrap();
        assert_eq!(resp.model_used, "qwen");
        assert!(!resp.fallback_used);
    }

    #[tokio::test]
    async fn named_request_routes_exactly() {
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Loaded,
                10 * MB,
                1,
            ),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "gpt-4o",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let resp = engine.invoke(chat_request(Some("gpt-4o"))).await.unwrap();
        assert_eq!(resp.model_used, "gpt-4o");
        assert_eq!(resp.provider, "openai");
    }

    #[tokio::test]
    async fn retryable_failure_falls_back_to_next_candidate() {
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model("qwen", Capability::Chat, ModelResidency::Loaded, 10 * MB, 1)
                .behavior(InvokeBehavior::FailRetryable),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible)
                .with_model("gpt", Capability::Chat, ModelResidency::Remote, 0, 32)
                .behavior(InvokeBehavior::Ok),
        );
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let resp = engine.invoke(chat_request(None)).await.unwrap();
        assert_eq!(resp.provider, "openai");
        assert!(resp.fallback_used);
    }

    #[tokio::test]
    async fn invalid_request_does_not_fall_back() {
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model("qwen", Capability::Chat, ModelResidency::Loaded, 10 * MB, 1)
                .behavior(InvokeBehavior::FailInvalid),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible)
                .with_model("gpt", Capability::Chat, ModelResidency::Remote, 0, 32)
                .behavior(InvokeBehavior::Ok),
        );
        let cloud_calls = cloud.invoke_calls.clone();
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let err = engine.invoke(chat_request(None)).await.unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
        // The cloud candidate must never have been tried.
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn named_strict_does_not_fall_back() {
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model("qwen", Capability::Chat, ModelResidency::Loaded, 10 * MB, 1)
                .behavior(InvokeBehavior::FailRetryable),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible)
                .with_model("gpt", Capability::Chat, ModelResidency::Remote, 0, 32)
                .behavior(InvokeBehavior::Ok),
        );
        let cloud_calls = cloud.invoke_calls.clone();
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        // Default policy is strict for named requests.
        let err = engine.invoke(chat_request(Some("qwen"))).await.unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        assert_eq!(cloud_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn ten_concurrent_requests_respect_concurrency_limit() {
        let provider = Arc::new(
            TestProvider::new("cloud", ProviderKind::OpenAiCompatible)
                .with_model("m", Capability::Chat, ModelResidency::Remote, 0, 4)
                .behavior(InvokeBehavior::SlowOkMs(40)),
        );
        let max_seen = provider.max_in_flight.clone();
        let engine = build_engine(vec![provider], 1000, 0);
        engine.refresh_resources().await;

        let mut handles = Vec::new();
        for _ in 0..10 {
            let engine = Arc::clone(&engine);
            handles.push(tokio::spawn(async move {
                engine.invoke(chat_request(None)).await
            }));
        }
        for h in handles {
            assert!(h.await.unwrap().is_ok());
        }
        // The semaphore must have capped concurrency at the model's limit.
        assert!(
            max_seen.load(Ordering::SeqCst) <= 4,
            "over-admitted requests"
        );
    }

    #[tokio::test]
    async fn memory_pressure_evicts_lru_idle_model() {
        let provider = Arc::new(TestProvider::new("ollama", ProviderKind::Ollama));
        let engine = build_engine(vec![provider], 100, 0); // 100 MB budget

        for name in ["old", "new"] {
            let mut card = ModelCard::new("ollama", name, Capability::Chat, CostTier::Low);
            card.residency = ModelResidency::Loaded;
            card.refresh_status();
            let id = card.id.clone();
            engine.models.insert(id.clone(), card);
            engine.memory.allocate(&id, 40 * MB);
        }
        engine.memory.touch("ollama/new"); // make "old" the LRU victim
        assert_eq!(engine.memory.used_bytes(), 80 * MB);

        // Admitting 40 MB needs 20 MB freed → evict exactly the LRU model.
        engine
            .admit_memory("ollama/incoming", 40 * MB)
            .await
            .unwrap();

        assert_eq!(
            engine.models.get("ollama/old").unwrap().residency,
            ModelResidency::Unloaded
        );
        assert_eq!(
            engine.models.get("ollama/new").unwrap().residency,
            ModelResidency::Loaded
        );
        assert!(engine.memory.lease_count("ollama/incoming") == 0);
    }

    #[tokio::test]
    async fn leased_models_are_never_evicted() {
        let provider = Arc::new(TestProvider::new("ollama", ProviderKind::Ollama));
        let engine = build_engine(vec![provider], 100, 0);

        let mut card = ModelCard::new("ollama", "busy", Capability::Chat, CostTier::Low);
        card.residency = ModelResidency::Loaded;
        card.refresh_status();
        let id = card.id.clone();
        engine.models.insert(id.clone(), card);
        engine.memory.allocate(&id, 90 * MB);
        engine.memory.lease(&id); // in-flight inference

        // Cannot evict the only (leased) model → memory pressure.
        let err = engine
            .admit_memory("ollama/incoming", 40 * MB)
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::MemoryPressure { .. }));
        assert_eq!(
            engine.models.get("ollama/busy").unwrap().residency,
            ModelResidency::Loaded
        );
    }

    #[tokio::test]
    async fn load_failure_rolls_back_reservation() {
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model(
                    "qwen",
                    Capability::Chat,
                    ModelResidency::Unloaded,
                    10 * MB,
                    1,
                )
                .failing_load(),
        );
        let engine = build_engine(vec![provider], 100, 0);
        engine.refresh_resources().await;

        let err = engine.invoke(chat_request(None)).await.unwrap_err();
        assert!(matches!(err, EngineError::ProviderError { .. }));
        // The reservation must have been rolled back.
        assert_eq!(engine.memory.used_bytes(), 0);
        assert_eq!(
            engine.models.get("ollama/qwen").unwrap().residency,
            ModelResidency::Unloaded
        );
    }

    #[tokio::test]
    async fn successful_load_reserves_and_records_memory() {
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Unloaded,
                20 * MB,
                1,
            ),
        );
        let engine = build_engine(vec![provider], 100, 0);
        engine.refresh_resources().await;

        engine.invoke(chat_request(None)).await.unwrap();
        assert_eq!(engine.memory.used_bytes(), 20 * MB);
        assert_eq!(
            engine.models.get("ollama/qwen").unwrap().residency,
            ModelResidency::Loaded
        );
        // A load event must be in the lifecycle history.
        assert!(
            engine
                .lifecycle_history()
                .iter()
                .any(|r| r.model_id == "ollama/qwen" && r.event == "load")
        );
    }

    #[tokio::test]
    async fn loading_model_is_not_evicted() {
        let provider = Arc::new(TestProvider::new("ollama", ProviderKind::Ollama));
        let engine = build_engine(vec![provider], 100, 0);

        // A model whose load is in flight, holding most of the budget.
        let mut card = ModelCard::new("ollama", "loading", Capability::Chat, CostTier::Low);
        card.residency = ModelResidency::Loading;
        card.refresh_status();
        let id = card.id.clone();
        engine.models.insert(id.clone(), card);
        engine.memory.allocate(&id, 90 * MB);

        // It must be protected, so admission cannot satisfy the request.
        assert!(engine.eviction_protected_ids().contains(&id));
        let err = engine
            .admit_memory("ollama/incoming", 40 * MB)
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::MemoryPressure { .. }));
        assert_eq!(
            engine.models.get(&id).unwrap().residency,
            ModelResidency::Loading
        );
    }

    #[tokio::test]
    async fn rediscovery_releases_reservation_for_unloaded_model() {
        // The provider reports the model as Unloaded on discovery.
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Unloaded,
                10 * MB,
                1,
            ),
        );
        let engine = build_engine(vec![provider], 100, 0);

        // The engine currently believes the model is loaded and reserved.
        let mut card = ModelCard::new("ollama", "qwen", Capability::Chat, CostTier::Low);
        card.residency = ModelResidency::Loaded;
        card.refresh_status();
        engine.models.insert(card.id.clone(), card);
        engine.memory.allocate("ollama/qwen", 10 * MB);
        assert_eq!(engine.memory.used_bytes(), 10 * MB);

        // Rediscovery reconciles: the backend says Unloaded, so the reservation
        // is released to match reality.
        engine.refresh_resources().await;
        assert_eq!(
            engine.models.get("ollama/qwen").unwrap().residency,
            ModelResidency::Unloaded
        );
        assert_eq!(engine.memory.used_bytes(), 0);
    }

    async fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        // Up to ~4s, returning as soon as the condition holds.
        for _ in 0..400 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cond()
    }

    fn scoped_request(cap: Capability, app: &str, session: &str) -> InferenceRequest {
        let mut r = request(cap, None, FallbackPolicy::default());
        r.app_id = Some(app.into());
        r.session_id = Some(session.into());
        r
    }

    #[tokio::test]
    async fn hint_warms_next_model_concurrently() {
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model("qwen", Capability::Chat, ModelResidency::Loaded, 10 * MB, 1)
                .with_model(
                    "kokoro",
                    Capability::Tts,
                    ModelResidency::Unloaded,
                    10 * MB,
                    1,
                ),
        );
        let engine = build_engine(vec![provider], 1000, 0);
        engine.refresh_resources().await;

        let mut req = chat_request(None);
        req.hint_next = Some("tts".into());
        engine.invoke(req).await.unwrap();

        let engine_ref = Arc::clone(&engine);
        let warmed = wait_until(move || engine_ref.is_resident("ollama/kokoro")).await;
        assert!(warmed, "the hinted TTS model should have been warmed");
        assert!(engine.preflight_stats().warms_started >= 1);
    }

    #[tokio::test]
    async fn subscription_warms_and_protects_model() {
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "kokoro",
                Capability::Tts,
                ModelResidency::Unloaded,
                10 * MB,
                1,
            ),
        );
        let engine = build_engine(vec![provider], 1000, 1);
        engine.refresh_resources().await;

        engine.subscribe(Some("mofa-fm".into()), None, vec![Capability::Tts], None);

        let engine_ref = Arc::clone(&engine);
        let warmed = wait_until(move || engine_ref.is_resident("ollama/kokoro")).await;
        assert!(warmed, "subscription should warm its capability's model");
        // A subscribed resident model is protected from eviction.
        assert!(
            engine
                .eviction_protected_ids()
                .contains(&"ollama/kokoro".to_string())
        );
    }

    #[tokio::test]
    async fn history_predicts_and_confirms_hits() {
        let chat = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Loaded,
                10 * MB,
                4,
            ),
        );
        let tts = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "tts-1",
                Capability::Tts,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let engine = build_engine(vec![chat, tts], 1000, 0);
        engine.refresh_resources().await;

        // Build a Chat → Tts habit within one session.
        for _ in 0..5 {
            engine
                .invoke(scoped_request(Capability::Chat, "fm", "s1"))
                .await
                .unwrap();
            engine
                .invoke(scoped_request(Capability::Tts, "fm", "s1"))
                .await
                .unwrap();
        }

        let stats = engine.preflight_stats();
        assert!(stats.predictions > 0, "history should produce predictions");
        assert!(stats.hits > 0, "predictions should be confirmed as hits");
    }

    /// The headline article-to-podcast flow across Stages 3-5: a hinted chat
    /// call warms TTS concurrently, the follow-up TTS call is served by the
    /// already-warm model, and both then unload on idle.
    #[tokio::test]
    async fn article_to_podcast_flow() {
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model(
                    "qwen",
                    Capability::Chat,
                    ModelResidency::Unloaded,
                    20 * MB,
                    1,
                )
                .with_model(
                    "kokoro",
                    Capability::Tts,
                    ModelResidency::Unloaded,
                    10 * MB,
                    1,
                ),
        );
        let engine = build_engine(vec![provider], 1000, 1);
        engine.refresh_resources().await;

        // Step 1: LLM translation, hinting that TTS is next.
        let mut chat = scoped_request(Capability::Chat, "mofa-fm", "s1");
        chat.hint_next = Some("tts".into());
        let chat_resp = engine.invoke(chat).await.unwrap();
        assert_eq!(chat_resp.model_used, "qwen");

        // The hint should have warmed kokoro concurrently.
        let engine_ref = Arc::clone(&engine);
        assert!(
            wait_until(move || engine_ref.is_resident("ollama/kokoro")).await,
            "TTS should be warmed by the hint before it is requested"
        );

        // Step 2: TTS synthesis — served by the already-resident model.
        let tts_resp = engine
            .invoke(scoped_request(Capability::Tts, "mofa-fm", "s1"))
            .await
            .unwrap();
        assert_eq!(tts_resp.model_used, "kokoro");
        assert!(!tts_resp.fallback_used);
        assert!(engine.preflight_stats().warms_started >= 1);

        // Step 3: both models unload once idle, releasing memory.
        let engine_ref = Arc::clone(&engine);
        assert!(
            wait_until(move || engine_ref.memory.used_bytes() == 0).await,
            "idle models should unload and release memory"
        );
    }

    #[tokio::test]
    async fn idle_sweep_unloads_stale_models() {
        let provider = Arc::new(TestProvider::new("ollama", ProviderKind::Ollama));
        let engine = build_engine(vec![provider], 100, 1); // 1s idle timeout

        let mut card = ModelCard::new("ollama", "qwen", Capability::Chat, CostTier::Low);
        card.residency = ModelResidency::Loaded;
        card.refresh_status();
        let id = card.id.clone();
        engine.models.insert(id.clone(), card);
        engine.memory.allocate(&id, 30 * MB);

        // The idle timeout uses monotonic last-access; wait past it then sweep.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        engine.run_idle_sweep().await;

        assert_eq!(
            engine.models.get(&id).unwrap().residency,
            ModelResidency::Unloaded
        );
        assert_eq!(engine.memory.used_bytes(), 0);
        assert!(
            engine
                .lifecycle_history()
                .iter()
                .any(|r| r.event == "idle_unload")
        );
    }
}
