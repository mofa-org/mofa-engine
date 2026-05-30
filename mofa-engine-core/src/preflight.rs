//! Preflight prediction via Markov chain.
//!
//! Tracks transitions between capabilities (e.g. Chat → Tts) and
//! predicts the next capability so the engine can pre-warm models.

use mofa_kernel::Capability;
use std::collections::HashMap;
use std::sync::Mutex;

/// Decay factor applied every `DECAY_INTERVAL` transitions.
const DECAY_FACTOR: f64 = 0.95;
/// Number of transitions between decay applications.
const DECAY_INTERVAL: u64 = 100;
/// Minimum observations before making a prediction.
const MIN_OBSERVATIONS: u64 = 3;

/// Markov chain transition tracker with decay.
pub struct PreflightPredictor {
    inner: Mutex<PreflightInner>,
}

struct PreflightInner {
    /// Transition counts: from_capability → (to_capability → count)
    transitions: HashMap<Capability, HashMap<Capability, f64>>,
    /// Total transitions observed
    total_transitions: u64,
    /// Last capability observed
    last_capability: Option<Capability>,
}

/// Prediction result.
#[derive(Debug, Clone)]
pub struct Prediction {
    /// Predicted next capability
    pub capability: Capability,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
}

impl PreflightPredictor {
    /// Create a new predictor.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PreflightInner {
                transitions: HashMap::new(),
                total_transitions: 0,
                last_capability: None,
            }),
        }
    }

    /// Record that a capability was just used.
    /// This updates the transition counts from the previous capability.
    pub fn record(&self, capability: Capability) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if let Some(prev) = inner.last_capability {
            let entry = inner
                .transitions
                .entry(prev)
                .or_default()
                .entry(capability)
                .or_insert(0.0);
            *entry += 1.0;

            inner.total_transitions += 1;

            // Apply decay periodically
            if inner.total_transitions.is_multiple_of(DECAY_INTERVAL) {
                for counts in inner.transitions.values_mut() {
                    for count in counts.values_mut() {
                        *count *= DECAY_FACTOR;
                    }
                }
            }
        }

        inner.last_capability = Some(capability);
    }

    /// Predict the most likely next capability given the current one.
    ///
    /// Returns `None` if there are not enough observations.
    pub fn predict(&self, current: Capability) -> Option<Prediction> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        let counts = inner.transitions.get(&current)?;
        let total: f64 = counts.values().sum();

        if (total as u64) < MIN_OBSERVATIONS {
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

impl Default for PreflightPredictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_prediction_without_observations() {
        let p = PreflightPredictor::new();
        assert!(p.predict(Capability::Chat).is_none());
    }

    #[test]
    fn no_prediction_below_threshold() {
        let p = PreflightPredictor::new();
        p.record(Capability::Chat);
        p.record(Capability::Tts);
        // Only 1 transition Chat→Tts, need 3
        assert!(p.predict(Capability::Chat).is_none());
    }

    #[test]
    fn predicts_after_enough_observations() {
        let p = PreflightPredictor::new();

        // Record Chat → Tts four times
        for _ in 0..4 {
            p.record(Capability::Chat);
            p.record(Capability::Tts);
        }

        let pred = p.predict(Capability::Chat).unwrap();
        assert_eq!(pred.capability, Capability::Tts);
        assert!(pred.confidence > 0.9);
    }

    #[test]
    fn handles_multiple_successors() {
        let p = PreflightPredictor::new();

        // Chat → Tts (3x), Chat → Asr (1x)
        for _ in 0..3 {
            p.record(Capability::Chat);
            p.record(Capability::Tts);
        }
        p.record(Capability::Chat);
        p.record(Capability::Asr);

        let pred = p.predict(Capability::Chat).unwrap();
        assert_eq!(pred.capability, Capability::Tts);
        assert!(pred.confidence > 0.7);
    }
}
