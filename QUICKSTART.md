# MoFA Engine: 5-Minute Quickstart Guide

Get up and running with **MoFA Engine** in under 5 minutes. No complex configuration, no mandatory cloud API keys.

---

## 30-Second Instant Showcase (Zero-Install Demo)

To run the complete automated scenario test suite and generate all real artifacts (`.mp4` video, `.mp3` podcasts, `.md` code review reports, `.json` data) in 5 seconds:

```bash
git clone https://github.com/mofa-org/mofa-engine.git
cd mofa-engine
bash quickstart.sh demo
```

Check your generated files:
```bash
ls -lh output/
```
You will find:
* `output/explainer_video.mp4` — Full AI-generated explainer video
* `output/podcast_episode.mp3` — Multi-voice conversational podcast
* `output/meeting_minutes.md` & `output/meeting_brief.mp3` — Executive meeting minutes + narrated voice brief
* `output/review_report.md` — AI code review report with deep thinking trace
* `output/extracted_receipt.json` — Multimodal VLM structured data extraction
* `output/subtitles.srt` & `output/subtitles.vtt` — Synchronized subtitle files

---

## Running the Full Stack (Local AI + Web Studio)

### Step 1: Start the Full Stack (Native or Docker)

**Option A — One-Command Shell Script (Recommended):**
```bash
bash quickstart.sh
```

**Option B — Docker Compose:**
```bash
docker compose up -d
```

This automatically boots:
- **MoFA Engine Gateway:** `http://127.0.0.1:8420`
- **Web Studio UI:** `http://localhost:3000`
- **Prometheus Metrics:** `http://127.0.0.1:8420/metrics`
- **Ollama Model Service:** `http://127.0.0.1:11434`

### Step 2: Run Automated Benchmarks & Integration Tests
```bash
bash quickstart.sh benchmark # Latency, TTFT, and Warmup speedup benchmark
bash quickstart.sh test # End-to-end scenario integration test suite
```

---

## Running Individual Scenarios

Every scenario takes natural language / real input and outputs a production-ready artifact:

### 1. Flagship Explainer Video (Scenario S4)
Generates script, scene images via Stable Diffusion, Kokoro voice narration, and renders `.mp4` video with FFmpeg:
```bash
python3 examples/explainer_video.py --topic "How Neural Networks Learn"
```
*Artifact:* `output/explainer_video.mp4`

### 2. AI Code Review with Deep Thinking (Scenario S2)
Reviews git diffs using high-effort reasoning streams and emits structured markdown reports:
```bash
# Review a local patch or branch diff
git diff HEAD~1 | python3 examples/code_review.py

# Or run with the bundled security vulnerability sample:
python3 examples/code_review.py --diff-file examples/samples/sample_diff.patch
```
*Artifact:* `output/review_report.md`

### 3. Meeting Recording → Minutes & Audio Brief (Scenario S1)
Transcribes audio recordings with speaker diarization and extracts structured action items + 30s audio brief:
```bash
python3 examples/meeting_brief.py --audio examples/samples/sample_meeting.wav --narrate
```
*Artifacts:* `output/meeting_minutes.md` and `output/meeting_brief.mp3`

### 4. Document AI & Receipt Extraction (Scenario S3)
Extracts structured JSON from photos and scans using Vision Language Models ($0.00 local inference):
```bash
python3 examples/doc_ai.py --images examples/samples/sample_receipt.png
```
*Artifact:* `output/extracted_receipt.json`

### 5. Article to Multilingual Podcast Matrix (Scenario S6)
Rewrites technical articles into natural conversational dialogue and synthesizes multi-voice audio:
```bash
python3 mofa-fm/article_to_podcast.py --article examples/samples/sample_article.txt
```
*Artifact:* `output/podcast_episode.mp3`

### 6. Provider Race Benchmark (Scenario S7)
Benchmarks identical queries concurrently across local GPU models and cloud providers:
```bash
python3 examples/01_provider_race.py --prompt "Explain quantum computing in 3 sentences."
```
*Artifact:* `output/provider_comparison.md`

---

## Dual-Track Observability

MoFA Engine includes real-time telemetry tracking cost, token usage, memory residency, and latency:

1. Open `http://localhost:3000` in your browser.
2. Navigate to **Dual-Track View** or **Data Flow Audit**.
3. Observe how local GPU requests report **$0.00 cost** while cloud requests record token quotas and estimated USD costs side-by-side.

---

## Self-Healing Diagnostics (`mofa doctor`)

Whenever you encounter issues or want to check environment readiness:
```bash
# Run the self-healing doctor
bash quickstart.sh doctor
# Or directly via Python
python3 mofa-fm/mofa_doctor.py
```

---

## Configuration & Custom Providers

For complete details on writing `config.toml`, defining custom OpenAI/DeepSeek models, setting memory budgets, or wiring custom CLI process adapters, check out the **[Configuration & Provider Guide](docs/configuration_guide.md)**.
