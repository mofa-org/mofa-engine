//! Preflight prediction via per-scope Markov chains.
//!
//! Tracks transitions between capabilities (e.g. `Chat → Tts`) so the engine can
//! pre-warm the next model before it is requested. History is keyed by an
//! application/session **scope** so one application's behavior never pollutes
//! another's predictions, with a global chain as a fallback when a scope has not
//! yet accumulated enough samples.

use mofa_kernel::Capability;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Decay factor applied every `DECAY_INTERVAL` transitions.
const DECAY_FACTOR: f64 = 0.95;
/// Number of transitions between decay applications.
const DECAY_INTERVAL: u64 = 100;
/// Maximum number of distinct (non-global) scopes retained. Scope keys come from
/// caller-supplied app/session IDs, so this bounds memory against an unbounded
/// stream of unique identifiers by evicting the least-recently-used scope.
const MAX_SCOPES: usize = 2048;
/// Scope key under which cross-application history is aggregated.
pub(crate) const GLOBAL_SCOPE: &str = "__global__";

/// Per-scope Markov chain transition tracker with decay.
pub(crate) struct PreflightPredictor {
    inner: Mutex<HashMap<String, ScopeChain>>,
}

struct ScopeChain {
    /// Transition counts: from_capability → (to_capability → weight).
    transitions: HashMap<Capability, HashMap<Capability, f64>>,
    /// Total transitions observed in this scope.
    total_transitions: u64,
    /// Last capability observed in this scope.
    last_capability: Option<Capability>,
    /// Last time this scope was recorded into or predicted from (for LRU).
    last_access: Instant,
}

impl ScopeChain {
    fn new() -> Self {
        Self {
            transitions: HashMap::new(),
            total_transitions: 0,
            last_capability: None,
            last_access: Instant::now(),
        }
    }

    /// Observe a capability used within this scope, recording the `prev → this`
    /// transition against the scope's own running order.
    fn observe(&mut self, capability: Capability) {
        self.last_access = Instant::now();
        if let Some(prev) = self.last_capability {
            self.record_transition(prev, capability);
        }
        self.last_capability = Some(capability);
    }

    /// Add a single `from → to` transition to this chain's counts, applying
    /// decay on the interval. Unlike [`observe`](Self::observe) it does not
    /// touch `last_capability`, so it can mirror a transition that already
    /// happened elsewhere (e.g. into the global aggregate) without inventing a
    /// spurious edge from whatever this chain saw last.
    fn record_transition(&mut self, from: Capability, to: Capability) {
        self.last_access = Instant::now();
        *self
            .transitions
            .entry(from)
            .or_default()
            .entry(to)
            .or_insert(0.0) += 1.0;
        self.total_transitions += 1;

        if self.total_transitions.is_multiple_of(DECAY_INTERVAL) {
            for counts in self.transitions.values_mut() {
                for count in counts.values_mut() {
                    *count *= DECAY_FACTOR;
                }
            }
        }
    }

    fn predict(&self, current: Capability, min_samples: u64) -> Option<Prediction> {
        let counts = self.transitions.get(&current)?;
        let total: f64 = counts.values().sum();
        // Compare in f64: `total` may be fractional after decay, and a lossy
        // cast to u64 would be an unnecessary truncation.
        if total < min_samples as f64 {
            return None;
        }
        counts
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(cap, count)| Prediction {
                capability: *cap,
                confidence: count / total,
            })
    }
}

/// Prediction result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Prediction {
    /// Predicted next capability.
    pub capability: Capability,
    /// Confidence score (0.0 – 1.0).
    pub confidence: f64,
}

impl PreflightPredictor {
    /// Create a new predictor.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ScopeChain>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record that a capability was used within `scope`.
    ///
    /// The observation updates the scope's own chain, and the *same* per-scope
    /// transition (if any) is mirrored into the global aggregate. The global
    /// chain therefore only ever accumulates transitions that genuinely occurred
    /// within some scope — an edge that spanned two different scopes (because
    /// their requests interleaved) is never invented.
    pub(crate) fn record(&self, scope: &str, capability: Capability) {
        let mut scopes = self.lock();
        Self::evict_if_full(&mut scopes, scope);

        // Observe into the scope's own chain, capturing the transition edge it
        // just formed (its previous capability, if any).
        let edge_from = {
            let chain = scopes
                .entry(scope.to_string())
                .or_insert_with(ScopeChain::new);
            let prev = chain.last_capability;
            chain.observe(capability);
            prev
        };

        // Mirror only that real edge into the global aggregate.
        if scope != GLOBAL_SCOPE
            && let Some(from) = edge_from
        {
            scopes
                .entry(GLOBAL_SCOPE.to_string())
                .or_insert_with(ScopeChain::new)
                .record_transition(from, capability);
        }
    }

    /// Evict the least-recently-used non-global scope when at capacity and the
    /// incoming scope is new. The global scope is never evicted.
    fn evict_if_full(scopes: &mut HashMap<String, ScopeChain>, incoming: &str) {
        if scopes.len() < MAX_SCOPES || scopes.contains_key(incoming) {
            return;
        }
        if let Some(victim) = scopes
            .iter()
            .filter(|(k, _)| k.as_str() != GLOBAL_SCOPE)
            .min_by_key(|(_, c)| c.last_access)
            .map(|(k, _)| k.clone())
        {
            scopes.remove(&victim);
        }
    }

    /// Predict the next capability for `scope` given `current`.
    ///
    /// Tries the scope's own chain first; if it lacks enough samples or
    /// confidence, falls back to the global chain. Returns `None` when neither
    /// clears `min_samples` and `confidence_threshold`.
    pub(crate) fn predict(
        &self,
        scope: &str,
        current: Capability,
        min_samples: u64,
        confidence_threshold: f64,
    ) -> Option<Prediction> {
        let mut scopes = self.lock();
        // Touch the scope so an actively-predicted-from scope is not evicted.
        if let Some(chain) = scopes.get_mut(scope) {
            chain.last_access = Instant::now();
        }
        let scoped = scopes
            .get(scope)
            .and_then(|chain| chain.predict(current, min_samples))
            .filter(|p| p.confidence >= confidence_threshold);
        if scoped.is_some() {
            return scoped;
        }
        if scope == GLOBAL_SCOPE {
            return None;
        }
        scopes
            .get(GLOBAL_SCOPE)
            .and_then(|chain| chain.predict(current, min_samples))
            .filter(|p| p.confidence >= confidence_threshold)
    }

    /// Number of scopes currently retained (test/diagnostic use).
    #[cfg(test)]
    fn scope_count(&self) -> usize {
        self.lock().len()
    }
}

impl Default for PreflightPredictor {
    fn default() -> Self {
        Self::new()
    }
}

/// Live counters describing Preflight effectiveness.
#[derive(Debug, Default)]
pub(crate) struct PreflightMetrics {
    warms_started: AtomicU64,
    warms_completed: AtomicU64,
    warms_failed: AtomicU64,
    warms_skipped: AtomicU64,
    predictions: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
}

/// Serializable snapshot of [`PreflightMetrics`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightStats {
    /// Warm tasks spawned.
    pub warms_started: u64,
    /// Warm tasks that completed successfully.
    pub warms_completed: u64,
    /// Warm tasks that failed.
    pub warms_failed: u64,
    /// Warm requests skipped (deduplicated, disabled, or memory-unsafe).
    pub warms_skipped: u64,
    /// History/hint predictions produced.
    pub predictions: u64,
    /// Predictions later confirmed by the next request.
    pub hits: u64,
    /// Predictions the next request did not match.
    pub misses: u64,
}

impl PreflightMetrics {
    /// A warm task was spawned.
    pub(crate) fn warm_started(&self) {
        self.warms_started.fetch_add(1, Ordering::Relaxed);
    }
    /// A warm task completed successfully.
    pub(crate) fn warm_completed(&self) {
        self.warms_completed.fetch_add(1, Ordering::Relaxed);
    }
    /// A warm task failed.
    pub(crate) fn warm_failed(&self) {
        self.warms_failed.fetch_add(1, Ordering::Relaxed);
    }
    /// A warm was skipped (deduplicated, disabled, or unsafe).
    pub(crate) fn warm_skipped(&self) {
        self.warms_skipped.fetch_add(1, Ordering::Relaxed);
    }
    /// A prediction was produced.
    pub(crate) fn prediction(&self) {
        self.predictions.fetch_add(1, Ordering::Relaxed);
    }
    /// A prior prediction matched the next request.
    pub(crate) fn hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }
    /// A prior prediction did not match the next request.
    pub(crate) fn miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Read all counters into a `PreflightStats`.
    ///
    /// Each counter is read independently with relaxed ordering, so under heavy
    /// concurrent updates the fields may reflect slightly different instants;
    /// they are monotonic and eventually consistent, which is what the metrics
    /// consumers need.
    pub(crate) fn snapshot(&self) -> PreflightStats {
        PreflightStats {
            warms_started: self.warms_started.load(Ordering::Relaxed),
            warms_completed: self.warms_completed.load(Ordering::Relaxed),
            warms_failed: self.warms_failed.load(Ordering::Relaxed),
            warms_skipped: self.warms_skipped.load(Ordering::Relaxed),
            predictions: self.predictions.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLES: u64 = 3;
    const THRESHOLD: f64 = 0.6;

    #[test]
    fn no_prediction_without_observations() {
        let p = PreflightPredictor::new();
        assert!(
            p.predict("app", Capability::Chat, SAMPLES, THRESHOLD)
                .is_none()
        );
    }

    #[test]
    fn no_prediction_below_threshold() {
        let p = PreflightPredictor::new();
        p.record("app", Capability::Chat);
        p.record("app", Capability::Tts);
        // Only one Chat→Tts transition; need three samples.
        assert!(
            p.predict("app", Capability::Chat, SAMPLES, THRESHOLD)
                .is_none()
        );
    }

    #[test]
    fn predicts_after_enough_observations() {
        let p = PreflightPredictor::new();
        for _ in 0..4 {
            p.record("app", Capability::Chat);
            p.record("app", Capability::Tts);
        }
        let pred = p
            .predict("app", Capability::Chat, SAMPLES, THRESHOLD)
            .unwrap();
        assert_eq!(pred.capability, Capability::Tts);
        assert!(pred.confidence > 0.9);
    }

    #[test]
    fn scopes_do_not_pollute_each_other() {
        let p = PreflightPredictor::new();
        // App A always does Chat → Tts.
        for _ in 0..5 {
            p.record("app-a", Capability::Chat);
            p.record("app-a", Capability::Tts);
        }
        // App B always does Chat → Asr.
        for _ in 0..5 {
            p.record("app-b", Capability::Chat);
            p.record("app-b", Capability::Asr);
        }
        assert_eq!(
            p.predict("app-a", Capability::Chat, SAMPLES, THRESHOLD)
                .unwrap()
                .capability,
            Capability::Tts
        );
        assert_eq!(
            p.predict("app-b", Capability::Chat, SAMPLES, THRESHOLD)
                .unwrap()
                .capability,
            Capability::Asr
        );
    }

    #[test]
    fn unknown_scope_falls_back_to_global() {
        let p = PreflightPredictor::new();
        for _ in 0..5 {
            p.record("app-a", Capability::Chat);
            p.record("app-a", Capability::Tts);
        }
        // A brand-new scope has no history, but global aggregates app-a's.
        let pred = p
            .predict("fresh-scope", Capability::Chat, SAMPLES, THRESHOLD)
            .unwrap();
        assert_eq!(pred.capability, Capability::Tts);
    }

    #[test]
    fn global_chain_ignores_cross_scope_transitions() {
        let p = PreflightPredictor::new();
        // Two scopes run concurrently with interleaved requests. App A always
        // does Chat → Tts; app B always does Asr → Vlm. If the global chain
        // observed the interleaved stream it would learn phantom edges like
        // Chat → Asr (A's Chat followed by B's Asr), which never happened in any
        // real sequence.
        for _ in 0..6 {
            p.record("app-a", Capability::Chat);
            p.record("app-b", Capability::Asr);
            p.record("app-a", Capability::Tts);
            p.record("app-b", Capability::Vlm);
        }
        // A fresh scope falls back to the global aggregate. It must predict the
        // real edge Chat → Tts, not the phantom Chat → Asr the old code learned.
        let pred = p
            .predict("fresh", Capability::Chat, SAMPLES, THRESHOLD)
            .expect("global should predict the real transition");
        assert_eq!(pred.capability, Capability::Tts);
        assert!(pred.confidence > 0.9);
    }

    #[test]
    fn scope_count_is_bounded() {
        let p = PreflightPredictor::new();
        // Record far more distinct scopes than the cap allows.
        for i in 0..(MAX_SCOPES + 200) {
            let scope = format!("session-{i}");
            p.record(&scope, Capability::Chat);
            p.record(&scope, Capability::Tts);
        }
        // Bounded by the cap plus the always-retained global scope.
        assert!(
            p.scope_count() <= MAX_SCOPES + 1,
            "scope map grew to {} entries",
            p.scope_count()
        );
    }

    #[test]
    fn metrics_snapshot_counts() {
        let m = PreflightMetrics::default();
        m.warm_started();
        m.warm_started();
        m.warm_completed();
        m.hit();
        let snap = m.snapshot();
        assert_eq!(snap.warms_started, 2);
        assert_eq!(snap.warms_completed, 1);
        assert_eq!(snap.hits, 1);
        assert_eq!(snap.misses, 0);
    }
}
