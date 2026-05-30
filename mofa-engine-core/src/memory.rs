//! Memory manager with LRU eviction.
//!
//! Tracks per-model memory allocations, detects real system memory,
//! and evicts least-recently-used models when under pressure.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// Tracks memory allocations and enforces a budget.
pub struct MemoryManager {
    /// Total budget in bytes
    budget_bytes: u64,
    /// Per-model allocations: model_id → (bytes, last_access)
    allocations: Mutex<HashMap<String, Allocation>>,
}

struct Allocation {
    bytes: u64,
    last_access: Instant,
}

impl MemoryManager {
    /// Create a new memory manager.
    ///
    /// If `budget_mb` is `None`, auto-detect from system RAM (use 70% of total).
    pub fn new(budget_mb: Option<u64>) -> Self {
        let budget_bytes = match budget_mb {
            Some(mb) => mb * 1024 * 1024,
            None => Self::detect_system_memory(),
        };

        tracing::info!(
            "memory budget: {} MB",
            budget_bytes / (1024 * 1024)
        );

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

    /// Total budget in bytes.
    pub fn budget_bytes(&self) -> u64 {
        self.budget_bytes
    }

    /// Currently used bytes.
    pub fn used_bytes(&self) -> u64 {
        let allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        allocs.values().map(|a| a.bytes).sum()
    }

    /// Available bytes.
    pub fn available_bytes(&self) -> u64 {
        self.budget_bytes.saturating_sub(self.used_bytes())
    }

    /// Record that a model is using memory. Updates last-access time.
    pub fn allocate(&self, model_id: &str, bytes: u64) {
        let mut allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        allocs.insert(
            model_id.to_string(),
            Allocation {
                bytes,
                last_access: Instant::now(),
            },
        );
    }

    /// Remove a model's allocation.
    pub fn deallocate(&self, model_id: &str) {
        let mut allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        allocs.remove(model_id);
    }

    /// Touch a model (update its last-access time).
    pub fn touch(&self, model_id: &str) {
        let mut allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = allocs.get_mut(model_id) {
            a.last_access = Instant::now();
        }
    }

    /// Check if there is enough room for `needed_bytes`.
    pub fn can_fit(&self, needed_bytes: u64) -> bool {
        self.available_bytes() >= needed_bytes
    }

    /// Find the least-recently-used model that can be evicted.
    ///
    /// `protected` is a set of model IDs that must not be evicted (e.g. resident models).
    pub fn lru_candidate(&self, protected: &[String]) -> Option<String> {
        let allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        allocs
            .iter()
            .filter(|(id, _)| !protected.contains(id))
            .min_by_key(|(_, a)| a.last_access)
            .map(|(id, _)| id.clone())
    }

    /// Return a snapshot of all allocations (model_id, bytes).
    pub fn snapshot(&self) -> Vec<(String, u64)> {
        let allocs = self.allocations.lock().unwrap_or_else(|e| e.into_inner());
        allocs
            .iter()
            .map(|(id, a)| (id.clone(), a.bytes))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_and_track() {
        let mm = MemoryManager::new(Some(100)); // 100 MB
        assert_eq!(mm.budget_bytes(), 100 * 1024 * 1024);
        assert_eq!(mm.used_bytes(), 0);

        mm.allocate("model-a", 10 * 1024 * 1024);
        assert_eq!(mm.used_bytes(), 10 * 1024 * 1024);

        mm.allocate("model-b", 20 * 1024 * 1024);
        assert_eq!(mm.used_bytes(), 30 * 1024 * 1024);

        mm.deallocate("model-a");
        assert_eq!(mm.used_bytes(), 20 * 1024 * 1024);
    }

    #[test]
    fn can_fit_check() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("m1", 90 * 1024 * 1024);
        assert!(!mm.can_fit(20 * 1024 * 1024));
        assert!(mm.can_fit(10 * 1024 * 1024));
    }

    #[test]
    fn lru_eviction_order() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("old", 10 * 1024 * 1024);
        // tiny delay so timestamps differ
        std::thread::sleep(std::time::Duration::from_millis(10));
        mm.allocate("new", 10 * 1024 * 1024);

        let candidate = mm.lru_candidate(&[]).unwrap();
        assert_eq!(candidate, "old");
    }

    #[test]
    fn lru_respects_protected() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("protected", 10 * 1024 * 1024);
        std::thread::sleep(std::time::Duration::from_millis(10));
        mm.allocate("evictable", 10 * 1024 * 1024);

        let candidate = mm.lru_candidate(&["protected".into()]).unwrap();
        assert_eq!(candidate, "evictable");
    }

    #[test]
    fn touch_updates_access_time() {
        let mm = MemoryManager::new(Some(100));
        mm.allocate("a", 10 * 1024 * 1024);
        std::thread::sleep(std::time::Duration::from_millis(10));
        mm.allocate("b", 10 * 1024 * 1024);

        // 'a' is older, but touch it to make it newer
        std::thread::sleep(std::time::Duration::from_millis(10));
        mm.touch("a");

        let candidate = mm.lru_candidate(&[]).unwrap();
        assert_eq!(candidate, "b");
    }
}
