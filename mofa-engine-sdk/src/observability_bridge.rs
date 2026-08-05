//! Observability integration bridge.
//!
//! This module handles translating core engine events into the observability subsystem's format.
//! It also seeds initial metric values from the engine's status on startup.

use mofa_engine_core::Engine;
use mofa_kernel::EngineEvent;
use mofa_observability::collector::{EventSender, Labels, MetricsState};
use mofa_observability::events::{
    EngineEvent as ObsEngineEvent, EventEnvelope, ModelLoaded, ModelUnloaded, RequestCompleted,
    RequestReceived, UnloadReason,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::broadcast;

struct RequestContext {
    capability: mofa_observability::events::Capability,
    model_id: String,
    backend: String,
}

/// Runs the observability bridge.
///
/// Consumes engine events and translates them into observability events.
/// Seeds initial memory and model-count gauges from the engine's status snapshot.
pub async fn run(
    mut engine_events: broadcast::Receiver<EngineEvent>,
    sender: EventSender,
    engine: Arc<Engine>,
    metrics_state: Option<Arc<RwLock<MetricsState>>>,
) {
    let mut cache: HashMap<String, RequestContext> = HashMap::new();

    // ── Seed gauges from the engine's current state ──────────────────────
    // Discovery has already run by the time the bridge starts, so the
    // engine knows which models are loaded and how much memory is used.
    let status = engine.status().await;
    tracing::info!(
        "Bridge: seeding gauges — {} models loaded, {:.1} MiB used / {:.1} MiB budget",
        status.loaded_models,
        status.memory_used_bytes as f64 / 1_048_576.0,
        status.memory_budget_bytes as f64 / 1_048_576.0,
    );

    // Set memory gauges directly via MetricsState instead of faking eviction events.
    if let Some(ref ms) = metrics_state {
        let mut state = ms.write().await;
        state.memory_used_bytes.set(
            mofa_observability::collector::Labels::new(),
            status.memory_used_bytes as f64,
        );
        state.memory_budget_bytes.set(
            mofa_observability::collector::Labels::new(),
            status.memory_budget_bytes as f64,
        );
    }

    // Set models_loaded gauge by emitting one ModelLoaded per loaded model
    let caps = engine.capabilities().await;
    for card in &caps {
        if matches!(
            card.residency,
            mofa_kernel::ModelResidency::Loaded | mofa_kernel::ModelResidency::Remote
        ) {
            sender.send(EventEnvelope::now(ObsEngineEvent::ModelLoaded(
                ModelLoaded {
                    model_id: card.id.clone(),
                    backend: card.provider.clone(),
                    capability: map_capability(&card.capability),
                    load_duration_ms: 0,
                    memory_bytes: card.memory_estimate_bytes,
                },
            )));
        }
    }

    // ── Event loop with periodic gauge sync ────────────────────────────
    let mut gauge_interval = tokio::time::interval(std::time::Duration::from_secs(10));
    gauge_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event_result = engine_events.recv() => {
                match event_result {
                    Ok(event) => {
                match event {
                    EngineEvent::RequestStarted {
                        request_id,
                        capability,
                        model_id,
                    } => {
                        let obs_cap = capability
                            .as_ref()
                            .map(map_capability)
                            .unwrap_or(mofa_observability::events::Capability::Chat);

                        // Extract provider from "provider/model" format
                        let backend = if model_id.contains('/') {
                            model_id.split('/').next().unwrap_or("unknown").to_string()
                        } else {
                            "local".to_string()
                        };

                        cache.insert(
                            request_id.clone(),
                            RequestContext {
                                capability: obs_cap,
                                model_id: model_id.clone(),
                                backend: backend.clone(),
                            },
                        );

                        let obs_event = ObsEngineEvent::RequestReceived(RequestReceived {
                            capability: obs_cap,
                            model: Some(model_id),
                            hint: None,
                        });
                        let envelope =
                            EventEnvelope::now(obs_event).with_request_id(&request_id);
                        sender.send(envelope);
                    }
                    EngineEvent::RequestCompleted {
                        request_id,
                        duration_ms,
                        success,
                    } => {
                        let (obs_model_id, obs_capability, obs_backend) = cache
                            .remove(&request_id)
                            .map(|ctx| (ctx.model_id, ctx.capability, ctx.backend))
                            .unwrap_or_else(|| {
                                (
                                    "unknown".into(),
                                    mofa_observability::events::Capability::Chat,
                                    "unknown".into(),
                                )
                            });

                        let obs_event =
                            ObsEngineEvent::RequestCompleted(RequestCompleted {
                                model_id: obs_model_id,
                                backend: obs_backend,
                                capability: obs_capability,
                                duration_ms,
                                ttft_ms: None,
                                tokens_in: None,
                                tokens_out: None,
                                model_was_hot: None,
                                success,
                                error_code: None,
                                is_local: None,
                            });
                        let envelope =
                            EventEnvelope::now(obs_event).with_request_id(&request_id);
                        sender.send(envelope);
                    }
                    EngineEvent::ModelResidencyChanged {
                        model_id,
                        old,
                        new,
                    } => {
                        use mofa_kernel::ModelResidency;
                        match (old, new) {
                            // Model loaded into memory
                            (_, ModelResidency::Loaded) => {
                                let backend = if model_id.contains('/') {
                                    model_id.split('/').next().unwrap_or("unknown").to_string()
                                } else {
                                    "local".to_string()
                                };
                                sender.send(EventEnvelope::now(
                                    ObsEngineEvent::ModelLoaded(ModelLoaded {
                                        model_id: model_id.clone(),
                                        backend,
                                        capability:
                                            mofa_observability::events::Capability::Chat,
                                        load_duration_ms: 0,
                                        memory_bytes: 0,
                                    }),
                                ));
                            }
                            // Model unloaded from memory
                            (ModelResidency::Loaded, _) => {
                                sender.send(EventEnvelope::now(
                                    ObsEngineEvent::ModelUnloaded(ModelUnloaded {
                                        model_id: model_id.clone(),
                                        reason: UnloadReason::Explicit,
                                        memory_freed_bytes: 0,
                                    }),
                                ));
                            }
                            _ => {} // Loading, Unloading — intermediate states, skip
                        }
                    }
                    EngineEvent::MemoryChanged {
                        used_bytes,
                        total_bytes,
                    } => {
                        // Set the memory gauges directly from the authoritative engine value
                        if let Some(ref ms) = metrics_state {
                            let mut state = ms.write().await;
                            state.memory_used_bytes.set(
                                mofa_observability::collector::Labels::new(),
                                used_bytes as f64,
                            );
                            state.memory_budget_bytes.set(
                                mofa_observability::collector::Labels::new(),
                                total_bytes as f64,
                            );
                        }
                    }
                    EngineEvent::DiscoveryCompleted {
                        provider, models, ..
                    } => {
                        tracing::info!(
                            provider = %provider,
                            models = models,
                            "Bridge: discovery completed"
                        );
                    }
                    _ => {
                        // ModelStatusChanged, ProviderHealthChanged — not mapped yet
                    }
                }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Observability bridge lagged, skipped {} events", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::info!("Observability bridge shutting down");
                        break;
                    }
                }
            }
            _ = gauge_interval.tick() => {
                // Periodic gauge sync: directly write memory and model-count
                // gauges from the authoritative engine state every 10 seconds.
                if let Some(ref ms) = metrics_state {
                    let status = engine.status().await;
                    let mut state = ms.write().await;
                    state.memory_used_bytes.set(Labels::new(), status.memory_used_bytes as f64);
                    state.memory_budget_bytes.set(Labels::new(), status.memory_budget_bytes as f64);
                    state.models_loaded.set(Labels::new(), status.loaded_models as f64);

                    tracing::trace!(
                        memory_used_mib = status.memory_used_bytes as f64 / 1_048_576.0,
                        memory_budget_mib = status.memory_budget_bytes as f64 / 1_048_576.0,
                        loaded_models = status.loaded_models,
                        "Bridge: periodic gauge sync"
                    );
                }
            }
        }
    }
}

fn map_capability(cap: &mofa_kernel::Capability) -> mofa_observability::events::Capability {
    use mofa_kernel::Capability as K;
    use mofa_observability::events::Capability as O;
    match cap {
        K::Chat => O::Chat,
        K::Tts => O::Tts,
        K::Asr => O::Asr,
        K::Vlm => O::Vlm,
        K::ImageGen => O::ImageGen,
        K::VideoGen => O::VideoGen,
        K::Embedding => O::Embedding,
        _ => {
            tracing::warn!("Unknown capability variant, defaulting to Chat for observability");
            O::Chat
        }
    }
}
