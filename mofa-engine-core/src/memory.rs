//! Reservation-based memory manager with LRU eviction.
//!
//! Memory is admitted *before* a model is loaded: callers atomically reserve
//! capacity, evict idle models if needed, and only then ask the backend to
//! load. This prevents the time-of-check/time-of-use race where two concurrent
//! loads each observe enough free memory and together overcommit the device.
//!
//! Three quantities are tracked per model:
//! - **reserved** — the bytes admitted against the budget (a conservative
//!   estimate, later reconciled toward observed usage);
//! - **observed** — actual usage reported by the backend, when available;
//! - **leases** — in-flight inferences; a model with active leases is never
//!   evicted, even under pressure.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Tracks per-model memory reservations and enforces a global budget.
pub struct MemoryManager {
    /// Total budget in bytes.
    budget_bytes: u64,
    /// Per-model allocations.
    allocations: Mutex<HashMap<String, Allocation>>,
}

struct Allocation {
    /// Bytes admitted against the budget. Accounting always uses this figure.
    reserved_bytes: u64,
    /// Actual usage reported by the backend, when known.
    observed_bytes: Option<u64>,
    /// Monotonic time of the last access, for LRU and idle-timeout decisions.
    last_access: Instant,
    /// Active inference leases. A model with `leases > 0` is never evicted.
    leases: u32,
}

impl Allocation {
    fn new(reserved_bytes: u64) -> Self {
        Self {
            reserved_bytes,
            observed_bytes: None,
            last_access: Instant::now(),
            leases: 0,
        }
    }
}

/// A point-in-time view of one model's memory accounting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationSnapshot {
    /// Model identifier.
    pub model_id: String,
    /// Bytes reserved against the budget.
    pub reserved_bytes: u64,
    /// Bytes observed by the backend, if reported.
    pub observed_bytes: Option<u64>,
    /// Active inference leases.
    pub leases: u32,
}

impl MemoryManager {
    /// Create a new memory manager.
    ///
    /// If `budget_mb` is `None`, auto-detect from system RAM (use 70% of total).
    pub fn new(budget_mb: Option<u64>) -> Self {
        let budget_bytes = match budget_mb {
            Some(mb) => mb.saturating_mul(1024 * 1024),
            None => Self::detect_system_memory(),
        };

        tracing::info!("memory budget: {} MB", budget_bytes / (1024 * 1024));

        Self {
            budget_bytes,
            allocations: Mutex::new(HashMap::new()),
        }
    }

    /// Detect available system memory and use 70% as budget.
    fn detect_system_memory() -> u64 {
        let sys = sysinfo::System::new_all();
        let total = sys.total_memory(); // bytes
        (total as f64 * 0.7) as u64
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Allocation>> {
        self.allocations.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Total budget in bytes.
    pub fn budget_bytes(&self) -> u64 {
        self.budget_bytes
    }

    /// Currently reserved bytes across all models.
    pub fn used_bytes(&self) -> u64 {
        self.lock().values().map(|a| a.reserved_bytes).sum()
    }

    /// Bytes still available within the budget.
    pub fn available_bytes(&self) -> u64 {
        self.budget_bytes.saturating_sub(self.used_bytes())
    }

    /// Whether `needed_bytes` currently fits within the budget without eviction.
    pub fn can_fit(&self, needed_bytes: u64) -> bool {
        self.available_bytes() >= needed_bytes
    }

    /// Atomically reserve `bytes` for `model_id` if it fits within the budget.
    ///
    /// The check and the insert happen under a single lock, so concurrent
    /// reservations can never jointly exceed the budget. Re-reserving a model
    /// that already holds an allocation accounts only for the *delta*, and
    /// preserves its lease count. Returns `true` if the reservation succeeded.
    pub fn try_reserve(&self, model_id: &str, bytes: u64) -> bool {
        let mut allocs = self.lock();
        let used_by_others: u64 = allocs
            .iter()
            .filter(|(id, _)| id.as_str() != model_id)
            .map(|(_, a)| a.reserved_bytes)
            .sum();
        if used_by_others.saturating_add(bytes) > self.budget_bytes {
            return false;
        }
        match allocs.get_mut(model_id) {
            Some(existing) => {
                existing.reserved_bytes = bytes;
                existing.last_access = Instant::now();
            }
            None => {
                allocs.insert(model_id.to_string(), Allocation::new(bytes));
            }
        }
        true
    }

    /// Unconditionally record a reservation, ignoring the budget.
    ///
    /// Intended for tests and for accounting state the backend reports as already
    /// resident. Prefer [`try_reserve`](Self::try_reserve) for admission.
    pub fn allocate(&self, model_id: &str, bytes: u64) {
        let mut allocs = self.lock();
        match allocs.get_mut(model_id) {
            Some(existing) => {
                existing.reserved_bytes = bytes;
                existing.last_access = Instant::now();
            }
            None => {
                allocs.insert(model_id.to_string(), Allocation::new(bytes));
            }
        }
    }

    /// Release a model's reservation entirely.
    pub fn deallocate(&self, model_id: &str) {
        self.lock().remove(model_id);
    }

    /// Reconcile a reservation with memory the backend actually reports.
    ///
    /// The observed value is recorded for reporting, and the reservation is
    /// raised to it when the model uses more than estimated, so accounting never
    /// undercounts real usage.
    pub fn reconcile(&self, model_id: &str, observed_bytes: u64) {
        let mut allocs = self.lock();
        if let Some(a) = allocs.get_mut(model_id) {
            a.observed_bytes = Some(observed_bytes);
            a.reserved_bytes = a.reserved_bytes.max(observed_bytes);
        }
    }

    /// Update a model's last-access time (for LRU and idle-timeout decisions).
    pub fn touch(&self, model_id: &str) {
        if let Some(a) = self.lock().get_mut(model_id) {
            a.last_access = Instant::now();
        }
    }

    /// Acquire an inference lease, protecting the model from eviction.
    ///
    /// No-op for models without an allocation (e.g. cloud-backed models, which
    /// hold no local memory and are never eviction candidates).
    pub fn lease(&self, model_id: &str) {
        if let Some(a) = self.lock().get_mut(model_id) {
            a.leases = a.leases.saturating_add(1);
            a.last_access = Instant::now();
        }
    }

    /// Release a previously acquired inference lease.
    pub fn release_lease(&self, model_id: &str) {
        if let Some(a) = self.lock().get_mut(model_id) {
            a.leases = a.leases.saturating_sub(1);
            a.last_access = Instant::now();
        }
    }

    /// Number of active leases on a model.
    pub fn lease_count(&self, model_id: &str) -> u32 {
        self.lock().get(model_id).map(|a| a.leases).unwrap_or(0)
    }

    /// Find the least-recently-used evictable model.
    ///
    /// Models in `protected`, and any model holding an active lease, are never
    /// returned.
    pub fn lru_candidate(&self, protected: &[String]) -> Option<String> {
        self.lock()
            .iter()
            .filter(|(id, a)| a.leases == 0 && !protected.iter().any(|p| p == *id))
            .min_by_key(|(_, a)| a.last_access)
            .map(|(id, _)| id.clone())
    }

    /// Return every model whose idle time meets or exceeds `idle`, excluding
    /// leased and `protected` models. Used by the idle-timeout sweep.
    pub fn idle_candidates(&self, idle: Duration, protected: &[String]) -> Vec<String> {
        self.lock()
            .iter()
            .filter(|(id, a)| {
                a.leases == 0
                    && !protected.iter().any(|p| p == *id)
                    && a.last_access.elapsed() >= idle
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Snapshot of all current allocations, sorted by model id for determinism.
    pub fn snapshot(&self) -> Vec<AllocationSnapshot> {
        let mut out: Vec<AllocationSnapshot> = self
            .lock()
            .iter()
            .map(|(id, a)| AllocationSnapshot {
                model_id: id.clone(),
                reserved_bytes: a.reserved_bytes,
                observed_bytes: a.observed_bytes,
                leases: a.leases,
            })
            .collect();
        out.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn allocate_and_track() {
        let mm = MemoryManager::new(Some(100));
        assert_eq!(mm.budget_bytes(), 100 * MB);
        assert_eq!(mm.used_bytes(), 0);

        mm.allocate("model-a", 10 * MB);
        assert_eq!(mm.used_bytes(), 10 * MB);

        mm.allocate("model-b", 20 * MB);
        assert_eq!(mm.used_bytes(), 30 * MB);

        mm.deallocate("model-a");
        assert_eq!(mm.used_bytes(), 20 * MB);
    }

    #[test]
    fn try_reserve_respects_budget() {
        let mm = MemoryManager::new(Some(100));
        assert!(mm.try_reserve("a", 60 * MB));
        // 60 used, only 40 left → a 50 MB reservation must fail.
        assert!(!mm.try_reserve("b", 50 * MB));
        assert!(mm.try_reserve("b", 40 * MB));
        assert_eq!(mm.used_bytes(), 100 * MB);
    }

    #[test]
    fn try_reserve_same_model_accounts_delta_not_sum() {
        let mm = MemoryManager::new(Some(100));
        assert!(mm.try_reserve("a", 60 * MB));
        // Re-reserving the same model at 80 MB replaces, not adds → fits.
        assert!(mm.try_reserve("a", 80 * MB));
        assert_eq!(mm.used_bytes(), 80 * MB);
    }

    #[test]
    fn reconcile_raises_reservation_to_observed() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("a", 10 * MB);
        mm.reconcile("a", 25 * MB);
        assert_eq!(mm.used_bytes(), 25 * MB);
        // A lower observed value does not shrink the conservative reservation.
        mm.reconcile("a", 5 * MB);
        assert_eq!(mm.used_bytes(), 25 * MB);
    }

    #[test]
    fn can_fit_check() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("m1", 90 * MB);
        assert!(!mm.can_fit(20 * MB));
        assert!(mm.can_fit(10 * MB));
    }

    #[test]
    fn lru_eviction_order() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("old", 10 * MB);
        std::thread::sleep(Duration::from_millis(10));
        mm.allocate("new", 10 * MB);
        assert_eq!(mm.lru_candidate(&[]).unwrap(), "old");
    }

    #[test]
    fn lru_respects_protected() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("protected", 10 * MB);
        std::thread::sleep(Duration::from_millis(10));
        mm.allocate("evictable", 10 * MB);
        assert_eq!(
            mm.lru_candidate(&["protected".into()]).unwrap(),
            "evictable"
        );
    }

    #[test]
    fn leased_models_are_never_lru_candidates() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("leased", 10 * MB);
        std::thread::sleep(Duration::from_millis(10));
        mm.allocate("free", 10 * MB);
        mm.lease("leased");
        // "leased" is older but is protected by its lease.
        assert_eq!(mm.lru_candidate(&[]).unwrap(), "free");
        mm.release_lease("leased");
        assert_eq!(mm.lease_count("leased"), 0);
    }

    #[test]
    fn touch_updates_access_time() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("a", 10 * MB);
        std::thread::sleep(Duration::from_millis(10));
        mm.allocate("b", 10 * MB);
        std::thread::sleep(Duration::from_millis(10));
        mm.touch("a");
        assert_eq!(mm.lru_candidate(&[]).unwrap(), "b");
    }

    #[test]
    fn idle_candidates_reports_only_stale_unleased() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("stale", 10 * MB);
        mm.allocate("leased_stale", 10 * MB);
        mm.lease("leased_stale");
        std::thread::sleep(Duration::from_millis(20));
        mm.allocate("fresh", 10 * MB);

        let idle = mm.idle_candidates(Duration::from_millis(10), &[]);
        assert_eq!(idle, vec!["stale".to_string()]);
    }

    #[test]
    fn snapshot_is_sorted_and_complete() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("b", 20 * MB);
        mm.allocate("a", 10 * MB);
        mm.reconcile("a", 15 * MB);
        let snap = mm.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].model_id, "a");
        assert_eq!(snap[0].observed_bytes, Some(15 * MB));
        assert_eq!(snap[1].model_id, "b");
    }
}
