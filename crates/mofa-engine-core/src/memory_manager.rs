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
