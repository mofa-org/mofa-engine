//! Per-provider circuit breaker.
//!
//! Prevents cascading failures by temporarily stopping requests to
//! unhealthy providers. Three states: Closed → Open → HalfOpen → Closed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Provider is deemed unhealthy — all requests fail fast.
    Open,
    /// Probe mode — one request is allowed through to test recovery.
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone)]
pub(crate) struct CircuitBreakerConfig {
    /// Number of consecutive failures before opening the circuit.
    pub failure_threshold: u32,
    /// Seconds to wait in Open before transitioning to HalfOpen.
    pub cool_down_secs: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cool_down_secs: 30,
        }
    }
}

struct ProviderBreaker {
    state: CircuitState,
    failure_count: u32,
    last_failure: Option<Instant>,
    /// Whether a half-open probe has been admitted and is awaiting its outcome.
    /// Guarantees that exactly one probe is in flight while half-open.
    probe_in_flight: bool,
    /// When the in-flight half-open probe was admitted, used to self-heal if the
    /// caller never records the probe's outcome.
    probe_started: Option<Instant>,
    config: CircuitBreakerConfig,
}

impl ProviderBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure: None,
            probe_in_flight: false,
            probe_started: None,
            config,
        }
    }

    /// How long to wait for a half-open probe to report an outcome before
    /// assuming it was dropped and admitting a fresh one. Floored at 1s so a
    /// zero cool-down (used in tests) still admits exactly one probe at a time.
    fn probe_timeout(&self) -> Duration {
        Duration::from_secs(self.config.cool_down_secs.max(1))
    }

    /// Check if a request is allowed through.
    ///
    /// In `HalfOpen`, admits exactly one probe at a time: the first caller after
    /// the cool-down wins, and concurrent callers fail fast until that probe's
    /// outcome is recorded (which moves the circuit to `Closed` or `Open`). If an
    /// admitted probe never records an outcome (e.g. its task was dropped), the
    /// slot self-heals after `probe_timeout` so the provider is not black-holed
    /// forever.
    fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cool-down period has elapsed.
                if let Some(last) = self.last_failure
                    && last.elapsed() >= Duration::from_secs(self.config.cool_down_secs)
                {
                    self.state = CircuitState::HalfOpen;
                    self.probe_in_flight = true;
                    self.probe_started = Some(Instant::now());
                    tracing::info!("circuit breaker → half_open (admitting one probe)");
                    return true;
                }
                false
            }
            CircuitState::HalfOpen => {
                if self.probe_in_flight {
                    // A probe is being tested; reject everyone else — unless the
                    // probe has gone silent past its timeout, in which case admit
                    // a fresh one so a dropped probe cannot wedge the circuit.
                    let stale = self
                        .probe_started
                        .map(|t| t.elapsed() >= self.probe_timeout())
                        .unwrap_or(true);
                    if stale {
                        self.probe_started = Some(Instant::now());
                        return true;
                    }
                    return false;
                }
                self.probe_in_flight = true;
                self.probe_started = Some(Instant::now());
                true
            }
        }
    }

    /// Record a successful request.
    fn record_success(&mut self) {
        match self.state {
            // The authorized probe succeeded → recover.
            CircuitState::HalfOpen => {
                self.state = CircuitState::Closed;
                self.failure_count = 0;
                self.probe_in_flight = false;
                self.probe_started = None;
            }
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            // A late success from a request dispatched before the circuit opened
            // must not reopen the gates; the cool-down governs recovery.
            CircuitState::Open => {}
        }
    }

    /// Record a failed request.
    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                    tracing::warn!("circuit breaker → open (failures: {})", self.failure_count);
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — back to Open and release the probe slot.
                self.state = CircuitState::Open;
                self.probe_in_flight = false;
                self.probe_started = None;
                tracing::warn!("circuit breaker → open (probe failed)");
            }
            CircuitState::Open => {
                // Already open.
            }
        }
    }
}

/// Manages circuit breakers for all providers.
pub(crate) struct CircuitBreakerRegistry {
    breakers: Mutex<HashMap<String, ProviderBreaker>>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerRegistry {
    /// Create a new registry with the given configuration.
    pub(crate) fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Check if a request to the given provider is allowed.
    pub(crate) fn allow_request(&self, provider: &str) -> bool {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        let breaker = breakers
            .entry(provider.to_string())
            .or_insert_with(|| ProviderBreaker::new(self.config.clone()));
        breaker.allow_request()
    }

    /// Record a successful request for a provider.
    pub(crate) fn record_success(&self, provider: &str) {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = breakers.get_mut(provider) {
            b.record_success();
        }
    }

    /// Record a failed request for a provider.
    pub(crate) fn record_failure(&self, provider: &str) {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        let breaker = breakers
            .entry(provider.to_string())
            .or_insert_with(|| ProviderBreaker::new(self.config.clone()));
        breaker.record_failure();
    }

    /// Get the current state for a provider.
    pub(crate) fn state(&self, provider: &str) -> CircuitState {
        let breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        breakers
            .get(provider)
            .map(|b| b.state)
            .unwrap_or(CircuitState::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            cool_down_secs: 1, // short for tests
        }
    }

    #[test]
    fn closed_allows_requests() {
        let reg = CircuitBreakerRegistry::new(test_config());
        assert!(reg.allow_request("provider-a"));
        assert_eq!(reg.state("provider-a"), CircuitState::Closed);
    }

    #[test]
    fn opens_after_threshold_failures() {
        let reg = CircuitBreakerRegistry::new(test_config());
        reg.record_failure("p");
        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Closed);

        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Open);
        assert!(!reg.allow_request("p"));
    }

    #[test]
    fn transitions_to_half_open_after_cooldown() {
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 0, // instant cooldown for test
        };
        let reg = CircuitBreakerRegistry::new(cfg);

        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Open);

        // Cooldown is 0 seconds, so next allow_request should transition
        std::thread::sleep(Duration::from_millis(10));
        assert!(reg.allow_request("p"));
        assert_eq!(reg.state("p"), CircuitState::HalfOpen);
    }

    #[test]
    fn success_in_half_open_closes_circuit() {
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 0,
        };
        let reg = CircuitBreakerRegistry::new(cfg);

        reg.record_failure("p");
        std::thread::sleep(Duration::from_millis(10));
        reg.allow_request("p"); // → HalfOpen
        reg.record_success("p");
        assert_eq!(reg.state("p"), CircuitState::Closed);
    }

    #[test]
    fn half_open_admits_exactly_one_probe() {
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 0,
        };
        let reg = CircuitBreakerRegistry::new(cfg);

        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Open);
        std::thread::sleep(Duration::from_millis(10));

        // First caller after cool-down wins the single probe slot.
        assert!(reg.allow_request("p"));
        assert_eq!(reg.state("p"), CircuitState::HalfOpen);

        // Concurrent callers while the probe is in flight are rejected.
        assert!(!reg.allow_request("p"));
        assert!(!reg.allow_request("p"));

        // Once the probe succeeds, the circuit closes and admits traffic again.
        reg.record_success("p");
        assert_eq!(reg.state("p"), CircuitState::Closed);
        assert!(reg.allow_request("p"));
    }

    #[test]
    fn half_open_probe_slot_is_released_on_reopen() {
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 0,
        };
        let reg = CircuitBreakerRegistry::new(cfg);

        reg.record_failure("p");
        std::thread::sleep(Duration::from_millis(10));
        assert!(reg.allow_request("p")); // → HalfOpen, probe in flight
        assert!(!reg.allow_request("p")); // rejected while in flight
        reg.record_failure("p"); // probe fails → Open, slot released
        assert_eq!(reg.state("p"), CircuitState::Open);

        // After another cool-down, a fresh single probe is admitted.
        std::thread::sleep(Duration::from_millis(10));
        assert!(reg.allow_request("p"));
        assert!(!reg.allow_request("p"));
    }

    #[test]
    fn late_success_does_not_reopen_an_open_circuit() {
        // A success recorded for a request that was in flight before the circuit
        // opened must not flip it back to Closed.
        let reg = CircuitBreakerRegistry::new(test_config());
        reg.record_failure("p");
        reg.record_failure("p");
        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Open);

        reg.record_success("p");
        assert_eq!(reg.state("p"), CircuitState::Open);
        assert!(!reg.allow_request("p"));
    }

    #[test]
    fn stuck_half_open_probe_self_heals() {
        // A probe that never records an outcome must not black-hole the provider.
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 1, // probe_timeout floors at 1s
        };
        let reg = CircuitBreakerRegistry::new(cfg);
        reg.record_failure("p");
        std::thread::sleep(Duration::from_millis(1100));

        // First probe admitted, then it goes silent (no success/failure recorded).
        assert!(reg.allow_request("p"));
        assert_eq!(reg.state("p"), CircuitState::HalfOpen);
        assert!(!reg.allow_request("p")); // still within probe timeout

        // After the probe timeout, a fresh probe is admitted rather than the
        // circuit staying wedged forever.
        std::thread::sleep(Duration::from_millis(1100));
        assert!(reg.allow_request("p"));
    }

    #[test]
    fn failure_in_half_open_reopens_circuit() {
        let cfg = CircuitBreakerConfig {
            failure_threshold: 1,
            cool_down_secs: 0,
        };
        let reg = CircuitBreakerRegistry::new(cfg);

        reg.record_failure("p");
        std::thread::sleep(Duration::from_millis(10));
        reg.allow_request("p"); // → HalfOpen
        reg.record_failure("p");
        assert_eq!(reg.state("p"), CircuitState::Open);
    }
}
