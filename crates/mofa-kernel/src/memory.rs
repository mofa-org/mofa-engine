//! Memory manager trait.

use async_trait::async_trait;
use crate::EngineError;

#[async_trait]
pub trait MemoryManager: Send + Sync {
    fn total_memory(&self) -> u64;
    fn used_memory(&self) -> u64;
    fn available_memory(&self) -> u64;

    async fn can_fit(&self, required_bytes: u64) -> bool;

    async fn reserve(&self, model_id: &str, bytes: u64) -> Result<(), EngineError>;

    async fn release(&self, model_id: &str) -> Result<u64, EngineError>;

    async fn evict_for(&self, required_bytes: u64) -> Result<Vec<String>, EngineError>;
}
