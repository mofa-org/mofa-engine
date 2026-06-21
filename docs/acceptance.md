# MoFA Engine Acceptance Tracking

This file converts the RFC acceptance checklist into implementation evidence.

## Stage 0-2 Baseline

- [x] Rust CI gates exist for formatting, Clippy, and tests.
- [x] Deterministic mock-backed core tests do not require Ollama or cloud keys.
- [x] Provider configuration is validated before production startup.
- [x] Provider health is tracked separately from circuit breaker state.
- [x] Manual discovery refresh is available through the engine and HTTP API.
- [x] Stale discovered models are removed during refresh.
- [x] Canonical model IDs use `provider/model`.
- [x] Ambiguous short model names are rejected.
- [x] Unsupported provider operations return typed errors.

## MVP Acceptance Still Pending

- [ ] Auto-discover local MLX models.
- [ ] Local Qwen via Ollama handles type-level LLM calls.
- [ ] Named GPT request routes to cloud exactly.
- [ ] Local TTS returns a managed audio artifact.
- [ ] Hint Preflight starts TTS loading before LLM completion.
- [ ] Idle models unload and release accounted memory.
- [ ] Local TTS failure follows documented fallback policy.
- [ ] HTTP, Rust, and Python interfaces complete equivalent flows.
- [ ] mofa-fm completes English article to Chinese podcast end to end.

