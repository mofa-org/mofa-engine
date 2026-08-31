# MoFA Engine — Multimodal Model Catalog & Setup Matrix

This guide documents all supported local hardware engines and cloud AI providers in MoFA Engine, including configuration instructions, pricing tiers, and hardware requirements.

---

## 1. Supported Model Matrix

| Modality / Capability | Local Engine ($0.00) | Cloud Engine (Hybrid Acceleration) | Routing Alias |
|---|---|---|---|
| **Chat & Fast Reasoning** | Ollama `gemma3:4b` / `qwen2.5:7b` | Google `gemini-3.6-flash` / OpenAI `gpt-4o` | `capability = "chat"` |
| **Deep Thinking Reasoning** | Ollama `qwen2.5:7b` (`effort="high"`) | DeepSeek `deepseek-r1` / Google `gemini-2.5-pro` | `reasoning = { effort = "high" }` |
| **Text-to-Speech (TTS)** | Kokoro Neural TTS (`zh-female-1`, `en-narrator`) | Google `gemini-2.5-flash-preview-tts` | `capability = "tts"` |
| **Speech-to-Text (ASR)** | FunASR (`paraformer-zh-en`) / Whisper | OpenAI `whisper-1` | `capability = "asr"` |
| **Multimodal Vision (VLM)** | Ollama `llava:latest` | Google `gemini-3.6-flash` / OpenAI `gpt-4o` | `capability = "vlm"` |
| **Vector Embeddings** | Ollama `nomic-embed-text` | OpenAI `text-embedding-3-small` | `capability = "embedding"` |
| **Image Generation** | Stable Diffusion v1.5 | Fireworks / OpenAI `dall-e-3` | `capability = "image_gen"` |

---

## 2. Dual-Track Pricing Engine Table

MoFA Engine's `mofa-observability` pricing engine computes exact real-time costs down to **$0.00001** per invocation:

| Provider | Model | Input Cost / 1K Tokens | Output Cost / 1K Tokens | Typical S1 Cost |
|---|---|:---:|:---:|:---:|
| **Local Hardware** | All Ollama / Kokoro / FunASR | **$0.00000** | **$0.00000** | **$0.0000** |
| **Google Gemini** | `gemini-3.6-flash` | **$0.000075** | **$0.000300** | **$0.0003** |
| **Google Gemini** | `gemini-2.5-flash-preview-tts` | **$0.000100** | Audio Stream | **$0.0001** |
| **Google Gemini** | `gemini-2.5-pro` | **$0.001250** | **$0.005000** | **$0.0042** |
| **DeepSeek** | `deepseek-r1` | **$0.000550** | **$0.002190** | **$0.0018** |
| **OpenAI** | `gpt-4o` | **$0.002500** | **$0.010000** | **$0.0125** |

---

## 3. Quickstart Configuration Profiles

### Profile A: 100% Free On-Device Profile ($0.00)
Runs entirely on Apple Silicon (M-series) or x86 laptops with zero external network access:

```toml
[listen]
host = "127.0.0.1"
port = 8420

[memory]
budget_mb = 12288
idle_timeout_secs = 120

[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
models = ["gemma3:4b", "qwen2.5:7b", "llava:latest", "nomic-embed-text"]
cost_tier = "free"

[[providers]]
name = "kokoro"
kind = "open_ai_compatible"
base_url = "http://127.0.0.1:8421"
models = ["kokoro"]
cost_tier = "free"
```

### Profile B: Zero-Cost Hybrid Acceleration Profile (Local + Google Gemini)
Uses local hardware by default, bursting to Google Gemini Free-Tier for complex reasoning or studio-grade TTS:

```toml
[listen]
host = "127.0.0.1"
port = 8420

[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
models = ["gemma3:4b", "qwen2.5:7b", "llava:latest", "nomic-embed-text"]
cost_tier = "free"

[[providers]]
name = "gemini"
kind = "open_ai_compatible"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key = "${GEMINI_API_KEY}"
models = ["gemini-3.6-flash", "gemini-flash-latest"]
cost_tier = "standard"

[[providers]]
name = "gemini-tts"
kind = "local_tts"
command = "python3"
args = ["mofa-fm/gemini_tts.py", "--output", "${output}"]
models = ["gemini-2.5-flash-preview-tts"]
cost_tier = "standard"
```

---

## 4. Environment Variables Reference

| Variable | Description | Example |
|---|---|---|
| `GEMINI_API_KEY` | Google AI Studio API key for Gemini 2.5 Flash and Native TTS | `export GEMINI_API_KEY="AIzaSy..."` |
| `OPENAI_API_KEY` | OpenAI platform API key for GPT-4o and Whisper-1 | `export OPENAI_API_KEY="sk-proj-..."` |
| `DEEPSEEK_API_KEY` | DeepSeek API key for DeepSeek-R1 deep thinking | `export DEEPSEEK_API_KEY="sk-..."` |
| `MOFA_API_TOKEN` | Bearer token required for engine `/v1` gateway access | `export MOFA_API_TOKEN="my-secret-token"` |
