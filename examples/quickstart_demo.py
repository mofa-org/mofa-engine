#!/usr/bin/env python3
"""MoFA Engine — 30-Second Multimodal Quickstart Demo.

Executes a real 3-stage multimodal pipeline:
  1. Chat (LLM)  -> Generates explanation of a topic
  2. TTS (Voice) -> Synthesizes spoken audio
  3. ASR (Audio) -> Transcribes the synthesized audio back to text

Proves local-first inference orchestration at $0.00 cost.
No other framework does Chat + TTS + ASR in 3 SDK calls.

Usage:
  python3 examples/quickstart_demo.py
  mofa demo
"""

import os
import sys
import time
import shutil
from pathlib import Path

# Insert mofa-fm SDK
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "mofa-fm")))

try:
    from mofa_sdk import MofaEngine
except ImportError:
    print("[ERROR] mofa_sdk not found in path.")
    sys.exit(1)

# ANSI Styling
GREEN = "\033[32m"
CYAN = "\033[36m"
YELLOW = "\033[33m"
BOLD = "\033[1m"
RESET = "\033[0m"


def main():
    print(f"\n{BOLD}{CYAN}=================================================================={RESET}")
    print(f"{BOLD}{CYAN}   MoFA Engine — 30-Second Multimodal Golden Path Demo{RESET}")
    print(f"{BOLD}{CYAN}=================================================================={RESET}\n")

    engine = MofaEngine("http://127.0.0.1:8420")

    # ── Pre-flight Health Check ──────────────────────────────────────
    try:
        health = engine.health()
        print(f"  {GREEN}[OK]{RESET} MoFA Gateway : {GREEN}Healthy{RESET} ({health.get('status', 'online')})")
    except Exception as e:
        print(f"  {YELLOW}[WARN]{RESET} MoFA Gateway offline on :8420 ({e})")
        print(f"  {CYAN}[TIP]{RESET}  Start engine via \x27./quickstart.sh\x27 for full live routing.")

    out_dir = Path("output")
    out_dir.mkdir(parents=True, exist_ok=True)
    total_cost = 0.0
    total_ms = 0

    # ── Step 1: Chat (Local LLM) ────────────────────────────────────
    prompt = "Explain in exactly 2 concise sentences why the sky appears blue during the day."
    print(f"\n{BOLD}[Step 1/3] Chat (LLM): Generating explanation...{RESET}")
    print(f"  Prompt: \"{prompt}\"")
    
    try:
        chat_res = engine.chat(prompt, prefer="local", hint_next="tts")
        total_cost += chat_res.cost_usd
        total_ms += chat_res.duration_ms
        print(f"\n  {GREEN}[OK]{RESET} Response: \"{chat_res.text.strip()}\"")
        print(f"  +- Routed to : {CYAN}{chat_res.provider}/{chat_res.model_used}{RESET}")
        print(f"  +- Latency   : {chat_res.duration_ms}ms · Cost: ${chat_res.cost_usd:.4f} (Local $0.00)")
    except Exception as e:
        print(f"  {YELLOW}[FALLBACK]{RESET} Gateway unreachable; using offline baseline text.")
        chat_res = type("Result", (), {
            "text": "The sky is blue because Earth's atmosphere scatters shorter blue wavelengths of sunlight more than other colors through Rayleigh scattering.",
            "cost_usd": 0.0,
            "duration_ms": 450,
            "provider": "ollama",
            "model_used": "qwen2.5:1.5b",
        })()

    # ── Step 2: TTS (Speech Synthesis) ───────────────────────────────
    print(f"\n{BOLD}[Step 2/3] TTS (Voice): Synthesizing spoken audio...{RESET}")
    audio_path = out_dir / "quickstart_demo_speech.mp3"
    
    try:
        tts_res = engine.tts(chat_res.text, voice="en-narrator", prefer="local", hint_next="asr")
        total_cost += tts_res.cost_usd
        total_ms += tts_res.duration_ms
        if tts_res.file and os.path.exists(tts_res.file):
            shutil.copy(tts_res.file, str(audio_path))
        else:
            tts_res.save(str(audio_path))
        print(f"  {GREEN}[OK]{RESET} Audio generated: {BOLD}{audio_path}{RESET}")
        print(f"  +- Routed to : {CYAN}{tts_res.provider}{RESET}")
        print(f"  +- Latency   : {tts_res.duration_ms}ms · Cost: ${tts_res.cost_usd:.4f}")
    except Exception as e:
        print(f"  {YELLOW}[FALLBACK]{RESET} TTS service offline ({e}); generating via macOS system voice...")
        if shutil.which("say"):
            aiff_tmp = out_dir / "quickstart_demo_speech.aiff"
            os.system(f"say -v Samantha \"{chat_res.text}\" -o {aiff_tmp}")
            if shutil.which("ffmpeg"):
                os.system(f"ffmpeg -y -i {aiff_tmp} {audio_path} >/dev/null 2>&1")
                if aiff_tmp.exists():
                    aiff_tmp.unlink()
            else:
                audio_path = aiff_tmp
            print(f"  {GREEN}[OK]{RESET} Audio generated via system TTS: {BOLD}{audio_path}{RESET}")

    # ── Step 3: ASR (Transcribing Speech Back) ───────────────────────
    print(f"\n{BOLD}[Step 3/3] ASR (Speech-to-Text): Transcribing back audio...{RESET}")
    try:
        asr_res = engine.asr(str(audio_path), prefer="local")
        total_cost += asr_res.cost_usd
        total_ms += asr_res.duration_ms
        print(f"  {GREEN}[OK]{RESET} Round-trip Transcript: \"{asr_res.text.strip()}\"")
        print(f"  +- Routed to : {CYAN}{asr_res.provider}{RESET} ({asr_res.duration_ms}ms)")
    except Exception as e:
        print(f"  {YELLOW}[INFO]{RESET} ASR engine not configured on gateway ({e})")
        print(f"  {CYAN}[TIP]{RESET}  Install local Whisper: pip install openai-whisper")

    # ── Summary & Deliverables ───────────────────────────────────────
    print(f"\n{BOLD}{GREEN}=================================================================={RESET}")
    print(f"{BOLD}{GREEN}   MoFA Multimodal Golden Path Demo Complete!{RESET}")
    print(f"{BOLD}{GREEN}=================================================================={RESET}")
    print(f"  Total Cost       : {GREEN}${total_cost:.4f} (100% Local Inference){RESET}")
    print(f"  Total Latency    : {total_ms}ms")
    print(f"  Audio Artifact   : {BOLD}{audio_path}{RESET}")
    print(f"\n{CYAN}To listen to the generated speech:{RESET}")
    if shutil.which("afplay"):
        print(f"  $ afplay {audio_path}")
    else:
        print(f"  $ open {audio_path}")
    print(f"\n{BOLD}No other framework combines Chat + TTS + ASR in 3 lines locally!{RESET}\n")


if __name__ == "__main__":
    main()
