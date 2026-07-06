//! Capability subscriptions.
//!
//! An application declares at startup which capabilities it will use. The engine
//! keeps subscribed capabilities' models warm and protects them from eviction
//! for the lifetime of the subscription (until it is explicitly removed or its
//! TTL expires). Subscriptions are owned by an `app_id`/`session_id` so they can
//! be reasoned about and cleaned up per application.

use mofa_kernel::Capability;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Registry of active capability subscriptions.
pub struct SubscriptionRegistry {
    inner: Mutex<Vec<Subscription>>,
    next_id: AtomicU64,
}

#[derive(Clone)]
struct Subscription {
    id: u64,
    app_id: Option<String>,
    session_id: Option<String>,
    capabilities: Vec<Capability>,
    expires_at: Option<Instant>,
}

impl Subscription {
    fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|e| now >= e)
    }
}

/// Public view of a subscription.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriptionInfo {
    /// Unique subscription id.
    pub id: u64,
    /// Owning application, if provided.
    pub app_id: Option<String>,
    /// Owning session, if provided.
    pub session_id: Option<String>,
    /// Subscribed capabilities.
    pub capabilities: Vec<Capability>,
    /// Seconds until expiry, or `None` if the subscription does not expire.
    pub expires_in_secs: Option<u64>,
}

impl SubscriptionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Subscription>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a subscription, returning its id.
    ///
    /// `ttl` bounds the subscription's lifetime; pass `None` for one that lives
    /// until explicitly removed.
    pub fn subscribe(
        &self,
        app_id: Option<String>,
        session_id: Option<String>,
        capabilities: Vec<Capability>,
        ttl: Option<Duration>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let expires_at = ttl.map(|d| Instant::now() + d);
        self.lock().push(Subscription {
            id,
            app_id,
            session_id,
            capabilities,
            expires_at,
        });
        id
    }

    /// Remove a subscription by id. Returns whether one was removed.
    pub fn unsubscribe(&self, id: u64) -> bool {
        let mut subs = self.lock();
        let before = subs.len();
        subs.retain(|s| s.id != id);
        subs.len() != before
    }

    /// The set of capabilities currently kept warm by any live subscription.
    ///
    /// Prunes and reads under a single lock, so the result is a consistent
    /// snapshot and expired entries are dropped exactly once.
    pub fn active_capabilities(&self) -> HashSet<Capability> {
        let now = Instant::now();
        let mut subs = self.lock();
        subs.retain(|s| !s.is_expired(now));
        subs.iter()
            .flat_map(|s| s.capabilities.iter().copied())
            .collect()
    }

    /// Whether `capability` is currently subscribed.
    pub fn is_subscribed(&self, capability: Capability) -> bool {
        self.active_capabilities().contains(&capability)
    }

    /// List all live subscriptions.
    pub fn list(&self) -> Vec<SubscriptionInfo> {
        let now = Instant::now();
        let mut subs = self.lock();
        subs.retain(|s| !s.is_expired(now));
        subs.iter()
            .map(|s| SubscriptionInfo {
                id: s.id,
                app_id: s.app_id.clone(),
                session_id: s.session_id.clone(),
                capabilities: s.capabilities.clone(),
                expires_in_secs: s
                    .expires_at
                    .map(|e| e.saturating_duration_since(now).as_secs()),
            })
            .collect()
    }
}

impl Default for SubscriptionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_and_query() {
        let reg = SubscriptionRegistry::new();
        let id = reg.subscribe(
            Some("mofa-fm".into()),
            None,
            vec![Capability::Chat, Capability::Tts],
            None,
        );
        assert!(reg.is_subscribed(Capability::Chat));
        assert!(reg.is_subscribed(Capability::Tts));
        assert!(!reg.is_subscribed(Capability::Asr));
        assert_eq!(reg.list().len(), 1);

        assert!(reg.unsubscribe(id));
        assert!(!reg.is_subscribed(Capability::Chat));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn unsubscribe_unknown_is_false() {
        let reg = SubscriptionRegistry::new();
        assert!(!reg.unsubscribe(999));
    }

    #[test]
    fn expired_subscriptions_are_pruned() {
        let reg = SubscriptionRegistry::new();
        reg.subscribe(
            None,
            None,
            vec![Capability::Tts],
            Some(Duration::from_millis(10)),
        );
        assert!(reg.is_subscribed(Capability::Tts));
        std::thread::sleep(Duration::from_millis(20));
        assert!(!reg.is_subscribed(Capability::Tts));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn list_reports_remaining_ttl() {
        let reg = SubscriptionRegistry::new();
        reg.subscribe(
            None,
            None,
            vec![Capability::Chat],
            Some(Duration::from_secs(3600)),
        );
        let info = reg.list();
        assert_eq!(info.len(), 1);
        let ttl = info[0].expires_in_secs.unwrap();
        assert!(ttl > 3590 && ttl <= 3600);
    }
}
