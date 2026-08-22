//! Streaming — real per-token output, and time-to-first-token.
//!
//! The engine streams genuine incremental tokens from the backend (Ollama NDJSON
//! and cloud SSE are both parsed into the internal `StreamChunk` stream), not a
//! whole answer sliced up after the fact. This demo makes that visible by
//! printing the **time to first token** — the number that only moves if streaming
//! is real — then the tokens as they arrive, and the token/cost total at the end.
//!
//! Run (needs a chat backend; local Ollama is fine):
//!   cargo run -p mofa-engine-sdk --example streaming_chat
//!
//! With no backend it prints the structured error (including any `failed_chain`).

use std::io::Write;
use std::time::Instant;

use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, StreamChunk};

#[tokio::main]
async fn main() {
    let config = EngineConfig::load(None);
    let engine = match Engine::try_new(config).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh_resources().await;

    let request = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![Message {
            role: "user".into(),
            content: "Write a short haiku about local-first inference.".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    println!("== streaming chat — watch time-to-first-token ==\n");
    let started = Instant::now();
    let mut first_token_at: Option<u128> = None;

    let mut rx = engine.invoke_stream(request);
    while let Some(chunk) = rx.recv().await {
        match chunk {
            StreamChunk::Started {
                provider,
                model_used,
                ..
            } => {
                println!("[routed to {provider}/{model_used}]\n");
            }
            StreamChunk::Text { delta } => {
                // Record when the very first content token lands — the honest
                // latency-to-first-token that pseudo-streaming can't improve.
                if first_token_at.is_none() {
                    first_token_at = Some(started.elapsed().as_millis());
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            StreamChunk::Completed {
                tokens_used,
                cost_usd,
                ..
            } => {
                let ttft = first_token_at
                    .map(|ms| format!("{ms}ms"))
                    .unwrap_or_else(|| "n/a".into());
                let cost = cost_usd
                    .map(|c| format!("${c:.6}"))
                    .unwrap_or_else(|| "$0.00 (local)".into());
                println!(
                    "\n\n[first token: {ttft} · total: {}ms · tokens {:?} · {cost}]",
                    started.elapsed().as_millis(),
                    tokens_used,
                );
            }
            StreamChunk::Error(info) => {
                eprintln!("\nerror [{:?}]: {}", info.code, info.message);
                for a in &info.failed_chain {
                    eprintln!("  tried {}/{}: {}", a.provider, a.model, a.reason);
                }
            }
            _ => {}
        }
    }
}
