//! Scenario S7 — Multi-Vendor Unified Access.
//!
//! The same question, sent to **every** chat model the engine discovered — local
//! and cloud — through one interface, one request type, one error shape. The
//! output is a side-by-side table: provider · latency · tokens · cost. This is
//! the infrastructure scenario made visible: adding a vendor is config, not code,
//! and the **dual-track cost** column shows local models landing at `$0` next to
//! metered cloud ones — the comparison a plain OpenAI-compatible proxy can't draw
//! because it never sees the local side.
//!
//! Run (more interesting with both a local Ollama and a cloud key configured):
//!   cargo run -p mofa-engine-sdk --example provider_race
//!
//! Models that can't serve (unavailable, over budget) show up as a row with their
//! failure reason rather than aborting the race — fault tolerance on display.

use std::time::Instant;

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, Message};

const QUESTION: &str = "Explain what a circuit breaker does, in two sentences.";

fn main() {
    let config = EngineConfig::load(None);
    let engine = match EmbeddedEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh();

    // Every chat-capable model becomes a lane in the race. We pin each request to
    // one model by id so the router targets exactly that backend instead of
    // picking a winner — the whole point is to compare them.
    let lanes: Vec<_> = engine
        .capabilities()
        .into_iter()
        .filter(|c| c.capabilities.contains(&Capability::Chat))
        .collect();

    println!("== S7 Multi-Vendor Race ==");
    println!("question: {QUESTION}\n");
    if lanes.is_empty() {
        println!("no chat backends discovered — start Ollama or set a cloud API key.");
        return;
    }
    println!(
        "{:<30} {:>8}  {:>8}  {:>12}",
        "provider/model", "ms", "tokens", "cost"
    );
    println!("{}", "-".repeat(64));

    // Track the cheapest and fastest successful lanes for a closing summary.
    let mut best_cost: Option<(String, f64)> = None;
    let mut best_latency: Option<(String, u128)> = None;

    for card in &lanes {
        let request = InferenceRequest {
            capability: Some(Capability::Chat),
            model: Some(card.id.clone()),
            messages: vec![Message {
                role: "user".into(),
                content: QUESTION.into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        // Wall-clock the call ourselves too, so an error row still reports how long
        // the attempt took before failing.
        let started = Instant::now();
        match engine.invoke(request) {
            Ok(resp) => {
                let elapsed = started.elapsed().as_millis();
                let cost = resp.cost_usd.unwrap_or(0.0);
                let cost_str = if cost > 0.0 {
                    format!("${cost:.6}")
                } else {
                    "$0.00 local".into()
                };
                println!(
                    "{:<30} {:>8} {:>8}  {:>12}",
                    card.id,
                    resp.duration_ms,
                    resp.tokens_used
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "-".into()),
                    cost_str,
                );

                // Fold this lane into the running winners.
                match &best_cost {
                    Some((_, c)) if *c <= cost => {}
                    _ => best_cost = Some((card.id.clone(), cost)),
                }
                match &best_latency {
                    Some((_, ms)) if *ms <= elapsed => {}
                    _ => best_latency = Some((card.id.clone(), elapsed)),
                }
            }
            Err(e) => {
                let info = e.info();
                println!("{:<30} {:>8}  {:>8}  {:>12}", card.id, "-", "-", "failed");
                println!("    └─ {:?}: {}", info.code, info.message);
            }
        }
    }

    println!();
    if let Some((id, cost)) = best_cost {
        let label = if cost > 0.0 {
            format!("${cost:.6}")
        } else {
            "$0.00 (local, free)".into()
        };
        println!("cheapest: {id} — {label}");
    }
    if let Some((id, ms)) = best_latency {
        println!("fastest:  {id} — {ms}ms");
    }
}
