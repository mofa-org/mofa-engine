# MoFA Engine Skill

## What is it
A multimodal model execution engine that manages model lifecycles. You call it to run models — it handles loading, unloading, memory management, and backend selection automatically.

## Connection
Engine runs at `http://127.0.0.1:8420`.

## Check capabilities
```
GET /v1/capabilities
```
Returns all available model types and specific models. Call this first to understand what the engine can do.

## Run a model
```
POST /v1/run
{
  "type": "tts",                    // model type: llm, tts, asr, image_gen, video_gen, vlm
  "input": {"text": "..."},         // text input
  "hint": {"next": "asr"}           // optional: tell engine what you need next
}
```

You can also specify a model by name:
```
{"model": "gpt-4o", "input": {"text": "..."}}
```

### Input formats
- Text: `{"text": "..."}`
- Audio/image/video: `{"file": "/path/to/file"}`
- System prompt: `{"text": "...", "prompt": "You are a translator"}`

### Output formats
- Text: `{"output": {"text": "..."}}`
- Audio/image/video: `{"output": {"file": "/path/to/output"}}`

## Check engine status
```
GET /v1/status
```
Returns memory usage, loaded models, backend health.

## Common patterns
- **Article → Podcast**: Call LLM (hint: tts) → Call TTS
- **Speech to text**: Call ASR with audio file path
- **Translation**: Call LLM with translator system prompt

## Key behaviors
- Don't manage model loading/unloading — the engine handles it
- Hints are optional but improve performance (preloading)
- If you don't specify a model, just say the type ("tts") and the engine picks one
- Local models are preferred over cloud when available
- If a model fails, the engine automatically tries fallback models

## Python SDK
```python
from mofa_engine_sdk import MofaEngine

engine = MofaEngine()
result = engine.run_llm("Translate: Hello world", prompt="Translate to Chinese")
print(result.output_text)

result = engine.run_tts("你好世界")
print(result.output_file)  # path to MP3
```
