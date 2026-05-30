//! The core engine — ties together backends, scheduler, memory, and preflight.

use crate::config::EngineConfig;
use crate::discovery::Discovery;
use crate::memory_manager::MemoryManagerImpl;
use crate::preflight::PreflightManager;
use crate::scheduler::SmartScheduler;
use dashmap::DashMap;
use mofa_kernel::*;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

pub struct Engine {
    config: EngineConfig,
    backends: DashMap<BackendType, Arc<dyn ModelBackend>>,
    models: DashMap<String, ModelInfo>,
    scheduler: Arc<SmartScheduler>,
    memory: Arc<MemoryManagerImpl>,
    preflight: Arc<PreflightManager>,
    idle_tracker: DashMap<String, Instant>,
    last_model_type: parking_lot::Mutex<Option<ModelType>>,
}

impl Engine {
    pub async fn new(config: EngineConfig) -> Arc<Self> {
        let memory_budget = config.memory_budget_bytes();
        info!(budget_mb = memory_budget / 1024 / 1024, "initializing engine");

        let engine = Arc::new(Self {
            config: config.clone(),
            backends: DashMap::new(),
            models: DashMap::new(),
            scheduler: Arc::new(SmartScheduler::new()),
            memory: Arc::new(MemoryManagerImpl::new(memory_budget)),
            preflight: Arc::new(PreflightManager::new()),
            idle_tracker: DashMap::new(),
            last_model_type: parking_lot::Mutex::new(None),
        });

        engine.discover().await;
        engine.start_idle_reaper(config.idle_timeout_secs);
        engine
    }

    async fn discover(&self) {
        let discovered = Discovery::discover_all(&self.config).await;
        for (backend, models) in discovered {
            let bt = backend.backend_type();
            self.backends.insert(bt, Arc::from(backend));
            for m in models {
                info!(id = %m.id, name = %m.name, "registered model");
                self.models.insert(m.id.clone(), m);
            }
        }
    }

    fn start_idle_reaper(self: &Arc<Self>, timeout_secs: u64) {
        let engine = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                let now = Instant::now();
                let to_unload: Vec<String> = engine.idle_tracker.iter()
                    .filter(|e| now.duration_since(*e.value()).as_secs() >= timeout_secs)
                    .map(|e| e.key().clone())
                    .collect();

                for model_id in to_unload {
                    if let Some(info) = engine.models.get(&model_id) {
                        if info.status == ModelStatus::Loaded {
                            info!(model = %model_id, "idle timeout — unloading");
                            let _ = engine.unload_model(&model_id).await;
                        }
                    }
                }
            }
        });
    }

    pub fn capabilities(&self) -> CapabilitiesResponse {
        let models: Vec<ModelInfo> = self.models.iter().map(|e| e.value().clone()).collect();
        CapabilitiesResponse { models }
    }

    pub fn status(&self) -> EngineStatus {
        let loaded: Vec<ModelInfo> = self.models.iter()
            .filter(|e| e.value().status == ModelStatus::Loaded || e.value().status == ModelStatus::Running)
            .map(|e| e.value().clone())
            .collect();

        let backends: Vec<BackendStatus> = self.backends.iter().map(|e| {
            let bt = *e.key();
            let count = self.models.iter()
                .filter(|m| m.value().backend == bt)
                .count();
            BackendStatus {
                backend_type: bt,
                healthy: true,
                model_count: count,
            }
        }).collect();

        EngineStatus {
            total_memory_bytes: self.memory.total_memory(),
            used_memory_bytes: self.memory.used_memory(),
            available_memory_bytes: self.memory.available_memory(),
            loaded_models: loaded,
            backends,
        }
    }

    pub async fn run(&self, request: RunRequest) -> Result<RunResponse, EngineError> {
        let start = Instant::now();
        let request_id = uuid::Uuid::new_v4().to_string();

        let model_type = request.model_type.as_deref()
            .map(|t| t.parse::<ModelType>())
            .transpose()
            .map_err(|e| EngineError::InvalidInput(e))?;

        let model_name = request.model.as_deref();

        let available: Vec<ModelInfo> = self.models.iter().map(|e| e.value().clone()).collect();
        let model_id = self.scheduler.select_model(model_type, model_name, &available).await?;

        let info = self.models.get(&model_id)
            .ok_or_else(|| EngineError::ModelNotFound(model_id.clone()))?
            .clone();

        // Ensure model is loaded
        if info.status != ModelStatus::Loaded && info.status != ModelStatus::Running {
            self.load_model(&model_id).await?;
        }

        // Mark as running
        if let Some(mut m) = self.models.get_mut(&model_id) {
            m.status = ModelStatus::Running;
        }

        // Handle hint — prefetch next model
        if let Some(ref hint) = request.hint {
            self.handle_hint(hint, model_type).await;
        }

        // Run inference
        let backend = self.backends.get(&info.backend)
            .ok_or_else(|| EngineError::BackendNotAvailable(format!("{}", info.backend)))?;

        let result = backend.run(&model_id, &request.input).await;

        // On failure, try fallback
        let (output, actual_model_name, actual_backend) = match result {
            Ok(out) => (out, info.name.clone(), info.backend.to_string()),
            Err(e) => {
                warn!(model = %model_id, error = %e, "inference failed, trying fallback");
                match self.try_fallback(&info, &request.input).await {
                    Ok((out, fallback_info)) => (out, fallback_info.name, fallback_info.backend.to_string()),
                    Err(_) => return Err(e),
                }
            }
        };

        // Mark as loaded (not running)
        if let Some(mut m) = self.models.get_mut(&model_id) {
            m.status = ModelStatus::Loaded;
        }
        self.idle_tracker.insert(model_id.clone(), Instant::now());
        self.memory.touch(&model_id);

        // Record transition for history learning
        if let Some(mt) = model_type {
            let mut last = self.last_model_type.lock();
            if let Some(prev) = *last {
                self.preflight.record_transition(prev, mt);
            }
            *last = Some(mt);
        }

        let duration = start.elapsed();

        Ok(RunResponse {
            output,
            model_used: actual_model_name,
            backend: actual_backend,
            duration_ms: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            request_id,
        })
    }

    async fn load_model(&self, model_id: &str) -> Result<(), EngineError> {
        let info = self.models.get(model_id)
            .ok_or_else(|| EngineError::ModelNotFound(model_id.to_string()))?
            .clone();

        let mem_needed = info.memory_bytes;
        if mem_needed > 0 && !self.memory.can_fit(mem_needed).await {
            let to_evict = self.memory.evict_for(mem_needed).await?;
            for evict_id in to_evict {
                self.unload_model(&evict_id).await?;
            }
        }

        if let Some(mut m) = self.models.get_mut(model_id) {
            m.status = ModelStatus::Loading;
        }

        let backend = self.backends.get(&info.backend)
            .ok_or_else(|| EngineError::BackendNotAvailable(format!("{}", info.backend)))?;

        backend.load_model(model_id).await?;

        if mem_needed > 0 {
            self.memory.reserve(model_id, mem_needed).await?;
        }

        if let Some(mut m) = self.models.get_mut(model_id) {
            m.status = ModelStatus::Loaded;
        }
        self.idle_tracker.insert(model_id.to_string(), Instant::now());

        info!(model = %model_id, "model loaded");
        Ok(())
    }

    async fn unload_model(&self, model_id: &str) -> Result<(), EngineError> {
        let info = self.models.get(model_id)
            .ok_or_else(|| EngineError::ModelNotFound(model_id.to_string()))?
            .clone();

        if let Some(mut m) = self.models.get_mut(model_id) {
            m.status = ModelStatus::Unloading;
        }

        if let Some(backend) = self.backends.get(&info.backend) {
            let _ = backend.unload_model(model_id).await;
        }

        self.memory.release(model_id).await?;
        self.idle_tracker.remove(model_id);

        if let Some(mut m) = self.models.get_mut(model_id) {
            m.status = ModelStatus::Available;
        }

        info!(model = %model_id, "model unloaded");
        Ok(())
    }

    async fn handle_hint(&self, hint: &PrefetchHint, current_type: Option<ModelType>) {
        if let Some(ref next) = hint.next {
            let target_type = next.parse::<ModelType>().ok();
            let target_name = if target_type.is_none() { Some(next.as_str()) } else { None };

            let available: Vec<ModelInfo> = self.models.iter()
                .map(|e| e.value().clone())
                .collect();

            if let Ok(model_id) = self.scheduler.select_model(target_type, target_name, &available).await {
                let status = self.models.get(&model_id).map(|m| m.status);
                if status == Some(ModelStatus::Available) {
                    info!(model = %model_id, hint = %next, "preflight: preloading from hint");
                    let _ = self.load_model(&model_id).await;
                }
            }
        }

        if let Some(ct) = current_type {
            if let Some(predicted) = self.preflight.predict_next(ct) {
                let available: Vec<ModelInfo> = self.models.iter()
                    .map(|e| e.value().clone())
                    .collect();
                if let Ok(model_id) = self.scheduler.select_model(Some(predicted), None, &available).await {
                    let status = self.models.get(&model_id).map(|m| m.status);
                    if status == Some(ModelStatus::Available) {
                        info!(model = %model_id, predicted = %predicted, "preflight: preloading from history");
                        let _ = self.load_model(&model_id).await;
                    }
                }
            }
        }
    }

    async fn try_fallback(&self, failed_info: &ModelInfo, input: &ModelInput) -> Result<(ModelOutput, ModelInfo), EngineError> {
        let alternatives: Vec<ModelInfo> = self.models.iter()
            .filter(|e| {
                let m = e.value();
                m.model_type == failed_info.model_type
                    && m.id != failed_info.id
                    && m.backend != failed_info.backend
            })
            .map(|e| e.value().clone())
            .collect();

        for alt in &alternatives {
            info!(fallback = %alt.id, "trying fallback model");
            if alt.status != ModelStatus::Loaded {
                if let Err(e) = self.load_model(&alt.id).await {
                    warn!(model = %alt.id, error = %e, "fallback load failed");
                    continue;
                }
            }
            if let Some(backend) = self.backends.get(&alt.backend) {
                match backend.run(&alt.id, input).await {
                    Ok(output) => {
                        if let Some(mut m) = self.models.get_mut(&alt.id) {
                            m.status = ModelStatus::Loaded;
                        }
                        self.idle_tracker.insert(alt.id.clone(), Instant::now());
                        return Ok((output, alt.clone()));
                    }
                    Err(e) => {
                        warn!(model = %alt.id, error = %e, "fallback run failed");
                    }
                }
            }
        }

        Err(EngineError::InferenceFailed("all fallbacks exhausted".to_string()))
    }
}
