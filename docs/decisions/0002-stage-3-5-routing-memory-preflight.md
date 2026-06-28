# ADR 0002: Stage 3-5 — Routing, Memory Lifecycle, and Preflight

## Status

Accepted for the Stage 3-5 implementation.

## Context

ADR 0001 froze the kernel contracts and registry. Stages 3-5 turn that baseline
into a system that selects models explainably, manages constrained memory
safely, and reduces cold-start latency without destabilising scheduling. The
prototype had a single selection path, no real concurrency control, no memory
reservation, no idle eviction, and a global (cross-application) preflight chain.

## Decisions

### Routing & failure handling (Stage 3)

- The router produces one **ranked candidate plan** (`route_ranked`). The engine
  walks it in order; the first entry is the primary, each subsequent one a
  failover. Primary and failover therefore never diverge.
- A **static memory-feasibility** filter removes local models whose estimate
  exceeds the whole budget; remote (cloud) models are exempt.
- Fallback advances **only on retryable errors**. `InvalidRequest` and
  `UnsupportedOperation` fail immediately. This satisfies "invalid input does
  not trigger fallback."
- **Named routing is strict by default** (only the named model is attempted).
  `fallback_policy = allow_named` appends capability candidates after the named
  model. Capability requests fail over across all candidates unless
  `fallback_policy = disabled`.
- **Concurrency admission** is a per-model `tokio::Semaphore` sized by the
  model's `max_concurrency`, acquired with a configurable queue timeout. Busy
  models are no longer hard-filtered from routing; they queue. A queue-wait
  timeout is retryable, so it can fail over to another candidate.
- The circuit breaker admits **exactly one half-open probe**; concurrent callers
  fail fast until the probe's outcome is recorded.
- All phase timeouts (overall request, queue, load, inference, discovery,
  health) are configurable under `[timeouts]`.

### Memory & lifecycle (Stage 4)

- Memory is **reserved atomically before loading**. The evict-then-reserve
  critical section runs under a `load_gate`; the slow backend load runs outside
  it so independent models can load in parallel without overcommitting.
- The manager tracks **reserved** (accounting) and **observed** (reported by the
  backend) bytes separately, reconciling reserved upward to observed.
- **Leases** protect in-flight models from eviction; eviction and the idle sweep
  both skip leased and subscription-protected models.
- A **supervised idle-timeout task** (holding a `Weak<Engine>`, aborted on drop)
  unloads models idle past `memory.idle_timeout_secs` using monotonic time.
- Failed/timed-out loads **roll back** the reservation. When nothing can be
  freed, admission returns a structured `MemoryPressure` error.
- Lifecycle history (bounded ring buffer) and current allocations are exposed via
  `/v1/lifecycle` and `/v1/memory`.

### Preflight (Stage 5)

- Signal priority follows the RFC feedback: **hint → subscription → history**.
- History is a **per-scope Markov chain** keyed by `session_id`, else `app_id`,
  else a shared global scope. A scope with too little data falls back to global;
  one application's ordering never leaks into another's predictions.
- Predictions are gated by `min_samples` and `confidence_threshold`.
- Warm tasks are **deduplicated per model** and **cancellable**, and route
  speculative loads through the same reservation/eviction admission as real
  requests, so a memory-unsafe prediction simply fails to load.
- Subscriptions are app/session-owned with a TTL or explicit removal; subscribed
  resident models are eviction-protected.
- Effectiveness counters (warms, predictions, hits, misses) are exposed via
  `/v1/preflight`.

## Consequences

- Subscription protection can, by design, make memory pressure unsatisfiable
  rather than break a keep-alive policy; the engine returns `MemoryPressure` in
  that case. Revisit if subscription/pressure conflict needs a softer policy
  (RFC open question 8).
- History and lifecycle state are in-memory only; persistence across restarts is
  deferred (RFC open question 11) behind clean seams.
- The local TTS backend (Stage 6) and Python UniFFI bindings (Stage 7) remain
  out of scope; cloud TTS and the HTTP API cover the demo path meanwhile.
