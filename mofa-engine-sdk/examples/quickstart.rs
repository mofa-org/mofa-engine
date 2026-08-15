//! Quickstart — the 5-minute "does it actually run?" demo.
//!
//! This is the shortest path from a fresh checkout to a real answer: boot the
//! embedded engine, show what backends it discovered, and run one chat request.
//! The point is not the answer — it's the line printed after it: **which
//! provider served the request, local or cloud, how long it took, and what it
//! cost**. That single line is the whole MoFA pitch (one interface, intelligent
//! routing, local-first, cost visible) made concrete.
//!
//! Run (offline: just needs a local Ollama with any chat model pulled):
//!   ollama serve &            # if not already running
//!   ollama pull llama3.2      # or any chat model
//!   cargo run -p mofa-engine-sdk --example quickstart
//!
//! With no backend at all it prints the structured error instead of panicking.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, Message};

fn main() {
    // Configuration comes from ./config.toml when present, otherwise environment
    // auto-detection (cloud keys) + a local Ollama at 127.0.0.1:11434.
    let config = EngineConfig::load(None);
    let engine = match EmbeddedEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh();

    // Show the routing table the engine has to work with. On a laptop with only
    // Ollama running this lists local models at cost tier "free"; add a cloud key
    // and cloud models appear alongside them — same interface, both routable.
    println!("== MoFA quickstart ==\n");
    let cards = engine.capabilities();
    if cards.is_empty() {
        println!("no backends discovered — start Ollama or set a cloud API key, then retry.\n");
    } else {
        println!("discovered {} model(s):", cards.len());
        for card in &cards {
            println!(
                "  · {:<28} [{}]  {:?}",
                card.id, card.provider, card.cost_tier
            );
        }
        println!();
    }

    // One ordinary chat request. `prefer` is left at its default (`auto`), so the
    // seven-dimensional router picks local-first and only reaches for cloud when a
    // local model can't serve — nothing here is pinned by hand.
    let request = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![Message {
            role: "user".into(),
            content: "In one sentence, what is MoFA Engine?".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    match engine.invoke(request) {
        Ok(resp) => {
            println!(
                "answer:\n  {}\n",
                resp.text.as_deref().unwrap_or("(no text)").trim()
            );
            // The MoFA pitch, made concrete: where it ran and what it cost.
            println!(
                "routed to {}/{}  ·  {}ms  ·  tokens {:?}  ·  cost {}",
                resp.provider,
                resp.model_used,
                resp.duration_ms,
                resp.tokens_used,
                resp.cost_usd
                    .map(|c| format!("${c:.6}"))
                    .unwrap_or_else(|| "$0.00 (local/free)".into()),
            );
            if let Some(reason) = &resp.routing_reason {
                println!("why: {reason}");
            }
        }
        Err(e) => {
            let info = e.info();
            eprintln!("error [{:?}]: {}", info.code, info.message);
            for a in &info.failed_chain {
                eprintln!("  tried {}/{}: {}", a.provider, a.model, a.reason);
            }
        }
    }
}
