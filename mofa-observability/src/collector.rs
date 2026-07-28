//! # MoFA Engine — Metrics Collector
//!
//! Receives engine events from a channel and updates in-memory metric state.
//! The Prometheus renderer reads this state when scraped.
//!
//! Design:
//! - Single writer (collector task), multiple readers (Prometheus scrapes).
//! - No blocking I/O. No disk. Just in-memory atomics behind a RwLock.
//! - Bounded event channel with drop-oldest backpressure.
//! - Memory gauge reconciles against absolute values from eviction events
//!   to prevent cumulative drift under event loss.

use crate::events::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};

// ─── Labels ──────────────────────────────────────────────────────────────────

/// Metric labels. Sorted by key for consistent ordering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Labels(Vec<(String, String)>);

impl Labels {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.push((key.into(), value.into()));
        self.0.sort_by(|a, b| a.0.cmp(&b.0));
        self
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for Labels {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Metric Types ────────────────────────────────────────────────────────────

/// A single histogram's accumulated state.
#[derive(Debug, Clone)]
pub struct HistogramValue {
    /// (upper_bound, cumulative_count) — sorted ascending.
    pub buckets: Vec<(f64, u64)>,
    pub sum: f64,
    pub count: u64,
}

impl HistogramValue {
    fn new(boundaries: &[f64]) -> Self {
        let buckets = boundaries.iter().map(|&b| (b, 0u64)).collect();
        Self {
            buckets,
            sum: 0.0,
            count: 0,
        }
    }

    fn observe(&mut self, value: f64) {
        self.sum += value;
        self.count += 1;
        for bucket in &mut self.buckets {
            if value <= bucket.0 {
                bucket.1 += 1;
            }
        }
    }
}

/// A family of labeled counters (e.g., mofa_requests_total with labels).
#[derive(Debug, Clone)]
pub struct CounterFamily {
    pub name: &'static str,
    pub help: &'static str,
    pub values: HashMap<Labels, u64>,
    pub last_seen: HashMap<Labels, Instant>,
}

impl CounterFamily {
    fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            values: HashMap::new(),
            last_seen: HashMap::new(),
        }
    }

    fn inc(&mut self, labels: Labels) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0) += 1;
    }

    fn inc_by(&mut self, labels: Labels, n: u64) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0) += n;
    }

    fn evict_stale(&mut self, now: Instant, max_idle: Duration) {
        let last_seen = &mut self.last_seen;
        self.values.retain(|k, _| {
            if let Some(&time) = last_seen.get(k) {
                now.saturating_duration_since(time) < max_idle
            } else {
                true
            }
        });
        self.last_seen
            .retain(|_, v| now.saturating_duration_since(*v) < max_idle);
    }
}

/// A family of labeled histograms.
#[derive(Debug, Clone)]
pub struct HistogramFamily {
    pub name: &'static str,
    pub help: &'static str,
    pub bucket_boundaries: Vec<f64>,
    pub values: HashMap<Labels, HistogramValue>,
    pub last_seen: HashMap<Labels, Instant>,
}

impl HistogramFamily {
    fn new(name: &'static str, help: &'static str, boundaries: Vec<f64>) -> Self {
        Self {
            name,
            help,
            bucket_boundaries: boundaries,
            values: HashMap::new(),
            last_seen: HashMap::new(),
        }
    }

    fn observe(&mut self, labels: Labels, value: f64) {
        self.last_seen.insert(labels.clone(), Instant::now());
        self.values
            .entry(labels)
            .or_insert_with(|| HistogramValue::new(&self.bucket_boundaries))
            .observe(value);
    }

    fn evict_stale(&mut self, now: Instant, max_idle: Duration) {
        let last_seen = &mut self.last_seen;
        self.values.retain(|k, _| {
            if let Some(&time) = last_seen.get(k) {
                now.saturating_duration_since(time) < max_idle
            } else {
                true
            }
        });
        self.last_seen
            .retain(|_, v| now.saturating_duration_since(*v) < max_idle);
    }
}

/// A family of labeled gauges.
#[derive(Debug, Clone)]
pub struct GaugeFamily {
    pub name: &'static str,
    pub help: &'static str,
    pub values: HashMap<Labels, f64>,
    pub last_seen: HashMap<Labels, Instant>,
}

impl GaugeFamily {
    fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            values: HashMap::new(),
            last_seen: HashMap::new(),
        }
    }

    pub fn set(&mut self, labels: Labels, value: f64) {
        self.last_seen.insert(labels.clone(), Instant::now());
        self.values.insert(labels, value);
    }

    fn inc(&mut self, labels: Labels) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0.0) += 1.0;
    }

    fn dec(&mut self, labels: Labels) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0.0) -= 1.0;
    }

    fn add(&mut self, labels: Labels, n: f64) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0.0) += n;
    }

    fn sub(&mut self, labels: Labels, n: f64) {
        self.last_seen.insert(labels.clone(), Instant::now());
        *self.values.entry(labels).or_insert(0.0) -= n;
    }

    fn evict_stale(&mut self, now: Instant, max_idle: Duration) {
        let last_seen = &mut self.last_seen;
        self.values.retain(|k, _| {
            if let Some(&time) = last_seen.get(k) {
                now.saturating_duration_since(time) < max_idle
            } else {
                true
            }
        });
        self.last_seen
            .retain(|_, v| now.saturating_duration_since(*v) < max_idle);
    }
}

// ─── Metrics State ───────────────────────────────────────────────────────────

/// All metric state. One instance per engine.
/// The collector writes, the Prometheus renderer reads.
#[derive(Debug, Clone)]
pub struct MetricsState {
    // ── Counters ─────────────────────────────────────────────────────────
    pub requests_total: CounterFamily,
    pub model_loads_total: CounterFamily,
    pub model_unloads_total: CounterFamily,
    pub failovers_total: CounterFamily,
    pub evictions_total: CounterFamily,
    pub preflight_predictions_total: CounterFamily,
    pub preflight_hits_total: CounterFamily,
    pub preflight_misses_total: CounterFamily,
    pub tokens_input_total: CounterFamily,
    pub tokens_output_total: CounterFamily,
    pub thought_tokens_total: CounterFamily,
    pub events_dropped_total: CounterFamily,

    // ── Histograms ───────────────────────────────────────────────────────
    pub request_duration_seconds: HistogramFamily,
    pub model_load_seconds: HistogramFamily,
    pub ttft_seconds: HistogramFamily,

    // ── Gauges ───────────────────────────────────────────────────────────
    pub memory_used_bytes: GaugeFamily,
    pub memory_budget_bytes: GaugeFamily,
    pub models_loaded: GaugeFamily,
    pub active_requests: GaugeFamily,
    pub estimated_cost_usd: GaugeFamily,
}

impl MetricsState {
    /// Create a fresh state with all metrics initialized.
    /// Bucket boundaries are calibrated to Phase 0 measurements.
    pub fn new() -> Self {
        Self {
            // Counters
            requests_total: CounterFamily::new("mofa_requests_total", "Total inference requests"),
            model_loads_total: CounterFamily::new(
                "mofa_model_loads_total",
                "Total model load operations",
            ),
            model_unloads_total: CounterFamily::new(
                "mofa_model_unloads_total",
                "Total model unload operations",
            ),
            failovers_total: CounterFamily::new("mofa_failovers_total", "Total failover events"),
            evictions_total: CounterFamily::new(
                "mofa_evictions_total",
                "Total memory pressure evictions",
            ),
            preflight_predictions_total: CounterFamily::new(
                "mofa_preflight_predictions_total",
                "Total Preflight predictions",
            ),
            preflight_hits_total: CounterFamily::new(
                "mofa_preflight_hits_total",
                "Total correct Preflight predictions",
            ),
            preflight_misses_total: CounterFamily::new(
                "mofa_preflight_misses_total",
                "Total incorrect Preflight predictions",
            ),
            tokens_input_total: CounterFamily::new(
                "mofa_tokens_input_total",
                "Total input tokens processed",
            ),
            tokens_output_total: CounterFamily::new(
                "mofa_tokens_output_total",
                "Total output tokens generated",
            ),
            thought_tokens_total: CounterFamily::new(
                "mofa_thought_tokens_total",
                "Total reasoning thought tokens generated",
            ),
            estimated_cost_usd: GaugeFamily::new(
                "mofa_estimated_cost_usd",
                "Total estimated USD cost incurred",
            ),
            events_dropped_total: CounterFamily::new(
                "mofa_events_dropped_total",
                "Total events dropped due to channel backpressure",
            ),

            // Histograms — bucket boundaries from Phase 0 measurements.
            //
            // mofa_request_duration_seconds:
            //   Phase 0: article gen = 13–18s, warm inference = 0.1–0.6s.
            request_duration_seconds: HistogramFamily::new(
                "mofa_request_duration_seconds",
                "Request duration in seconds",
                vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0, 20.0, 30.0, 60.0],
            ),
            // mofa_model_load_seconds:
            //   Phase 0: 1.5s (Qwen 7B Mac) to 97s (Qwen 7B FP16 CUDA).
            //   Most common range: 2–5s.
            model_load_seconds: HistogramFamily::new(
                "mofa_model_load_seconds",
                "Time to load a model from cold",
                vec![0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0],
            ),
            // mofa_ttft_seconds:
            //   Phase 0: warm TTFT = 0.1–0.2s (Mac), 0.4–0.6s (CUDA).
            //   Cold TTFT = 1.5–5.5s. Fine granularity in 0.1–0.2s range
            //   because that's where Preflight's value shows up.
            ttft_seconds: HistogramFamily::new(
                "mofa_ttft_seconds",
                "Time to first token",
                vec![0.05, 0.1, 0.15, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0],
            ),

            // Gauges
            memory_used_bytes: GaugeFamily::new(
                "mofa_memory_used_bytes",
                "Current engine memory usage in bytes",
            ),
            memory_budget_bytes: GaugeFamily::new(
                "mofa_memory_budget_bytes",
                "Total engine memory budget in bytes",
            ),
            models_loaded: GaugeFamily::new(
                "mofa_models_loaded",
                "Number of models currently loaded",
            ),
            active_requests: GaugeFamily::new(
                "mofa_active_requests",
                "Number of requests currently in flight",
            ),
        }
    }

    /// Process a single engine event, updating all relevant metrics.
    pub fn process_event(&mut self, envelope: &EventEnvelope) {
        match &envelope.event {
            EngineEvent::RequestReceived(e) => {
                self.active_requests.inc(Labels::new());
                // RequestReceived doesn't increment requests_total —
                // that happens on RequestCompleted with success/error status.
                let _ = e; // capability is used for routing, not counted here.
            }

            EngineEvent::RoutingDecision(_) => {
                // Routing metadata is captured for future decision-record phases.
                // Phase A: no derived metrics from routing decisions.
            }

            EngineEvent::RequestCompleted(e) => {
                tracing::info!(
                    trace_id = ?envelope.trace_id,
                    request_id = ?envelope.request_id,
                    model = %e.model_id,
                    duration_ms = e.duration_ms,
                    success = e.success,
                    "Request completed"
                );

                // Counter: requests by capability × provider × locality × model × status
                let status = if e.success { "success" } else { "error" };
                let backend_lower = e.backend.to_lowercase();
                let locality = if backend_lower == "ollama"
                    || backend_lower == "kokoro"
                    || backend_lower == "funasr"
                    || backend_lower == "local"
                {
                    "local"
                } else {
                    "cloud"
                };

                self.requests_total.inc(
                    Labels::new()
                        .add("capability", e.capability.to_string())
                        .add("provider", &e.backend)
                        .add("locality", locality)
                        .add("model", &e.model_id)
                        .add("status", status),
                );

                // Histogram: request duration
                self.request_duration_seconds.observe(
                    Labels::new().add("capability", e.capability.to_string()),
                    e.duration_ms as f64 / 1000.0,
                );

                // Histogram: TTFT (if present)
                if let Some(ttft) = e.ttft_ms {
                    self.ttft_seconds.observe(
                        Labels::new()
                            .add("model", &e.model_id)
                            .add("backend", &e.backend),
                        ttft as f64 / 1000.0,
                    );
                }

                // Counter: tokens & cost
                let tokens_in = e.tokens_in.unwrap_or(180);
                let tokens_out = e.tokens_out.unwrap_or(320);

                self.tokens_input_total
                    .inc_by(Labels::new().add("model", &e.model_id), tokens_in);
                self.tokens_output_total
                    .inc_by(Labels::new().add("model", &e.model_id), tokens_out);

                let cost = crate::pricing::estimate_cost_usd(
                    &e.backend,
                    &e.model_id,
                    tokens_in as u32,
                    tokens_out as u32,
                );
                let backend_lower = e.backend.to_lowercase();
                let locality = if backend_lower == "ollama"
                    || backend_lower == "kokoro"
                    || backend_lower == "funasr"
                    || backend_lower == "local"
                {
                    "local"
                } else {
                    "cloud"
                };
                self.estimated_cost_usd.add(
                    Labels::new()
                        .add("provider", &e.backend)
                        .add("locality", locality)
                        .add("model", &e.model_id),
                    cost,
                );

                // Gauge: active requests down
                self.active_requests.dec(Labels::new());
            }

            EngineEvent::ModelLoaded(e) => {
                tracing::info!(
                    model = %e.model_id,
                    backend = %e.backend,
                    load_duration_ms = e.load_duration_ms,
                    "Model loaded"
                );

                // Counter
                self.model_loads_total.inc(
                    Labels::new()
                        .add("model", &e.model_id)
                        .add("backend", &e.backend),
                );

                // Histogram: load duration
                self.model_load_seconds.observe(
                    Labels::new()
                        .add("model", &e.model_id)
                        .add("backend", &e.backend),
                    e.load_duration_ms as f64 / 1000.0,
                );

                // Gauge: memory up, model count up
                self.memory_used_bytes
                    .add(Labels::new(), e.memory_bytes as f64);
                self.models_loaded.inc(Labels::new());
            }

            EngineEvent::ModelUnloaded(e) => {
                // Counter
                self.model_unloads_total.inc(
                    Labels::new()
                        .add("model", &e.model_id)
                        .add("reason", e.reason.to_string()),
                );

                // Gauge: memory down, model count down
                self.memory_used_bytes
                    .sub(Labels::new(), e.memory_freed_bytes as f64);
                self.models_loaded.dec(Labels::new());
            }

            EngineEvent::EvictionTriggered(e) => {
                tracing::warn!(
                    evicted_model = %e.evicted_model,
                    memory_before = e.memory_before_bytes,
                    memory_after = e.memory_after_bytes,
                    "Eviction triggered"
                );

                self.evictions_total.inc(Labels::new());

                // Reconcile memory gauge against the absolute post-eviction value.
                // This corrects any cumulative drift caused by dropped ModelUnloaded
                // the authoritative value from the eviction subsystem.
                self.memory_used_bytes
                    .set(Labels::new(), e.memory_after_bytes as f64);
                self.memory_budget_bytes
                    .set(Labels::new(), e.budget_bytes as f64);
            }

            EngineEvent::PreflightSignal(e) => {
                self.preflight_predictions_total
                    .inc(Labels::new().add("source", e.source.to_string()));
            }

            EngineEvent::PreflightHit(_) => {
                self.preflight_hits_total.inc(Labels::new());
            }

            EngineEvent::PreflightMiss(_) => {
                self.preflight_misses_total.inc(Labels::new());
            }

            EngineEvent::ProviderDiscovered(_) => {
                // Logged, not metricked. Provider discovery is a startup event.
            }

            EngineEvent::FailoverTriggered(e) => {
                tracing::error!(
                    failed_model = %e.failed_model,
                    fallback_model = %e.fallback_model,
                    "Failover triggered"
                );

                // We don't have capability on FailoverTriggered, so label-free.
                let _ = e;
                self.failovers_total.inc(Labels::new());
            }
        }
    }

    /// Evict metric labels that haven't been updated in `max_idle` duration.
    pub fn evict_stale_labels(&mut self, max_idle: Duration) {
        let now = Instant::now();

        // Counters
        self.requests_total.evict_stale(now, max_idle);
        self.model_loads_total.evict_stale(now, max_idle);
        self.model_unloads_total.evict_stale(now, max_idle);
        self.failovers_total.evict_stale(now, max_idle);
        self.evictions_total.evict_stale(now, max_idle);
        self.preflight_predictions_total.evict_stale(now, max_idle);
        self.preflight_hits_total.evict_stale(now, max_idle);
        self.preflight_misses_total.evict_stale(now, max_idle);
        self.tokens_input_total.evict_stale(now, max_idle);
        self.tokens_output_total.evict_stale(now, max_idle);
        self.events_dropped_total.evict_stale(now, max_idle);

        // Histograms
        self.request_duration_seconds.evict_stale(now, max_idle);
        self.model_load_seconds.evict_stale(now, max_idle);
        self.ttft_seconds.evict_stale(now, max_idle);

        // Gauges
        self.memory_used_bytes.evict_stale(now, max_idle);
        self.memory_budget_bytes.evict_stale(now, max_idle);
        self.models_loaded.evict_stale(now, max_idle);
        self.active_requests.evict_stale(now, max_idle);
        self.estimated_cost_usd.evict_stale(now, max_idle);
    }
}

impl Default for MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Event Channel ───────────────────────────────────────────────────────────

/// Sender half of the event channel. Clone this for each event source.
#[derive(Clone)]
pub struct EventSender {
    tx: mpsc::Sender<EventEnvelope>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl EventSender {
    /// Send an event. If the channel is full, the event is dropped
    /// and the dropped counter is incremented. Use for high-volume telemetry.
    pub fn send(&self, event: EventEnvelope) {
        if self.tx.try_send(event).is_err() {
            self.dropped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Send a critical event asynchronously. If the channel is full, this will await
    /// and apply backpressure to the caller instead of dropping the event.
    /// Use for rare, critical events like `FailoverTriggered` and `EvictionTriggered`.
    pub async fn send_critical(
        &self,
        event: EventEnvelope,
    ) -> Result<(), mpsc::error::SendError<EventEnvelope>> {
        self.tx.send(event).await
    }

    /// Send a critical event synchronously. If the channel is full, this will block
    /// the current thread. Must not be called from within an async runtime context.
    #[allow(clippy::result_large_err)]
    pub fn send_blocking(
        &self,
        event: EventEnvelope,
    ) -> Result<(), mpsc::error::SendError<EventEnvelope>> {
        self.tx.blocking_send(event)
    }

    /// How many events have been dropped due to backpressure.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Receiver half of the event channel. Owned by the collector.
pub struct EventReceiver {
    rx: mpsc::Receiver<EventEnvelope>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

/// Create a bounded event channel.
///
/// `capacity`: max events buffered before backpressure kicks in.
/// Returns (sender, receiver). Clone the sender for multiple producers.
pub fn create_event_channel(capacity: usize) -> (EventSender, EventReceiver) {
    let (tx, rx) = mpsc::channel(capacity);
    let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));

    let sender = EventSender {
        tx,
        dropped: Arc::clone(&dropped),
    };
    let receiver = EventReceiver { rx, dropped };

    (sender, receiver)
}

// ─── Metrics Collector ───────────────────────────────────────────────────────

/// The collector. Owns the shared metrics state and runs the processing loop.
///
/// Usage:
/// ```ignore
/// let (sender, receiver) = create_event_channel(1024);
/// let collector = MetricsCollector::new(receiver);
/// let state = collector.state(); // share with Prometheus renderer
/// tokio::spawn(collector.run());
/// ```
pub struct MetricsCollector {
    state: Arc<RwLock<MetricsState>>,
    receiver: EventReceiver,
}

impl MetricsCollector {
    pub fn new(receiver: EventReceiver) -> Self {
        Self {
            state: Arc::new(RwLock::new(MetricsState::new())),
            receiver,
        }
    }

    /// Get a handle to the shared metrics state.
    /// Pass this to the Prometheus renderer.
    pub fn state(&self) -> Arc<RwLock<MetricsState>> {
        Arc::clone(&self.state)
    }

    /// Run the collector loop. Processes events until the channel closes.
    /// Also periodically evicts stale metric labels to prevent memory leaks.
    pub async fn run(mut self) {
        let mut cleanup_interval = tokio::time::interval(Duration::from_secs(60));
        let max_idle = Duration::from_secs(600); // 10 minutes

        loop {
            tokio::select! {
                envelope_opt = self.receiver.rx.recv() => {
                    match envelope_opt {
                        Some(envelope) => {
                            let mut state = self.state.write().await;

                            // Sync the dropped counter into metrics state.
                            let dropped = self
                                .receiver
                                .dropped
                                .swap(0, std::sync::atomic::Ordering::Relaxed);
                            if dropped > 0 {
                                state
                                    .events_dropped_total
                                    .inc_by(Labels::new(), dropped);
                            }

                            state.process_event(&envelope);
                        }
                        None => break, // Channel closed
                    }
                }
                _ = cleanup_interval.tick() => {
                    let mut state = self.state.write().await;
                    state.evict_stale_labels(max_idle);
                }
            }
        }
    }
}

/// Initialize the observability pipeline.
/// Spawns the collector loop and returns the shared state and event sender.
pub fn init_pipeline(_otlp_endpoint: Option<&str>) -> (Arc<RwLock<MetricsState>>, EventSender) {
    let (sender, receiver) = create_event_channel(10000);
    let collector_loop = MetricsCollector::new(receiver);
    let state = collector_loop.state();

    tokio::spawn(async move {
        collector_loop.run().await;
    });

    (state, sender)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> MetricsState {
        MetricsState::new()
    }

    // ── Counter tests ────────────────────────────────────────────────────

    #[test]
    fn test_request_completed_increments_counter() {
        let mut state = make_state();
        let event = EventEnvelope::now(EngineEvent::RequestCompleted(RequestCompleted {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            duration_ms: 15000,
            ttft_ms: Some(115),
            tokens_in: Some(50),
            tokens_out: Some(277),
            model_was_hot: Some(true),
            success: true,
            error_code: None,
        }));

        state.process_event(&event);

        let labels = Labels::new()
            .add("capability", "chat")
            .add("provider", "ollama")
            .add("locality", "local")
            .add("model", "qwen2.5:7b")
            .add("status", "success");
        assert_eq!(state.requests_total.values[&labels], 1);
    }

    #[test]
    fn test_five_requests_counted() {
        let mut state = make_state();

        for _ in 0..5 {
            let event = EventEnvelope::now(EngineEvent::RequestCompleted(RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: true,
                error_code: None,
            }));
            state.process_event(&event);
        }

        let labels = Labels::new()
            .add("capability", "chat")
            .add("provider", "ollama")
            .add("locality", "local")
            .add("model", "qwen2.5:7b")
            .add("status", "success");
        assert_eq!(state.requests_total.values[&labels], 5);
    }

    #[test]
    fn test_error_requests_separate_from_success() {
        let mut state = make_state();

        // 3 successes
        for _ in 0..3 {
            state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
                RequestCompleted {
                    model_id: "qwen2.5:7b".into(),
                    backend: "ollama".into(),
                    capability: Capability::Chat,
                    duration_ms: 15000,
                    ttft_ms: None,
                    tokens_in: None,
                    tokens_out: None,
                    model_was_hot: None,
                    success: true,
                    error_code: None,
                },
            )));
        }

        // 1 error
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 500,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: false,
                error_code: Some("model_not_found".into()),
            },
        )));

        let success = Labels::new()
            .add("capability", "chat")
            .add("provider", "ollama")
            .add("locality", "local")
            .add("model", "qwen2.5:7b")
            .add("status", "success");
        let error = Labels::new()
            .add("capability", "chat")
            .add("provider", "ollama")
            .add("locality", "local")
            .add("model", "qwen2.5:7b")
            .add("status", "error");

        assert_eq!(state.requests_total.values[&success], 3);
        assert_eq!(state.requests_total.values[&error], 1);
    }

    // ── Histogram tests ──────────────────────────────────────────────────

    #[test]
    fn test_model_load_histogram_bucketing() {
        let mut state = make_state();

        // Phase 0: Qwen 7B cold load on Mac = 1.532s
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));

        let labels = Labels::new()
            .add("backend", "ollama")
            .add("model", "qwen2.5:7b");
        let hist = &state.model_load_seconds.values[&labels];

        // 1.532s should be in the le=2.0 bucket but NOT le=1.0
        assert_eq!(hist.buckets[1].1, 0); // le=1.0: no
        assert_eq!(hist.buckets[2].1, 1); // le=2.0: yes
        assert_eq!(hist.count, 1);
        assert!((hist.sum - 1.532).abs() < 0.001);
    }

    #[test]
    fn test_ttft_histogram_warm_vs_cold() {
        let mut state = make_state();

        // Phase 0: warm TTFT = 0.115s
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: Some(115),
                tokens_in: None,
                tokens_out: None,
                model_was_hot: Some(true),
                success: true,
                error_code: None,
            },
        )));

        let labels = Labels::new()
            .add("backend", "ollama")
            .add("model", "qwen2.5:7b");
        let hist = &state.ttft_seconds.values[&labels];

        // 0.115s should be in le=0.15 but NOT le=0.1
        assert_eq!(hist.buckets[1].1, 0); // le=0.1: no
        assert_eq!(hist.buckets[2].1, 1); // le=0.15: yes
    }

    // ── Gauge tests ──────────────────────────────────────────────────────

    #[test]
    fn test_models_loaded_gauge_up_down() {
        let mut state = make_state();

        // Load a model
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));
        assert_eq!(state.models_loaded.values[&Labels::new()], 1.0);

        // Load another
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "gemma3:4b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 2428,
            memory_bytes: 3_300_000_000,
        })));
        assert_eq!(state.models_loaded.values[&Labels::new()], 2.0);

        // Unload one
        state.process_event(&EventEnvelope::now(EngineEvent::ModelUnloaded(
            ModelUnloaded {
                model_id: "gemma3:4b".into(),
                reason: UnloadReason::IdleTimeout,
                memory_freed_bytes: 3_300_000_000,
            },
        )));
        assert_eq!(state.models_loaded.values[&Labels::new()], 1.0);
    }

    #[test]
    fn test_memory_gauge_tracks_bytes() {
        let mut state = make_state();

        // Load Qwen 7B (4.7 GB)
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));
        assert_eq!(
            state.memory_used_bytes.values[&Labels::new()],
            4_700_000_000.0
        );

        // Load Gemma 4B (3.3 GB)
        state.process_event(&EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "gemma3:4b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 2428,
            memory_bytes: 3_300_000_000,
        })));
        assert_eq!(
            state.memory_used_bytes.values[&Labels::new()],
            8_000_000_000.0
        );

        // Unload Gemma
        state.process_event(&EventEnvelope::now(EngineEvent::ModelUnloaded(
            ModelUnloaded {
                model_id: "gemma3:4b".into(),
                reason: UnloadReason::Eviction,
                memory_freed_bytes: 3_300_000_000,
            },
        )));
        assert_eq!(
            state.memory_used_bytes.values[&Labels::new()],
            4_700_000_000.0
        );
    }

    #[test]
    fn test_active_requests_gauge() {
        let mut state = make_state();

        // Request arrives
        state.process_event(&EventEnvelope::now(EngineEvent::RequestReceived(
            RequestReceived {
                capability: Capability::Chat,
                model: None,
                hint: Some("tts".into()),
            },
        )));
        assert_eq!(state.active_requests.values[&Labels::new()], 1.0);

        // Another arrives
        state.process_event(&EventEnvelope::now(EngineEvent::RequestReceived(
            RequestReceived {
                capability: Capability::Tts,
                model: None,
                hint: None,
            },
        )));
        assert_eq!(state.active_requests.values[&Labels::new()], 2.0);

        // First completes
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: None,
                tokens_in: None,
                tokens_out: None,
                model_was_hot: None,
                success: true,
                error_code: None,
            },
        )));
        assert_eq!(state.active_requests.values[&Labels::new()], 1.0);
    }

    // ── Preflight tests ──────────────────────────────────────────────────

    #[test]
    fn test_preflight_accuracy_from_counters() {
        let mut state = make_state();

        // 3 hits + 1 miss = 75% accuracy
        for _ in 0..3 {
            state.process_event(&EventEnvelope::now(EngineEvent::PreflightHit(
                PreflightHit {
                    predicted_capability: Capability::Tts,
                    cold_start_avoided_ms: 2404,
                },
            )));
        }
        state.process_event(&EventEnvelope::now(EngineEvent::PreflightMiss(
            PreflightMiss {
                predicted_capability: Capability::Tts,
                actual_capability: Capability::Chat,
            },
        )));

        let hits = state.preflight_hits_total.values[&Labels::new()];
        let misses = state.preflight_misses_total.values[&Labels::new()];
        let accuracy = hits as f64 / (hits + misses) as f64;

        assert_eq!(hits, 3);
        assert_eq!(misses, 1);
        assert!((accuracy - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_preflight_predictions_by_source() {
        let mut state = make_state();

        // 2 hint predictions + 1 history prediction
        for _ in 0..2 {
            state.process_event(&EventEnvelope::now(EngineEvent::PreflightSignal(
                PreflightSignal {
                    predicted_capability: Capability::Tts,
                    confidence: 1.0,
                    source: SignalSource::Hint,
                },
            )));
        }
        state.process_event(&EventEnvelope::now(EngineEvent::PreflightSignal(
            PreflightSignal {
                predicted_capability: Capability::Tts,
                confidence: 0.9,
                source: SignalSource::History,
            },
        )));

        let hint_labels = Labels::new().add("source", "hint");
        let history_labels = Labels::new().add("source", "history");

        assert_eq!(state.preflight_predictions_total.values[&hint_labels], 2);
        assert_eq!(state.preflight_predictions_total.values[&history_labels], 1);
    }

    // ── Token counting tests ─────────────────────────────────────────────

    #[test]
    fn test_token_counting() {
        let mut state = make_state();

        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "qwen2.5:7b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 15000,
                ttft_ms: None,
                tokens_in: Some(50),
                tokens_out: Some(277),
                model_was_hot: None,
                success: true,
                error_code: None,
            },
        )));

        let labels = Labels::new().add("model", "qwen2.5:7b");
        assert_eq!(state.tokens_input_total.values[&labels], 50);
        assert_eq!(state.tokens_output_total.values[&labels], 277);
    }

    // ── Channel + collector integration test ─────────────────────────────

    #[tokio::test]
    async fn test_collector_processes_events_from_channel() {
        let (sender, receiver) = create_event_channel(64);
        let collector = MetricsCollector::new(receiver);
        let state = collector.state();

        // Start collector in background
        let handle = tokio::spawn(collector.run());

        // Send events
        sender.send(EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
            model_id: "qwen2.5:7b".into(),
            backend: "ollama".into(),
            capability: Capability::Chat,
            load_duration_ms: 1532,
            memory_bytes: 4_700_000_000,
        })));

        sender.send(EventEnvelope::now(EngineEvent::PreflightSignal(
            PreflightSignal {
                predicted_capability: Capability::Tts,
                confidence: 1.0,
                source: SignalSource::Hint,
            },
        )));

        // Drop sender to close channel — collector loop will exit
        drop(sender);
        handle.await.unwrap();

        // Verify state
        let state = state.read().await;
        assert_eq!(state.models_loaded.values[&Labels::new()], 1.0);

        let hint_labels = Labels::new().add("source", "hint");
        assert_eq!(state.preflight_predictions_total.values[&hint_labels], 1);
    }

    // ── Backpressure test ────────────────────────────────────────────────

    #[test]
    fn test_backpressure_counts_drops() {
        // Channel of size 2
        let (sender, _receiver) = create_event_channel(2);

        // Fill channel
        for _ in 0..5 {
            sender.send(EventEnvelope::now(EngineEvent::EvictionTriggered(
                EvictionTriggered {
                    evicted_model: "test".into(),
                    memory_before_bytes: 100,
                    memory_after_bytes: 50,
                    budget_bytes: 100,
                },
            )));
        }

        // At least some should be dropped (channel size = 2, sent 5)
        assert!(sender.dropped_count() >= 3);
    }

    #[tokio::test]
    async fn test_send_critical_blocks() {
        // Channel of size 1
        let (sender, mut receiver) = create_event_channel(1);

        // Fill channel
        sender.send(EventEnvelope::now(EngineEvent::EvictionTriggered(
            EvictionTriggered {
                evicted_model: "test1".into(),
                memory_before_bytes: 100,
                memory_after_bytes: 50,
                budget_bytes: 100,
            },
        )));

        // Try to send critical event. It should block because the channel is full.
        // We use tokio::time::timeout to prove it blocks.
        let critical_event =
            EventEnvelope::now(EngineEvent::FailoverTriggered(FailoverTriggered {
                failed_model: "failed".into(),
                failed_backend: "backend".into(),
                fallback_model: "fallback".into(),
                fallback_backend: "backend".into(),
            }));

        let send_future = sender.send_critical(critical_event.clone());
        let result = tokio::time::timeout(Duration::from_millis(50), send_future).await;

        // It should timeout because it's blocking
        assert!(result.is_err());

        // Now drain the channel
        let _ = receiver.rx.recv().await;

        // Sending should now succeed
        let send_result = sender.send_critical(critical_event).await;
        assert!(send_result.is_ok());
    }

    // ── Label cardinality test ───────────────────────────────────────────

    #[test]
    fn test_label_cardinality_bounded() {
        let mut state = make_state();

        // Simulate 100 events across all 7 capabilities × 2 statuses = 14 series max
        let capabilities = [
            Capability::Chat,
            Capability::Tts,
            Capability::Asr,
            Capability::Vlm,
            Capability::ImageGen,
            Capability::VideoGen,
            Capability::Embedding,
        ];

        for cap in &capabilities {
            for success in [true, false] {
                state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
                    RequestCompleted {
                        model_id: "test".into(),
                        backend: "ollama".into(),
                        capability: *cap,
                        duration_ms: 1000,
                        ttft_ms: None,
                        tokens_in: None,
                        tokens_out: None,
                        model_was_hot: None,
                        success,
                        error_code: None,
                    },
                )));
            }
        }

        // 7 capabilities × 2 statuses = 14 series. Within defined cardinality bounds.
        assert_eq!(state.requests_total.values.len(), 14);
    }

    #[test]
    fn test_evict_stale_labels() {
        let mut state = make_state();
        let labels_fresh = Labels::new().add("model", "fresh");
        let labels_stale = Labels::new().add("model", "stale");

        // Insert labels
        state.requests_total.inc(labels_fresh.clone());
        state.requests_total.inc(labels_stale.clone());

        // Override the last_seen for the stale label to be 20 minutes ago
        let twenty_mins_ago = Instant::now() - Duration::from_secs(1200);
        state
            .requests_total
            .last_seen
            .insert(labels_stale.clone(), twenty_mins_ago);

        // Evict labels older than 10 minutes
        state.evict_stale_labels(Duration::from_secs(600));

        // The stale label should be gone, the fresh one should remain
        assert!(state.requests_total.values.contains_key(&labels_fresh));
        assert!(!state.requests_total.values.contains_key(&labels_stale));
        assert!(state.requests_total.last_seen.contains_key(&labels_fresh));
        assert!(!state.requests_total.last_seen.contains_key(&labels_stale));
    }

    #[test]
    fn test_cost_accumulation_local_vs_cloud() {
        let mut state = make_state();

        // Local request (Ollama) -> $0.00
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "gemma3:4b".into(),
                backend: "ollama".into(),
                capability: Capability::Chat,
                duration_ms: 1000,
                ttft_ms: None,
                tokens_in: Some(1000),
                tokens_out: Some(1000),
                model_was_hot: Some(true),
                success: true,
                error_code: None,
            },
        )));

        // Cloud request (OpenAI gpt-4o) -> $0.0125
        state.process_event(&EventEnvelope::now(EngineEvent::RequestCompleted(
            RequestCompleted {
                model_id: "gpt-4o".into(),
                backend: "openai".into(),
                capability: Capability::Chat,
                duration_ms: 500,
                ttft_ms: None,
                tokens_in: Some(1000),
                tokens_out: Some(1000),
                model_was_hot: Some(true),
                success: true,
                error_code: None,
            },
        )));

        let local_labels = Labels::new()
            .add("provider", "ollama")
            .add("locality", "local")
            .add("model", "gemma3:4b");
        let cloud_labels = Labels::new()
            .add("provider", "openai")
            .add("locality", "cloud")
            .add("model", "gpt-4o");

        assert_eq!(
            *state.estimated_cost_usd.values.get(&local_labels).unwrap(),
            0.0
        );
        let cloud_cost = *state.estimated_cost_usd.values.get(&cloud_labels).unwrap();
        assert!((cloud_cost - 0.0125).abs() < 1e-5);
    }
}
