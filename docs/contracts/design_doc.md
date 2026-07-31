# MoFA Engine: Contract Hard Gate Architecture Design Document

**Document Version:** v1.1.0-EMPIRICALLY-VERIFIED  
**Date:** 2026-07-21  
**Status:** Empirically Verified against `origin/engine` (Aayan's upstream branch)  
**Authors:** Ashutosh (Lead Owner: Observability, SDK, UI, Demos) & Aayan (Engine Core Co-Developer)  
**Target Repository:** `mofa-org/mofa-engine`  
**Default Engine Address:** `127.0.0.1:8420` (Per PRD §6.5)

---

## 1. Purpose & Empirical Cross-Verification Methodology

The **Contract Hard Gate** phase freezes all inter-crate API specs, schema definitions, and metric contracts before parallel feature development proceeds.

To guarantee **ZERO ASSUMPTIONS**, every contract schema in `docs/contracts/` has been empirically cross-verified line-by-line against Aayan's upstream `origin/engine` branch (`commit 5287b436cd3d49beb03bbf4f516ad1a3ec94fab0`).

---

## 2. Verified Deliverable Manifest

The `docs/contracts/` directory contains 5 verified deliverables:

1. **`docs/contracts/openapi_v3.yaml`**: Complete OpenAPI v3.0.3 REST & Streaming API specification covering `/v1/invoke`, `/v1/invoke/stream`, `/v1/models/load`, `/v1/models/unload`, `/v1/subscriptions`, `/v1/status`, `/v1/events`, `/metrics`.
2. **`docs/contracts/metrics_spec.md`**: Metric Naming & Tagging Taxonomy defining 18 metric families with mandatory `provider` and `locality="local" | "cloud"` labels.
3. **`docs/contracts/engine_event_schema.json`**: JSON Schema defining all 10 verified `EngineEvent` SSE stream variants (`model_status_changed`, `model_residency_changed`, `request_started`, `request_completed`, `memory_changed`, `model_evicted`, `preflight_warm_started`, `preflight_warm_completed`, `provider_health_changed`, `discovery_completed`).
4. **`docs/contracts/inference_request_schema.json`**: JSON Schema for `InferenceRequest` including `locality` (`auto`, `prefer_local`, `local_only`), `fallback_policy` (`capability_only`, `disabled`, `allow_named`), `app_id`, `session_id`, `hint_next`, and `params`.
5. **`docs/contracts/design_doc.md`**: This empirical contract governance design document.

---

## 3. Verified Code & Network Conventions

### 3.1 Network Port Conventions (PRD §6.5 & Config Aligned)
* **MoFA Engine Gateway Daemon**: `127.0.0.1:8420` (Default REST & SSE daemon port).
* **Local Kokoro / Crane TTS Bridge**: `127.0.0.1:8421` (OpenAI-compatible TTS server).
* **Local Ollama LLM Instance**: `127.0.0.1:11434`.
* **Prometheus Metrics Collector**: `http://localhost:9090`.
* **Grafana Observability Dashboard**: `http://localhost:3000` (or `3001`).
* **React Frontend Dev Server**: `http://localhost:5173` (or `3000`).

---

### 3.2 Verified Upstream Rust Data Structures (`origin/engine`)

#### StreamChunk discriminators (`types.rs`):
```rust
pub enum StreamChunk {
    Started { request_id: String, model_used: String, provider: String },
    Text { delta: String },
    Completed { duration_ms: u64, tokens_used: Option<u32>, prompt_tokens: Option<u32>, completion_tokens: Option<u32>, cost_usd: Option<f64>, file: Option<String>, fallback_used: bool, routing_reason: Option<String> },
    Error(ErrorInfo),
}
```

#### Locality Guardrails (`types.rs`):
```rust
pub enum Locality {
    Auto,
    PreferLocal,
    LocalOnly,
}
```

---

## 4. Mentor & Governance Verification Status

* [x] Verified against `origin/engine` commit `5287b436cd3d49beb03bbf4f516ad1a3ec94fab0`.
* [x] All 10 `EngineEvent` variants matched 1-to-1 with Rust `enum EngineEvent`.
* [x] All `StreamChunk` variants matched 1-to-1 with SSE payload stream.
* [x] All `InferenceRequest` fields verified against `mofa-kernel/src/types.rs`.
* [x] Zero assumptions remaining.

**Verification Status:** **100% EMPIRICALLY VERIFIED & LOCKED LOCALLY**
