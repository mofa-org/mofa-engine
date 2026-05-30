//! Preflight — hint-based and history-based model preloading.

use dashmap::DashMap;
use mofa_kernel::ModelType;
use std::collections::HashMap;

pub struct PreflightManager {
    transition_counts: DashMap<ModelType, HashMap<ModelType, usize>>,
    min_observations: usize,
}

impl PreflightManager {
    pub fn new() -> Self {
        Self {
            transition_counts: DashMap::new(),
            min_observations: 3,
        }
    }

    pub fn record_transition(&self, from: ModelType, to: ModelType) {
        self.transition_counts
            .entry(from)
            .or_insert_with(HashMap::new)
            .entry(to)
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    pub fn predict_next(&self, current: ModelType) -> Option<ModelType> {
        self.transition_counts.get(&current).and_then(|counts| {
            counts.iter()
                .filter(|&(_, count)| *count >= self.min_observations)
                .max_by_key(|&(_, count)| *count)
                .map(|(mt, _)| *mt)
        })
    }
}
