//! Scenario S6 — Podcast / Long-Audio Matrix.
//!
//! An article → a natural, colloquial **podcast episode you can play**. Two
//! stages, both local-first: rewrite the article into spoken-style script
//! (`Chat`), then synthesize narration (`Tts`). The rewrite carries
//! `hint_next="tts"`, so the engine warms the TTS model *while* the script is
//! being written — the second stage starts from a hot model instead of a cold
//! one (the cross-capability warmup that gateways proxying tokens don't have).
//!
//! This is the Rust port of `mofa-fm/article_to_podcast.py`, the repo's original
//! runnable demo. The whole chain runs offline on Ollama + a local TTS backend.
//!
//! Run (needs a chat model + a TTS backend; both can be local):
//!   cargo run -p mofa-engine-sdk --example podcast -- ./article.txt
//!   cargo run -p mofa-engine-sdk --example podcast        # uses a built-in sample
//!
//! Each stage degrades gracefully: a missing backend prints the structured error
//! and stops rather than shipping a half-made episode.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, InferenceResponse, Message};

/// A short built-in article so the demo runs with zero arguments.
const SAMPLE_ARTICLE: &str = "\
Local-first AI is having a moment. As open models get small enough to run on a \
laptop, developers are discovering they can keep sensitive data on-device, cut \
cloud bills to zero for most requests, and still reach for a hosted model when a \
task genuinely needs one. The catch has always been glue code: every provider \
ships its own SDK, its own auth, its own error shapes. An orchestration layer \
that unifies them — routing locally when it can and to the cloud when it must — \
is what turns a pile of models into a product.";

fn main() {
    // Load the article from a file argument, or fall back to the built-in sample.
    let article = match std::env::args().nth(1) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("could not read {path}: {e}");
                return;
            }
        },
        None => SAMPLE_ARTICLE.to_string(),
    };

    let config = EngineConfig::load(None);
    let engine = match EmbeddedEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh();

    println!("== S6 Podcast — article → narrated episode ==\n");

    // 1. Chat: rewrite the article into a spoken-style script. `hint_next="tts"`
    // warms the synthesis model during this step so stage 2 has no cold start.
    let rewrite = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![
            Message {
                role: "system".into(),
                content: "Rewrite the article as a natural, colloquial podcast \
                          monologue under 200 words. No headings, just what the \
                          host would say aloud."
                    .into(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: article,
                ..Default::default()
            },
        ],
        hint_next: Some("tts".into()),
        ..Default::default()
    };
    let script = match Podcast::run(&engine, rewrite, "Chat (rewrite)") {
        Some(r) => r.text.unwrap_or_default(),
        None => return,
    };
    println!("\nscript:\n{}\n", script.trim());

    // 2. TTS: script → a playable audio file. Local Crane/Kokoro when available,
    // cloud tts-1 as fallback under `auto`.
    let narrate = InferenceRequest {
        capability: Some(Capability::Tts),
        messages: vec![Message {
            role: "user".into(),
            content: script,
            ..Default::default()
        }],
        params: serde_json::json!({ "voice": "narrator", "format": "mp3" }),
        ..Default::default()
    };
    if let Some(r) = Podcast::run(&engine, narrate, "TTS (narration)") {
        match r.file.as_deref() {
            Some(path) => println!("\n🎧 episode: {path}"),
            None => println!("\n(TTS returned no file path)"),
        }
    }
}

/// Namespace for this demo's helpers.
struct Podcast;

impl Podcast {
    /// Invoke one stage, reporting where it routed (and cost) on success or the
    /// structured error on failure. Returns `None` to stop the pipeline.
    fn run(
        engine: &EmbeddedEngine,
        req: InferenceRequest,
        stage: &str,
    ) -> Option<InferenceResponse> {
        match engine.invoke(req) {
            Ok(resp) => {
                let cost = resp
                    .cost_usd
                    .map(|c| format!("${c:.6}"))
                    .unwrap_or_else(|| "$0.00 (local)".into());
                println!(
                    "[{stage}] served by {} in {}ms · {cost}",
                    resp.provider, resp.duration_ms
                );
                Some(resp)
            }
            Err(e) => {
                let info = e.info();
                eprintln!("[{stage}] error [{:?}]: {}", info.code, info.message);
                for a in &info.failed_chain {
                    eprintln!("    tried {}/{}: {}", a.provider, a.model, a.reason);
                }
                None
            }
        }
    }
}
