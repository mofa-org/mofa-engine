#!/usr/bin/env python3
"""MoFA Engine — Self-Healing Environment Diagnostic Tool (`mofa doctor`).

Checks the health of the MoFA Engine Gateway daemon, local Ollama models,
TTS/ASR providers, system tools (FFmpeg, Python), and per-scenario readiness.
Generates actionable copy-paste fix commands if anything is missing.

Usage:
  python3 mofa-fm/mofa_doctor.py
  mofa doctor
"""

import os
import sys
import json
import shutil
import urllib.request
import urllib.error
from typing import Dict, List, Any, Tuple

# ANSI color styling
GREEN = "\033[32m"
YELLOW = "\033[33m"
RED = "\033[31m"
CYAN = "\033[36m"
BOLD = "\033[1m"
RESET = "\033[0m"


def check_url(url: str, timeout: float = 2.0) -> Tuple[bool, Any]:
    """Helper to perform HTTP GET and return (success, parsed_json_or_text)."""
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "mofa-doctor/0.1"})
        with urllib.request.urlopen(req, timeout=timeout) as response:
            if response.status == 200:
                content = response.read().decode("utf-8")
                try:
                    return True, json.loads(content)
                except Exception:
                    return True, content
    except Exception as e:
        return False, str(e)
    return False, "Unknown error"


def run_doctor() -> int:
    print(f"\n{BOLD}{CYAN}=================================================================={RESET}")
    print(f"{BOLD}{CYAN}   MoFA Engine — Environment Diagnostic & Doctor (`mofa doctor`){RESET}")
    print(f"{BOLD}{CYAN}=================================================================={RESET}\n")

    fixes: List[str] = []
    has_chat_model = False
    has_vision_model = False

    # ── 1. Engine Gateway & Core ─────────────────────────────────────
    print(f"{BOLD}1. Engine & AI Provider Status{RESET}")
    engine_ok, engine_data = check_url("http://127.0.0.1:8420/health")
    if engine_ok:
        uptime = engine_data.get("uptime_seconds", 0) if isinstance(engine_data, dict) else "?"
        print(f"  {GREEN}[OK]{RESET}   MoFA Engine Gateway   : {GREEN}Healthy{RESET} on port 8420 (uptime: {uptime}s)")
    else:
        print(f"  {RED}[ERROR]{RESET} MoFA Engine Gateway   : {RED}Offline{RESET} on http://127.0.0.1:8420")
        fixes.append("./quickstart.sh                   # Starts the MoFA Engine Gateway daemon")

    # ── 2. Ollama Local LLM Service ──────────────────────────────────
    ollama_ok, ollama_data = check_url("http://127.0.0.1:11434/api/tags")
    ollama_models = []
    if ollama_ok and isinstance(ollama_data, dict):
        models_raw = ollama_data.get("models", [])
        ollama_models = [m.get("name") for m in models_raw if "name" in m]
        print(f"  {GREEN}[OK]{RESET}   Ollama LLM Service    : {GREEN}Online{RESET} on port 11434 ({len(ollama_models)} models discovered)")
        
        for m in ollama_models:
            is_vlm = any(v in m.lower() for v in ["llava", "vision", "vl", "minicpm"])
            is_embed = "embed" in m.lower()
            tag = "VLM / Vision" if is_vlm else ("Embedding" if is_embed else "Chat / Reasoning")
            if is_vlm:
                has_vision_model = True
            elif not is_embed:
                has_chat_model = True
            print(f"         +- {m:<20} : {CYAN}[{tag}]{RESET}")
        
        if not has_chat_model:
            print(f"  {YELLOW}[WARN]{RESET} No local Chat model found in Ollama.")
            fixes.append("ollama pull qwen2.5:1.5b         # Fast 1.5B chat model for instant local runs")
        if not has_vision_model:
            print(f"  {YELLOW}[WARN]{RESET} No local Vision (VLM) model found in Ollama.")
            fixes.append("ollama pull llava                # Vision understanding model for Scenario S3")
    else:
        print(f"  {YELLOW}[WARN]{RESET} Ollama LLM Service    : {YELLOW}Offline / Not Detected{RESET} on port 11434")
        fixes.append("ollama serve                     # Start Ollama service (or \x27brew install ollama\x27)")

    # ── 3. TTS & Speech Synthesis ────────────────────────────────────
    kokoro_ok, _ = check_url("http://127.0.0.1:8421/health")
    has_system_tts = shutil.which("say") is not None or shutil.which("espeak") is not None
    
    if kokoro_ok:
        print(f"  {GREEN}[OK]{RESET}   Kokoro Neural TTS     : {GREEN}Online{RESET} on port 8421 (Multi-voice neural audio)")
    elif has_system_tts:
        voice_engine = "macOS \x27say\x27" if shutil.which("say") else "Linux \x27espeak\x27"
        print(f"  {GREEN}[OK]{RESET}   System Audio TTS      : {GREEN}Available{RESET} via {voice_engine} (Zero-config fallback)")
    else:
        print(f"  {RED}[ERROR]{RESET} Text-to-Speech Engine : {RED}No voice synthesizer detected{RESET}")
        fixes.append("pip install kokoro-onnx soundfile # Install neural TTS package")

    # ── 4. ASR & Speech-to-Text ──────────────────────────────────────
    has_whisper = shutil.which("whisper") is not None or os.path.exists("./mofa-fm/.venv/bin/whisper")
    has_funasr = shutil.which("funasr") is not None
    if has_whisper or has_funasr:
        asr_type = "FunASR CLI" if has_funasr else "Whisper CLI"
        print(f"  {GREEN}[OK]{RESET}   Speech-to-Text (ASR)  : {GREEN}Available{RESET} via {asr_type}")
    else:
        print(f"  {YELLOW}[WARN]{RESET} Speech-to-Text (ASR)  : {YELLOW}Not Found{RESET} (S1 will fall back to pre-recorded sample transcript)")
        fixes.append("pip install openai-whisper       # Enable local speech recognition for Scenario S1")

    # ── 5. Cloud AI Providers (Dual-Track) ───────────────────────────
    print(f"\n{BOLD}2. Cloud AI Providers & Hybrid Acceleration{RESET}")
    has_gemini = bool(os.environ.get("GEMINI_API_KEY") or os.environ.get("GOOGLE_API_KEY"))
    has_openai = bool(os.environ.get("OPENAI_API_KEY"))
    has_deepseek = bool(os.environ.get("DEEPSEEK_API_KEY"))
    has_fireworks = bool(os.environ.get("FIREWORKS_API_KEY"))

    if has_gemini:
        print(f"  {GREEN}[OK]{RESET}   Google Gemini AI      : {GREEN}Configured{RESET} (Gemini 2.5 Flash Chat & Native TTS active)")
    else:
        print(f"  {CYAN}[INFO]{RESET} Google Gemini AI      : {CYAN}Unset{RESET} (export GEMINI_API_KEY for free-tier cloud burst)")

    if has_openai:
        print(f"  {GREEN}[OK]{RESET}   OpenAI Cloud          : {GREEN}Configured{RESET} (GPT-4o & Whisper cloud fallback active)")
    else:
        print(f"  {CYAN}[INFO]{RESET} OpenAI Cloud          : {CYAN}Unset{RESET} (export OPENAI_API_KEY for GPT-4o)")

    if has_deepseek:
        print(f"  {GREEN}[OK]{RESET}   DeepSeek Reasoning    : {GREEN}Configured{RESET} (DeepSeek-R1 deep thinking active)")
    if has_fireworks:
        print(f"  {GREEN}[OK]{RESET}   Fireworks AI          : {GREEN}Configured{RESET} (Serverless open models active)")

    # ── 6. System Tools & Utilities ──────────────────────────────────
    print(f"\n{BOLD}3. System Tools & Media Engines{RESET}")
    python_ver = sys.version.split()[0]
    print(f"  {GREEN}[OK]{RESET}   Python Runtime        : Found v{python_ver}")

    if shutil.which("ffmpeg"):
        print(f"  {GREEN}[OK]{RESET}   FFmpeg Video Engine   : {GREEN}Found{RESET} (Full video rendering enabled)")
    else:
        print(f"  {YELLOW}[WARN]{RESET} FFmpeg Video Engine   : {YELLOW}Not Found{RESET} (Needed for S4 Explainer Video composition)")
        fixes.append("brew install ffmpeg              # Video composition engine for S4")

    # ── 7. Scenario Readiness Matrix ─────────────────────────────────
    print(f"\n{BOLD}4. Scenario Readiness Matrix{RESET}")
    scenarios = [
        ("S1 Meeting Brief", has_chat_model and (has_whisper or True), "ASR -> Chat Minutes -> TTS Audio"),
        ("S2 Code Review", has_chat_model, "High-effort reasoning thought stream on Git diff"),
        ("S3 Document AI", has_vision_model, "Multimodal VLM structured receipt extraction"),
        ("S4 Explainer Video", has_chat_model and bool(shutil.which("ffmpeg")), "Script -> ImageGen -> TTS -> MP4 Video"),
        ("S5 Privacy Moat", has_chat_model, "Confidential local chat (0% cloud leak)"),
        ("S6 Podcast Studio", has_chat_model and (kokoro_ok or has_system_tts), "Article -> Multi-voice audio narration"),
        ("S7 Provider Race", has_chat_model or engine_ok, "Local vs Cloud dual-track latency/cost benchmark"),
    ]

    for name, ready, desc in scenarios:
        status_badge = f"{GREEN}[READY]{RESET}" if ready else f"{YELLOW}[DEGRADED]{RESET}"
        print(f"  {status_badge} {BOLD}{name:<20}{RESET} : {desc}")

    # ── 7. Actionable Fix Suggestions ────────────────────────────────
    if fixes:
        print(f"\n{BOLD}{YELLOW}=================================================================={RESET}")
        print(f"{BOLD}{YELLOW}   Actionable Fix Commands (To get 100% full capability):{RESET}")
        print(f"{BOLD}{YELLOW}=================================================================={RESET}")
        for fix in fixes:
            print(f"  $ {fix}")
        print()
    else:
        print(f"\n{BOLD}{GREEN}=================================================================={RESET}")
        print(f"{BOLD}{GREEN}   All Systems Operational! Ready for 5-Minute Demos.{RESET}")
        print(f"{BOLD}{GREEN}=================================================================={RESET}\n")

    return 0 if (engine_ok and has_chat_model) else 1


if __name__ == "__main__":
    sys.exit(run_doctor())
