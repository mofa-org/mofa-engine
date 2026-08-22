# mofa-podcast

Turn an article into a narrated podcast episode — a real, playable `.mp3` —
driven end-to-end by the MoFA engine. Local-first, so on a laptop with Ollama it
runs fully offline at `$0`.

```
mofa-podcast <SOURCE> [--out episode.mp3] [--prefer local] [--keep-script]
```

`<SOURCE>` is an `http(s)://` URL, a local file, or `-` for stdin.

## 5-minute run (offline, macOS)

```bash
# One-time: a local chat model
ollama pull qwen2.5:0.5b        # or any chat model

# Article file (or a URL) → a playable episode. Nothing else to install:
# the app auto-provisions a `say`-based local voice.
cargo run -p mofa-podcast -- ./article.txt --out episode.mp3 --keep-script
open episode.mp3
```

The pipeline is two engine calls: `Chat` rewrites the article into a spoken-style
script (with `hint_next="tts"` warming the voice), then `Tts` synthesizes the
narration. Each stage prints where it routed and what it cost.

## How offline TTS works

Narration needs no setup: the engine auto-registers the OS-native voice (macOS
`say`, Linux `espeak`) as a built-in `system-tts` backend whenever a config
declares no TTS. This app therefore ships only a minimal Ollama config (under
`~/Library/Caches/mofa-podcast/`) and lets the engine supply the voice. `mp3` is
produced when `lame`/`ffmpeg` is present, otherwise the episode is written as
`.wav`. Pass `--config` to use your own engine config (e.g. a Crane/Kokoro voice)
and `--voice` to pick a system voice.

## Framework findings (fixed)

Building this app drove two engine fixes (see `SESSION.md`): a built-in zero-config
system voice, and `params.voice`/`format` passthrough to the voice backend. This
app now relies on those fixes rather than working around them — no wrapper script.
