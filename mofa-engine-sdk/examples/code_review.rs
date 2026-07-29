//! Scenario S2 — Code/PR Review Agent.
//!
//! Demonstrates the engine's deep-thinking path: a high-effort `reasoning`
//! request whose **thought chain streams as `Reasoning` chunks, distinct from the
//! final answer's `Text` chunks**, with token/cost reported on completion. When a
//! model declares a `reasoning_tier`, `effort=high` routes to the stronger tier.
//!
//! Run (needs a chat/reasoning backend, e.g. Ollama or a cloud key):
//!   cargo run -p mofa-engine-sdk --example code_review
//!
//! The demo degrades gracefully: with no capable backend it prints the structured
//! error (including any `failed_chain`) instead of panicking.

use std::io::Write;

use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, Reasoning, ReasoningEffort, StreamChunk};

const DIFF: &str = r#"--- a/auth.rs
+++ b/auth.rs
@@
-    let token = req.header("authorization").unwrap();
+    let token = req.header("authorization").unwrap_or("");
     if token == expected { grant() } else { deny() }
"#;

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

    let prompt = format!(
        "Review the following diff. Annotate issues (severity: blocker/major/minor) \
         and suggest fixes:\n{DIFF}"
    );
    let request = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![
            Message {
                role: "system".into(),
                content: "You are a senior code reviewer. Think step by step, then \
                          give a concise annotated report."
                    .into(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: prompt,
                ..Default::default()
            },
        ],
        // High effort routes to the strongest reasoning tier (see `reasoning_tier`
        // in config) and surfaces the thought chain.
        reasoning: Some(Reasoning {
            effort: ReasoningEffort::High,
            include: true,
        }),
        // Cost control (S2): cap spend at $0.05/review. A pricier cloud candidate
        // is priced out (listed in `failed_chain`) so a cheaper/local reasoning
        // model wins; a local distilled R1 estimates to $0 and always fits.
        max_cost_usd: Some(0.05),
        ..Default::default()
    };

    println!("== S2 Code Review — streaming (reasoning vs answer) ==\n");
    let mut rx = engine.invoke_stream(request);
    let mut in_thoughts = false;
    let mut in_answer = false;
    while let Some(chunk) = rx.recv().await {
        match chunk {
            StreamChunk::Started {
                model_used,
                provider,
                ..
            } => {
                println!("[routed to {provider}/{model_used}]\n");
            }
            StreamChunk::Reasoning { delta } => {
                if !in_thoughts {
                    print!("🧠 thinking: ");
                    in_thoughts = true;
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            StreamChunk::Text { delta } => {
                if !in_answer {
                    println!("\n\n📝 report:");
                    in_answer = true;
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            StreamChunk::Completed {
                tokens_used,
                cost_usd,
                ..
            } => {
                println!(
                    "\n\n[done — tokens: {:?}, est. cost: {:?}]",
                    tokens_used, cost_usd
                );
            }
            StreamChunk::Error(info) => {
                eprintln!("\nerror [{:?}]: {}", info.code, info.message);
                for attempt in &info.failed_chain {
                    eprintln!(
                        "  tried {}/{}: {}",
                        attempt.provider, attempt.model, attempt.reason
                    );
                }
            }
            _ => {}
        }
    }
}
