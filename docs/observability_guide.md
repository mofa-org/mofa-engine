# MoFA Engine — Observability Guide

**Version:** 1.0 · **Last Updated:** 2026-08-06
**PRD Reference:** §5.1–§5.4 (Metric Taxonomy, Cost Metering, Dual-Track Dashboard, SSE Stream)

---

## 1. Overview

MoFA Engine ships a **dual-track observability subsystem** that records every inference request with two independent dimensions:

| Track | What It Measures | Key Signals |
|---|---|---|
| **Local** | On-device execution via Ollama, Kokoro TTS, FunASR | Memory (RAM/VRAM), cold start latency, model residency, evictions |
| **Cloud** | Remote API calls to OpenAI, DeepSeek, Anthropic, DashScope | USD cost, token consumption, HTTP 429 quota errors |

Every metric carries **mandatory `provider` and `locality` labels**, enabling side-by-side comparison of local hardware footprint vs cloud financial cost in a single Grafana panel.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────┐
│ mofa-engine-core  (Aayan's kernel)                      │
│   emit EngineEvent::RequestCompleted { ... }            │
│   emit EngineEvent::ModelLoaded { ... }                 │
│   emit EngineEvent::MemoryChanged { ... }               │
└──────────────────┬──────────────────────────────────────┘
                   │ broadcast channel
                   ▼
┌─────────────────────────────────────────────────────────┐
│ observability_bridge::run()  (mofa-engine-sdk)          │
│   Translates kernel events → ObsEngineEvent             │
│   Seeds memory/model gauges on startup                  │
└──────────────────┬──────────────────────────────────────┘
                   │ bounded mpsc channel (2048)
                   ▼
┌─────────────────────────────────────────────────────────┐
│ MetricsCollector::run()  (mofa-observability)           │
│   Maintains MetricsState: counters, histograms, gauges  │
│   Pricing engine: estimate_cost_usd()                   │
│   Label eviction (stale labels garbage-collected)       │
└──────────────────┬──────────────────────────────────────┘
                   │ Arc<RwLock<MetricsState>>
                   ▼
┌─────────────────────────────────────────────────────────┐
│ GET /metrics  (Prometheus text exposition)               │
│ GET /v1/events  (SSE real-time stream)                   │
│ GET /v1/cost  (JSON cost summary)                        │
│ GET /v1/status  (JSON model/memory state)                │
└─────────────────────────────────────────────────────────┘
```

---

## 3. Metric Taxonomy (18 Families)

All metrics use the `mofa_` namespace prefix. Full specification in [`docs/contracts/metrics_spec.md`](contracts/metrics_spec.md).

### 3.1 Financial & Token Metrics

| Metric | Type | Labels | Description |
|---|---|---|---|
| `mofa_estimated_cost_usd` | Counter | `provider`, `locality`, `model` | Accumulated USD cost. Local = $0.00 always. |
| `mofa_tokens_total` | Counter | `provider`, `locality`, `model`, `type` | Token count by `prompt` / `completion` direction. |
| `mofa_thought_tokens_total` | Counter | `provider`, `locality`, `model` | Deep reasoning tokens (DeepSeek R1, etc.). |

### 3.2 Local Execution & Memory

| Metric | Type | Labels | Description |
|---|---|---|---|
| `mofa_cold_start_seconds` | Histogram | `provider`, `locality`, `model` | Time from load trigger to first inference. |
| `mofa_warmup_hits_total` | Counter | `provider`, `locality`, `source` | Preflight warming hits by `hint` / `subscription` / `markov`. |
| `mofa_memory_usage_bytes` | Gauge | `provider`, `locality`, `type`, `status` | RAM/VRAM footprint (`reserved` vs `observed`). |
| `mofa_model_residency_status` | Gauge | `provider`, `locality`, `model` | `0`=unloaded, `1`=loading, `2`=loaded, `3`=remote. |
| `mofa_idle_evictions_total` | Counter | `provider`, `locality`, `model` | LRU evictions from VRAM under memory pressure. |

### 3.3 System Health & Throughput

| Metric | Type | Labels | Description |
|---|---|---|---|
| `mofa_requests_total` | Counter | `provider`, `locality`, `capability`, `status` | Cumulative requests by status code. |
| `mofa_request_duration_seconds` | Histogram | `provider`, `locality`, `capability` | End-to-end latency distribution. |
| `mofa_provider_health` | Gauge | `provider`, `locality` | `1` = healthy, `0` = unhealthy. |
| `mofa_circuit_breaker_state` | Gauge | `provider` | `0`=closed, `1`=open, `2`=half_open. |
| `mofa_active_connections` | Gauge | `protocol` | Active REST and SSE connections. |
| `mofa_preflight_predictions_total` | Counter | `scope_id`, `result` | Markov preflight prediction outcomes. |
| `mofa_cloud_quota_errors_total` | Counter | `provider` | HTTP 429 rate limit errors from cloud. |
| `mofa_sse_events_emitted_total` | Counter | `event_type` | SSE telemetry events emitted. |
| `mofa_label_evictions_total` | Counter | `collector` | High-cardinality label garbage collection. |
| `mofa_quality_gate_evaluations_total` | Counter | `pipeline`, `result` | VLM/ffprobe quality gate outcomes. |

---

## 4. Cost Engine & Pricing Matrix

The cost engine in [`mofa-observability/src/pricing.rs`](../mofa-observability/src/pricing.rs) computes per-request USD cost estimates.

**Rule:** All local providers always cost **$0.00**. Cloud providers are billed per 1,000 tokens.

| Provider | Model Pattern | Prompt ($/1K tokens) | Completion ($/1K tokens) |
|---|---|---|---|
| **ollama** (local) | any | $0.000 | $0.000 |
| **kokoro** (local) | any | $0.000 | $0.000 |
| **funasr** (local) | any | $0.000 | $0.000 |
| openai | gpt-4o | $0.0025 | $0.0100 |
| openai | gpt-4 | $0.0300 | $0.0600 |
| openai | gpt-3.5 | $0.0005 | $0.0015 |
| deepseek | deepseek-* | $0.00055 | $0.00219 |
| anthropic | claude-* | $0.0030 | $0.0150 |
| dashscope | qwen-* | $0.0028 | $0.0084 |
| *(any other cloud)* | *(fallback)* | $0.0020 | $0.0060 |

**Example cost calculation:**
```
GPT-4o request: 500 prompt tokens + 200 completion tokens
  Prompt:     500 / 1000 × $0.0025 = $0.00125
  Completion: 200 / 1000 × $0.0100 = $0.00200
  Total:                             $0.00325
```

---

## 5. Dual-Track Dashboard

The React dashboard at `mofa-frontend/` provides real-time observability across multiple views:

### 5.1 Dual-Track Comparison View (`DualTrackView.tsx`)
The core view. Shows side-by-side:
- **Left panel:** Local hardware metrics (memory usage vs budget, loaded models, cold start history)
- **Right panel:** Cloud financial metrics (accumulated $, token burn rate, quota errors)
- **Center gauge:** Memory budget utilization percentage

### 5.2 Cost & Token Dashboard (`CostTokenDashboard.tsx`)
- Per-provider token breakdown (prompt vs completion)
- Accumulated USD cost with **budget cap gauge** (alerts at 60% yellow, 85% red)
- Local savings calculator (how much $ saved by running locally)

### 5.3 Latency & Availability Dashboard (`LatencyAvailabilityDashboard.tsx`)
- Per-capability request duration histograms
- Provider success rate bars (local vs cloud side-by-side)
- Memory budget gauge with GB usage

### 5.4 Data Flow Audit (`DataFlowAudit.tsx`)
- Per-session locality compliance view
- Verifies that `prefer=local` + `data_class=confidential` requests never leaked to cloud
- Color-coded: green = compliant, red = violation

### 5.5 Routing Decision Log (`RoutingDecisionLog.tsx`)
- Chronological log of every routing decision
- Shows `routing_reason`, selected provider, whether failover was triggered

### 5.6 Model Efficiency Table (`ModelEfficiencyTable.tsx`)
- Per-model tokens/second, average latency, cost efficiency
- Sortable by any column

---

## 6. Prometheus & Grafana Integration

### 6.1 Scrape Configuration

Add MoFA Engine to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'mofa-engine'
    scrape_interval: 15s
    static_configs:
      - targets: ['127.0.0.1:8420']
    metrics_path: '/metrics'
```

### 6.2 Key PromQL Queries

**Request rate by locality (local vs cloud):**
```promql
sum(rate(mofa_requests_total[5m])) by (locality)
```

**Token burn rate by provider:**
```promql
sum(rate(mofa_tokens_total[5m])) by (provider, type)
```

**Cloud cost accumulation:**
```promql
sum(mofa_estimated_cost_usd) by (provider)
```

**Cold start P95 latency:**
```promql
histogram_quantile(0.95, rate(mofa_cold_start_seconds_bucket[15m]))
```

**Memory utilization percentage:**
```promql
mofa_memory_usage_bytes{status="observed"} / mofa_memory_usage_bytes{status="reserved"} * 100
```

**Preflight warming hit rate:**
```promql
sum(mofa_warmup_hits_total) / (sum(mofa_warmup_hits_total) + sum(mofa_preflight_predictions_total{result="rejected"})) * 100
```

### 6.3 Suggested Alert Rules

```yaml
groups:
  - name: mofa-alerts
    rules:
      # Budget cap exceeded 85%
      - alert: MofaCostBudgetHigh
        expr: sum(mofa_estimated_cost_usd) > 0.85
        for: 1m
        labels:
          severity: warning
        annotations:
          summary: "MoFA cloud cost approaching budget cap"

      # Provider circuit breaker open
      - alert: MofaCircuitBreakerOpen
        expr: mofa_circuit_breaker_state == 1
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "Circuit breaker open for {{ $labels.provider }}"

      # High error rate (>10% in 5m window)
      - alert: MofaHighErrorRate
        expr: >
          sum(rate(mofa_requests_total{status=~"4..|5.."}[5m])) by (provider)
          / sum(rate(mofa_requests_total[5m])) by (provider) > 0.10
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "Error rate >10% for {{ $labels.provider }}"

      # Memory pressure (>90% of budget)
      - alert: MofaMemoryPressure
        expr: >
          mofa_memory_usage_bytes{status="observed"}
          / mofa_memory_usage_bytes{status="reserved"} > 0.90
        for: 1m
        labels:
          severity: warning
        annotations:
          summary: "VRAM usage exceeding 90% of budget"

      # Cold start spike (P95 > 10s)
      - alert: MofaColdStartSlow
        expr: histogram_quantile(0.95, rate(mofa_cold_start_seconds_bucket[15m])) > 10
        for: 5m
        labels:
          severity: info
        annotations:
          summary: "Cold start P95 exceeding 10 seconds"
```

---

## 7. SSE Event Stream

### 7.1 Connecting

```python
from mofa_sdk import MofaEngine

engine = MofaEngine()

# Basic event stream
for event in engine.events():
    print(event["type"], event)

# With disconnect recovery and filtering
for event in engine.events(
    last_event_id="evt-00042",
    event_filter=["inference_complete", "model_loaded"]
):
    print(event)
```

### 7.2 Raw HTTP

```bash
# Basic stream
curl -N http://127.0.0.1:8420/v1/events

# With disconnect resume
curl -N -H "Last-Event-ID: evt-00042" http://127.0.0.1:8420/v1/events

# With event type filtering
curl -N "http://127.0.0.1:8420/v1/events?filter=inference_complete,model_loaded"
```

### 7.3 Event Types

| Event Type | When Emitted | Key Fields |
|---|---|---|
| `request_received` | New inference request accepted | `capability`, `model`, `hint` |
| `routing_decision` | Router selects a model | `selected_model`, `selected_backend`, `reason`, `is_fallback` |
| `request_completed` | Inference finishes | `model_id`, `backend`, `duration_ms`, `success`, `tokens_in/out` |
| `model_loaded` | Model finishes loading into memory | `model_id`, `backend`, `load_duration_ms`, `memory_bytes` |
| `model_unloaded` | Model evicted or idle-timed out | `model_id`, `reason`, `memory_freed_bytes` |
| `memory_changed` | Memory allocation updated | `used_bytes`, `total_bytes` |
| `eviction_triggered` | Memory pressure forced eviction | `evicted_model`, `memory_before/after_bytes` |
| `failover_triggered` | Primary provider failed, fallback used | `failed_model`, `fallback_model` |
| `preflight_signal` | Markov predictor fires | `predicted_capability`, `confidence`, `source` |
| `preflight_hit` | Prediction was correct | `predicted_capability`, `cold_start_avoided_ms` |
| `preflight_miss` | Prediction was wrong | `predicted_capability`, `actual_capability` |

---

## 8. API Endpoints Reference

| Endpoint | Method | Description |
|---|---|---|
| `/metrics` | GET | Prometheus text exposition (engine-core + observability collector) |
| `/v1/status` | GET | JSON: loaded models, memory usage/budget, provider health |
| `/v1/cost` | GET | JSON: per-provider token counts and accumulated USD cost |
| `/v1/events` | GET | SSE stream of real-time engine events |
| `/v1/invoke` | POST | Unified inference endpoint (all capabilities) |

---

## 9. Troubleshooting

### Metrics show $0 for cloud requests
The cost engine requires real `tokens_in` / `tokens_out` values on `EngineEvent::RequestCompleted`. If these are `None`, the collector uses fallback estimates (180 prompt / 320 completion tokens). Ensure the kernel is populating token counts from provider responses.

### Memory gauge not updating
The observability bridge seeds memory gauges on startup from the engine's `status()` response. If the engine starts before Ollama, the initial seed may be zero. The gauge self-corrects on the next `MemoryChanged` event.

### High label cardinality warning
The collector runs periodic label eviction (configurable, default: labels unused for 10+ minutes are garbage-collected). If you see `mofa_label_evictions_total` increasing, it means dynamic model names are cycling. This is expected behavior.

### Dashboard shows "Engine Offline"
The React dashboard polls `/v1/status` every 2 seconds. If the engine is not running on `127.0.0.1:8420`, the `EngineOffline.tsx` error view appears. Start the engine with:
```bash
./target/release/mofa-engine --config mofa_hybrid.toml
```
