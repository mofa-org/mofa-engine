# MoFA Engine — scenario examples

Runnable demos that exercise the engine end-to-end through the native SDK
(`EmbeddedEngine`) and the async `Engine`. Each maps to a PRD product scenario and
its acceptance anchor ("replicate the demo = scenario accepted"). All degrade
gracefully: with no capable backend they print the structured error (including any
`failed_chain`) instead of panicking.

| Example | Scenario | Engine capabilities exercised |
|---|---|---|
| `explainer_video.rs` | **S4** Explainer Video (flagship) | `Chat`→`ImageGen`→`Tts`→`Asr` orchestration + **hard quality gate** (ffprobe duration + slideshow-risk + VLM seam) — "no gate, no output" |
| `code_review.rs` | **S2** Code/PR Review | `reasoning.effort` (tier routing) + **streamed thought chain** (`Reasoning` vs `Text` chunks) + `max_cost_usd` budget ceiling + token/cost |
| `doc_ai.rs` | **S3** Document/Screenshot AI | `Vlm` understanding, multimodal `Message.images`, `detail` billing tier |
| `meeting_brief.rs` | **S1** Meeting → Minutes + Brief | local-first `Asr → Chat → Tts` pipeline, `prefer=local` + `data_class=confidential`, `hint_next` warmup |

## Running

```bash
# S4 — flagship. Orchestration degrades gracefully; pass a composed mp4 to run
# the real quality gate (needs ffmpeg/ffprobe on PATH).
cargo run -p mofa-engine-sdk --example explainer_video -- ./final.mp4

# S2 — needs a chat/reasoning backend (Ollama or a cloud key)
cargo run -p mofa-engine-sdk --example code_review

# S3 — needs a VLM-capable backend (cloud via liter-llm); pass an image
cargo run -p mofa-engine-sdk --example doc_ai -- ./receipt.jpg

# S1 — needs local ASR + a chat model + TTS; pass a recording
cargo run -p mofa-engine-sdk --example meeting_brief -- ./meeting.wav
```

Configuration is loaded from `./config.toml` (see `config.example.toml`) or falls
back to environment auto-detection (`OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, …) and a
local Ollama at `127.0.0.1:11434`.
