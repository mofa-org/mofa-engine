# Scenario S7: Multi-Vendor Unified Access / Provider Race

## 1. Scenario Overview
Scenario S7 is a foundational infrastructure scenario that demonstrates MoFA Engine's multi-vendor routing capabilities, cost tracking, and hard data flow constraints. 
As defined in the PRD v3.1 (§2.2), this is a Priority P1 (Must) infrastructure task. It is the foundation for multi-provider routing and latency/cost benchmarking that all scenarios build upon. 
It demonstrates the engine's ability to take the exact same prompt and intelligently route it across multiple providers, allowing users to compare latency, token usage, cost, and response quality.

## 2. Architecture
The following diagram illustrates how the MoFA router processes a single request across multiple providers based on explicit routing constraints (`prefer=local` vs `prefer=cloud`), while evaluating the seven-dimensional scoring system.

```mermaid
graph TD
    A[Single Prompt] --> B[MoFA Router]
    
    subgraph Seven-Dimensional Scoring
    direction TB
    S1[Residency x1000]
    S2[Health x800]
    S3[Capability x200]
    S4[Locality x100]
    S5[Cost x50]
    S6[Priority x25]
    S7[Capacity x20]
    end
    
    B -.-> |Default Routing| Seven-Dimensional Scoring
    
    B -->|prefer=local| C[Local Provider: Ollama]
    B -->|prefer=cloud| D[Cloud Provider: Fireworks AI]
    
    C -->|Loaded > Loading > Unloaded > Remote| E[Compare Results]
    D --> E
    
    E --> F[Latency]
    E --> G[Cost]
    E --> H[Quality]
```

## 3. Required Infrastructure

| Component | Role | Port | Configuration |
|-----------|------|------|---------------|
| MoFA Engine Core | API Gateway & Orchestrator | 8420 | `mofa_hybrid.toml` |
| Ollama | Local Provider (`qwen2.5:7b`, `gemma3:4b`) | 11434 | Free, strict privacy |
| Fireworks AI | Cloud Provider | - | Requires `FIREWORKS_API_KEY` |

## 4. Setup Instructions

To prepare the environment for the benchmark:

```bash
# 1. Start the core components
bash quickstart.sh

# 2. Pull the required local model
ollama pull qwen2.5:7b

# 3. Export API keys for cloud providers
export FIREWORKS_API_KEY=your_key_here
```

## 5. How to Run

The scenario is executed using the Python benchmark script [`examples/01_provider_race.py`](file:///Users/ashum9/mofa/mofa-engine/examples/01_provider_race.py).

### 5a. Provider comparison (needs both local and cloud)
Run the full benchmark to race the local model against the cloud provider:
```bash
python3 examples/01_provider_race.py --prompt "Explain quantum computing in 3 sentences."
```

### 5b. Local-only test
Test strict data privacy routing by forcing the local provider:
```bash
python3 examples/01_provider_race.py --prompt "Explain quantum computing" --prefer local
```

### 5c. Mock Mode
Run the benchmark using synthetic data (useful for offline testing or CI):
```bash
python3 examples/01_provider_race.py --mock
```

## 6. Expected Output

The script outputs an ANSI-colored matrix to the console and generates a side-by-side comparison markdown artifact at `output/provider_comparison.md`.

**Sample Table:**

| Provider / Model | Locality | Latency | TTFT | Velocity | Cost ($) |
|---|---|---|---|---|---|
| **Ollama (qwen2.5:7b)** | Local (Free) | 0.42s | 0.08s | 114.2 tok/s | $0.000000 |
| **Fireworks AI (deepseek-v4)** | Cloud | 0.35s | 0.22s | 148.5 tok/s | $0.000215 |

## 7. Technical Detail

- The benchmark utilizes the `engine.chat()` method from the [MoFA SDK](file:///Users/ashum9/mofa/mofa-engine/mofa-fm/mofa_sdk.py), injecting `prefer="local"` and `prefer="cloud"` separately for the two competing runs.
- **Metrics Collected:** Total latency, Time To First Token (TTFT), token usage, velocity (tokens/sec), and estimated cost.
- **Routing Intelligence:** This scenario proves the gateway's ability to abstract backend differences.
- **Scoring Engine:** If `prefer` is omitted, MoFA evaluates providers using a 7-dimensional score: residency×1000, health×800, capability×200, locality×100, cost×50, priority×25, capacity×20.

## 8. PRD Acceptance Criteria

| Criterion | Test | Expected |
|-----------|------|----------|
| Multi-vendor config | Check [`mofa_hybrid.toml`](file:///Users/ashum9/mofa/mofa-engine/mofa_hybrid.toml) | ≥3 vendors configurable |
| `prefer=local` | Run with `--prefer local` | Routes **only** to local providers |
| `prefer=cloud` | Run with `--prefer cloud` | Routes **only** to cloud providers |
| Scoring fallback | Run without `--prefer` | Follows 7-dimensional scoring |
| Observability | Check script output metrics | Latency/cost observable per provider |

## 9. Troubleshooting

> [!WARNING]
> **Ollama First Run is Slow:** The first request sent to Ollama may take 10-20 seconds as it loads the model weights into VRAM. Subsequent requests will hit the warm cache.

- **Only local works:** You are likely missing API keys for the cloud providers.
- **Fireworks 401 Unauthorized:** Check that your `FIREWORKS_API_KEY` environment variable is exported correctly.

## 10. Provider Configuration

Providers are managed declaratively in [`mofa_hybrid.toml`](file:///Users/ashum9/mofa/mofa-engine/mofa_hybrid.toml). To add a new provider (e.g., Fireworks AI), use the `openai_compatible` kind:

```toml
[[providers]]
name = "fireworks"
kind = "openai_compatible"
base_url = "https://api.fireworks.ai/inference/v1"
api_key_env = "FIREWORKS_API_KEY"

[[providers.models]]
name = "accounts/fireworks/models/deepseek-v4-flash"
capability = "chat"
```
