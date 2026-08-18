# mofa-explainer

Turn a topic into a finished explainer video — a real, playable `.mp4` with
narration and subtitles — driven end-to-end by the MoFA engine. Local-first: on a
laptop with Ollama it runs fully offline at `$0`.

```
mofa-explainer "<TOPIC>" [--seconds 30] [--scenes 4] [--voice Samantha] [--out explainer.mp4]
```

## 5-minute run (offline, macOS)

```bash
ollama pull qwen2.5:0.5b            # any chat model
brew install ffmpeg                  # composition + probing (one hard dependency)

cargo run -p mofa-explainer -- "How neural networks learn" --seconds 20 --out explainer.mp4
open explainer.mp4
```

## Pipeline

`Chat` (scene script) → `ImageGen` (scene visuals) → `Tts` (narration) → **FFmpeg
compose** (slideshow + subtitles + audio) → engine **quality gate** ("no gate, no
output"). FFmpeg is reused for composition, not re-implemented (PRD scope). Each
stage prints where it routed and what it cost.

## Graceful degradation (offline)

- **Narration** always works offline via the engine's built-in `system-tts` voice
  (macOS `say` / Linux `espeak`) — no setup.
- **No image backend** (no local SD, no cloud key): scene imagery can't be
  AI-generated offline (that needs a model), so the app renders real per-scene
  **title cards** with FFmpeg instead of a blank frame — genuine, varying visuals.
- **No ASR**: subtitle *words* are exact (from the script); only their *timing* is
  approximate (length-proportional) rather than word-accurate `Asr` timestamps.

Configure a `local_image_gen`/cloud image backend (or `local_asr`) and the same
pipeline uses real scene imagery / word-accurate timing automatically. See
`SESSION.md` for why offline image gen and ASR need a model (not a zero-config
fix), unlike the built-in voice.

## Requirements

- FFmpeg (`ffmpeg` + `ffprobe`) on `PATH`.
- A chat backend (local Ollama or a cloud key).
- Narration needs no setup — the engine's built-in system voice is used. Title
  cards use a system font (macOS/Linux locations are auto-detected).
