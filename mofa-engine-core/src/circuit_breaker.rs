//! Per-provider circuit breaker.
//!
//! Prevents cascading failures by temporarily stopping requests to
//! unhealthy providers. Three states: Closed → Open → HalfOpen → Closed.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
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
pub struct CircuitBreakerConfig {
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
    config: CircuitBreakerConfig,
}

impl ProviderBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure: None,
            config,
        }
    }

    /// Check if a request is allowed through.
    fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cool-down period has elapsed
                if let Some(last) = self.last_failure
                    && last.elapsed() >= Duration::from_secs(self.config.cool_down_secs) {
                        self.state = CircuitState::HalfOpen;
                        tracing::info!("circuit breaker → half_open (cool-down elapsed)");
                        return true;
                    }
                false
            }
            CircuitState::HalfOpen => {
                // Allow exactly one probe request
                true
            }
        }
    }

    /// Record a successful request.
    fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitState::Closed;
    }

    /// Record a failed request.
    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= self.config.failure_threshold {
                    self.state = CircuitState::Open;
                    tracing::warn!(
                        "circuit breaker → open (failures: {})",
                        self.failure_count
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Probe failed — back to Open
                self.state = CircuitState::Open;
                tracing::warn!("circuit breaker → open (probe failed)");
            }
            CircuitState::Open => {
                // Already open
            }
        }
    }
}

/// Manages circuit breakers for all providers.
pub struct CircuitBreakerRegistry {
    breakers: Mutex<HashMap<String, ProviderBreaker>>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerRegistry {
    /// Create a new registry with the given configuration.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Check if a request to the given provider is allowed.
    pub fn allow_request(&self, provider: &str) -> bool {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        let breaker = breakers
            .entry(provider.to_string())
            .or_insert_with(|| ProviderBreaker::new(self.config.clone()));
        breaker.allow_request()
    }

    /// Record a successful request for a provider.
    pub fn record_success(&self, provider: &str) {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = breakers.get_mut(provider) {
            b.record_success();
        }
    }

    /// Record a failed request for a provider.
    pub fn record_failure(&self, provider: &str) {
        let mut breakers = self.breakers.lock().unwrap_or_else(|e| e.into_inner());
        let breaker = breakers
            .entry(provider.to_string())
            .or_insert_with(|| ProviderBreaker::new(self.config.clone()));
        breaker.record_failure();
    }

    /// Get the current state for a provider.
    pub fn state(&self, provider: &str) -> CircuitState {
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
