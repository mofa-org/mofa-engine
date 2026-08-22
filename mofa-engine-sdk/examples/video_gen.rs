//! Scenario S4 (video) — text-to-video at the **API level**.
//!
//! The engine exposes `Capability::VideoGen` through the same contract as every
//! other capability, so a prompt can be turned into a real clip by a cloud video
//! model — ByteDance's **Seedance** via the Volcengine Ark / BytePlus task API —
//! without the caller knowing anything about task submission, polling, or the
//! download. Configure it (see `config.example.toml`, `kind = "cloud_video_gen"`)
//! and the request below returns a managed `mp4` artifact:
//!
//!   [[providers]]
//!   name = "seedance"
//!   kind = "cloud_video_gen"
//!   api_key = "env:ARK_API_KEY"
//!     [[providers.models]]
//!     name = "doubao-seedance-1-0-pro-250528"
//!     capability = "video_gen"
//!
//! With no video backend configured (the default offline setup) the demo still
//! runs: it reports that no `video_gen` model is available and prints the typed
//! routing error, rather than pretending to render — the honest degradation the
//! rest of the scenarios follow.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, InferenceRequest, Message, Prefer};

const PROMPT: &str = "A paper airplane glides over a sunlit desk, then banks toward the window. Cinematic, soft light.";

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

    println!("== S4 Video — text-to-video via the video_gen capability ==");
    println!("prompt: {PROMPT}\n");

    // Surface what the engine can actually do: is any video_gen model reachable?
    // This is the "support at the API level" check — the capability is first-class
    // whether or not a backend happens to be configured right now.
    let video_models: Vec<_> = engine
        .capabilities()
        .into_iter()
        .filter(|c| c.capabilities.contains(&Capability::VideoGen))
        .collect();

    if video_models.is_empty() {
        println!("no video_gen backend configured.");
        println!(
            "add a cloud_video_gen (Seedance) or local_video_gen provider — see \
             config.example.toml — then re-run to render a real clip."
        );
    } else {
        println!("video_gen backends available:");
        for c in &video_models {
            println!("  - {}/{}  [{:?}]", c.provider, c.name, c.availability);
        }
        println!();
    }

    // Issue the request regardless: with a backend it renders and returns an mp4;
    // without one it returns a typed `no_capable_model` error. Either way the
    // *call site* is identical — that is the point of API-level support.
    let request = InferenceRequest {
        capability: Some(Capability::VideoGen),
        messages: vec![Message {
            role: "user".into(),
            content: PROMPT.into(),
            ..Default::default()
        }],
        // Seedance generation knobs travel in `params` (ratio / resolution /
        // duration / fps / seed, and `image_url` for image-to-video).
        params: serde_json::json!({
            "ratio": "16:9",
            "resolution": "720p",
            "duration": 5
        }),
        // Video generation is inherently a cloud call here, so allow it.
        prefer: Prefer::Auto,
        ..Default::default()
    };

    match engine.invoke(request) {
        Ok(resp) => {
            println!(
                "[video_gen] served by {} in {}ms",
                resp.provider, resp.duration_ms
            );
            match resp.file {
                Some(path) => println!(
                    "clip saved → {path}\n(verify it with the quality gate before shipping)"
                ),
                None => println!("backend returned no file artifact"),
            }
        }
        Err(e) => {
            let info = e.info();
            println!(
                "[video_gen] not rendered — {:?}: {}",
                info.code, info.message
            );
        }
    }
}
