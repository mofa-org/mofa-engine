//! Mock Event Harness for MoFA Engine Observability
//!
//! Generates synthetic engine events and serves them over an HTTP `/metrics` endpoint.
//! This allows testing the Prometheus renderer and populating Grafana dashboards
//! before the real engine is fully integrated.
//!
//! Run with: `cargo run --example mock_harness`

use axum::{extract::State, routing::get, Router};
use mofa_observability::{
    collector::{create_event_channel, EventSender, MetricsCollector, MetricsState},
    events::*,
    prometheus,
};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::RwLock;

/// Starts the HTTP server serving `/metrics`.
async fn metrics_handler(State(state): State<Arc<RwLock<MetricsState>>>) -> String {
    let state_reader = state.read().await;
    prometheus::render(&state_reader)
}

/// Generates a continuous stream of synthetic engine events.
async fn simulate_engine_traffic(sender: EventSender) {
    let mut rng = StdRng::from_entropy();

    // Available models for the mock
    let models = [
        ("qwen2.5:7b", "ollama", Capability::Chat),
        ("gemma3:4b", "ollama", Capability::Chat),
        ("whisper-1", "openai", Capability::Asr),
        ("kokoro", "local", Capability::Tts),
    ];

    // Initial setup: Load a couple of models to have baseline memory.
    let m1 = models[0];
    sender.send(EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
        model_id: m1.0.into(),
        backend: m1.1.into(),
        capability: m1.2,
        load_duration_ms: rng.gen_range(1500..3500),
        memory_bytes: 4_700_000_000,
    })));

    let m2 = models[3];
    sender.send(EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
        model_id: m2.0.into(),
        backend: m2.1.into(),
        capability: m2.2,
        load_duration_ms: rng.gen_range(500..1200),
        memory_bytes: 800_000_000,
    })));

    // Continuous event loop
    loop {
        // Sleep between 50ms and 500ms to generate roughly 2-20 requests/sec.
        tokio::time::sleep(Duration::from_millis(rng.gen_range(50..500))).await;

        let action = rng.gen_range(0..100);
        let selected_model = models.choose(&mut rng).unwrap();

        match action {
            // 5% chance: Load a new model
            0..=4 => {
                sender.send(EventEnvelope::now(EngineEvent::ModelLoaded(ModelLoaded {
                    model_id: selected_model.0.into(),
                    backend: selected_model.1.into(),
                    capability: selected_model.2,
                    load_duration_ms: rng.gen_range(1000..5000),
                    memory_bytes: rng.gen_range(500_000_000..8_000_000_000),
                })));
            }

            // 5% chance: Unload a model
            5..=9 => {
                let reason = match rng.gen_range(0..3) {
                    0 => UnloadReason::IdleTimeout,
                    1 => UnloadReason::Eviction,
                    _ => UnloadReason::Explicit,
                };
                sender.send(EventEnvelope::now(EngineEvent::ModelUnloaded(
                    ModelUnloaded {
                        model_id: selected_model.0.into(),
                        reason,
                        memory_freed_bytes: rng.gen_range(500_000_000..8_000_000_000),
                    },
                )));

                // If eviction, also trigger eviction event
                if reason == UnloadReason::Eviction {
                    sender.send_critical(EventEnvelope::now(EngineEvent::EvictionTriggered(
                        EvictionTriggered {
                            evicted_model: selected_model.0.into(),
                            memory_before_bytes: 15_000_000_000,
                            memory_after_bytes: 12_000_000_000,
                            budget_bytes: 16_000_000_000,
                        },
                    ))).await.expect("Failed to send critical event");
                }
            }

            // 10% chance: Preflight Hit/Miss
            10..=19 => {
                let is_hit = rng.gen_bool(0.85); // 85% accuracy
                let source = if rng.gen_bool(0.7) {
                    SignalSource::History
                } else {
                    SignalSource::Hint
                };

                sender.send(EventEnvelope::now(EngineEvent::PreflightSignal(
                    PreflightSignal {
                        predicted_capability: selected_model.2,
                        confidence: rng.gen_range(0.6..1.0),
                        source,
                    },
                )));

                if is_hit {
                    sender.send(EventEnvelope::now(EngineEvent::PreflightHit(
                        PreflightHit {
                            predicted_capability: selected_model.2,
                            cold_start_avoided_ms: rng.gen_range(1000..5000),
                        },
                    )));
                } else {
                    let mut wrong_cap = Capability::Chat;
                    if selected_model.2 == Capability::Chat {
                        wrong_cap = Capability::Tts;
                    }
                    sender.send(EventEnvelope::now(EngineEvent::PreflightMiss(
                        PreflightMiss {
                            predicted_capability: selected_model.2,
                            actual_capability: wrong_cap,
                        },
                    )));
                }
            }

            // 1% chance: Failover
            20..=20 => {
                sender.send_critical(EventEnvelope::now(EngineEvent::FailoverTriggered(
                    FailoverTriggered {
                        failed_model: selected_model.0.into(),
                        failed_backend: selected_model.1.into(),
                        fallback_model: "fallback-model".into(),
                        fallback_backend: "cloud".into(),
                    },
                ))).await.expect("Failed to send critical event");
            }

            // 79% chance: Normal Inference Request
            _ => {
                // 1. Request Received
                sender.send(EventEnvelope::now(EngineEvent::RequestReceived(
                    RequestReceived {
                        capability: selected_model.2,
                        model: None,
                        hint: None,
                    },
                )));

                // Sleep slightly to simulate inference time
                let duration_ms = match selected_model.2 {
                    Capability::Chat => rng.gen_range(200..15000), // Text generation is variable
                    _ => rng.gen_range(100..2000),                 // API calls faster
                };
                tokio::time::sleep(Duration::from_millis(duration_ms.min(100))).await;

                let success = rng.gen_bool(0.98); // 98% success rate

                let ttft_ms = if selected_model.2 == Capability::Chat && success {
                    Some(rng.gen_range(50..800))
                } else {
                    None
                };

                let tokens_in = if selected_model.2 == Capability::Chat {
                    Some(rng.gen_range(10..1000))
                } else {
                    None
                };

                let tokens_out = if selected_model.2 == Capability::Chat && success {
                    Some(rng.gen_range(5..800))
                } else {
                    None
                };

                // 2. Request Completed
                sender.send(EventEnvelope::now(EngineEvent::RequestCompleted(
                    RequestCompleted {
                        model_id: selected_model.0.into(),
                        backend: selected_model.1.into(),
                        capability: selected_model.2,
                        duration_ms,
                        ttft_ms,
                        tokens_in,
                        tokens_out,
                        model_was_hot: Some(rng.gen_bool(0.9)), // Usually hot
                        success,
                        error_code: if success {
                            None
                        } else {
                            Some("internal_error".into())
                        },
                    },
                )));
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("mofa_observability=info".parse().unwrap()))
        .init();

    println!("Starting MoFA Engine Observability Mock Harness...");

    // 1. Initialize Event Channel (capacity 1024)
    let (sender, receiver) = create_event_channel(1024);

    // 2. Initialize Metrics Collector
    let collector = MetricsCollector::new(receiver);
    let state = collector.state();

    // 3. Spawn Collector Task
    tokio::spawn(collector.run());

    // 4. Spawn Event Generator Task
    tokio::spawn(simulate_engine_traffic(sender));

    // 5. Setup HTTP Server for Prometheus scraping
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(state);

    let port = 9090;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("=======================================================");
    println!("🚀 Mock Harness Running!");
    println!("📊 Prometheus Endpoint: http://localhost:{}/metrics", port);
    println!("   Try running: curl http://localhost:{}/metrics", port);
    println!("=======================================================");

    axum::serve(listener, app).await?;

    Ok(())
}
