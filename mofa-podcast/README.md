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

The engine's `local_tts` backend shells out to a TTS command. Rather than making
you install one, on macOS this app auto-provisions a tiny wrapper around the
built-in `say` command (transcoded to mp3 via `lame`/`ffmpeg`) and points a
generated config at it. Both the wrapper and config live under
`~/Library/Caches/mofa-podcast/`. Pass `--config` to use your own engine config
(e.g. a real Crane/Kokoro voice) instead.

On non-macOS, configure a `local_tts` backend (see `config.example.toml`) or pass
`--config`; chat still runs, but TTS needs a backend.

## Framework findings

Building this app surfaced two engine gaps (tracked in `SESSION.md`): the engine
ships no zero-config local TTS, and `local_tts` doesn't pass `voice`/`speed`/
`format` params through to the command. Both are worked around here and are engine
TODOs — this app exists partly to drive those improvements.
