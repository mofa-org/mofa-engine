//! Scenario S1 — Meeting/Lecture → Minutes + Audio Brief.
//!
//! Demonstrates a confidential, local-first pipeline: long-audio **ASR →
//! structured minutes (chat) → 30s audio brief (TTS)**. The whole chain is pinned
//! on-device with `prefer=local` + `data_class=confidential`, so a confidential
//! recording never leaves the machine (it fails rather than falling back to cloud).
//! `hint_next="tts"` warms the TTS model while the minutes are generated.
//!
//! Run (needs local ASR + a chat model + local/cloud TTS configured):
//!   cargo run -p mofa-engine-sdk --example meeting_brief -- ./meeting.wav
//!
//! Each stage degrades gracefully: a missing backend prints the structured error
//! (a confidential stage that can't run locally fails on purpose) and stops.

use mofa_engine_core::EngineConfig;
use mofa_engine_sdk::EmbeddedEngine;
use mofa_kernel::{Capability, DataClass, InferenceRequest, InferenceResponse, Message, Prefer};

fn main() {
    let audio = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "meeting.wav".into());

    let config = EngineConfig::load(None);
    let engine = match EmbeddedEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("engine init failed: {e}");
            return;
        }
    };
    engine.refresh();

    println!("== S1 Meeting Brief — confidential, local-first ==");
    println!("audio: {audio}\n");

    // 1. ASR — long audio → transcript, pinned local (confidential), with speaker
    // diarization so the minutes can attribute resolutions/todos to people.
    let asr = InferenceRequest {
        capability: Some(Capability::Asr),
        input_file: Some(audio),
        prefer: Prefer::Local,
        data_class: DataClass::Confidential,
        params: serde_json::json!({ "diarize": true }),
        hint_next: Some("chat".into()),
        ..Default::default()
    };
    let transcript = match MeetingBrief::run(&engine, asr, "ASR") {
        Some(r) => r.text.unwrap_or_default(),
        None => return,
    };
    println!("transcript: {}\n", MeetingBrief::preview(&transcript));

    // 2. Chat — extract structured minutes; warm TTS meanwhile.
    let chat = InferenceRequest {
        capability: Some(Capability::Chat),
        messages: vec![
            Message {
                role: "system".into(),
                content: "Extract structured meeting minutes: resolutions, todos, \
                          risks, and responsible people."
                    .into(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: transcript,
                ..Default::default()
            },
        ],
        prefer: Prefer::Local,
        data_class: DataClass::Confidential,
        hint_next: Some("tts".into()),
        ..Default::default()
    };
    let minutes = match MeetingBrief::run(&engine, chat, "Chat (minutes)") {
        Some(r) => r.text.unwrap_or_default(),
        None => return,
    };
    println!("minutes:\n{}\n", MeetingBrief::preview(&minutes));

    // 3. TTS — minutes → a short audio brief.
    let tts = InferenceRequest {
        capability: Some(Capability::Tts),
        messages: vec![Message {
            role: "user".into(),
            content: minutes,
            ..Default::default()
        }],
        prefer: Prefer::Local,
        data_class: DataClass::Confidential,
        params: serde_json::json!({ "voice": "zh-female-1" }),
        ..Default::default()
    };
    if let Some(r) = MeetingBrief::run(&engine, tts, "TTS (brief)") {
        println!("audio brief: {}", r.file.as_deref().unwrap_or("(no file)"));
    }
}

/// Namespace for this demo's helpers.
struct MeetingBrief;

impl MeetingBrief {
    /// Invoke one stage, printing routing/cost on success or the structured error
    /// (including any failover chain) on failure. Returns `None` to stop the pipeline.
    fn run(
        engine: &EmbeddedEngine,
        req: InferenceRequest,
        stage: &str,
    ) -> Option<InferenceResponse> {
        match engine.invoke(req) {
            Ok(resp) => {
                println!(
                    "[{stage}] served by {} in {}ms",
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

    /// Truncate long text for a readable preview.
    fn preview(s: &str) -> String {
        let s = s.trim();
        if s.chars().count() > 200 {
            format!("{}…", s.chars().take(200).collect::<String>())
        } else {
            s.to_string()
        }
    }
}
