//! LRU-based memory manager implementation.

use async_trait::async_trait;
use dashmap::DashMap;
use mofa_kernel::{EngineError, MemoryManager as MemoryManagerTrait};
use std::time::Instant;

pub struct MemoryManagerImpl {
    total: u64,
    allocations: DashMap<String, u64>,
    last_used: DashMap<String, Instant>,
    resident: DashMap<String, bool>,
}

impl MemoryManagerImpl {
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total: total_bytes,
            allocations: DashMap::new(),
            last_used: DashMap::new(),
            resident: DashMap::new(),
        }
    }

    pub fn touch(&self, model_id: &str) {
        self.last_used.insert(model_id.to_string(), Instant::now());
    }

    pub fn set_resident(&self, model_id: &str, is_resident: bool) {
        self.resident.insert(model_id.to_string(), is_resident);
    }

    pub fn lru_order(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<_> = self.allocations.iter()
            .filter(|e| {
                !self.resident.get(e.key()).is_some_and(|v| *v)
            })
            .map(|e| {
                let last = self.last_used.get(e.key())
                    .map(|v| *v)
                    .unwrap_or_else(Instant::now);
                (e.key().clone(), *e.value(), last)
            })
            .collect();
        entries.sort_by(|a, b| a.2.cmp(&b.2));
        entries.into_iter().map(|(id, bytes, _)| (id, bytes)).collect()
    }
}

#[async_trait]
impl MemoryManagerTrait for MemoryManagerImpl {
    fn total_memory(&self) -> u64 {
        self.total
    }

    fn used_memory(&self) -> u64 {
        self.allocations.iter().map(|e| *e.value()).sum()
    }

    fn available_memory(&self) -> u64 {
        self.total.saturating_sub(self.used_memory())
    }

    async fn can_fit(&self, required_bytes: u64) -> bool {
        self.available_memory() >= required_bytes
    }

    async fn reserve(&self, model_id: &str, bytes: u64) -> Result<(), EngineError> {
        if !self.can_fit(bytes).await {
            return Err(EngineError::InsufficientMemory {
                need: bytes,
                available: self.available_memory(),
            });
        }
        self.allocations.insert(model_id.to_string(), bytes);
        self.touch(model_id);
        Ok(())
    }

    async fn release(&self, model_id: &str) -> Result<u64, EngineError> {
        let freed = self.allocations.remove(&model_id.to_string())
            .map(|(_, v)| v)
            .unwrap_or(0);
        self.last_used.remove(&model_id.to_string());
        self.resident.remove(&model_id.to_string());
        Ok(freed)
    }

    async fn evict_for(&self, required_bytes: u64) -> Result<Vec<String>, EngineError> {
        let mut to_evict = Vec::new();
        let mut freed = 0u64;

        for (id, bytes) in self.lru_order() {
            if self.available_memory() + freed >= required_bytes {
                break;
            }
            to_evict.push(id);
            freed += bytes;
        }

        if self.available_memory() + freed < required_bytes {
            return Err(EngineError::InsufficientMemory {
                need: required_bytes,
                available: self.available_memory() + freed,
            });
        }

        Ok(to_evict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mofa_kernel::MemoryManager as MemoryManagerTrait;

    #[tokio::test]
    async fn test_reserve_and_release() {
        let mm = MemoryManagerImpl::new(1024);
        assert_eq!(mm.total_memory(), 1024);
        assert_eq!(mm.available_memory(), 1024);

        mm.reserve("model_a", 256).await.unwrap();
        assert_eq!(mm.used_memory(), 256);
        assert_eq!(mm.available_memory(), 768);

        mm.reserve("model_b", 512).await.unwrap();
        assert_eq!(mm.used_memory(), 768);

        let freed = mm.release("model_a").await.unwrap();
        assert_eq!(freed, 256);
        assert_eq!(mm.used_memory(), 512);
    }

    #[tokio::test]
    async fn test_insufficient_memory() {
        let mm = MemoryManagerImpl::new(100);
        let result = mm.reserve("big_model", 200).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let mm = MemoryManagerImpl::new(1000);
        mm.reserve("model_a", 400).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        mm.reserve("model_b", 400).await.unwrap();

        // model_a is older, should be evicted first
        let evicted = mm.evict_for(500).await.unwrap();
        assert_eq!(evicted, vec!["model_a"]);
    }

    #[tokio::test]
    async fn test_resident_not_evicted() {
        let mm = MemoryManagerImpl::new(1000);
        mm.reserve("resident_model", 400).await.unwrap();
        mm.set_resident("resident_model", true);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        mm.reserve("normal_model", 400).await.unwrap();

        let evicted = mm.evict_for(500).await.unwrap();
        assert_eq!(evicted, vec!["normal_model"]);
    }
}
