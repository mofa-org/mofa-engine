//! Scenario S4 — Explainer Video (flagship).
//!
//! One-sentence topic → a finished `mp4`. MoFA is the **orchestration layer + a
//! thin pipeline runner**: it drives `Chat (+reasoning) → ImageGen → TTS → ASR`
//! and then enforces a **hard quality gate** on the composed video, so a broken
//! render (empty file, a still slideshow, or footage that doesn't match the
//! script) is never shipped as "finished" — "no gate, no output".
//!
//! The composition step itself (scenes + narration + captions → `mp4`) reuses
//! Remotion/FFmpeg and is **not** re-implemented here (PRD scope boundary). Pass
//! an already-composed `mp4` to see the real gate run end-to-end:
//!
//!   cargo run -p mofa-engine-sdk --example explainer_video -- ./final.mp4
//!
//! With no file and no backends the pipeline still runs, degrading gracefully and
//! explaining what each stage would produce.

use std::path::Path;

use mofa_engine_core::quality_gate::QualityGate;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, Prefer, Reasoning, ReasoningEffort};

const TOPIC: &str = "Make a 45s animation explainer: How neural networks learn";

#[tokio::main]
async fn main() {
    let final_mp4 = std::env::args().nth(1);

    let config = EngineConfig::load(None);
    let engine = match Engine::try_new(config).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh_resources().await;

    println!("== S4 Explainer Video — orchestration + hard quality gate ==");
    println!("topic: {TOPIC}\n");

    // 1. Script (Chat + optional reasoning), warming ImageGen for the next stage.
    let script = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![Message {
            role: "user".into(),
            content: format!("{TOPIC}\nWrite a short scene-by-scene narration script."),
            ..Default::default()
        }],
        reasoning: Some(Reasoning {
            effort: ReasoningEffort::Medium,
            include: false,
        }),
        hint_next: Some("image_gen".into()),
        ..Default::default()
    };
    let script = stage(&engine, script, "Script (chat)").await;

    // 2. Scene images (ImageGen, prefer local SD when available).
    let scene_prompt = "flat vector animation frame: a neuron adjusting its weights";
    let image = InferenceRequest {
        capability: Some(Capability::ImageGen),
        messages: vec![Message {
            role: "user".into(),
            content: scene_prompt.into(),
            ..Default::default()
        }],
        params: serde_json::json!({ "size": "1024x1024" }),
        // Offline flagship: prefer local SD (cost ~= $0), fall back to cloud only
        // under `auto`. Here we keep it local-first to showcase on-device gen.
        prefer: Prefer::Local,
        hint_next: Some("tts".into()),
        ..Default::default()
    };
    stage(&engine, image, "Scene image (image_gen)").await;

    // 3. Narration (TTS).
    let narration = InferenceRequest {
        capability: Some(Capability::Tts),
        messages: vec![Message {
            role: "user".into(),
            content: script.clone().unwrap_or_else(|| TOPIC.into()),
            ..Default::default()
        }],
        hint_next: Some("asr".into()),
        ..Default::default()
    };
    stage(&engine, narration, "Narration (tts)").await;

    // 4. (compose) — external: scenes + narration + captions → final.mp4 via
    //    Remotion/FFmpeg. Out of engine scope; supply the result as an argument.
    println!("\n[compose] scenes + narration + captions → final.mp4 (external: Remotion/FFmpeg)");

    // 5. Quality gate — the hard gate. Runs against a real mp4 when provided.
    println!("\n== quality gate ==");
    match final_mp4.as_deref() {
        Some(path) => run_gate(Path::new(path)).await,
        None => println!(
            "no composed mp4 supplied — pass one to run the gate:\n  \
             cargo run -p mofa-engine-sdk --example explainer_video -- ./final.mp4"
        ),
    }
}

/// Run one orchestration stage, reporting where it was routed or the structured
/// error (the pipeline continues either way — this demo showcases orchestration,
/// not a live render). Returns the text output when present.
async fn stage(engine: &Engine, req: InferenceRequest, label: &str) -> Option<String> {
    match engine.invoke(req).await {
        Ok(resp) => {
            println!(
                "[{label}] served by {} in {}ms",
                resp.provider, resp.duration_ms
            );
            resp.text
        }
        Err(e) => {
            let info = e.info();
            println!("[{label}] skipped — {:?}: {}", info.code, info.message);
            None
        }
    }
}

/// Run the real quality gate over a composed video and print the verdict. The
/// optional VLM image/text check is omitted here (`None`); a full pipeline would
/// pass the engine's `Vlm` verdict over sampled frames.
async fn run_gate(path: &Path) {
    let report = QualityGate::new().check(path, None).await;
    for c in &report.checks {
        println!(
            "  [{}] {}: {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
    }
    if report.passed {
        println!("verdict: ACCEPTED ✅");
    } else {
        println!("verdict: REJECTED ❌ (no gate, no output)");
    }
}
