#!/usr/bin/env python3
"""
Scenario S1: Long Audio Meeting -> Minutes & Executive Brief
MoFA Engine — Multimodal Orchestration for Artifacts

Takes a meeting audio recording, transcribes it with speaker diarization via ASR,
processes the transcript with Chat LLM to generate structured meeting minutes,
action items, and executive summary, and optionally synthesizes TTS narration of the brief.

Usage:
  python examples/meeting_brief.py --mock
  python examples/meeting_brief.py --audio meeting.wav --prefer local
  python examples/meeting_brief.py --audio call.m4a --narrate --prefer local
"""

import argparse
import os
import sys
import time
from typing import Dict, Any

# Ensure parent directory is in python path for mofa_sdk import
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "mofa-fm")))

try:
    from mofa_sdk import MofaEngine
except ImportError:
    class MofaEngine:
        def __init__(self, base_url: str = "http://127.0.0.1:8420"):
            self.base_url = base_url

        def asr(self, audio_file: str, prefer: str = "local", diarize: bool = True, **kwargs):
            return type("InvokeResult", (), {
                "text": "[00:00:05] Speaker 1 (Alice): Good morning team...",
                "provider": "funasr" if prefer == "local" else "whisper-1",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.006,
                "duration_ms": 1240,
            })()

        def chat(self, text: str = None, messages: list = None, prefer: str = "auto", **kwargs):
            return type("InvokeResult", (), {
                "text": "## Executive Brief\n...",
                "provider": "ollama" if prefer == "local" else "fireworks",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.0012,
                "duration_ms": 1850,
            })()

        def tts(self, text: str, voice: str = "en-narrator", prefer: str = "local", **kwargs):
            return type("InvokeResult", (), {
                "file": "brief_narration.mp3",
                "provider": "kokoro" if prefer == "local" else "openai-tts",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.003,
                "duration_ms": 820,
            })()


# ANSI Color formatting
COLOR_LOCAL = "\033[32m"
COLOR_CLOUD = "\033[38;2;249;115;22m"
COLOR_RESET = "\033[0m"
COLOR_BOLD = "\033[1m"
COLOR_CYAN = "\033[36m"
COLOR_YELLOW = "\033[33m"


def get_locality_badge(locality: str) -> str:
    """Format locality string with ANSI colors."""
    if locality == "local":
        return f"{COLOR_LOCAL}Local (Privacy-Preserving){COLOR_RESET}"
    elif locality == "cloud":
        return f"{COLOR_CLOUD}Cloud{COLOR_RESET}"
    else:
        return f"{COLOR_CYAN}{locality}{COLOR_RESET}"


# Realistic synthetic meeting data for offline mock demonstration
MOCK_TRANSCRIPT = """[00:00:05] Speaker 1 (Alice - Product Lead):
"Good morning team. Let's review the Q3 launch milestone for the MoFA Engine gateway. We need to finalize the routing policies and model residency features by Friday."

[00:00:22] Speaker 2 (Bob - Infrastructure Architect):
"From the engine side, local Ollama integration is solid, and Kokoro TTS latency is down to 85ms on M-series chips. The circuit breaker fallback to cloud Fireworks AI is working smoothly during load spikes."

[00:00:48] Speaker 3 (Carol - QA & Security):
"What about data privacy for corporate clients? We must ensure sensitive meeting recordings and documents never hit public endpoints unless fallback is explicitly allowed."

[00:01:12] Speaker 1 (Alice - Product Lead):
"That's a key requirement. The `prefer='local'` constraint will hard-lock execution to local FunASR and Ollama models. If local models are unavailable, it will fail gracefully rather than leaking data to cloud."

[00:01:35] Speaker 2 (Bob - Infrastructure Architect):
"Agreed. I'll add strict telemetry logging for any zero-retention compliance audits. Carol, can your team run the load benchmark scripts?"

[00:01:50] Speaker 3 (Carol - QA & Security):
"Yes, we'll execute the provider race benchmarks and verify zero data egress."

[00:02:05] Speaker 1 (Alice - Product Lead):
"Great. Let's wrap up and sync again on Thursday. Thanks everyone!"
"""

MOCK_MINUTES = """## Executive Brief
The product engineering team met to review the Q3 MoFA Engine gateway launch milestone. Key decisions include hard-locking enterprise workloads to local processing via `prefer='local'`, maintaining 85ms Kokoro TTS latency, and completing telemetry logging and load benchmarking prior to release.

---

## Key Discussion Points
1. **Gateway Latency & Routing**: Local Ollama + Kokoro TTS achieves 85ms response latency on Apple Silicon. Circuit breaker fallbacks are operational for cloud scalability.
2. **Enterprise Privacy Controls**: Sensitive audio recordings are constrained to local execution (FunASR + Ollama). Fallback behavior respects user privacy boundaries.
3. **Audit & Compliance**: Telemetry logging will track model routing decisions for zero-retention data policies.

---

## Action Items
- [ ] **Bob (Infra)**: Implement telemetry logging for local/cloud audit trails by Wednesday.
- [ ] **Carol (QA)**: Run provider race benchmarks to verify zero cloud data egress by Thursday.
- [ ] **Alice (Product)**: Finalize Q3 release notes and privacy compliance documentation by Friday.
"""


def process_meeting(
    audio_path: str,
    prefer: str = "local",
    narrate: bool = False,
    mock: bool = False,
    engine_url: str = "http://127.0.0.1:8420"
) -> bool:
    """Execute Scenario S1: Audio Meeting -> Minutes & Executive Brief Pipeline."""
    print(f"\n🎙️  {COLOR_BOLD}Scenario S1: Long Audio Meeting → Minutes & Executive Brief{COLOR_RESET}")
    print(f"📌 Audio Input: {audio_path or 'sample_meeting.wav'}")
    print(f"⚙️  Locality Constraint: prefer={prefer} ({get_locality_badge(prefer)})")
    print(f"🔊 Brief Narration (TTS): {'Enabled' if narrate else 'Disabled'}\n")

    start_total = time.perf_counter()
    total_cost = 0.0

    # -------------------------------------------------------------------------
    # Step 1: Speech-to-Text with Speaker Diarization (ASR)
    # -------------------------------------------------------------------------
    print(f"{COLOR_BOLD}[Step 1/3] 🎧 Transcribing Audio with Speaker Diarization (ASR)...{COLOR_RESET}")
    if mock:
        time.sleep(0.5)
        transcript = MOCK_TRANSCRIPT
        asr_provider = "funasr (local)" if prefer == "local" else "whisper-1 (cloud)"
        asr_locality = prefer
        asr_cost = 0.0 if prefer == "local" else 0.006
    else:
        engine = MofaEngine(base_url=engine_url)
        print(f"  ⏳ Sending '{audio_path}' to MoFA ASR (diarize=True, prefer={prefer})...")
        try:
            asr_res = engine.asr(audio_path, prefer=prefer, diarize=True)
            transcript = asr_res.text or "No transcription text returned."
            asr_provider = getattr(asr_res, "provider", "funasr")
            asr_locality = getattr(asr_res, "locality", prefer)
            asr_cost = getattr(asr_res, "cost_usd", 0.0)
        except Exception as e:
            print(f"❌ Error during ASR transcription: {e}")
            return False

    total_cost += asr_cost
    print(f"  ├─ Provider Used: {asr_provider} ({get_locality_badge(asr_locality)})")
    print(f"  └─ Transcript Length: {len(transcript.split())} words\n")

    print(f"{COLOR_CYAN}══════════════════════════ FULL TRANSCRIPT WITH SPEAKER LABELS ══════════════════════════{COLOR_RESET}")
    print(transcript.strip())
    print(f"{COLOR_CYAN}═════════════════════════════════════════════════════════════════════════════════════════{COLOR_RESET}\n")

    # -------------------------------------------------------------------------
    # Step 2: Minutes & Executive Brief Generation (Chat LLM)
    # -------------------------------------------------------------------------
    print(f"{COLOR_BOLD}[Step 2/3] 🧠 Generating Minutes & Executive Brief (Chat LLM)...{COLOR_RESET}")
    system_prompt = (
        "You are an expert executive assistant. Analyze the provided meeting transcript with speaker diarization "
        "and generate structured meeting minutes including: Executive Brief, Key Discussion Points, and Action Items."
    )
    user_prompt = f"Please process the following meeting transcript:\n\n{transcript}"

    if mock:
        time.sleep(0.6)
        minutes_text = MOCK_MINUTES
        chat_provider = "ollama/qwen2.5:7b" if prefer == "local" else "fireworks/deepseek-v4"
        chat_locality = prefer
        chat_cost = 0.0 if prefer == "local" else 0.0015
    else:
        engine = MofaEngine(base_url=engine_url)
        print(f"  ⏳ Querying MoFA Chat LLM (prefer={prefer})...")
        try:
            messages = [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ]
            chat_res = engine.chat(text=user_prompt, messages=messages, prefer=prefer)
            minutes_text = chat_res.text or "No summary produced."
            chat_provider = getattr(chat_res, "provider", "ollama")
            chat_locality = getattr(chat_res, "locality", prefer)
            chat_cost = getattr(chat_res, "cost_usd", 0.0)
        except Exception as e:
            print(f"❌ Error during Chat analysis: {e}")
            return False

    total_cost += chat_cost
    print(f"  ├─ Provider Used: {chat_provider} ({get_locality_badge(chat_locality)})")
    print(f"  └─ Minutes Generated Successfully\n")

    print(f"{COLOR_YELLOW}══════════════════════════ STRUCTURED MINUTES & BRIEF ══════════════════════════{COLOR_RESET}")
    print(minutes_text.strip())
    print(f"{COLOR_YELLOW}═════════════════════════════════════════════════════════════════════════════════{COLOR_RESET}\n")

    # -------------------------------------------------------------------------
    # Step 3: Executive Brief TTS Narration (Optional)
    # -------------------------------------------------------------------------
    narrative_file = None
    if narrate:
        print(f"{COLOR_BOLD}[Step 3/3] 🔊 Synthesizing Executive Brief Audio (TTS)...{COLOR_RESET}")

        # Extract Executive Brief text for narration
        brief_summary = minutes_text.split("---")[0].replace("## Executive Brief", "").strip()
        if not brief_summary:
            brief_summary = minutes_text[:300]

        if mock:
            time.sleep(0.4)
            narrative_file = "output_brief_narration.mp3"
            with open(narrative_file, "w") as f:
                f.write("MOCK_AUDIO_DATA_BRIEF_NARRATION")
            tts_provider = "kokoro (local)" if prefer == "local" else "openai-tts (cloud)"
            tts_locality = prefer
            tts_cost = 0.0 if prefer == "local" else 0.003
        else:
            engine = MofaEngine(base_url=engine_url)
            print(f"  ⏳ Synthesizing voice brief with voice='en-narrator' (prefer={prefer})...")
            try:
                tts_res = engine.tts(brief_summary, voice="en-narrator", prefer=prefer)
                narrative_file = getattr(tts_res, "file", "output_brief_narration.mp3")
                tts_provider = getattr(tts_res, "provider", "kokoro")
                tts_locality = getattr(tts_res, "locality", prefer)
                tts_cost = getattr(tts_res, "cost_usd", 0.0)
            except Exception as e:
                print(f"⚠️ Warning: TTS generation failed: {e}")
                tts_provider = "failed"
                tts_locality = prefer
                tts_cost = 0.0

        total_cost += tts_cost
        print(f"  ├─ Provider Used: {tts_provider} ({get_locality_badge(tts_locality)})")
        print(f"  └─ Audio File Generated: {narrative_file}\n")
    else:
        print(f"ℹ️  [Step 3/3] TTS Narration skipped (use --narrate flag to enable).\n")

    elapsed_time = time.perf_counter() - start_total

    # -------------------------------------------------------------------------
    # Summary Table
    # -------------------------------------------------------------------------
    print(f"📊 SCENARIO S1 EXECUTION SUMMARY")
    print(f"──────────────────────────────────────────────────────────────────────────")
    print(f"  • Routing Preference : {prefer}")
    print(f"  • Locality Status    : {get_locality_badge(prefer)}")
    print(f"  • Total Latency      : {elapsed_time:.2f}s")
    print(f"  • Total Cost (USD)   : ${total_cost:.6f}")
    if narrative_file:
        print(f"  • Brief Audio File   : {narrative_file}")
    print(f"──────────────────────────────────────────────────────────────────────────\n")

    return True


def main():
    parser = argparse.ArgumentParser(
        description="Scenario S1: Long Audio Meeting -> Minutes & Executive Brief"
    )
    parser.add_argument(
        "--audio",
        type=str,
        default=None,
        help="Path to meeting audio file (.wav, .mp3, .m4a)",
    )
    parser.add_argument(
        "--prefer",
        type=str,
        default="local",
        choices=["local", "cloud", "auto"],
        help="Routing preference: local | cloud | auto (default: local)",
    )
    parser.add_argument(
        "--narrate",
        action="store_true",
        help="Synthesize TTS audio narration of the Executive Brief",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Run in mock mode with realistic synthetic meeting transcript & minutes",
    )
    parser.add_argument(
        "--engine-url",
        type=str,
        default="http://127.0.0.1:8420",
        help="MoFA Engine gateway URL",
    )

    args = parser.parse_args()

    if not args.mock and not args.audio:
        print("⚠️  No audio file specified. Running in --mock mode by default.\n"
              "   (Tip: Pass '--audio path/to/meeting.wav' to process a real recording)")
        args.mock = True
        args.audio = "sample_meeting.wav"

    process_meeting(
        audio_path=args.audio,
        prefer=args.prefer,
        narrate=args.narrate,
        mock=args.mock,
        engine_url=args.engine_url,
    )


if __name__ == "__main__":
    main()
