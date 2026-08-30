# MoFA Engine — Configuration & Provider Guide

This guide provides a comprehensive reference for configuring the **MoFA Engine (`mofa-engine`)**. Whether you are deploying on a single local laptop, an on-premises GPU workstation, or a hybrid enterprise cloud cluster, this document explains every configuration key, data type, default value, and provider setup.

---

## Table of Contents

1. [Configuration File Resolution Order](#1-configuration-file-resolution-order)
2. [Top-Level Sections Reference](#2-top-level-sections-reference)
 - [`[listen]`](#listen)
 - [`[memory]`](#memory)
 - [`[timeouts]`](#timeouts)
 - [`[observability]`](#observability)
 - [`[artifacts]`](#artifacts)
 - [`[security]`](#security)
 - [`[preflight]`](#preflight)
3. [Provider Configuration (`[[providers]]`)](#3-provider-configuration-providers)
 - [Supported Provider Kinds](#supported-provider-kinds)
 - [Model Definitions (`[[providers.models]]`)](#model-definitions-providersmodels)
4. [Step-by-Step Provider Examples](#4-step-by-step-provider-examples)
 - [1. Ollama (Local LLM & VLM)](#1-ollama-local-llm--vlm)
 - [2. Kokoro (Local Neural Voice TTS)](#2-kokoro-local-neural-voice-tts)
 - [3. OpenAI & OpenAI-Compatible (GPT-4o, DeepSeek, Groq, Together)](#3-openai--openai-compatible)
 - [4. Google Gemini (Chat, TTS, Image)](#4-google-gemini)
 - [5. Local Process Adapters (Whisper ASR, Custom Image & Video)](#5-local-process-adapters)
5. [Complete Ready-to-Use Configuration Templates](#5-complete-ready-to-use-configuration-templates)
 - [Template A: 100% Local Offline (`config.local.toml`)](#template-a-100-local-offline)
 - [Template B: Hybrid Local + Cloud Burst (`mofa_hybrid.toml`)](#template-b-hybrid-local--cloud-burst)
6. [Self-Healing Diagnostics (`mofa doctor`)](#6-self-healing-diagnostics-mofa-doctor)

---

## 1. Configuration File Resolution Order

When MoFA Engine starts, it searches for configuration in the following order:

1. **CLI Flag (Highest Priority):** `mofa-engine --config /path/to/config.toml`
2. **Current Working Directory:** `./config.toml` or `./mofa_hybrid.toml`
3. **User Configuration Directory:** `~/.config/mofa-engine/config.toml`
4. **Environment Auto-Detection (Fallback):** Default fallback configuration if no file is present.

---

## 2. Top-Level Sections Reference

### `[listen]`
Controls how the engine binds its HTTP/REST and Server-Sent Events (SSE) API server.

```toml
[listen]
host = "127.0.0.1" # IP to bind ("127.0.0.1" for local-only, "0.0.0.0" for network access)
port = 8420 # TCP Port (Default: 8420)
```

| Field | Type | Default | Description |
|---|---|---|---|
| `host` | `string` | `"127.0.0.1"` | Loopback protects against unauthorized network access. Use `"0.0.0.0"` in Docker. |
| `port` | `integer` | `8420` | Port for the REST gateway, SSE stream, and `/metrics`. |

---

### `[memory]`
Controls RAM & VRAM budget allocations and automatic model lifecycle eviction.

```toml
[memory]
budget_mb = 12000 # Maximum RAM/VRAM budget in MB (Omit for auto-detect 70% system RAM)
idle_timeout_secs = 300 # Seconds of inactivity before unloading idle models from memory
```

| Field | Type | Default | Description |
|---|---|---|---|
| `budget_mb` | `integer` (optional) | `None` (70% system RAM) | Safety budget preventing OS-level out-of-memory kernel kills. |
| `idle_timeout_secs` | `integer` | `120` | Idle timeout before LRU memory eviction sweeps cold models. |

---

### `[timeouts]`
Bounds latency for every phase of inference to prevent runaway hung processes.

```toml
[timeouts]
request_secs = 60 # Total timeout per request across all fallback candidates
queue_secs = 10 # Maximum wait time in concurrency admission queue
load_secs = 30 # Maximum wait time when loading a cold model into VRAM
inference_secs = 45 # Maximum generation duration for a single candidate
discovery_secs = 5 # Timeout for provider startup probing
health_secs = 3 # Timeout for heartbeat health checks
```

---

### `[observability]`
Configures OpenTelemetry and Prometheus metric collection.

```toml
[observability]
enabled = true # Expose /metrics endpoint
otlp_endpoint = "http://localhost:4317" # Optional OTLP gRPC endpoint for Grafana/Jaeger
```

---

### `[artifacts]`
Controls where engine-generated media (audio recordings, speech WAVs, generated MP4 videos) are stored and how long they persist.

```toml
[artifacts]
dir = "output" # Storage directory (Default: system temp directory)
retention_secs = 3600 # Auto-delete files older than X seconds (0 disables auto-delete)
```

---

### `[security]`
Restricts local filesystem traversal when processing inputs from remote clients.

```toml
[security]
input_roots = ["/Users/username/data", "output"] # Paths allowed for local file processing
```

---

## 3. Provider Configuration (`[[providers]]`)

MoFA Engine connects to AI models through declared `[[providers]]` blocks.

### General Provider Fields

```toml
[[providers]]
name = "ollama" # Unique provider identifier
kind = "ollama" # Backend adapter kind (see table below)
base_url = "http://127.0.0.1:11434" # Endpoint URL
api_key = "local" # API key or "local" / env var
priority = 10 # Priority weight for 7D scoring (higher = preferred)
enabled = true # Enable/disable provider toggle
```

### Supported Provider Kinds

| Kind | Target Systems | Description |
|---|---|---|
| `ollama` | Ollama Local Engine | Local LLMs (Gemma, Llama, Qwen) and VLMs (LLaVA). Discovers tags automatically. |
| `openai_compatible` | OpenAI, DeepSeek, Groq, Together, Kokoro, vLLM | Standard OpenAI `/v1/chat/completions`, `/v1/audio/speech`, `/v1/embeddings`. |
| `local_tts` | Python CLI, Kokoro CLI, Coqui | Subprocess adapter executing CLI command to generate audio files. |
| `local_asr` | Whisper CLI, FunASR CLI | Subprocess adapter executing CLI command to transcribe audio files. |
| `local_image_gen` | Python Diffusion, SD CLI | Subprocess adapter executing CLI command to generate image files. |
| `local_video_gen` | FFmpeg, Video Assembly | Subprocess adapter executing CLI command to generate MP4 videos. |

---

### Model Definitions (`[[providers.models]]`)

Each provider lists its served models and their **capabilities**:

```toml
[[providers.models]]
name = "gemma3:4b" # Model name passed to provider
capability = "chat" # Capability served (see capabilities below)
context_length = 32768 # Context window in tokens
cost_tier = "free" # "free", "low", "medium", "high"
```

#### Valid Capabilities
- `chat` — Text generation & multi-turn reasoning
- `vlm` — Multimodal vision-language understanding (images + text)
- `tts` — Text-to-speech audio synthesis (generates `.wav` / `.mp3`)
- `asr` — Automated speech recognition (transcribes audio to text)
- `image_gen` — Text-to-image generation (generates `.png`)
- `video_gen` — Multimodal video composition (generates `.mp4`)
- `embedding` — Vector embeddings for semantic search & RAG

---

## 4. Step-by-Step Provider Examples

### 1. Ollama (Local LLM & VLM)
```toml
[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
priority = 10
enabled = true

[[providers.models]]
name = "gemma3:4b"
capability = "chat"

[[providers.models]]
name = "qwen2.5:7b"
capability = "chat"

[[providers.models]]
name = "llava:latest"
capability = "vlm"

[[providers.models]]
name = "nomic-embed-text:latest"
capability = "embedding"
```

---

### 2. Kokoro (Local Neural Voice TTS)
```toml
[[providers]]
name = "kokoro"
kind = "openai_compatible"
base_url = "http://127.0.0.1:8421/v1"
api_key = "local"
priority = 10
enabled = true

[[providers.models]]
name = "kokoro"
capability = "tts"
```

---

### 3. OpenAI & OpenAI-Compatible

#### OpenAI (GPT-4o & DALL-E 3)
```toml
[[providers]]
name = "openai"
kind = "openai_compatible"
base_url = "https://api.openai.com/v1"
api_key = "sk-proj-YOUR_OPENAI_KEY"
priority = 5
enabled = true

[[providers.models]]
name = "gpt-4o"
capability = "chat"

[[providers.models]]
name = "dall-e-3"
capability = "image_gen"

[[providers.models]]
name = "text-embedding-3-small"
capability = "embedding"
```

#### DeepSeek
```toml
[[providers]]
name = "deepseek"
kind = "openai_compatible"
base_url = "https://api.deepseek.com/v1"
api_key = "sk-YOUR_DEEPSEEK_KEY"
priority = 8
enabled = true

[[providers.models]]
name = "deepseek-chat"
capability = "chat"

[[providers.models]]
name = "deepseek-reasoner"
capability = "chat"
```

---

### 4. Google Gemini

```toml
[[providers]]
name = "gemini"
kind = "openai_compatible"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key = "AIzaSyYOUR_GEMINI_KEY"
priority = 7
enabled = true

[[providers.models]]
name = "gemini-3.6-flash"
capability = "chat"

[[providers.models]]
name = "gemini-flash-latest"
capability = "chat"
```

---

### 5. Local Process Adapters

Local CLI tools are integrated seamlessly using placeholder string interpolation:
- `{input}` / `{text_file}` / `{prompt}` → Input text or file
- `{output}` → Target destination path

#### Local Whisper Speech-to-Text (ASR)
```toml
[[providers]]
name = "funasr"
kind = "local_asr"
base_url = ""
command = "whisper"
args = ["{input}", "--model", "tiny", "--language", "English", "--output_format", "txt", "--output_dir", "output"]
output_format = "txt"
priority = 10
enabled = true

[[providers.models]]
name = "whisper-tiny"
capability = "asr"
```

#### Local Image Generator (Python Diffusion Adapter)
```toml
[[providers]]
name = "gemini-image"
kind = "local_image_gen"
base_url = ""
command = "python3"
args = ["mofa-fm/gemini_image.py", "--prompt", "{prompt}", "--output", "{output}"]
output_format = "png"
priority = 10
enabled = true

[[providers.models]]
name = "neural-diffusion-card"
capability = "image_gen"
```

---

## 5. Complete Ready-to-Use Configuration Templates

### Template A: 100% Local Offline (`config.local.toml`)
*Guarantees complete air-gapped isolation with $0.00 cloud cost.*

```toml
[listen]
host = "127.0.0.1"
port = 8420

[memory]
idle_timeout_secs = 300

[observability]
enabled = true

[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"

[[providers.models]]
name = "gemma3:4b"
capability = "chat"

[[providers.models]]
name = "qwen2.5:7b"
capability = "chat"

[[providers.models]]
name = "llava:latest"
capability = "vlm"

[[providers.models]]
name = "nomic-embed-text:latest"
capability = "embedding"

[[providers]]
name = "kokoro"
kind = "openai_compatible"
base_url = "http://127.0.0.1:8421/v1"
api_key = "local"

[[providers.models]]
name = "kokoro"
capability = "tts"

[[providers]]
name = "local-image"
kind = "local_image_gen"
command = "python3"
args = ["mofa-fm/gemini_image.py", "--prompt", "{prompt}", "--output", "{output}"]
output_format = "png"

[[providers.models]]
name = "local-image-card"
capability = "image_gen"
```

---

### Template B: Hybrid Local + Cloud Burst (`mofa_hybrid.toml`)
*Prioritizes local models first, seamlessly failing over to cloud providers when local limits are exceeded.*

```toml
[listen]
host = "127.0.0.1"
port = 8420

[memory]
idle_timeout_secs = 300

[observability]
enabled = true

# 1. Local LLMs (Priority 10)
[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
priority = 10

[[providers.models]]
name = "gemma3:4b"
capability = "chat"

[[providers.models]]
name = "llava:latest"
capability = "vlm"

# 2. Local TTS (Priority 10)
[[providers]]
name = "kokoro"
kind = "openai_compatible"
base_url = "http://127.0.0.1:8421/v1"
api_key = "local"
priority = 10

[[providers.models]]
name = "kokoro"
capability = "tts"

# 3. Cloud Burst LLM (Priority 5 — used when cloud requested or local busy)
[[providers]]
name = "gemini"
kind = "openai_compatible"
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key = "${GEMINI_API_KEY}"
priority = 10
cost_tier = "low"

[[providers.models]]
name = "gemini-3.6-flash"
capability = "chat"

[[providers.models]]
name = "gemini-flash-latest"
capability = "chat"

# 4. Image Generation Adapter (Priority 10)
[[providers]]
name = "gemini-image"
kind = "local_image_gen"
command = "python3"
args = ["mofa-fm/gemini_image.py", "--prompt", "{prompt}", "--output", "{output}"]
output_format = "png"
priority = 10

[[providers.models]]
name = "gemini-2.5-flash-image"
capability = "image_gen"
```

---

## 6. Self-Healing Diagnostics (`mofa doctor`)

Whenever you edit your configuration file, add new models, or encounter connection warnings, run the built-in diagnostic doctor:

```bash
# Run via CLI
python3 mofa-fm/mofa_doctor.py

# Or via Quickstart
bash quickstart.sh doctor
```

### What `mofa doctor` checks:
1. **Engine Gateway**: Confirms port 8420 is listening and responds to health pings.
2. **Provider Availability**: Probes Ollama (port 11434), Kokoro (port 8421), and Whisper.
3. **Model Capabilities**: Lists all discovered models with their mapped capability (`[Chat]`, `[VLM]`, `[TTS]`, `[Embedding]`).
4. **Cloud API Keys**: Verifies if `GEMINI_API_KEY` or `OPENAI_API_KEY` are present.
5. **System Dependencies**: Verifies Python 3, PyTorch, FFmpeg, and ffprobe for video composition.
6. **Scenario Readiness Matrix**: Prints a green `[READY]` check for all 7 MoFA flagship delivery scenarios.
