//! Scenario S3 — Document/Screenshot AI.
//!
//! Demonstrates vision-language understanding: a photo (receipt / screenshot) plus
//! a question, extracted into **structured data**. Images ride on
//! `Message.images` (URL, `data:` URL, or local path) and the `detail` tier
//! (`low | high | auto`) passes through to the vendor's image billing.
//!
//! Run (needs a VLM-capable backend, e.g. a cloud key via liter-llm):
//!   cargo run -p mofa-engine-sdk --example doc_ai -- ./receipt.jpg
//!
//! With no capable backend it prints the structured error instead of panicking.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, Message};

fn main() {
    // The image reference: a CLI arg, or a placeholder URL for a dry run.
    let image = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com/receipt.jpg".to_string());

    let config = EngineConfig::load(None);
    let engine = match EmbeddedEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh();

    let request = InferenceRequest {
        capability: Some(Capability::Vlm),
        messages: vec![Message {
            role: "user".into(),
            content: "Extract amount, date, and category from this receipt. \
                      Respond as JSON."
                .into(),
            images: vec![image.clone()],
        }],
        // `detail=low` keeps batch cost down; edge cases can re-run at `high`.
        params: serde_json::json!({ "detail": "low" }),
        ..Default::default()
    };

    println!("== S3 Document AI — VLM extract ==");
    println!("image: {image}\n");
    match engine.invoke(request) {
        Ok(resp) => {
            println!("provider: {}", resp.provider);
            println!("extracted: {}", resp.text.as_deref().unwrap_or("(no text)"));
            if let Some(cost) = resp.cost_usd {
                println!("est. cost: ${cost:.6}");
            }
        }
        Err(e) => {
            let info = e.info();
            eprintln!("error [{:?}]: {}", info.code, info.message);
        }
    }
}
