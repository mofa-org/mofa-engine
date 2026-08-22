# mofa-studio

The **out-of-the-box** creative studio: a single binary that boots the MoFA
engine and serves a browser UI where anyone types a prompt and gets a **real
image or video** back — no SDK, no CLI, no code.

This is the "usable, validated, users-directly-experience-it" app: the framework
wearing a face a non-developer can use, with **which provider served the request,
local vs cloud, and cost** shown for every generation — the practical value made
visible.

```bash
cargo run -p mofa-studio          # then open http://127.0.0.1:8787
```

## Offline vs. real generation

Out of the box, chat is local Ollama and there is no zero-config image/video
backend, so those buttons report honestly that no backend is configured. Set a
key and the **same UI** renders real media:

```bash
# Image + chat via the Agnes AI free omni-modal gateway (OpenAI-compatible):
AGNES_API_KEY=sk-... \
AGNES_BASE_URL=https://apihub.agnes-ai.com/v1 \
  cargo run -p mofa-studio

# The same AGNES_API_KEY also enables video (Agnes /videos task API) — nothing
# extra to set. For Seedance via Volcengine Ark instead, use ARK_API_KEY.
```

Chat + image go through one OpenAI-compatible provider; video is a separate
task-based endpoint wired via the `cloud_video_gen` backend (`dialect = "agnes"`).
Model ids are overridable: `AGNES_CHAT_MODEL`, `AGNES_IMAGE_MODEL`,
`AGNES_VIDEO_MODEL`.

## How it works

- `GET /` — the embedded single-page UI (no build step, no external assets).
- `GET /api/capabilities` — what the engine can serve right now (drives the chips).
- `POST /api/generate` `{ prompt, mode: "image" | "video" }` — runs the request
  through the ordinary engine contract (`Capability::ImageGen` / `VideoGen`) and
  returns the artifact URL plus `provider · local/cloud · cost · duration`.
- `/artifacts/*` — the produced files, served from the engine's artifact dir.

Because everything goes through the engine, the studio inherits routing, failover,
cost/observability, and the local-first `prefer` semantics for free.

## Status

Scaffold: image and video single-shot generation are wired end-to-end and
degrade cleanly offline. Planned next: streamed progress (SSE) for slow video
renders, and a one-prompt **clip** pipeline (script → scenes → narration →
composed video) reusing `mofa-explainer`.
