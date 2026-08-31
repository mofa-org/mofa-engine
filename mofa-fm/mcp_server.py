"""MoFA Engine MCP Server — Exposes engine capabilities as MCP tools, resources & prompts.

Allows Claude Desktop, Cursor, Cline, and Antigravity to use MoFA Engine as an
inference runtime via the Model Context Protocol (MCP).

Transports:
  stdio (default): python3 mofa-fm/mcp_server.py
  http:            python3 mofa-fm/mcp_server.py --transport http --port 8421

Requirements:
  pip install fastmcp

PRD v3.1 §6.2, §8.2 W4, §9.3.4 W4.
"""

import argparse
import io
import json
import os
import sys
from contextlib import redirect_stdout
from typing import Any, Dict, List, Optional

try:
    from fastmcp import FastMCP
    HAS_FASTMCP = True
except ImportError:
    HAS_FASTMCP = False
    FastMCP = None

# Add parent dir so mofa_sdk is importable
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mofa_sdk import MofaEngine, Pipeline

if HAS_FASTMCP:
    mcp = FastMCP("MoFA Engine")
else:
    class DummyMCP:
        def __init__(self):
            self._tools = {}
            self._resources = {}
            self._prompts = {}

        def tool(self, func=None, **kwargs):
            if func is not None:
                self._tools[func.__name__] = func
                return func
            def decorator(f):
                self._tools[f.__name__] = f
                return f
            return decorator

        def resource(self, uri: str, **kwargs):
            def decorator(f):
                self._resources[uri] = f
                return f
            return decorator

        def prompt(self, name: Optional[str] = None, **kwargs):
            def decorator(f):
                p_name = name or f.__name__
                self._prompts[p_name] = f
                return f
            return decorator

        def run(self, *args, **kwargs):
            raise ImportError("FastMCP is required to run the MCP server: pip install fastmcp")

    mcp = DummyMCP()

engine = MofaEngine()

# In-memory session store for multi-turn conversations (Improvement #6)
_SESSIONS: Dict[str, List[Dict[str, str]]] = {}
_MAX_HISTORY_PER_SESSION = 50


# ─── Core MCP Tools ───────────────────────────────────────────────────

@mcp.tool
def mofa_chat(
    message: str,
    model: str = "",
    prefer: str = "auto",
    hint_next: str = "",
    reasoning_effort: str = "",
    session_id: str = "",
) -> str:
    """Chat with AI models via MoFA Engine with optional deep thinking and multi-turn session memory.

    Routes to the best available model (local Ollama or cloud OpenAI/DeepSeek/Gemini).
    Supports locality preferences for privacy-sensitive workloads and multi-turn session tracking.

    Args:
        message: The user message to send.
        model: Specific model name (optional, auto-routed if empty).
        prefer: Routing preference — "local", "cloud", or "auto".
        hint_next: Next capability hint for warmup (e.g., "tts", "image_gen").
        reasoning_effort: Reasoning effort tier ("low", "medium", "high" for deep thinking).
        session_id: Optional session identifier for multi-turn conversational memory.
    """
    try:
        reasoning_param = {"effort": reasoning_effort} if reasoning_effort else None

        # Multi-turn conversation handling
        if session_id:
            history = _SESSIONS.setdefault(session_id, [])
            history.append({"role": "user", "content": message})
            if len(history) > _MAX_HISTORY_PER_SESSION:
                history = history[-_MAX_HISTORY_PER_SESSION:]
                _SESSIONS[session_id] = history

            result = engine.chat(
                message,
                messages=history,
                model=model or None,
                prefer=prefer if prefer != "auto" else None,
                hint_next=hint_next or None,
                reasoning=reasoning_param,
            )
            assistant_text = result.text or ""
            history.append({"role": "assistant", "content": assistant_text})
            turn_count = len(history) // 2

            response = assistant_text
            response += f"\n\n[session: {session_id} (turn {turn_count}), model: {result.model_used}, provider: {result.provider}, "
            response += f"locality: {result.locality}, cost: ${result.cost_usd:.4f}, "
            response += f"duration: {result.duration_ms}ms]"
            return response
        else:
            result = engine.chat(
                message,
                model=model or None,
                prefer=prefer if prefer != "auto" else None,
                hint_next=hint_next or None,
                reasoning=reasoning_param,
            )
            response = result.text or ""
            response += f"\n\n[model: {result.model_used}, provider: {result.provider}, "
            response += f"locality: {result.locality}, cost: ${result.cost_usd:.4f}, "
            response += f"duration: {result.duration_ms}ms]"
            return response
    except Exception as e:
        return f"[ERROR] Chat inference failed: {e}\n[TIP] Run 'mofa doctor' to inspect gateway models."


@mcp.tool
def mofa_session_clear(session_id: str = "") -> str:
    """Clear multi-turn chat history for a specific session or all active sessions.

    Args:
        session_id: Target session ID to reset. If empty, resets all active sessions.
    """
    global _SESSIONS
    if session_id:
        if session_id in _SESSIONS:
            turns = len(_SESSIONS[session_id]) // 2
            del _SESSIONS[session_id]
            return f"[OK] Cleared session '{session_id}' ({turns} conversation turns purged)."
        return f"[INFO] Session '{session_id}' was not found or already empty."
    else:
        total = len(_SESSIONS)
        _SESSIONS.clear()
        return f"[OK] Cleared all {total} active chat sessions."


@mcp.tool
def mofa_tts(
    text: str,
    voice: str = "zh-female-1",
    speed: float = 1.0,
    prefer: str = "auto",
) -> str:
    """Synthesize speech from text via MoFA Engine TTS with rich media artifact formatting.

    Supports voice aliases: zh-female-1, zh-male-1, en-narrator, af_alloy.
    Local voices (Kokoro/Crane) are free; cloud voices (tts-1) incur cost.

    Args:
        text: The text to synthesize into speech.
        voice: Voice alias or ID.
        speed: Playback speed (0.5–2.0).
        prefer: "local" for Kokoro/Crane, "cloud" for OpenAI/Gemini TTS.
    """
    try:
        result = engine.tts(
            text,
            voice=voice,
            speed=speed,
            prefer=prefer if prefer != "auto" else None,
        )
        output = "### 🔊 MoFA Speech Synthesis\n\n"
        if result.file:
            output += f"**Audio Artifact**: `{result.file}`\n"
            output += f"**Voice Profile**: `{voice}` | **Speed**: {speed}x\n"
            output += f"**Provider**: {result.provider} | **Locality**: {result.locality} | **Duration**: {result.duration_ms}ms\n\n"
            output += f"*(Playback: `afplay \"{result.file}\"` or `aplay \"{result.file}\"`)*"
        else:
            output += f"TTS completed.\n[provider: {result.provider}, locality: {result.locality}, duration: {result.duration_ms}ms]"
        return output
    except Exception as e:
        return f"[ERROR] TTS synthesis failed: {e}\n[TIP] Run 'mofa doctor' to inspect TTS provider."


@mcp.tool
def mofa_asr(
    audio_file: str,
    diarize: bool = False,
    language: str = "",
    prefer: str = "auto",
) -> str:
    """Transcribe audio to text via MoFA Engine ASR.

    Supports local FunASR and cloud Whisper. Files up to 25MB
    are uploaded via multipart; larger files use path reference.

    Args:
        audio_file: Absolute path to audio file (.wav, .mp3, .m4a).
        diarize: Enable speaker diarization (speaker attribution).
        language: Language hint (auto-detect if empty).
        prefer: "local" for FunASR, "cloud" for whisper-1.
    """
    try:
        result = engine.asr(
            audio_file,
            diarize=diarize,
            language=language or None,
            prefer=prefer if prefer != "auto" else None,
        )
        transcript = result.text or "(no transcript)"
        transcript += f"\n\n[provider: {result.provider}, locality: {result.locality}, "
        transcript += f"duration: {result.duration_ms}ms]"
        return transcript
    except Exception as e:
        return f"[ERROR] ASR transcription failed: {e}\n[TIP] Run 'mofa doctor' to inspect ASR setup."


@mcp.tool
def mofa_image_gen(
    prompt: str,
    size: str = "1024x1024",
    style: str = "",
    prefer: str = "auto",
) -> str:
    """Generate images from text via MoFA Engine ImageGen with native artifact presentation.

    Routes to local Stable Diffusion or cloud DALL-E/FLUX/Gemini.

    Args:
        prompt: Description of the image to generate.
        size: Output size (e.g., "1024x1024", "512x512").
        style: Style preset ("vivid", "natural", or empty for default).
        prefer: "local" for SD/SDXL, "cloud" for DALL-E/Gemini.
    """
    try:
        result = engine.image_gen(
            prompt,
            size=size,
            style=style or None,
            prefer=prefer if prefer != "auto" else None,
        )
        output = "### 🖼️ MoFA Generated Image\n\n"
        if result.url:
            output += f"![Generated Image]({result.url})\n\n"
            output += f"**Remote URL**: {result.url}\n"
        elif result.file:
            output += f"![Generated Image]({result.file})\n\n"
            output += f"**Local File**: `{result.file}`\n"
        else:
            output += "Image generated successfully.\n"
        output += f"**Resolution**: {size} | **Provider**: {result.provider} | **Locality**: {result.locality} | **Cost**: ${result.cost_usd:.4f} | **Duration**: {result.duration_ms}ms"
        return output
    except Exception as e:
        return f"[ERROR] Image generation failed: {e}\n[TIP] Run 'mofa doctor' to inspect ImageGen setup."


@mcp.tool
def mofa_understand(
    question: str,
    image_paths: List[str] = [],
    detail: str = "auto",
    prefer: str = "auto",
) -> str:
    """Analyze images with VLM via MoFA Engine (Vision / Document AI).

    Send images + a question to extract structured information,
    describe content, or run document AI extraction.

    Args:
        question: Question about the image(s).
        image_paths: List of absolute paths to image files.
        detail: Billing tier — "low", "high", or "auto".
        prefer: "local" for local VLM (llava), "cloud" for GPT-4o/Gemini.
    """
    try:
        result = engine.understand(
            images=image_paths if image_paths else None,
            question=question,
            detail=detail,
            prefer=prefer if prefer != "auto" else None,
        )
        response = result.text or "(no response)"
        response += f"\n\n[model: {result.model_used}, provider: {result.provider}, "
        response += f"locality: {result.locality}, cost: ${result.cost_usd:.4f}]"
        return response
    except Exception as e:
        return f"[ERROR] VLM understanding failed: {e}\n[TIP] Run 'mofa doctor' to inspect VLM setup."


@mcp.tool
def mofa_embed(
    text: str,
    model: str = "",
    prefer: str = "local",
) -> str:
    """Generate text embeddings for semantic search, vector search, and RAG via MoFA Engine (PRD §3.7).

    Args:
        text: Text to vectorize into dense vector embeddings.
        model: Specific embedding model (e.g., 'nomic-embed-text').
        prefer: 'local' for on-device Ollama embeddings, 'cloud' for commercial provider.
    """
    try:
        result = engine.embed(
            text,
            model=model or None,
            prefer=prefer if prefer != "auto" else None,
        )
        response = result.text or "[]"
        response += f"\n\n[model: {result.model_used}, provider: {result.provider}, "
        response += f"locality: {result.locality}, duration: {result.duration_ms}ms]"
        return response
    except Exception as e:
        return f"[ERROR] Embedding failed: {e}\n[TIP] Run 'mofa doctor' to inspect embedding models."


@mcp.tool
def mofa_run_pipeline(
    pipeline_type: str,
    input_text: str = "",
    input_file: str = "",
    voice: str = "en-narrator",
    prefer: str = "auto",
    reasoning_effort: str = "",
) -> str:
    """Execute declarative multimodal pipelines in a single atomic orchestration step (Improvement #5).

    Supported pipeline presets:
      - 'meeting_brief': Audio/transcript -> LLM structured minutes -> 30s executive audio brief.
      - 'podcast': Text/article -> Engaging multi-voice dialogue script -> Audio synthesis.
      - 'doc_ai': Image document -> VLM OCR schema and key-value extraction.
      - 'explainer_video': Topic -> Scriptwriting -> Visual image generation -> Voice narration.

    Args:
        pipeline_type: One of 'meeting_brief', 'podcast', 'doc_ai', 'explainer_video'.
        input_text: Text prompt, transcript, article, or topic input.
        input_file: File path for audio (ASR) or image (VLM) input.
        voice: Voice alias for speech synthesis ('en-narrator', 'zh-female-1', 'zh-male-1', 'af_alloy').
        prefer: Routing preference ('local', 'cloud', or 'auto').
        reasoning_effort: Reasoning effort tier ('low', 'medium', 'high').
    """
    ptype = pipeline_type.strip().lower()
    pref = prefer if prefer != "auto" else None
    reasoning_param = {"effort": reasoning_effort} if reasoning_effort else None

    try:
        if ptype == "meeting_brief":
            pipe = Pipeline(engine)
            if input_file and os.path.exists(input_file):
                pipe.asr(prefer=pref)
                pipe.chat("Extract the top key decision in 1 concise bullet point from: {input}", prefer=pref, reasoning=reasoning_param)
                pipe.tts(voice=voice, prefer=pref)
                res = pipe.run(audio=input_file)
            else:
                text_input = input_text or "Meeting discussion on architecture roadmap and milestone deliverables."
                pipe.chat("Extract the top key decision in 1 concise bullet point from: {input}", prefer=pref, reasoning=reasoning_param)
                pipe.tts(voice=voice, prefer=pref)
                res = pipe.run(input=text_input)

            summary = "### 📋 MoFA Pipeline: Meeting Brief (S1)\n\n"
            for i, step in enumerate(res.steps, 1):
                summary += f"**Step {i} ({step.provider}/{step.model_used})**: {step.locality.upper()} | {step.duration_ms}ms\n"
                if step.text:
                    summary += f"> {step.text.strip()}\n\n"
                if step.file:
                    summary += f"🔊 **Generated Audio Brief**: `{step.file}`\n\n"
            summary += f"\n**Total Latency**: {res.total_duration_ms}ms | **Total Cost**: ${res.total_cost:.6f} | **Locality**: {'LOCAL' if res.is_local else 'CLOUD'}"
            return summary

        elif ptype == "podcast":
            pipe = Pipeline(engine)
            article_text = input_text or "MoFA Engine enables local-first multimodal AI workflows with zero cloud cost."
            pipe.chat("Write a short 1-sentence podcast intro between Alex and Morgan about: {input}", prefer=pref, reasoning=reasoning_param)
            pipe.tts(voice=voice, prefer=pref)
            res = pipe.run(input=article_text)

            summary = "### 🎙️ MoFA Pipeline: Podcast Studio (S6)\n\n"
            for i, step in enumerate(res.steps, 1):
                summary += f"**Step {i} ({step.provider}/{step.model_used})**: {step.locality.upper()} | {step.duration_ms}ms\n"
                if step.text:
                    preview = step.text.strip()[:600]
                    summary += f"```text\n{preview}...\n```\n\n"
                if step.file:
                    summary += f"🎧 **Final Podcast Episode**: `{step.file}`\n\n"
            summary += f"\n**Total Latency**: {res.total_duration_ms}ms | **Total Cost**: ${res.total_cost:.6f} | **Locality**: {'LOCAL' if res.is_local else 'CLOUD'}"
            return summary

        elif ptype == "doc_ai":
            if not input_file:
                return "[ERROR] 'doc_ai' pipeline requires 'input_file' pointing to an image/document."
            question = input_text or "Extract all key-value pairs, dates, amounts, and vendor info as structured JSON."
            res = engine.understand(images=[input_file], question=question, prefer=pref)
            return (
                f"### 📄 MoFA Pipeline: Document AI (S3)\n\n"
                f"**Source Document**: `{input_file}`\n\n"
                f"**Extracted Data**:\n{res.text}\n\n"
                f"[model: {res.model_used}, provider: {res.provider}, locality: {res.locality}, cost: ${res.cost_usd:.4f}, duration: {res.duration_ms}ms]"
            )

        elif ptype == "explainer_video":
            pipe = Pipeline(engine)
            prompt = input_text or "How distributed consensus works in modern database engines."
            pipe.chat("Write a concise 3-sentence educational script explaining: {input}", prefer=pref, reasoning=reasoning_param)
            pipe.image_gen(prompt_template="Detailed educational visual infographic representing: {input}", prefer=pref)
            pipe.tts(voice=voice, prefer=pref)
            res = pipe.run(input=prompt)

            summary = "### 🎬 MoFA Pipeline: Explainer Video (S4)\n\n"
            for i, step in enumerate(res.steps, 1):
                summary += f"**Step {i} ({step.provider}/{step.model_used})**: {step.locality.upper()} | {step.duration_ms}ms\n"
                if step.text:
                    summary += f"> {step.text.strip()}\n\n"
                if step.file or step.url:
                    summary += f"🖼️/🔊 **Step Artifact**: `{step.file or step.url}`\n\n"
            summary += f"\n**Total Latency**: {res.total_duration_ms}ms | **Total Cost**: ${res.total_cost:.6f} | **Locality**: {'LOCAL' if res.is_local else 'CLOUD'}"
            return summary

        else:
            return f"[ERROR] Unknown pipeline type '{pipeline_type}'. Choose from: 'meeting_brief', 'podcast', 'doc_ai', 'explainer_video'."
    except Exception as e:
        return f"[ERROR] Pipeline execution failed: {e}\n[TIP] Run 'mofa doctor' to inspect required engines."


@mcp.tool
def mofa_doctor() -> str:
    """Run MoFA Engine environment diagnostic and readiness inspection.

    Checks engine health, Ollama models, TTS/ASR status, FFmpeg, and prints
    scenario readiness matrix with copy-paste fixes.
    """
    import importlib.util
    doctor_path = os.path.join(os.path.dirname(__file__), "mofa_doctor.py")
    if not os.path.exists(doctor_path):
        return "mofa_doctor.py not found"

    spec = importlib.util.spec_from_file_location("mofa_doctor", doctor_path)
    mod = importlib.util.module_from_spec(spec)

    buffer = io.StringIO()
    with redirect_stdout(buffer):
        try:
            spec.loader.exec_module(mod)
            mod.run_doctor()
        except SystemExit:
            pass
    return buffer.getvalue()


# ─── MCP Resources (PRD §8.2 / Improvement #1) ────────────────────────

@mcp.resource("mofa://models")
def get_models_resource() -> str:
    """Active local and cloud AI models and capabilities discovered by MoFA Engine."""
    try:
        caps = engine.capabilities()
        return json.dumps(caps, indent=2)
    except Exception as e:
        return json.dumps({"error": str(e), "tip": "Run mofa doctor to check gateway status"}, indent=2)


@mcp.resource("mofa://cost")
def get_cost_resource() -> str:
    """Real-time financial cost accounting, token consumption, and local savings ledger."""
    try:
        cost_data = engine.cost()
        return json.dumps(cost_data, indent=2)
    except Exception as e:
        return json.dumps({"error": str(e)}, indent=2)


@mcp.resource("mofa://status")
def get_status_resource() -> str:
    """Engine health status, residency gauge, and gateway telemetry."""
    try:
        status_data = engine.status()
        return json.dumps(status_data, indent=2)
    except Exception as e:
        return json.dumps({"error": str(e)}, indent=2)


@mcp.resource("mofa://scenarios")
def get_scenarios_resource() -> str:
    """Scenario readiness matrix and supported multi-modal pipelines."""
    scenarios = {
        "S1": {"name": "Meeting Brief", "modalities": ["ASR", "Chat", "TTS"], "status": "Ready", "pipeline": "meeting_brief"},
        "S2": {"name": "Code Review Agent", "modalities": ["Chat/DeepReasoning"], "status": "Ready", "pipeline": "code_review"},
        "S3": {"name": "Document AI OCR", "modalities": ["VLM"], "status": "Ready", "pipeline": "doc_ai"},
        "S4": {"name": "Explainer Video", "modalities": ["Chat", "ImageGen", "TTS", "FFmpeg"], "status": "Ready", "pipeline": "explainer_video"},
        "S5": {"name": "Privacy Moat", "modalities": ["Local LLM"], "status": "Ready", "pipeline": "confidential_chat"},
        "S6": {"name": "Podcast Studio", "modalities": ["Chat", "TTS"], "status": "Ready", "pipeline": "podcast"},
        "S7": {"name": "Provider Race Benchmark", "modalities": ["Multi-Provider LLM"], "status": "Ready", "pipeline": "race_benchmark"},
    }
    return json.dumps(scenarios, indent=2)


# ─── MCP Prompts (PRD §8.2 / Improvement #1) ──────────────────────────

@mcp.prompt("mofa_review_diff")
def prompt_review_diff(diff: str, detail: str = "high") -> str:
    """Generate a structured deep-reasoning code review prompt for a git diff."""
    return (
        f"You are the MoFA Engine Code Review Specialist. Review the following git diff with {detail}-effort reasoning:\n\n"
        f"```diff\n{diff}\n```\n\n"
        f"Please analyze:\n"
        f"1. Potential architectural defects, race conditions, or memory leaks.\n"
        f"2. Performance optimizations and locality opportunities (on-device vs cloud).\n"
        f"3. Strict adherence to contracts and error handling."
    )


@mcp.prompt("mofa_meeting_brief")
def prompt_meeting_brief(transcript: str, voice: str = "en-narrator") -> str:
    """Generate prompt to summarize meeting audio into a 30-second executive audio brief."""
    return (
        f"You are the MoFA Executive Meeting Assistant. Synthesize this meeting transcript into a concise 30-second executive audio brief for voice '{voice}':\n\n"
        f"Transcript:\n{transcript}\n\n"
        f"Format the output into:\n"
        f"1. Top 3 Executive Decisions\n"
        f"2. Action Items (Owner + Due Date)\n"
        f"3. 3-Sentence Spoken Summary for Audio Synthesis"
    )


@mcp.prompt("mofa_podcast_script")
def prompt_podcast_script(topic_or_article: str, num_speakers: int = 2) -> str:
    """Generate prompt to turn an article or technical topic into a multi-host podcast script."""
    return (
        f"Convert the following technical content into an engaging, dynamic {num_speakers}-speaker podcast dialogue:\n\n"
        f"Content:\n{topic_or_article}\n\n"
        f"Instructions:\n"
        f"- Format dialogue with speaker tags: [Alex] and [Morgan].\n"
        f"- Keep tone conversational, curious, and clear for audio listeners.\n"
        f"- Target 2–3 minutes of spoken audio."
    )


@mcp.prompt("mofa_extract_receipt")
def prompt_extract_receipt(image_path: str) -> str:
    """Generate prompt for VLM document AI receipt and invoice data extraction."""
    return (
        f"Analyze the image at path '{image_path}' using MoFA VLM understanding and extract structured JSON with:\n"
        f"- vendor_name (string)\n"
        f"- transaction_date (YYYY-MM-DD)\n"
        f"- line_items (array of {{item, quantity, price}})\n"
        f"- subtotal, tax, total_amount (numbers)\n"
        f"- currency (string)"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="MoFA Engine MCP Server")
    parser.add_argument(
        "--transport",
        choices=["stdio", "http"],
        default="stdio",
        help="Transport mode (default: stdio for Claude Desktop / Cursor)",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=8421,
        help="HTTP port (only used with --transport http)",
    )
    parser.add_argument(
        "--host",
        default="127.0.0.1",
        help="HTTP bind address (only used with --transport http)",
    )
    args = parser.parse_args()

    if args.transport == "http":
        mcp.run(transport="http", host=args.host, port=args.port)
    else:
        mcp.run(transport="stdio")

