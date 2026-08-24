# MoFA Engine — scenario examples

Runnable demos that exercise the engine end-to-end through the native SDK
(`EmbeddedEngine`) and the async `Engine`. Each maps to a PRD product scenario and
its acceptance anchor ("replicate the demo = scenario accepted"). All degrade
gracefully: with no capable backend they print the structured error (including any
`failed_chain`) instead of panicking.

Start with `quickstart` — one command, one answer, and the routing/cost line that
is the whole pitch. The rest map to PRD product scenarios.

| Example | Scenario | Engine capabilities exercised |
|---|---|---|
| `quickstart.rs` | — (5-minute entry) | boot + discover backends + one `Chat` call, printing **routed provider · local/cloud · latency · cost** |
| `explainer_video.rs` | **S4** Explainer Video (flagship) | `Chat`→`ImageGen`→`Tts`→`Asr` orchestration + **hard quality gate** (ffprobe duration + slideshow-risk + VLM seam) — "no gate, no output" |
| `video_gen.rs` | **S4** Text-to-Video | `VideoGen` at the API level via **Seedance** (Volcengine Ark task API: submit→poll→download) — one contract call returns a managed `mp4`; degrades to a typed error offline |
| `code_review.rs` | **S2** Code/PR Review | `reasoning.effort` (tier routing) + **streamed thought chain** (`Reasoning` vs `Text` chunks) + `max_cost_usd` budget ceiling + token/cost |
| `doc_ai.rs` | **S3** Document/Screenshot AI | `Vlm` understanding, multimodal `Message.images`, `detail` billing tier |
| `meeting_brief.rs` | **S1** Meeting → Minutes + Brief | local-first `Asr → Chat → Tts` pipeline, `prefer=local` + `data_class=confidential`, `hint_next` warmup |
| `podcast.rs` | **S6** Podcast / Long-Audio | `Chat` rewrite → `Tts` narration, `hint_next` cross-capability warmup — a playable episode, offline |
| `provider_race.rs` | **S7** Multi-Vendor Access | same question across **every** discovered chat model, side-by-side **dual-track cost** (local `$0` vs metered cloud) + fault tolerance |
| `streaming_chat.rs` | — (true streaming) | real per-token `invoke_stream`, **time-to-first-token** the number pseudo-streaming can't move |
| `private_local.rs` | **S5** Privacy moat | `prefer=local` + `data_class=confidential` — **fail, don't leak**: pins on-device or errors, never silently cloud |

## Running

```bash
# Start here — offline, just needs a local Ollama with any chat model pulled.
cargo run -p mofa-engine-sdk --example quickstart

# S4 — flagship. Orchestration degrades gracefully; pass a composed mp4 to run
# the real quality gate (needs ffmpeg/ffprobe on PATH).
cargo run -p mofa-engine-sdk --example explainer_video -- ./final.mp4

# S2 — needs a chat/reasoning backend (Ollama or a cloud key)
cargo run -p mofa-engine-sdk --example code_review

# S3 — needs a VLM-capable backend (cloud via liter-llm); pass an image
cargo run -p mofa-engine-sdk --example doc_ai -- ./receipt.jpg

# S1 — needs local ASR + a chat model + TTS; pass a recording
cargo run -p mofa-engine-sdk --example meeting_brief -- ./meeting.wav

# S6 — article → playable episode; needs a chat model + a TTS backend (both local).
cargo run -p mofa-engine-sdk --example podcast -- ./article.txt

# S7 — race the same question across every discovered chat model (local + cloud).
cargo run -p mofa-engine-sdk --example provider_race

# True per-token streaming, with time-to-first-token. Needs a chat backend.
cargo run -p mofa-engine-sdk --example streaming_chat

# Privacy moat — pins confidential text on-device (fail, don't leak).
cargo run -p mofa-engine-sdk --example private_local
```

Configuration is loaded from `./config.toml` (see `config.example.toml`) or falls
back to environment auto-detection (`OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, …) and a
local Ollama at `127.0.0.1:11434`.
