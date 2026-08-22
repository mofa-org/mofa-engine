//! The main engine orchestrator.
//!
//! `Engine` ties together providers, routing, discovery, health, lifecycle,
//! circuit breaking, reservation-based memory admission, concurrency control,
//! idle eviction, and observability.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use mofa_kernel::{
    BackendHealth, BackendStatus, Capability, CostTier, DataClass, EngineError, EngineEvent,
    EngineStatus, FailedAttempt, FallbackPolicy, InferenceRequest, InferenceResponse, Message,
    ModelCard, ModelId, ModelResidency, Prefer, Provider, ProviderHealth, ProviderKind,
    ResponsesRequest, ResponsesResponse, StreamChunk, StreamDelta,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast, mpsc};
use tokio::task::{AbortHandle, JoinHandle};

use crate::backends::{
    CloudVideoGenProvider, LiterLLMProvider, LocalAsrProvider, LocalImageGenProvider,
    LocalTtsProvider, LocalVideoGenProvider, OllamaProvider, OpenAiCompatProvider,
    SystemTtsProvider,
};
use crate::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerRegistry, CircuitState};
use crate::config::{EngineConfig, PreflightConfig, TimeoutConfig};
use crate::memory::{AllocationSnapshot, MemoryManager};
use crate::metrics::{EngineMetrics, MetricsGauges};
use crate::preflight::{GLOBAL_SCOPE, PreflightMetrics, PreflightPredictor, PreflightStats};
use crate::router::{RouteDecision, Router, RoutingProvider};
use crate::subscription::{SubscriptionInfo, SubscriptionRegistry};

/// Maximum number of lifecycle records retained in the rolling history.
const LIFECYCLE_CAPACITY: usize = 256;
/// Maximum number of in-flight predictions tracked for hit/miss accounting.
/// Bounds memory against an unbounded stream of unique scope identifiers.
const MAX_PENDING_PREDICTIONS: usize = 4096;
/// Maximum number of stored Responses-API conversations. Each continued turn
/// mints a new id (so a caller can branch from any prior response), so this
/// bounds total memory by evicting the oldest conversation when full.
const MAX_CONVERSATIONS: usize = 4096;

#[derive(Clone)]
struct RegisteredProvider {
    name: String,
    kind: ProviderKind,
    priority: u8,
    provider: Arc<dyn Provider>,
}

/// A single entry in the model lifecycle history.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A stored Responses-API conversation: the full running message history and
/// when it was created (for oldest-first eviction).
struct StoredConversation {
    messages: Vec<Message>,
    created: Instant,
}

/// A snapshot of the engine's memory accounting.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Stored Responses-API conversations, keyed by response id. Bounded by
    /// [`MAX_CONVERSATIONS`] via oldest-first eviction.
    conversations: DashMap<String, StoredConversation>,
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
    /// Background artifact-cleanup task; aborted on drop.
    artifact_task: Mutex<Option<JoinHandle<()>>>,
    /// Weak self-reference for background tasks.
    weak_self: OnceLock<Weak<Engine>>,
    /// Event broadcast channel.
    event_tx: broadcast::Sender<EngineEvent>,
    /// Bounded-cardinality metrics counters.
    metrics: EngineMetrics,
    /// Per-provider token pricing: name → (USD/1k prompt, USD/1k completion).
    pricing: HashMap<String, (f64, f64)>,
    /// Canonicalized allowlist of roots for request `input_file` paths; empty
    /// means any local path is accepted.
    input_roots: Vec<std::path::PathBuf>,
    /// Engine start time.
    started_at: Instant,
}

/// Pure routing/cost helpers, grouped as private associated functions rather than
/// free functions so they share the `Engine` namespace with the logic that uses them.
impl Engine {
    /// Completion tokens assumed when a request does not cap them via
    /// `params.max_tokens` — a deliberately generous default so the pre-flight
    /// budget estimate errs toward *over*-counting rather than blowing the ceiling.
    const DEFAULT_COMPLETION_TOKEN_ESTIMATE: u32 = 1024;

    /// Remaining time until `deadline`, saturating at zero.
    fn remaining(deadline: Instant) -> Duration {
        deadline.saturating_duration_since(Instant::now())
    }

    /// USD cost from per-1k-token prices. `None` when both prices are zero (free /
    /// local models), so no spurious zero cost is attributed.
    fn estimate_cost(
        price_in_per_1k: f64,
        price_out_per_1k: f64,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    ) -> Option<f64> {
        if price_in_per_1k == 0.0 && price_out_per_1k == 0.0 {
            return None;
        }
        let prompt = prompt_tokens.unwrap_or(0) as f64;
        let completion = completion_tokens.unwrap_or(0) as f64;
        Some(prompt / 1000.0 * price_in_per_1k + completion / 1000.0 * price_out_per_1k)
    }

    /// Rough prompt-token estimate for pre-flight budgeting, using the ~4-chars-per
    /// token rule of thumb over the message contents. Approximate by design (the
    /// authoritative token count only exists post-call); a floor of 1 avoids a
    /// zero estimate for an empty prompt.
    fn estimate_prompt_tokens(req: &InferenceRequest) -> u32 {
        // Count Unicode scalar values, not bytes: a byte count would over-count
        // multi-byte (e.g. CJK) prompts several-fold and could spuriously price a
        // request out of its `max_cost_usd` ceiling.
        let chars: usize = req.messages.iter().map(|m| m.content.chars().count()).sum();
        ((chars / 4) as u32).max(1)
    }

    /// Completion-token estimate for pre-flight budgeting: the request's
    /// `params.max_tokens` cap when present, otherwise
    /// [`Self::DEFAULT_COMPLETION_TOKEN_ESTIMATE`].
    fn estimate_completion_tokens(req: &InferenceRequest) -> u32 {
        req.params
            .get("max_tokens")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(Self::DEFAULT_COMPLETION_TOKEN_ESTIMATE)
    }
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
                ProviderKind::Ollama => {
                    Arc::new(OllamaProvider::new(&pc.name, &pc.base_url, &pc.models)?)
                }
                ProviderKind::OpenAiCompatible => Arc::new(OpenAiCompatProvider::with_output_dir(
                    &pc.name,
                    &pc.base_url,
                    pc.api_key.clone().unwrap_or_default(),
                    pc.models.clone(),
                    cost_tier,
                    config.artifacts.dir.clone(),
                )?),
                ProviderKind::LiterLlm => Arc::new(LiterLLMProvider::new(
                    &pc.name,
                    pc.api_key.clone(),
                    &pc.base_url,
                    pc.models.clone(),
                    cost_tier,
                    config.artifacts.dir.clone(),
                )?),
                ProviderKind::LocalTts => {
                    let command = pc.command.clone().ok_or_else(|| {
                        EngineError::Config(format!(
                            "provider '{}' (local_tts) requires a command",
                            pc.name
                        ))
                    })?;
                    Arc::new(LocalTtsProvider::new(
                        &pc.name,
                        command,
                        pc.args.clone(),
                        pc.output_format.clone(),
                        pc.output_dir.clone(),
                        pc.models.clone(),
                    ))
                }
                ProviderKind::LocalAsr => {
                    let command = pc.command.clone().ok_or_else(|| {
                        EngineError::Config(format!(
                            "provider '{}' (local_asr) requires a command",
                            pc.name
                        ))
                    })?;
                    Arc::new(
                        LocalAsrProvider::new(
                            &pc.name,
                            command,
                            pc.args.clone(),
                            pc.output_dir.clone(),
                            pc.models.clone(),
                        )
                        .with_diarize_args(pc.diarize_args.clone()),
                    )
                }
                ProviderKind::LocalImageGen => {
                    let command = pc.command.clone().ok_or_else(|| {
                        EngineError::Config(format!(
                            "provider '{}' (local_image_gen) requires a command",
                            pc.name
                        ))
                    })?;
                    Arc::new(LocalImageGenProvider::new(
                        &pc.name,
                        command,
                        pc.args.clone(),
                        pc.output_format.clone(),
                        pc.output_dir.clone(),
                        pc.models.clone(),
                    ))
                }
                ProviderKind::LocalVideoGen => {
                    let command = pc.command.clone().ok_or_else(|| {
                        EngineError::Config(format!(
                            "provider '{}' (local_video_gen) requires a command",
                            pc.name
                        ))
                    })?;
                    Arc::new(LocalVideoGenProvider::new(
                        &pc.name,
                        command,
                        pc.args.clone(),
                        pc.output_format.clone(),
                        pc.output_dir.clone(),
                        pc.models.clone(),
                    ))
                }
                ProviderKind::CloudVideoGen => Arc::new(CloudVideoGenProvider::new(
                    &pc.name,
                    &pc.dialect,
                    &pc.base_url,
                    pc.api_key.clone(),
                    pc.models.clone(),
                    cost_tier,
                    config.artifacts.dir.clone(),
                )?),
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

        // Zero-config local voice: when a real config is present but declares no TTS
        // backend, auto-register the OS-native voice (macOS `say` / Linux `espeak`)
        // as a low-priority fallback. This gives local-first audio scenarios (S1/S4/
        // S6) a working voice with nothing to install, while any configured TTS —
        // having a lower priority number — still wins the route. An empty config is
        // left untouched (no providers in, no providers out).
        let has_configured_provider = config.providers.iter().any(|pc| pc.enabled);
        let declares_tts = config.providers.iter().any(|pc| {
            pc.enabled
                && pc.models.iter().any(|m| {
                    mofa_kernel::Capability::from_str_loose(&m.capability)
                        == Some(mofa_kernel::Capability::Tts)
                })
        });
        if has_configured_provider && !declares_tts {
            let system_tts = SystemTtsProvider::new(config.artifacts.dir.clone());
            if system_tts.is_available() {
                tracing::info!(
                    "no TTS backend configured; registering built-in system voice as fallback"
                );
                providers.push(RegisteredProvider {
                    name: crate::backends::system_tts::SYSTEM_TTS_NAME.to_string(),
                    kind: ProviderKind::LocalTts,
                    // A large priority number is the least-preferred tier, so a
                    // configured voice always outranks this fallback.
                    priority: 99,
                    provider: Arc::new(system_tts),
                });
            }
        }

        // Canonicalize the input-path allowlist up front; unresolvable roots are
        // dropped so a typo cannot silently widen access.
        let input_roots = config
            .security
            .input_roots
            .iter()
            .filter_map(|r| std::fs::canonicalize(r).ok())
            .collect::<Vec<_>>();
        let artifact_sweeper = crate::artifacts::ArtifactSweeper::new(
            config.artifacts.dir.clone().map(std::path::PathBuf::from),
            Duration::from_secs(config.artifacts.retention_secs),
        );

        // Per-provider token pricing for cost tracking.
        let pricing = config
            .providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| {
                (
                    p.name.clone(),
                    (p.price_input_per_1k, p.price_output_per_1k),
                )
            })
            .collect::<HashMap<_, _>>();

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
            conversations: DashMap::new(),
            timeouts: config.timeouts.clone(),
            preflight_config: config.preflight.clone(),
            idle_timeout,
            semaphores: DashMap::new(),
            load_gate: AsyncMutex::new(()),
            lifecycle: Mutex::new(VecDeque::with_capacity(LIFECYCLE_CAPACITY)),
            lifecycle_seq: AtomicU64::new(0),
            idle_task: Mutex::new(None),
            artifact_task: Mutex::new(None),
            weak_self: OnceLock::new(),
            event_tx,
            metrics: EngineMetrics::default(),
            pricing,
            input_roots,
            started_at: Instant::now(),
        });
        let _ = engine.weak_self.set(Arc::downgrade(&engine));

        engine.refresh_resources().await;
        engine.spawn_idle_eviction();
        engine.spawn_artifact_sweep(artifact_sweeper);
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
                        // Reconcile with the engine's current view. Capture the
                        // residency *and* the live execution counters so a
                        // rediscovery that lands mid-request does not reset an
                        // in-flight model's `active_requests` (which would corrupt
                        // its Busy status and capacity score, and let a later
                        // `end_execution` saturate to zero). `.map` drops the shard
                        // guard immediately so the later insert cannot deadlock.
                        let previous = self
                            .models
                            .get(&card.id)
                            .map(|c| (c.residency, c.execution.active_requests));
                        if let Some((prev_residency, active_requests)) = previous {
                            // Freshly discovered cards start with zero in-flight
                            // requests; carry the live count forward. The new card's
                            // `max_concurrency` is kept so config changes still apply.
                            card.execution.active_requests = active_requests;
                            match prev_residency {
                                // A load is in flight; don't let discovery regress it.
                                ModelResidency::Loading => {
                                    card.residency = ModelResidency::Loading;
                                }
                                // The backend reports the model is no longer resident,
                                // so release the reservation we still held for it.
                                ModelResidency::Loaded
                                    if !matches!(card.residency, ModelResidency::Loaded) =>
                                {
                                    self.memory.deallocate(&card.id);
                                    freed = true;
                                }
                                _ => {}
                            }
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

    /// Run inference, recording request-level metrics + token/cost usage.
    pub async fn invoke(&self, req: InferenceRequest) -> Result<InferenceResponse, EngineError> {
        let started = Instant::now();
        // Capture audit-relevant fields before `req` is consumed by inference.
        let req_prefer = req.prefer;
        let req_data_class = req.data_class;
        let req_session = req.session_id.clone();
        let mut result = self.invoke_inner(req).await;
        let duration_ms = started.elapsed().as_millis() as u64;
        match &mut result {
            Ok(resp) => {
                resp.cost_usd =
                    self.compute_cost(&resp.provider, resp.prompt_tokens, resp.completion_tokens);
                self.metrics
                    .record_request(true, duration_ms, resp.fallback_used);
                let is_local = self.provider_is_local(&resp.provider);
                self.metrics.record_usage(
                    &resp.provider,
                    is_local,
                    resp.prompt_tokens,
                    resp.completion_tokens,
                    resp.cost_usd,
                );
                // Data-flow audit (S5): record where each request was actually
                // served so residency is traceable by request/session over time.
                self.audit_served(
                    &resp.request_id,
                    &resp.provider,
                    req_prefer,
                    req_data_class,
                    req_session.as_deref(),
                );
            }
            Err(_) => self.metrics.record_request(false, duration_ms, false),
        }
        result
    }

    // ----- Responses API (stateful multi-turn) --------------------------------

    /// Run one turn of the stateful Responses API (S2 deep multi-turn reasoning).
    ///
    /// Seeds the turn with the conversation stored under `previous_response_id`
    /// (or a fresh history with the optional `instructions` as a system message),
    /// appends this turn's explicit `messages` and the `input` shorthand, routes
    /// through the normal [`invoke`](Self::invoke) path (so routing, failover,
    /// metrics, cost, and the S5 audit all apply), then appends the assistant
    /// reply and stores the updated history under a **new** id. The caller passes
    /// that id back as `previous_response_id` to continue the conversation.
    ///
    /// Continuing mints a fresh id per turn (so a caller can branch from any prior
    /// response); the store is bounded by [`MAX_CONVERSATIONS`] via oldest-first
    /// eviction. TODO: a single very long chain stores one full-history snapshot
    /// per turn (quadratic in turns); a future revision can share a suffix or cap
    /// retained turns without changing this contract.
    pub async fn respond(&self, req: ResponsesRequest) -> Result<ResponsesResponse, EngineError> {
        let ResponsesRequest {
            previous_response_id,
            instructions,
            input,
            mut request,
        } = req;

        // 1. Seed history: continue a stored conversation, or start a fresh one
        //    (with optional system instructions). An unknown prior id is a hard
        //    error rather than a silent fresh start, so an expired/evicted chain
        //    surfaces to the caller.
        let mut messages: Vec<Message> = match previous_response_id.as_deref() {
            Some(prev) => self
                .conversations
                .get(prev)
                .map(|c| c.messages.clone())
                .ok_or_else(|| {
                    EngineError::InvalidRequest(format!(
                        "unknown previous_response_id '{prev}' (expired or never existed)"
                    ))
                })?,
            None => match instructions.filter(|s| !s.trim().is_empty()) {
                Some(system) => vec![Message {
                    role: "system".into(),
                    content: system,
                    ..Default::default()
                }],
                None => Vec::new(),
            },
        };

        // 2. Append this turn's new messages, then the `input` shorthand as a
        //    user turn.
        messages.append(&mut request.messages);
        if let Some(text) = input.filter(|t| !t.trim().is_empty()) {
            messages.push(Message {
                role: "user".into(),
                content: text,
                ..Default::default()
            });
        }
        if messages.iter().all(|m| m.content.trim().is_empty()) {
            return Err(EngineError::InvalidRequest(
                "responses request has no input (set `input`, `messages`, or continue a conversation)".into(),
            ));
        }

        // 3. Route + invoke with the accumulated history. Default to chat when the
        //    caller does not pin a capability, since Responses is a chat surface.
        request.messages = messages.clone();
        if request.capability.is_none() {
            request.capability = Some(Capability::Chat);
        }
        let response = self.invoke(request).await?;

        // 4. Append the assistant reply and persist the updated history under a
        //    new response id.
        if let Some(text) = response.text.clone().filter(|t| !t.is_empty()) {
            messages.push(Message {
                role: "assistant".into(),
                content: text,
                ..Default::default()
            });
        }
        let id = uuid::Uuid::new_v4().to_string();
        let message_count = messages.len();
        self.store_conversation(id.clone(), messages);

        Ok(ResponsesResponse {
            id,
            message_count,
            response,
        })
    }

    /// Persist a conversation under `id`, evicting the oldest stored conversation
    /// first when at [`MAX_CONVERSATIONS`] capacity so memory stays bounded.
    fn store_conversation(&self, id: String, messages: Vec<Message>) {
        if self.conversations.len() >= MAX_CONVERSATIONS
            && !self.conversations.contains_key(&id)
            && let Some(victim) = self
                .conversations
                .iter()
                .min_by_key(|e| e.created)
                .map(|e| e.key().clone())
        {
            self.conversations.remove(&victim);
        }
        self.conversations.insert(
            id,
            StoredConversation {
                messages,
                created: Instant::now(),
            },
        );
    }

    /// The stored message history for a conversation id, if still retained.
    pub fn conversation_messages(&self, id: &str) -> Option<Vec<Message>> {
        self.conversations.get(id).map(|c| c.messages.clone())
    }

    /// Forget a stored conversation. Returns whether one existed.
    pub fn delete_conversation(&self, id: &str) -> bool {
        self.conversations.remove(id).is_some()
    }

    /// Number of stored conversations. Test-only diagnostic, so it is neither
    /// public API nor compiled into release builds.
    #[cfg(test)]
    fn conversation_count(&self) -> usize {
        self.conversations.len()
    }

    /// Estimated USD cost for a request served by `provider`, from the
    /// configured per-1k-token prices. `None` when no price is configured
    /// (e.g. local models) or no tokens were reported.
    fn compute_cost(
        &self,
        provider: &str,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    ) -> Option<f64> {
        let (price_in, price_out) = self.pricing.get(provider).copied()?;
        Engine::estimate_cost(price_in, price_out, prompt_tokens, completion_tokens)
    }

    /// Conservative pre-flight cost estimate (USD) for serving `req` on
    /// `provider`, used to enforce a `max_cost_usd` ceiling *before* spending.
    ///
    /// Free/local providers (no configured price) and providers with unknown
    /// pricing estimate to `0.0`, so a ceiling never blocks a local model and an
    /// unpriced provider is never spuriously skipped. Cloud providers are priced
    /// from a rough token estimate (see [`estimate_prompt_tokens`] /
    /// [`estimate_completion_tokens`]); the authoritative cost is still computed
    /// post-call from real token counts by [`Self::compute_cost`].
    fn estimate_request_cost(&self, provider: &str, req: &InferenceRequest) -> f64 {
        self.compute_cost(
            provider,
            Some(Engine::estimate_prompt_tokens(req)),
            Some(Engine::estimate_completion_tokens(req)),
        )
        .unwrap_or(0.0)
    }

    /// Whether the named provider serves models on-device (local track). Unknown
    /// providers are treated as cloud so metrics never mislabel spend as local.
    fn provider_is_local(&self, provider: &str) -> bool {
        self.providers
            .iter()
            .find(|p| p.name == provider)
            .is_some_and(|p| p.kind.is_local())
    }

    /// Emit the S5 data-flow audit record for one served request: where it ran
    /// and under what residency constraint. Both the unary [`invoke`](Self::invoke)
    /// and the streaming path call this so *every* served request — not just
    /// non-streamed ones — is traceable by request/session over time.
    fn audit_served(
        &self,
        request_id: &str,
        provider: &str,
        prefer: Prefer,
        data_class: DataClass,
        session_id: Option<&str>,
    ) {
        tracing::info!(
            target: "mofa::audit",
            request_id = %request_id,
            session_id = session_id.unwrap_or(""),
            data_class = ?data_class,
            prefer = ?prefer,
            provider = %provider,
            locality = if self.provider_is_local(provider) { "local" } else { "cloud" },
            "request served"
        );
    }

    /// Run inference.
    ///
    /// Builds a single ranked candidate plan and walks it in order: the first
    /// candidate is the primary selection, and each subsequent one is a failover.
    /// Only *retryable* failures advance to the next candidate, so malformed or
    /// unsupported requests fail immediately rather than masquerading as
    /// transient errors. The candidate list itself encodes the fallback policy.
    async fn invoke_inner(&self, req: InferenceRequest) -> Result<InferenceResponse, EngineError> {
        self.reject_ambiguous_short_name(&req)?;
        self.check_input_path(&req)?;
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

        // Machine-readable routing reason of the primary candidate, surfaced on
        // failure so an Agent can see why this plan was chosen.
        let routing_reason = candidates.first().map(|d| d.reason.clone());
        let mut last_err: Option<EngineError> = None;
        // Complete per-candidate failure chain (PRD §4.2), in attempt order.
        let mut chain: Vec<FailedAttempt> = Vec::new();
        for (idx, decision) in candidates.iter().enumerate() {
            let model_id = decision.model.id.clone();
            let provider_name = decision.model.provider.clone();

            if Engine::remaining(overall_deadline).is_zero() {
                let e = EngineError::Timeout("overall request deadline exceeded".into());
                chain.push(FailedAttempt {
                    provider: provider_name,
                    model: ModelId::name(&model_id).to_string(),
                    reason: e.to_string(),
                });
                last_err = Some(e);
                break;
            }

            // Budget ceiling (PRD §S2/W3): skip a candidate whose estimated cost
            // would exceed the request's ceiling, recording it in the chain so an
            // Agent can see it was priced out (and could raise the budget or fall
            // back to a local model). Free/local candidates estimate to $0.
            if let Some(ceiling) = req.max_cost_usd {
                let est = self.estimate_request_cost(&provider_name, &req);
                if est > ceiling {
                    tracing::info!(
                        "candidate '{model_id}' skipped: est cost ${est:.4} exceeds ceiling ${ceiling:.4}"
                    );
                    chain.push(FailedAttempt {
                        provider: provider_name.clone(),
                        model: ModelId::name(&model_id).to_string(),
                        reason: format!("estimated cost ${est:.4} exceeds budget ${ceiling:.4}"),
                    });
                    last_err = Some(EngineError::NoCapableModel(format!(
                        "all candidates exceed cost ceiling ${ceiling:.4}"
                    )));
                    continue;
                }
            }

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
                    chain.push(FailedAttempt {
                        provider: provider_name,
                        model: ModelId::name(&model_id).to_string(),
                        reason: e.to_string(),
                    });
                    last_err = Some(e);
                }
            }
        }

        let last = last_err
            .unwrap_or_else(|| EngineError::NoCapableModel(Self::requested_capability(&req)));
        // Wrap the exhausted-candidates case so the caller sees the whole chain,
        // not just the last error. A lone failure keeps its original shape.
        if chain.len() <= 1 {
            return Err(last);
        }
        Err(EngineError::Failover {
            code: last.code(),
            message: last.to_string(),
            retryable: last.retryable(),
            chain,
            routing_reason,
        })
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
        let queue_budget = Engine::remaining(overall_deadline).min(self.timeouts.queue());
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

        let inference_budget = Engine::remaining(overall_deadline).min(self.timeouts.inference());
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

    /// Run inference and stream typed output chunks.
    ///
    /// Returns a receiver that yields a `Started` chunk, then `Text` deltas as
    /// the backend produces them, then a terminal `Completed` or `Error`. Errors
    /// are delivered in-band as [`StreamChunk::Error`] rather than as a `Result`,
    /// so a consumer only has to read one channel.
    ///
    /// Streaming targets the single best candidate. Unlike [`invoke`](Self::invoke)
    /// it does not fail over once output has begun, since partial output cannot
    /// be un-sent; a failure before the first token surfaces as `Error`.
    pub fn invoke_stream(&self, req: InferenceRequest) -> mpsc::Receiver<StreamChunk> {
        let (tx, rx) = mpsc::channel(64);
        if let Some(weak) = self.weak() {
            tokio::spawn(async move {
                if let Some(engine) = weak.upgrade() {
                    engine.run_stream(req, tx).await;
                }
            });
        }
        rx
    }

    /// Emit a terminal `Error` chunk for a streaming request that failed *before*
    /// generation began, recording the failed-request metric so a pre-dispatch
    /// failure (circuit open, no capable model, admission timeout, …) is counted
    /// exactly like the equivalent unary `invoke` failure rather than being
    /// invisible to `mofa_requests_total`. The terminal match after generation
    /// starts records metrics itself (with the real duration), so this helper is
    /// only used on the pre-dispatch error paths.
    async fn fail_stream(&self, out: &mpsc::Sender<StreamChunk>, err: EngineError) {
        self.metrics.record_request(false, 0, false);
        let _ = out.send(StreamChunk::Error(err.info())).await;
    }

    /// Drive one streaming request end to end, emitting chunks to `out`.
    async fn run_stream(&self, req: InferenceRequest, out: mpsc::Sender<StreamChunk>) {
        if let Err(e) = self
            .reject_ambiguous_short_name(&req)
            .and_then(|()| self.check_input_path(&req))
        {
            self.fail_stream(&out, e).await;
            return;
        }
        self.confirm_prediction(&req);

        let all_models = self.capabilities().await;
        let providers = self.routing_providers();
        let candidates = self.build_candidates(&all_models, &req, &providers);
        let Some(decision) = candidates.first() else {
            let err = EngineError::NoCapableModel(Self::requested_capability(&req));
            self.fail_stream(&out, err).await;
            return;
        };

        let model_id = decision.model.id.clone();
        let provider_name = decision.model.provider.clone();
        let max_concurrency = decision.model.execution.max_concurrency;
        let routing_reason = decision.reason.clone();
        let overall_deadline = Instant::now() + self.timeouts.request();

        // Budget ceiling parity with `invoke`: streaming targets a single
        // candidate, so if that candidate's estimated cost exceeds the request's
        // ceiling the stream fails up front rather than spending. Free/local
        // candidates estimate to $0 and are always affordable.
        if let Some(ceiling) = req.max_cost_usd {
            let est = self.estimate_request_cost(&provider_name, &req);
            if est > ceiling {
                let err = EngineError::NoCapableModel(format!(
                    "estimated cost ${est:.4} exceeds budget ${ceiling:.4}"
                ));
                self.fail_stream(&out, err).await;
                return;
            }
        }

        if !self.circuit_breakers.allow_request(&provider_name) {
            let err = EngineError::CircuitOpen(provider_name.clone());
            self.fail_stream(&out, err).await;
            return;
        }
        let Some(provider) = self.find_provider(&provider_name) else {
            let err = EngineError::Internal(format!("provider '{provider_name}' not found"));
            self.fail_stream(&out, err).await;
            return;
        };

        let _ = self.event_tx.send(EngineEvent::RequestStarted {
            request_id: req.request_id.clone(),
            capability: req.capability,
            model_id: model_id.clone(),
        });

        if let Err(e) = self
            .ensure_loaded(&model_id, &provider_name, &provider)
            .await
        {
            self.emit_request_completed(&req.request_id, 0, false);
            self.fail_stream(&out, e).await;
            return;
        }

        let sem = self.semaphore_for(&model_id, max_concurrency);
        let queue_budget = Engine::remaining(overall_deadline).min(self.timeouts.queue());
        let permit = match tokio::time::timeout(queue_budget, sem.acquire_owned()).await {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => {
                self.emit_request_completed(&req.request_id, 0, false);
                let err = EngineError::Internal("concurrency semaphore closed".into());
                self.fail_stream(&out, err).await;
                return;
            }
            Err(_) => {
                self.emit_request_completed(&req.request_id, 0, false);
                let err = EngineError::Timeout(format!("queue wait exceeded for '{model_id}'"));
                self.fail_stream(&out, err).await;
                return;
            }
        };

        self.begin_execution(&model_id);
        self.trigger_preflight(&req);

        let _ = out
            .send(StreamChunk::Started {
                request_id: req.request_id.clone(),
                model_used: ModelId::name(&model_id).to_string(),
                provider: provider_name.clone(),
            })
            .await;

        // Forward the provider's deltas into the output stream as they arrive,
        // mapping text vs thought-chain deltas to their respective chunk types.
        // The provider drops its sink when it returns, ending the forwarder, so
        // all deltas are flushed before the terminal chunk.
        let (delta_tx, mut delta_rx) = mpsc::channel::<StreamDelta>(64);
        let forward_out = out.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(delta) = delta_rx.recv().await {
                let chunk = match delta {
                    StreamDelta::Text(delta) => StreamChunk::Text { delta },
                    StreamDelta::Reasoning(delta) => StreamChunk::Reasoning { delta },
                };
                if forward_out.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        let inference_budget = Engine::remaining(overall_deadline).min(self.timeouts.inference());
        let start = Instant::now();
        let result =
            tokio::time::timeout(inference_budget, provider.stream(&model_id, &req, delta_tx))
                .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let _ = forwarder.await;
        self.end_execution(&model_id);
        drop(permit);

        let terminal = match result {
            Ok(Ok(resp)) => {
                self.circuit_breakers.record_success(&provider_name);
                self.record_transition(&req);
                self.emit_request_completed(&req.request_id, duration_ms, true);
                self.metrics.record_request(true, duration_ms, false);
                let cost_usd =
                    self.compute_cost(&provider_name, resp.prompt_tokens, resp.completion_tokens);
                self.metrics.record_usage(
                    &provider_name,
                    self.provider_is_local(&provider_name),
                    resp.prompt_tokens,
                    resp.completion_tokens,
                    cost_usd,
                );
                // Data-flow audit (S5): streamed requests are traced exactly like
                // unary ones, so the residency guarantee has no streaming blind spot.
                self.audit_served(
                    &req.request_id,
                    &provider_name,
                    req.prefer,
                    req.data_class,
                    req.session_id.as_deref(),
                );
                StreamChunk::Completed {
                    duration_ms,
                    tokens_used: resp.tokens_used,
                    prompt_tokens: resp.prompt_tokens,
                    completion_tokens: resp.completion_tokens,
                    cost_usd,
                    file: resp.file,
                    fallback_used: false,
                    routing_reason: Some(routing_reason),
                }
            }
            Ok(Err(e)) => {
                self.circuit_breakers.record_failure(&provider_name);
                self.emit_request_completed(&req.request_id, duration_ms, false);
                self.metrics.record_request(false, duration_ms, false);
                StreamChunk::Error(e.info())
            }
            Err(_) => {
                self.circuit_breakers.record_failure(&provider_name);
                let budget_ms = inference_budget.as_millis() as u64;
                self.emit_request_completed(&req.request_id, budget_ms, false);
                self.metrics.record_request(false, budget_ms, false);
                let err = EngineError::Timeout(format!(
                    "provider '{provider_name}' did not respond within {}s",
                    inference_budget.as_secs()
                ));
                StreamChunk::Error(err.info())
            }
        };
        let _ = out.send(terminal).await;
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
            // Bound the backend unload with the load timeout: during memory-pressure
            // admission this runs under `load_gate`, so a hung backend unload must
            // not wedge every concurrent load. A timed-out (or failed) unload still
            // falls through to release the engine's own accounting below.
            match tokio::time::timeout(self.timeouts.load(), provider.unload(model_id)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("backend unload of '{model_id}' failed: {e}"),
                Err(_) => {
                    tracing::warn!("backend unload of '{model_id}' timed out; releasing accounting")
                }
            }
        }
        // Frees the reservation and emits residency/memory events.
        self.set_model_residency(model_id, ModelResidency::Unloaded);
        // Distinguish the cause so metrics count each correctly: an idle sweep is
        // an idle-unload, an operator action is a plain unload, and everything
        // else (memory pressure) is an eviction.
        let event = match reason {
            "idle_timeout" => "idle_unload",
            "manual" => "unload",
            _ => "evict",
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
            ..Default::default()
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
    /// An explicit per-call `ttl` wins; when `None` is passed the configured
    /// `preflight.subscription_ttl_secs` default bounds the subscription (0
    /// disables expiry) so it cannot pin models warm forever.
    ///
    /// Returns the new subscription id.
    pub fn subscribe(
        &self,
        app_id: Option<String>,
        session_id: Option<String>,
        capabilities: Vec<Capability>,
        ttl: Option<Duration>,
    ) -> u64 {
        let ttl = ttl.or_else(|| {
            let secs = self.preflight_config.subscription_ttl_secs;
            (secs > 0).then(|| Duration::from_secs(secs))
        });
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

    /// Background task: periodically delete stale engine artifacts.
    fn spawn_artifact_sweep(&self, sweeper: crate::artifacts::ArtifactSweeper) {
        let retention = sweeper.retention();
        if retention.is_zero() {
            return; // retention 0 disables cleanup
        }
        let Some(weak) = self.weak_self.get().cloned() else {
            return;
        };
        // Sweep on roughly half the retention interval, bounded so tests and
        // long-lived daemons both behave.
        let tick = Duration::from_secs((retention.as_secs().max(2) / 2).clamp(1, 300));
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(tick);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if weak.upgrade().is_none() {
                    break;
                }
                let removed = sweeper.sweep();
                if removed > 0 {
                    tracing::debug!("artifact sweep removed {removed} stale file(s)");
                }
            }
        });
        *self.artifact_task.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    /// Reject a request whose local file inputs fall outside the configured
    /// allowlist — `input_file` AND any `messages[].images` entry that names a
    /// local path. http(s)/data: image URLs are not local paths and skip the
    /// check. A no-op when no roots are configured.
    fn check_input_path(&self, req: &InferenceRequest) -> Result<(), EngineError> {
        if self.input_roots.is_empty() {
            return Ok(());
        }
        if let Some(path) = req.input_file.as_deref() {
            self.check_path_within_roots(path, "input_file")?;
        }
        // Image references are resolved by the backends with a raw filesystem
        // read (ollama.rs base64-encodes them; liter_llm.rs then UPLOADS the
        // contents to a cloud provider) — an unchecked path is a local-file
        // read primitive that becomes data exfiltration, so they go through
        // the exact same allowlist as `input_file` before any routing.
        for message in &req.messages {
            for image in &message.images {
                if image.starts_with("http://")
                    || image.starts_with("https://")
                    || image.starts_with("data:")
                {
                    continue;
                }
                self.check_path_within_roots(image, "images[]")?;
            }
        }
        Ok(())
    }

    /// Validate one local path against the configured allowlist.
    fn check_path_within_roots(&self, path: &str, field: &str) -> Result<(), EngineError> {
        let canonical = std::fs::canonicalize(path).map_err(|_| {
            EngineError::InvalidRequest(format!("{field} '{path}' cannot be resolved"))
        })?;
        if self
            .input_roots
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            Ok(())
        } else {
            Err(EngineError::InvalidRequest(format!(
                "{field} '{path}' is outside the allowed roots"
            )))
        }
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
        self.metrics.record_lifecycle(event);
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

    /// Snapshot every provider's backend status, for `status()`.
    fn backend_statuses(&self) -> Vec<BackendStatus> {
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

    /// Render the current metrics in Prometheus text-exposition format.
    ///
    /// Combines the process-wide counters with point-in-time gauges sampled
    /// from the registry, memory manager, preflight counters, and per-provider
    /// health. Label cardinality is bounded: only the fixed provider set adds
    /// labels.
    pub fn metrics_prometheus(&self) -> String {
        let models = self.models_snapshot();
        let loaded = models
            .iter()
            .filter(|m| matches!(m.residency, ModelResidency::Loaded | ModelResidency::Remote))
            .count() as u64;
        let preflight = self.preflight_metrics.snapshot();
        let provider_up = self
            .providers
            .iter()
            .map(|p| {
                let health = self
                    .backend_health
                    .get(&p.name)
                    .map(|h| *h)
                    .unwrap_or(BackendHealth::Unknown);
                let up = health.is_routable()
                    && self.circuit_breakers.state(&p.name) != CircuitState::Open;
                (p.name.clone(), up)
            })
            .collect();

        let gauges = MetricsGauges {
            models_total: models.len() as u64,
            models_loaded: loaded,
            memory_used_bytes: self.memory.used_bytes(),
            memory_budget_bytes: self.memory.budget_bytes(),
            preflight_warms_started: preflight.warms_started,
            preflight_hits: preflight.hits,
            provider_up,
        };
        self.metrics.render_prometheus(&gauges)
    }

    /// Manually load (warm) a model by id. Used by the management interface.
    ///
    /// Goes through the same reservation-based admission and lifecycle path as
    /// an on-demand load, so memory accounting and eviction rules still hold.
    pub async fn load_model(&self, model_id: &str) -> Result<(), EngineError> {
        let provider_name = self
            .models
            .get(model_id)
            .map(|c| c.provider.clone())
            .ok_or_else(|| EngineError::InvalidRequest(format!("unknown model '{model_id}'")))?;
        let provider = self.find_provider(&provider_name).ok_or_else(|| {
            EngineError::Internal(format!("provider '{provider_name}' not found"))
        })?;
        self.ensure_loaded(model_id, &provider_name, &provider)
            .await
    }

    /// Manually unload a model by id. Returns whether the model was known.
    ///
    /// Refuses to unload a model with active leases (in-flight inference) so a
    /// manual action cannot corrupt a running request.
    pub async fn unload_model_manual(&self, model_id: &str) -> Result<bool, EngineError> {
        if !self.models.contains_key(model_id) {
            return Ok(false);
        }
        if self.memory.lease_count(model_id) > 0 {
            return Err(EngineError::InvalidRequest(format!(
                "model '{model_id}' is busy and cannot be unloaded"
            )));
        }
        self.unload_model(model_id, "manual").await;
        Ok(true)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        for task in [&self.idle_task, &self.artifact_task] {
            if let Some(handle) = task.lock().unwrap_or_else(|e| e.into_inner()).take() {
                handle.abort();
            }
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
        ModelId,
    };
    use std::sync::atomic::AtomicUsize;

    use crate::config::{
        EngineConfig, ListenConfig, MemoryConfig, ModelDef, PreflightConfig, ProviderConfig,
        SecurityConfig, TimeoutConfig,
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
            artifacts: Default::default(),
            security: Default::default(),
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
                    card.id = ModelId::canonical(&self.name, name);
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
                .find(|(n, ..)| ModelId::canonical(&self.name, n) == model_id)
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
                model_used: mofa_kernel::ModelId::name(model_id).into(),
                provider: self.name.clone(),
                duration_ms: 1,
                request_id: request.request_id.clone(),
                tokens_used: Some(1),
                fallback_used: false,
                routing_reason: None,
                ..Default::default()
            })
        }
    }

    fn build_engine(
        providers: Vec<Arc<dyn Provider>>,
        budget_mb: u64,
        idle_secs: u64,
    ) -> Arc<Engine> {
        build_engine_with_pricing(providers, budget_mb, idle_secs, HashMap::new())
    }

    fn build_engine_with_pricing(
        providers: Vec<Arc<dyn Provider>>,
        budget_mb: u64,
        idle_secs: u64,
        pricing: HashMap<String, (f64, f64)>,
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
            conversations: DashMap::new(),
            timeouts: TimeoutConfig::default(),
            preflight_config: PreflightConfig::default(),
            idle_timeout: Duration::from_secs(idle_secs),
            semaphores: DashMap::new(),
            load_gate: AsyncMutex::new(()),
            lifecycle: Mutex::new(VecDeque::new()),
            lifecycle_seq: AtomicU64::new(0),
            idle_task: Mutex::new(None),
            artifact_task: Mutex::new(None),
            weak_self: OnceLock::new(),
            event_tx,
            metrics: EngineMetrics::default(),
            pricing,
            input_roots: Vec::new(),
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
                ..Default::default()
            }],
            input_file: None,
            params: serde_json::Value::Null,
            hint_next: None,
            request_id: "test".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn engine_starts_with_empty_config() {
        let engine = Engine::new(minimal_config()).await;
        assert!(engine.capabilities().await.is_empty());
    }

    #[test]
    fn cost_estimation_from_prices() {
        // $0.01/1k in, $0.03/1k out; 1000 prompt + 500 completion tokens.
        let cost = Engine::estimate_cost(0.01, 0.03, Some(1000), Some(500)).unwrap();
        assert!((cost - (0.01 + 0.015)).abs() < 1e-9);
        // Zero prices (free / local model) → no cost attributed.
        assert!(Engine::estimate_cost(0.0, 0.0, Some(1000), Some(500)).is_none());
        // Missing token counts default to zero.
        assert_eq!(Engine::estimate_cost(0.01, 0.03, None, None), Some(0.0));
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
    async fn system_voice_backs_a_tts_less_config() {
        // A config with one enabled, non-TTS provider gains the built-in system
        // voice as a fallback wherever the OS provides one (macOS `say` / Linux
        // `espeak`). `true` stands in as a harmless, network-free provider command.
        let mut config = minimal_config();
        config.providers = vec![ProviderConfig {
            name: "img".into(),
            kind: "local_image_gen".into(),
            command: Some("true".into()),
            enabled: true,
            models: vec![ModelDef {
                name: "sd".into(),
                capability: "image_gen".into(),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let engine = Engine::new(config).await;
        engine.refresh_resources().await;

        let voice_available =
            crate::backends::system_tts::SystemTtsProvider::new(None).is_available();
        let injected = engine
            .capabilities()
            .await
            .iter()
            .any(|c| c.id == "system-tts/system");
        assert_eq!(injected, voice_available);

        // An empty config is left untouched: no providers in, no voice out.
        let empty = Engine::new(minimal_config()).await;
        empty.refresh_resources().await;
        assert!(
            empty
                .capabilities()
                .await
                .iter()
                .all(|c| c.id != "system-tts/system")
        );
    }

    #[tokio::test]
    async fn cloud_video_gen_config_exposes_a_video_capability() {
        // A `cloud_video_gen` provider (Seedance / Ark) wires through config → the
        // backend factory → discovery, exposing a first-class `video_gen`
        // capability. A dummy key keeps it network-free: `health()` and `discover()`
        // for this backend never touch the wire, so the card is `Configured`
        // without a live API.
        let mut config = minimal_config();
        config.providers = vec![ProviderConfig {
            name: "seedance".into(),
            kind: "cloud_video_gen".into(),
            api_key: Some("test-key".into()),
            cost_tier: "high".into(),
            enabled: true,
            models: vec![ModelDef {
                name: "doubao-seedance-1-0-pro".into(),
                capability: "video_gen".into(),
                ..Default::default()
            }],
            ..Default::default()
        }];
        let engine = Engine::new(config).await;
        engine.refresh_resources().await;

        let card = engine
            .capabilities()
            .await
            .into_iter()
            .find(|c| c.provider == "seedance")
            .expect("the configured cloud video provider is discovered");
        assert_eq!(card.capability, mofa_kernel::Capability::VideoGen);
        // Cloud model: remote residency, not counted against local memory.
        assert_eq!(card.residency, ModelResidency::Remote);
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
            artifacts: Default::default(),
            security: Default::default(),
            providers: vec![ProviderConfig {
                name: "disabled-ollama".into(),
                kind: "ollama".into(),
                base_url: "http://localhost:99999".into(),
                api_key: None,
                priority: 1,
                cost_tier: "free".into(),
                models: vec![],
                enabled: false,
                ..Default::default()
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
    async fn responses_api_accumulates_multi_turn_history() {
        use mofa_kernel::{ResponsesRequest, ResponsesResponse};

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

        // Turn 1: a fresh conversation with system instructions + user input.
        let first: ResponsesResponse = engine
            .respond(ResponsesRequest {
                instructions: Some("be terse".into()),
                input: Some("hello".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        // system + user + assistant("ok") = 3 messages stored under a fresh id.
        assert_eq!(first.message_count, 3);
        assert_eq!(first.response.text.as_deref(), Some("ok"));
        assert!(!first.id.is_empty());
        assert_eq!(engine.conversation_count(), 1);

        // Turn 2: continue from the prior response id with a new user input.
        let second = engine
            .respond(ResponsesRequest {
                previous_response_id: Some(first.id.clone()),
                input: Some("again".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        // Prior 3 + new user + new assistant = 5, and the new turn gets its own id.
        assert_eq!(second.message_count, 5);
        assert_ne!(second.id, first.id);

        // The stored history carries the full accumulated dialogue in order.
        let history = engine.conversation_messages(&second.id).unwrap();
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].role, "system");
        assert_eq!(history[1].content, "hello");
        assert_eq!(history[3].content, "again");

        // An unknown prior id is a hard (non-retryable) error, not a fresh start.
        let err = engine
            .respond(ResponsesRequest {
                previous_response_id: Some("does-not-exist".into()),
                input: Some("x".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidRequest(_)));
        assert!(!err.retryable());

        // A turn with no input at all is rejected.
        assert!(matches!(
            engine.respond(ResponsesRequest::default()).await,
            Err(EngineError::InvalidRequest(_))
        ));

        // Deleting a conversation forgets it.
        assert!(engine.delete_conversation(&second.id));
        assert!(engine.conversation_messages(&second.id).is_none());
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
    async fn local_only_never_routes_to_cloud() {
        // S5 privacy guardrail: a local_only request must be served locally or
        // fail — never fall through to a cloud provider.
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "gpt",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let engine = build_engine(vec![cloud], 1000, 0);
        engine.refresh_resources().await;

        let mut req = chat_request(None);
        req.prefer = mofa_kernel::Prefer::Local;
        // Only a cloud model exists → the request fails rather than leaving the device.
        assert!(matches!(
            engine.invoke(req).await,
            Err(EngineError::NoCapableModel(_))
        ));
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
    async fn exhausted_failover_returns_full_failed_chain() {
        // Both the local and cloud candidate fail retryably; the engine exhausts
        // the plan and returns a Failover error carrying *every* attempt (PRD §4.2),
        // not just the last one — so an Agent can see the whole chain.
        let local = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama)
                .with_model("qwen", Capability::Chat, ModelResidency::Loaded, 10 * MB, 1)
                .behavior(InvokeBehavior::FailRetryable),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible)
                .with_model("gpt", Capability::Chat, ModelResidency::Remote, 0, 32)
                .behavior(InvokeBehavior::FailRetryable),
        );
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let err = engine.invoke(chat_request(None)).await.unwrap_err();
        let info = err.info();
        assert_eq!(info.failed_chain.len(), 2);
        // Local is tried first (locality bias), then the cloud fallback.
        assert_eq!(info.failed_chain[0].provider, "ollama");
        assert_eq!(info.failed_chain[0].model, "qwen");
        assert_eq!(info.failed_chain[1].provider, "openai");
        assert!(info.retryable);
        assert!(info.routing_reason.is_some());
    }

    #[test]
    fn token_estimates_from_request() {
        // ~4 chars/token over message content; floor of 1 for an empty prompt.
        let mut req = chat_request(None); // content = "hello" (5 chars)
        assert_eq!(Engine::estimate_prompt_tokens(&req), 1);
        req.messages[0].content = "x".repeat(400);
        assert_eq!(Engine::estimate_prompt_tokens(&req), 100);
        // Completion defaults to the generous estimate, or the params cap.
        assert_eq!(
            Engine::estimate_completion_tokens(&req),
            Engine::DEFAULT_COMPLETION_TOKEN_ESTIMATE
        );
        req.params = serde_json::json!({ "max_tokens": 64 });
        assert_eq!(Engine::estimate_completion_tokens(&req), 64);
    }

    #[tokio::test]
    async fn budget_ceiling_prices_out_cloud_and_prefers_local() {
        // A free local model and a priced cloud model both serve chat. A request
        // whose per-call budget is below the cloud estimate must route to the
        // free local model instead of the (pricier, higher-priority) cloud one.
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
                "gpt",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        // openai: $0.01/1k in, $0.03/1k out → default estimate (1 + 1024 tokens)
        // ≈ $0.0307, well above the $0.001 ceiling. ollama is free.
        let pricing = HashMap::from([("openai".to_string(), (0.01, 0.03))]);
        let engine = build_engine_with_pricing(vec![local, cloud], 1000, 0, pricing);
        engine.refresh_resources().await;

        let mut req = chat_request(None);
        req.max_cost_usd = Some(0.001);
        let resp = engine.invoke(req).await.unwrap();
        assert_eq!(resp.provider, "ollama");
    }

    #[tokio::test]
    async fn budget_ceiling_exhausts_when_all_priced_out() {
        // Two priced cloud models, no local option; a sub-cent ceiling prices both
        // out, so the request fails over the whole plan and the budget reason is
        // recorded per candidate — the Agent sees *why* nothing ran, rather than
        // the engine silently overspending.
        let cloud_a = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "gpt",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let cloud_b = Arc::new(
            TestProvider::new("deepseek", ProviderKind::OpenAiCompatible).with_model(
                "r1",
                Capability::Chat,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let pricing = HashMap::from([
            ("openai".to_string(), (0.01, 0.03)),
            ("deepseek".to_string(), (0.005, 0.02)),
        ]);
        let engine = build_engine_with_pricing(vec![cloud_a, cloud_b], 1000, 0, pricing);
        engine.refresh_resources().await;

        let mut req = chat_request(None);
        req.max_cost_usd = Some(0.0001);
        let err = engine.invoke(req).await.unwrap_err();
        let info = err.info();
        assert_eq!(info.code, mofa_kernel::ErrorCode::NoCapableModel);
        assert_eq!(info.failed_chain.len(), 2);
        assert!(
            info.failed_chain
                .iter()
                .all(|a| a.reason.contains("exceeds budget"))
        );

        // Without the ceiling the same request succeeds on a cloud model.
        let resp = engine.invoke(chat_request(None)).await.unwrap();
        assert!(resp.provider == "openai" || resp.provider == "deepseek");
    }

    #[tokio::test]
    async fn prefer_local_routes_image_gen_to_local_backend() {
        // The offline flagship (S4): with a local image-gen backend present, a
        // `prefer=local` ImageGen request must route on-device rather than to the
        // higher-priority cloud image API — verifying the new `LocalImageGen` kind
        // participates in the local-first hard filter.
        let local = Arc::new(
            TestProvider::new("local-sd", ProviderKind::LocalImageGen).with_model(
                "sd",
                Capability::ImageGen,
                ModelResidency::Loaded,
                10 * MB,
                1,
            ),
        );
        let cloud = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible).with_model(
                "dall-e",
                Capability::ImageGen,
                ModelResidency::Remote,
                0,
                32,
            ),
        );
        let engine = build_engine(vec![local, cloud], 1000, 0);
        engine.refresh_resources().await;

        let mut req = request(Capability::ImageGen, None, FallbackPolicy::default());
        req.prefer = mofa_kernel::Prefer::Local;
        let resp = engine.invoke(req).await.unwrap();
        assert_eq!(resp.provider, "local-sd");
    }

    #[tokio::test]
    async fn local_tts_failure_falls_back_to_cloud_tts() {
        // A local TTS backend (preferred by locality) crashes mid-synthesis; the
        // engine must fail over to the configured cloud TTS model. This is RFC
        // acceptance step 8 exercised deterministically without a real backend.
        let local_tts = Arc::new(
            TestProvider::new("local-tts", ProviderKind::LocalTts)
                .with_model(
                    "kokoro",
                    Capability::Tts,
                    ModelResidency::Loaded,
                    10 * MB,
                    1,
                )
                .behavior(InvokeBehavior::FailRetryable),
        );
        let cloud_tts = Arc::new(
            TestProvider::new("openai", ProviderKind::OpenAiCompatible)
                .with_model("tts-1", Capability::Tts, ModelResidency::Remote, 0, 32)
                .behavior(InvokeBehavior::Ok),
        );
        let local_calls = local_tts.invoke_calls.clone();
        let engine = build_engine(vec![local_tts, cloud_tts], 1000, 0);
        engine.refresh_resources().await;

        let resp = engine
            .invoke(request(Capability::Tts, None, FallbackPolicy::default()))
            .await
            .unwrap();
        // The local backend was tried first, then the engine fell over to cloud.
        assert_eq!(local_calls.load(Ordering::SeqCst), 1);
        assert_eq!(resp.provider, "openai");
        assert_eq!(resp.model_used, "tts-1");
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

    async fn collect_stream(mut rx: mpsc::Receiver<StreamChunk>) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();
        while let Some(c) = rx.recv().await {
            chunks.push(c);
        }
        chunks
    }

    /// A provider that emits several real text deltas, to prove the engine
    /// forwards incremental output in order (not just the compatibility path).
    /// `reasoning` deltas (if any) are emitted first as thought-chain increments.
    struct MultiDeltaProvider {
        name: String,
        deltas: Vec<String>,
        reasoning: Vec<String>,
    }

    #[async_trait]
    impl Provider for MultiDeltaProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn kind(&self) -> ProviderKind {
            ProviderKind::Ollama
        }
        fn features(&self) -> Vec<BackendFeature> {
            vec![BackendFeature::Discovery, BackendFeature::Streaming]
        }
        async fn discover(&self) -> Result<Vec<ModelCard>, EngineError> {
            let mut card = ModelCard::new(&self.name, "streamer", Capability::Chat, CostTier::Free);
            card.residency = ModelResidency::Loaded;
            card.refresh_status();
            Ok(vec![card])
        }
        async fn health(&self) -> Result<BackendHealth, EngineError> {
            Ok(BackendHealth::Healthy)
        }
        async fn load(&self, model_id: &str) -> Result<LifecycleResult, EngineError> {
            Ok(LifecycleResult {
                model_id: model_id.into(),
                residency: ModelResidency::Loaded,
                memory_bytes: Some(0),
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
            Ok(InferenceResponse {
                text: Some(self.deltas.concat()),
                file: None,
                model_used: mofa_kernel::ModelId::name(model_id).into(),
                provider: self.name.clone(),
                duration_ms: 1,
                request_id: request.request_id.clone(),
                tokens_used: Some(self.deltas.len() as u32),
                fallback_used: false,
                routing_reason: None,
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            model_id: &str,
            request: &InferenceRequest,
            sink: mofa_kernel::StreamSink,
        ) -> Result<InferenceResponse, EngineError> {
            for r in &self.reasoning {
                let _ = sink.send(StreamDelta::Reasoning(r.clone())).await;
            }
            for d in &self.deltas {
                let _ = sink.send(StreamDelta::Text(d.clone())).await;
            }
            self.invoke(model_id, request).await
        }
    }

    #[tokio::test]
    async fn stream_emits_started_text_completed_in_order() {
        // Default (compat) streaming: one Text chunk carrying the full output.
        let provider = Arc::new(
            TestProvider::new("ollama", ProviderKind::Ollama).with_model(
                "qwen",
                Capability::Chat,
                ModelResidency::Loaded,
                10 * MB,
                1,
            ),
        );
        let engine = build_engine(vec![provider], 1000, 0);
        engine.refresh_resources().await;

        let chunks = collect_stream(engine.invoke_stream(chat_request(None))).await;
        assert!(matches!(chunks.first(), Some(StreamChunk::Started { .. })));
        assert!(matches!(chunks.last(), Some(StreamChunk::Completed { .. })));
        let text: String = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Text { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "ok");
    }

    #[tokio::test]
    async fn stream_forwards_incremental_deltas_in_order() {
        let provider = Arc::new(MultiDeltaProvider {
            name: "ollama".into(),
            deltas: vec!["Hello".into(), ", ".into(), "world".into()],
            reasoning: vec![],
        });
        let engine = build_engine(vec![provider], 1000, 0);
        engine.refresh_resources().await;

        let chunks = collect_stream(engine.invoke_stream(chat_request(None))).await;
        let deltas: Vec<String> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Text { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["Hello", ", ", "world"]);
        // Completed reports the token count the provider aggregated.
        match chunks.last() {
            Some(StreamChunk::Completed { tokens_used, .. }) => {
                assert_eq!(*tokens_used, Some(3));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_separates_reasoning_from_answer_deltas() {
        // S2: the thought chain surfaces as `Reasoning` chunks, distinct from the
        // final answer's `Text` chunks, and both precede the terminal `Completed`.
        let provider = Arc::new(MultiDeltaProvider {
            name: "ollama".into(),
            deltas: vec!["4".into()],
            reasoning: vec!["2+2".into(), " = ".into()],
        });
        let engine = build_engine(vec![provider], 1000, 0);
        engine.refresh_resources().await;

        let chunks = collect_stream(engine.invoke_stream(chat_request(None))).await;
        let reasoning: Vec<String> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Reasoning { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        let text: Vec<String> = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Text { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(reasoning, vec!["2+2", " = "]);
        assert_eq!(text, vec!["4"]);
        assert!(matches!(chunks.last(), Some(StreamChunk::Completed { .. })));
    }

    #[tokio::test]
    async fn input_path_allowlist_blocks_paths_outside_roots() {
        let root = tempfile::tempdir().unwrap();
        let allowed = root.path().join("clip.wav");
        std::fs::write(&allowed, b"x").unwrap();
        let outside = std::env::temp_dir().join("mofa_outside_allowlist.wav");
        std::fs::write(&outside, b"x").unwrap();

        let config = EngineConfig {
            security: SecurityConfig {
                input_roots: vec![root.path().to_string_lossy().to_string()],
            },
            ..minimal_config()
        };
        let engine = Engine::new(config).await;

        // A path outside the allowlist is rejected before any routing happens.
        let mut req = request(Capability::Asr, None, FallbackPolicy::default());
        req.input_file = Some(outside.to_string_lossy().to_string());
        assert!(matches!(
            engine.invoke(req).await,
            Err(EngineError::InvalidRequest(_))
        ));

        // A path inside the allowlist passes the check (then fails later only
        // because no ASR model is registered).
        let mut ok = request(Capability::Asr, None, FallbackPolicy::default());
        ok.input_file = Some(allowed.to_string_lossy().to_string());
        assert!(matches!(
            engine.invoke(ok).await,
            Err(EngineError::NoCapableModel(_))
        ));
    }

    #[tokio::test]
    async fn image_paths_outside_input_roots_are_rejected() {
        // #4 review (security): `messages[].images` entries that name LOCAL
        // FILES must pass the same `security.input_roots` validation as
        // `input_file` — the ollama path base64-reads them and the liter-llm
        // path uploads their contents to a cloud provider, so an unchecked
        // path is a local-file-read primitive that becomes exfiltration.
        let root = tempfile::tempdir().unwrap();
        let allowed = root.path().join("frame.png");
        std::fs::write(&allowed, b"x").unwrap();
        let outside = std::env::temp_dir().join("mofa_outside_allowlist.png");
        std::fs::write(&outside, b"x").unwrap();

        let config = EngineConfig {
            security: SecurityConfig {
                input_roots: vec![root.path().to_string_lossy().to_string()],
            },
            ..minimal_config()
        };
        let engine = Engine::new(config).await;

        let image_req = |image: String| {
            let mut req = request(Capability::Vlm, None, FallbackPolicy::default());
            req.messages = vec![Message {
                role: "user".into(),
                content: "describe this".into(),
                images: vec![image],
            }];
            req
        };

        // A local image path outside the allowlist is rejected before routing.
        assert!(matches!(
            engine
                .invoke(image_req(outside.to_string_lossy().to_string()))
                .await,
            Err(EngineError::InvalidRequest(_))
        ));
        // A path inside the allowlist passes the check (then fails later only
        // because no VLM model is registered).
        assert!(matches!(
            engine
                .invoke(image_req(allowed.to_string_lossy().to_string()))
                .await,
            Err(EngineError::NoCapableModel(_))
        ));
        // http(s) and data: URLs are not local paths — the allowlist does not
        // apply to them.
        for remote in ["https://example.com/a.png", "data:image/png;base64,AAAA"] {
            assert!(
                matches!(
                    engine.invoke(image_req(remote.into())).await,
                    Err(EngineError::NoCapableModel(_))
                ),
                "{remote} must not be treated as a local path"
            );
        }
    }

    #[tokio::test]
    async fn subscription_without_explicit_ttl_uses_config_default() {
        // #4 review: `PreflightConfig::subscription_ttl_secs` must actually
        // bound subscriptions — previously nothing read it, so a subscription
        // that omitted the TTL pinned its models warm forever.
        let config = EngineConfig {
            preflight: PreflightConfig {
                subscription_ttl_secs: 120,
                ..PreflightConfig::default()
            },
            ..minimal_config()
        };
        let engine = Engine::new(config).await;

        engine.subscribe(None, None, vec![Capability::Chat], None);
        let info = engine.subscriptions();
        assert_eq!(info.len(), 1);
        let ttl = info[0]
            .expires_in_secs
            .expect("the configured default TTL must apply when none is given");
        assert!(ttl > 110 && ttl <= 120, "ttl {ttl} should be ~120");

        // An explicit per-call TTL still wins over the config default.
        engine.subscribe(
            None,
            None,
            vec![Capability::Tts],
            Some(Duration::from_secs(5)),
        );
        let explicit = engine
            .subscriptions()
            .into_iter()
            .find(|s| s.capabilities == vec![Capability::Tts])
            .and_then(|s| s.expires_in_secs)
            .expect("explicit TTL reported");
        assert!(explicit <= 5, "explicit TTL must win, got {explicit}");
    }

    #[tokio::test]
    async fn stream_reports_no_capable_model_as_error_chunk() {
        let engine = build_engine(vec![], 100, 0);
        let chunks = collect_stream(engine.invoke_stream(chat_request(None))).await;
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            StreamChunk::Error(info) => {
                assert_eq!(info.code, mofa_kernel::ErrorCode::NoCapableModel);
            }
            other => panic!("expected Error chunk, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_predispatch_failure_is_counted_in_metrics() {
        // A stream rejected *before* generation begins (here: no capable model)
        // must still be counted as a handled+failed request, exactly like the
        // equivalent unary `invoke` failure — otherwise the streaming path
        // undercounts failures in `mofa_requests_total` / `_failed_total`.
        let engine = build_engine(vec![], 100, 0);
        let chunks = collect_stream(engine.invoke_stream(chat_request(None))).await;
        assert!(matches!(chunks.as_slice(), [StreamChunk::Error(_)]));

        let metrics = engine.metrics_prometheus();
        assert!(
            metrics.contains("mofa_requests_total 1"),
            "pre-dispatch stream failure was not counted as a handled request:\n{metrics}"
        );
        assert!(
            metrics.contains("mofa_requests_failed_total 1"),
            "pre-dispatch stream failure was not counted as a failed request:\n{metrics}"
        );
    }
}
