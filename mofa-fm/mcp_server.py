"""MoFA Engine MCP Server — Exposes engine capabilities as MCP tools.

Allows Claude Desktop, Cursor, and Cline to use MoFA Engine as an
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
import os
import sys
from contextlib import redirect_stdout
from typing import List

try:
    from fastmcp import FastMCP
    HAS_FASTMCP = True
except ImportError:
    HAS_FASTMCP = False
    FastMCP = None

# Add parent dir so mofa_sdk is importable
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mofa_sdk import MofaEngine

if HAS_FASTMCP:
    mcp = FastMCP("MoFA Engine")
else:
    class DummyMCP:
        def tool(self, func):
            return func
        def run(self, *args, **kwargs):
            raise ImportError("FastMCP is required to run the MCP server: pip install fastmcp")
    mcp = DummyMCP()

engine = MofaEngine()


@mcp.tool
def mofa_chat(
    message: str,
    model: str = "",
    prefer: str = "auto",
    hint_next: str = "",
    reasoning_effort: str = "",
) -> str:
    """Chat with AI models via MoFA Engine with optional deep thinking.

    Routes to the best available model (local Ollama or cloud OpenAI/DeepSeek).
    Supports locality preferences for privacy-sensitive workloads.

    Args:
        message: The user message to send.
        model: Specific model name (optional, auto-routed if empty).
        prefer: Routing preference — "local", "cloud", or "auto".
        hint_next: Next capability hint for warmup (e.g., "tts", "image_gen").
        reasoning_effort: Reasoning effort tier ("low", "medium", "high" for deep thinking).
    """
    try:
        reasoning_param = {"effort": reasoning_effort} if reasoning_effort else None
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
def mofa_tts(
    text: str,
    voice: str = "zh-female-1",
    speed: float = 1.0,
    prefer: str = "auto",
) -> str:
    """Synthesize speech from text via MoFA Engine TTS.

    Supports voice aliases: zh-female-1, zh-male-1, en-narrator.
    Local voices (Kokoro/Crane) are free; cloud voices (tts-1) incur cost.

    Args:
        text: The text to synthesize into speech.
        voice: Voice alias or ID.
        speed: Playback speed (0.5–2.0).
        prefer: "local" for Kokoro/Crane, "cloud" for OpenAI tts-1.
    """
    try:
        result = engine.tts(
            text,
            voice=voice,
            speed=speed,
            prefer=prefer if prefer != "auto" else None,
        )
        if result.file:
            return f"Audio saved to: {result.file}\n[provider: {result.provider}, locality: {result.locality}]"
        return f"TTS completed.\n[provider: {result.provider}, duration: {result.duration_ms}ms]"
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
    """Generate images from text via MoFA Engine ImageGen.

    Routes to local Stable Diffusion or cloud DALL-E/FLUX.

    Args:
        prompt: Description of the image to generate.
        size: Output size (e.g., "1024x1024", "512x512").
        style: Style preset ("vivid", "natural", or empty for default).
        prefer: "local" for SD/SDXL, "cloud" for DALL-E.
    """
    try:
        result = engine.image_gen(
            prompt,
            size=size,
            style=style or None,
            prefer=prefer if prefer != "auto" else None,
        )
        output = ""
        if result.url:
            output = f"Image URL: {result.url}"
        elif result.file:
            output = f"Image saved to: {result.file}"
        else:
            output = "Image generated."
        output += f"\n[provider: {result.provider}, locality: {result.locality}, "
        output += f"cost: ${result.cost_usd:.4f}]"
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
        prefer: "local" for local VLM (llava), "cloud" for GPT-4o.
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
