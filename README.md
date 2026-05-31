# MoFA Engine

**Multimodal Orchestration for Artifacts**

Your apps need AI — LLM, TTS, ASR, image gen — but you don't want to wire up six different SDKs, manage model loading, or worry about which provider is down today. MoFA Engine handles all of that. One endpoint, many models, smart routing.

你的应用需要 AI 能力——LLM、TTS、语音识别、图像生成——但你不想接六套 SDK、管模型加载、操心哪家 API 又挂了。MoFA Engine 帮你搞定。一个接口，多个模型，智能路由。

## What it does / 做什么

```
Your App ──▶ MoFA Engine ──▶ Ollama (local)
                          ──▶ OpenAI
                          ──▶ DeepSeek
                          ──▶ DashScope (Qwen)
                          ──▶ NVIDIA NIM
                          ──▶ Perplexity
                          ──▶ Zhipu (GLM)
                          ──▶ ... any OpenAI-compatible API
```

- **Capability routing** — ask for "chat" or "tts", engine picks the best available model
  按能力路由——你说"要 chat"或"要 tts"，引擎自动选最合适的模型
- **Local first** — prefers your Ollama models over cloud APIs (free > paid)
  本地优先——优先用你的 Ollama 模型，省钱
- **Auto-failover** — if a provider goes down, requests fall through to the next one
  自动降级——某个 provider 挂了，请求自动切到下一个
- **Circuit breaker** — stops hammering a broken provider, auto-recovers
  熔断保护——不会反复请求已挂的 provider，自动恢复
- **Preflight** — hint what you need next, engine pre-warms the model
  预加载——告诉引擎下一步要什么，提前热好模型

## Quick start / 快速开始

```bash
# Build
cargo build --release

# Run — auto-detects Ollama + any API keys in your environment
OPENAI_API_KEY=sk-... DEEPSEEK_API_KEY=sk-... ./target/release/mofa-engine

# Or just run with Ollama (no API keys needed)
./target/release/mofa-engine
```

Engine starts at `http://localhost:8420`. Open it in a browser — there's a dashboard.

引擎跑在 `http://localhost:8420`。浏览器打开就有管理面板。

## API

**Query capabilities / 查看能力**
```bash
curl localhost:8420/v1/capabilities
```

**Run inference / 跑推理**
```bash
# Let the engine pick a model
curl localhost:8420/v1/invoke -d '{
  "capability": "chat",
  "messages": [{"role": "user", "content": "Hello"}]
}'

# Or specify one
curl localhost:8420/v1/invoke -d '{
  "model": "deepseek-chat",
  "messages": [{"role": "user", "content": "Hello"}]
}'

# TTS
curl localhost:8420/v1/invoke -d '{
  "capability": "tts",
  "messages": [{"role": "user", "content": "Hello world"}]
}'
# → returns { "file": "/tmp/mofa_tts_xxx.mp3" }
```

**Hint next step / 提示下一步**
```bash
curl localhost:8420/v1/invoke -d '{
  "capability": "chat",
  "messages": [{"role": "user", "content": "Translate: hello"}],
  "hint_next": "tts"
}'
# Engine pre-warms a TTS model while your LLM request runs
# 引擎在跑 LLM 的同时预热 TTS 模型
```

## Python SDK

```python
from mofa_sdk import MofaEngine

engine = MofaEngine()  # localhost:8420

# Chat — engine picks the best model
r = engine.chat("Translate this to Chinese: hello world")
print(r.text)           # 你好世界
print(r.provider)       # ollama
print(r.duration_ms)    # 1200

# TTS
r = engine.tts("你好世界")
print(r.file)           # /tmp/mofa_tts_xxx.mp3

# Explicit model
r = engine.chat("Hello", model="gpt-4o-mini")
```

## Configuration / 配置

Zero-config works — the engine auto-detects Ollama at `localhost:11434` and reads API keys from environment variables:

零配置即可运行——引擎自动检测本地 Ollama，并从环境变量读 API key：

| Variable | Provider |
|---|---|
| *(always)* | Ollama (local) |
| `OPENAI_API_KEY` | OpenAI |
| `DEEPSEEK_API_KEY` | DeepSeek |
| `DASHSCOPE_API_KEY` | DashScope (Qwen) |
| `NVIDIA_API_KEY` | NVIDIA NIM |
| `PERPLEXITY_API_KEY` | Perplexity |
| `ZAI_API_KEY` | Zhipu (GLM) |

For more control, create a `config.toml`:

需要更细粒度的控制，可以写 `config.toml`：

```toml
[listen]
host = "0.0.0.0"
port = 8420

[memory]
budget_mb = 16384
idle_timeout_secs = 120

[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
priority = 1
cost_tier = "free"

[[providers]]
name = "deepseek"
kind = "openai_compatible"
base_url = "https://api.deepseek.com"
api_key = "${DEEPSEEK_API_KEY}"
priority = 5
cost_tier = "low"

[[providers.models]]
name = "deepseek-chat"
capability = "chat"
context_window = 64000
```

## How routing works / 路由逻辑

The engine scores every model on four dimensions and picks the highest:

引擎对每个模型做四维评分，选最高的：

| Dimension | Weight | Logic |
|---|---|---|
| **Availability** | ×1000 | Hot > Warming > Cold; Busy/Failed = skip |
| **Locality** | ×100 | Local (Ollama) > Cloud |
| **Cost** | ×50 | Free > Low > Medium > High |
| **Capability** | hard filter | Must match (chat ≠ tts) |

If the selected model fails, the engine tries the next best from a *different* provider.

选中的模型失败时，引擎自动从*另一个* provider 找备选。

## Architecture / 架构

```
mofa-kernel       Trait definitions. No implementations.
mofa-engine-core  Providers, router, memory manager, circuit breaker, preflight.
mofa-engine-sdk   HTTP API (Axum), SSE events, web dashboard.
mofa-engine-app   Binary entry point.
```

All OpenAI-compatible APIs share a single generic `OpenAiCompatProvider` — adding a new provider is just config, not code.

所有 OpenAI 兼容 API 共享一个通用 `OpenAiCompatProvider`——加新 provider 只需改配置，不用写代码。

## Tests / 测试

```bash
cargo test                          # 42 unit tests
python3 tests/test_local_models.py  # 20 local model tests (needs Ollama)
python3 tests/test_all_providers.py # 13 multi-provider tests (needs API keys)
cd e2e && npx playwright test       # 7 browser tests for dashboard
```

## License

MIT
