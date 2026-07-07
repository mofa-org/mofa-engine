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

## Stage 3 — Routing, Concurrency, and Failure Handling

- [x] Routing applies hard constraints, then ranks candidates and returns a
      reason (`router::route_ranked`).
- [x] Static memory feasibility filter drops models larger than the budget.
- [x] Primary selection and failover walk the *same* ranked candidate plan.
- [x] Capability routing prefers local models under the default policy.
- [x] Named routing resolves exactly; ambiguous short names are rejected.
- [x] A retryable local failure falls back to the next valid candidate.
- [x] Invalid input and unsupported operations do **not** trigger fallback.
- [x] Named routing is strict by default; `allow_named` opts into fallback.
- [x] Per-model concurrency admission via semaphores with a queue timeout.
- [x] Ten concurrent requests respect the model's concurrency limit (no over-admission).
- [x] Circuit breaker admits exactly one half-open probe.
- [x] Overall, queue, load, inference, discovery, and health timeouts are configurable.

## Stage 4 — Memory Manager and Model Lifecycle

- [x] Atomic reservation before loading (no time-of-check/time-of-use overcommit).
- [x] Reserved vs observed memory tracked separately; reconciled after load.
- [x] Active leases prevent eviction of in-flight models.
- [x] LRU eviction with incoming/subscription protection.
- [x] Reservation rolled back on load failure or timeout.
- [x] Supervised idle-timeout task unloads stale models and releases memory.
- [x] Lifecycle history and current allocations exposed via `/v1/lifecycle`, `/v1/memory`.
- [x] Memory pressure returns a structured error when nothing can be freed.

## Stage 5 — Preflight v1

- [x] Explicit hints warm the next model concurrently with the current request.
- [x] Warm tasks are deduplicated and cancellable; speculative loads go through
      normal memory admission.
- [x] Capability subscriptions (app/session-owned, TTL or explicit removal) warm
      and protect their models.
- [x] History is keyed by app/session with a global fallback; one app's history
      does not pollute another's predictions.
- [x] Predictions are gated by minimum samples and a confidence threshold.
- [x] Memory-unsafe predictions do not load a model.
- [x] Preflight effectiveness is visible via events and `/v1/preflight`.
- [x] History learning and speculative warming can be disabled by config.

## MVP Acceptance Still Pending (needs live backends / later stages)

- [ ] Auto-discover local MLX models. *(Stage 6.)*
- [x] Engine routes type-level LLM calls to a local model. *(Logic + tests done;
      live demo needs a running Ollama with Qwen.)*
- [x] Named request routes to the cloud model exactly. *(Logic + tests done;
      live demo needs a configured cloud key.)*
- [ ] Local TTS returns a managed audio artifact. *(Cloud TTS works today; local
      MLX/Kokoro is Stage 6.)*
- [x] Hint Preflight starts the next model loading before the current completes.
- [x] Idle models unload and release accounted memory.
- [x] A retryable failure follows the documented fallback policy (engine logic);
      local-TTS-to-cloud demo needs the Stage 6 local backend.
- [ ] Python UniFFI interface completes equivalent flows. *(Stage 7; HTTP done.)*
- [ ] mofa-fm completes English article to Chinese podcast end to end. *(Needs
      live Ollama + TTS; engine-side flow is in place.)*

