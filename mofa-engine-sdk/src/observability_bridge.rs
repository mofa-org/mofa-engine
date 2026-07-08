//! Observability integration bridge.
//!
//! This module handles translating core engine events into the observability subsystem's format.
//! It also seeds initial metric values from the engine's status on startup.

use mofa_engine_core::Engine;
use mofa_kernel::EngineEvent;
use mofa_observability::collector::EventSender;
use mofa_observability::events::{
    EngineEvent as ObsEngineEvent, EventEnvelope, EvictionTriggered, ModelLoaded, ModelUnloaded,
    RequestCompleted, RequestReceived, UnloadReason,
};
use std::collections::HashMap;
use std::sync::Arc;
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

    // Set memory gauges
    let _ = sender
        .send_critical(EventEnvelope::now(ObsEngineEvent::EvictionTriggered(
            EvictionTriggered {
                evicted_model: "_seed".into(),
                memory_before_bytes: status.memory_used_bytes,
                memory_after_bytes: status.memory_used_bytes,
                budget_bytes: status.memory_budget_bytes,
            },
        )))
        .await;

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

    // ── Event loop ───────────────────────────────────────────────────────
    loop {
        match engine_events.recv().await {
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
                        let mut envelope =
                            EventEnvelope::now(obs_event).with_request_id(&request_id);
                        if let Some((trace_id, span_id)) = derive_trace_context(&request_id) {
                            envelope = envelope.with_trace(trace_id, span_id);
                        }
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
                            });
                        let mut envelope =
                            EventEnvelope::now(obs_event).with_request_id(&request_id);
                        if let Some((trace_id, span_id)) = derive_trace_context(&request_id) {
                            envelope = envelope.with_trace(trace_id, span_id);
                        }
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
                        // Set the memory gauges from the authoritative engine value
                        let _ = sender
                            .send_critical(EventEnvelope::now(
                                ObsEngineEvent::EvictionTriggered(EvictionTriggered {
                                    evicted_model: "_memory_sync".into(),
                                    memory_before_bytes: used_bytes,
                                    memory_after_bytes: used_bytes,
                                    budget_bytes: total_bytes,
                                }),
                            ))
                            .await;
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

fn derive_trace_context(request_id: &str) -> Option<(String, String)> {
    let trace_id = request_id.replace('-', "");
    if trace_id.len() == 32 {
        let span_id = trace_id[0..16].to_string();
        Some((trace_id, span_id))
    } else {
        None
    }
}
