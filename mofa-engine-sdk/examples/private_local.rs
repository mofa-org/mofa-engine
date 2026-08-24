//! The privacy moat — `prefer=local` + `data_class=confidential`.
//!
//! MoFA's differentiator over any cloud gateway is a *hard* data-residency
//! boundary: a request marked `confidential` is pinned to local backends, and if
//! none can serve it the request **fails rather than falling back to the cloud**.
//! "Fail, not leak" is the guarantee a compliance team can actually sign off on.
//!
//! This demo sends the same prompt twice:
//!   1. `prefer=auto`        — normal routing (may pick cloud if it scores best).
//!   2. `prefer=local` +      — pinned on-device; the sensitive text never leaves
//!      `data_class=confidential`  the machine, even if that means failing.
//!
//! Run:
//!   cargo run -p mofa-engine-sdk --example private_local
//!
//! To *see* the guarantee bite, run it with no local backend available (stop
//! Ollama): the confidential call returns a structured error while the `auto`
//! call may still succeed via cloud — exactly the point.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, DataClass, InferenceRequest, Message, Prefer};

/// Stand-in for text a business would never want sent to a third-party API.
const SENSITIVE: &str =
    "Summarize this internal note: Q3 layoffs affect the Shanghai team; do not disclose.";

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

    println!("== privacy moat — fail, don't leak ==\n");

    // 1. Default routing: the engine is free to choose cloud if it scores best.
    let auto = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![Message {
            role: "user".into(),
            content: SENSITIVE.into(),
            ..Default::default()
        }],
        prefer: Prefer::Auto,
        ..Default::default()
    };
    describe(&engine, auto, "prefer=auto (unconstrained)");

    // 2. Confidential: pinned local. `data_class=confidential` alone pins it, and
    // `prefer=local` states the intent explicitly. If no local model can serve,
    // this returns an error with a `failed_chain` — it does not reach for cloud.
    let confidential = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![Message {
            role: "user".into(),
            content: SENSITIVE.into(),
            ..Default::default()
        }],
        prefer: Prefer::Local,
        data_class: DataClass::Confidential,
        ..Default::default()
    };
    describe(
        &engine,
        confidential,
        "prefer=local + confidential (pinned)",
    );
}

/// Invoke and report where the request actually ran — local vs cloud is the whole
/// story here — or, on failure, that it declined to leave the device.
fn describe(engine: &EmbeddedEngine, req: InferenceRequest, label: &str) {
    println!("── {label}");
    match engine.invoke(req) {
        Ok(resp) => {
            let cost = resp
                .cost_usd
                .map(|c| format!("${c:.6}"))
                .unwrap_or_else(|| "$0.00 (local/free)".into());
            println!(
                "   served by {} · {}ms · {cost}",
                resp.provider, resp.duration_ms
            );
            if let Some(reason) = &resp.routing_reason {
                println!("   routing: {reason}");
            }
            println!(
                "   answer: {}\n",
                resp.text.as_deref().unwrap_or("(no text)").trim()
            );
        }
        Err(e) => {
            let info = e.info();
            println!("   declined — {:?}: {}", info.code, info.message);
            for a in &info.failed_chain {
                println!("     considered {}/{}: {}", a.provider, a.model, a.reason);
            }
            println!("   → the confidential text was NOT sent anywhere.\n");
        }
    }
}
