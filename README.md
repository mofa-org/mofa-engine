# MoFA Engine

**Multimodal Orchestration for Artifacts**

[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Quickstart](https://img.shields.io/badge/Quickstart-5_Minutes-brightgreen.svg)](QUICKSTART.md)
[![Why MoFA](https://img.shields.io/badge/Why_MoFA-Differentiators-blueviolet.svg)](docs/why-mofa.md)

Your apps need AI — LLM, TTS, ASR, image generation, vision understanding — but you don't want to wire up six different SDKs, manage memory residency, or worry about cloud API bills. MoFA Engine is a high-performance, local-first inference orchestration gateway that turns prompts into production-ready artifacts.

---

## 30-Second Instant Demo (Zero Config)

Run all 7 scenarios in standalone mode and generate real artifacts (`.mp4` video, `.mp3` podcasts, `.md` reports, `.json` data) in 5 seconds:

```bash
git clone https://github.com/mofa-org/mofa-engine.git
cd mofa-engine
bash quickstart.sh demo
```

Check your generated files in `output/`:
```bash
ls -lh output/
```

---

## 5-Minute Full Stack Quickstart

### Native One-Command Launch:
```bash
bash quickstart.sh
```

### Docker Compose Launch:
```bash
docker compose up -d
```

- **Web Studio UI:** `http://localhost:3000` (Interactive S1–S7 Scenarios & Artifact Studio)
- **Dual-Track Observability UI:** `http://localhost:3000` (Click **"Observability"** in top header)
- **Grafana Production Dashboards:** `http://localhost:3001` (login: `admin` / `admin`)
- **Prometheus Metrics Console:** `http://localhost:9091` / `http://127.0.0.1:8420/metrics`
- **Engine API Gateway:** `http://127.0.0.1:8420`

 **Read the complete [5-Minute Quickstart Guide](QUICKSTART.md)** for detailed instructions. 
 **Read the [Configuration & Provider Guide](docs/configuration_guide.md)** to learn all TOML settings, provider types, and custom models. 
 **Read [Why MoFA?](docs/why-mofa.md)** to understand our architecture versus LangChain, CrewAI, AutoGen, and LiteLLM.

---

## End-to-End Scenarios & Deliverable Artifacts

Every scenario takes natural language input and outputs a verified deliverable file:

| Scenario | Guide | Command | Output Artifact | Locality / Cost |
|---|---|---|---|---|
| **S4 Explainer Video** | [S4 Guide](docs/scenario_guides/S4_explainer_video.md) | `python3 examples/explainer_video.py "AI Revolution"` | `output/explainer_video.mp4` | Local SD + Kokoro ($0.00) |
| **S6 Podcast Matrix** | [S6 Guide](docs/scenario_guides/S6_podcast_matrix.md) | `python3 mofa-fm/article_to_podcast.py` | `output/podcast_episode.mp3` | Local Ollama + Kokoro ($0.00) |
| **S7 Provider Race** | [S7 Guide](docs/scenario_guides/S7_provider_race.md) | `python3 examples/01_provider_race.py` | `output/provider_comparison.md` | Benchmark Matrix ($0.00) |
| **S1 Meeting Brief** | [S1 Guide](docs/scenario_guides/S1_meeting_brief.md) | `python3 examples/meeting_brief.py` | `output/meeting_minutes.md` + `.wav` | Local FunASR + Ollama ($0.00) |
| **S2 AI Code Review** | [S2 Guide](docs/scenario_guides/S2_code_review.md) | `git diff \| python3 examples/code_review.py` | `output/review_report.md` | Local Distilled R1 ($0.00) |
| **S3 Document AI** | [S3 Guide](docs/scenario_guides/S3_document_ai.md) | `python3 examples/doc_ai.py` | `output/extracted_receipt.json` | Local Qwen-VL ($0.00) |
| **S5 Privacy Moat** | [S5 Guide](docs/scenario_guides/S5_privacy_moat.md) | `python3 examples/meeting_brief.py --prefer local` | `output/meeting_minutes.md` | Air-Gapped Local ($0.00) |

---

## Architecture

```
Your App / Web Studio ── MoFA Engine (Port 8420)
 ├── Ollama (Local LLM / VLM: Free)
 ├── Kokoro TTS (Local Voice: Free)
 ├── FunASR / Whisper (Local Speech: Free)
 ├── PyTorch MPS (Stable Diffusion v1.5: Free)
 └── OpenAI / DeepSeek / Fireworks (Cloud Fallback)
```

- **7D Scoring Router:** Evaluates availability, locality, cost, priority, context window, and health.
- **Predictive Preflight Warmup:** Emits `hint_next` to warm downstream models (e.g. warming TTS during chat streaming) to eliminate cold starts.
- **Hard Privacy Moat (`prefer="local"`):** Guarantees zero cloud data egress for sensitive enterprise data.
- **Dual-Track Observability:** Real-time side-by-side telemetry contrasting local GPU resource consumption with cloud token costs.

---

## Python SDK

```python
from mofa_sdk import MofaEngine

engine = MofaEngine(base_url="http://127.0.0.1:8420")

# 1. Chat with predictive preflight warmup
r = engine.chat("Explain quantum computing in two sentences", hint_next="tts", prefer="local")
print(r.text)

# 2. Voice synthesis
speech = engine.tts(r.text, voice="af_heart")
print(speech.file) # -> /var/folders/.../mofa_tts_xxx.mp3

# 3. Vision understanding
doc = engine.understand(images=["receipt.png"], question="Extract invoice total and merchant name")
print(doc.text)
```

---

## Tests & Quality Gate

```bash
cargo test --release # 236 Rust unit and integration tests (0 failures)
cargo clippy --all-targets -- -D warnings # 0 Clippy warnings
cd mofa-frontend && npx eslint src/ # 0 frontend lint errors
npx vite build # Production bundle in ~230ms
bash quickstart.sh demo # All 7 end-to-end scenario verification
```

---

## License

MIT License — see [LICENSE](LICENSE) for details.
