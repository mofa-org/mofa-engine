"""MoFA Engine MCP Server — Exposes engine capabilities as MCP tools.

Allows Claude Desktop, Cursor, and Cline to use MoFA Engine as an
inference runtime via the Model Context Protocol (MCP).

Transports:
  stdio (default): python3 mcp_server.py
  http:            python3 mcp_server.py --transport http --port 8421

Requirements:
  pip install fastmcp

PRD v3.1 §6.2, §8.2 W4, §9.3.4 W4.
"""

import argparse
import sys
import os

from fastmcp import FastMCP

# Add parent dir so mofa_sdk is importable
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mofa_sdk import MofaEngine

mcp = FastMCP("MoFA Engine")

engine = MofaEngine()


@mcp.tool
def mofa_chat(
    message: str,
    model: str = "",
    prefer: str = "auto",
    hint_next: str = "",
) -> str:
    """Chat with AI models via MoFA Engine.

    Routes to the best available model (local Ollama or cloud OpenAI/DeepSeek).
    Supports locality preferences for privacy-sensitive workloads.

    Args:
        message: The user message to send.
        model: Specific model name (optional, auto-routed if empty).
        prefer: Routing preference — "local", "cloud", or "auto".
        hint_next: Next capability hint for warmup (e.g., "tts", "image_gen").
    """
    result = engine.chat(
        message,
        model=model or None,
        prefer=prefer if prefer != "auto" else None,
        hint_next=hint_next or None,
    )
    response = result.text or ""
    response += f"\n\n[model: {result.model_used}, provider: {result.provider}, "
    response += f"locality: {result.locality}, cost: ${result.cost_usd:.4f}, "
    response += f"duration: {result.duration_ms}ms]"
    return response


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
    result = engine.tts(
        text,
        voice=voice,
        speed=speed,
        prefer=prefer if prefer != "auto" else None,
    )
    if result.file:
        return f"Audio saved to: {result.file}\n[provider: {result.provider}, locality: {result.locality}]"
    return f"TTS completed.\n[provider: {result.provider}, duration: {result.duration_ms}ms]"


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


@mcp.tool
def mofa_understand(
    question: str,
    image_paths: list[str] = [],
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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="MoFA Engine MCP Server")
    parser.add_argument(
        "--transport",
        choices=["stdio", "http"],
        default="stdio",
        help="Transport mode (default: stdio for Claude Desktop)",
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
