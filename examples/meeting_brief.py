#!/usr/bin/env python3
"""S1 Meeting Brief: Meeting Audio -> Minutes + 30s Audio Brief (PRD v3.1 §2.2.1 S1).

Executes a 3-step multimodal pipeline:
  1. ASR (Speech-to-Text) -> Transcribes meeting recording with speaker diarization
  2. Chat (LLM Extraction) -> Extracts Decisions, Action Items, and Risks
  3. TTS (Speech Synthesis) -> Generates a 30s executive audio brief (.mp3)

Usage:
  python3 examples/meeting_brief.py
  python3 examples/meeting_brief.py --audio path/to/recording.wav --prefer local
  mofa run meeting
"""

import argparse
import os
import sys
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine

SAMPLE_AUDIO = Path(__file__).parent / "samples" / "sample_meeting.wav"
SAMPLE_TRANSCRIPT = Path(__file__).parent / "samples" / "sample_transcript.txt"


def main():
    parser = argparse.ArgumentParser(description="S1 Meeting Brief: Audio -> Minutes + 30s Audio Brief")
    parser.add_argument("--audio", default=str(SAMPLE_AUDIO), help="Path to meeting audio recording")
    parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"], help="Routing locality preference")
    parser.add_argument("--out", default="output", help="Output directory for generated artifacts")
    args = parser.parse_args()

    engine = MofaEngine()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    print("\n==================================================================")
    print("   Scenario S1: Meeting Audio -> Minutes + Audio Brief")
    print("==================================================================")

    # ── Step 1: ASR (Speech-to-Text) ──────────────────────────────────
    print(f"\n[Step 1/3] ASR: Transcribing meeting audio ({args.audio})...")
    transcript = ""
    try:
        asr_res = engine.asr(args.audio, prefer=args.prefer)
        transcript = asr_res.text or ""
        print(f"  [OK] Transcribed ({asr_res.provider}, {asr_res.duration_ms}ms)")
    except Exception as e:
        print(f"  [FALLBACK] Gateway ASR service unavailable ({e}); reading sample transcript...")
        print("  [TIP] Run 'mofa doctor' to inspect ASR setup.")
        if SAMPLE_TRANSCRIPT.exists():
            transcript = SAMPLE_TRANSCRIPT.read_text(encoding="utf-8")
        else:
            transcript = "Speaker 1 (Alice): We must lock enterprise data to local models by Friday.\nSpeaker 2 (Bob): Agreed, Kokoro TTS achieves 85ms on Apple Silicon."

    # ── Step 2: Chat (Structured Minutes Extraction) ──────────────────
    print("\n[Step 2/3] Chat: Extracting structured meeting minutes (hint_next=tts)...")
    minutes_prompt = (
        "Extract executive meeting minutes from the following transcript. "
        "Include: 1. Key Decisions, 2. Action Items (with assignees), 3. Risks & Blockers. "
        "Format as clean Markdown:\n\n" + transcript
    )
    minutes_res = engine.chat(
        minutes_prompt,
        hint_next="tts",
        prefer=args.prefer,
    )
    minutes_path = out / "meeting_minutes.md"
    minutes_res.save(str(minutes_path))
    print(f"  [OK] Minutes saved to: {minutes_path}")
    print(f"  +- Routed to : {minutes_res.provider}/{minutes_res.model_used}")
    print(f"  +- Latency   : {minutes_res.duration_ms}ms · Cost: ${minutes_res.cost_usd:.4f}\n")
    
    print("┌" + "─" * 68 + "┐")
    print("│                     EXTRACTED MEETING MINUTES                      │")
    print("├" + "─" * 68 + "┤")
    for line in (minutes_res.text or "").split("\n"):
        print(f"  {line}")
    print("└" + "─" * 68 + "┘")

    # ── Step 3: TTS (30s Audio Brief) ─────────────────────────────────
    print("\n[Step 3/3] TTS: Synthesizing executive audio brief...")
    summary = (minutes_res.text or "")[:400]
    tts_model = "gemini-tts/gemini-2.5-flash-preview-tts" if args.prefer == "cloud" else None
    brief_res = engine.tts(summary, model=tts_model, voice="en-narrator", prefer=args.prefer)
    brief_path = out / "meeting_brief.mp3"
    brief_res.save(str(brief_path))
    print(f"  [OK] Audio brief saved to: {brief_path}")
    print(f"  +- Routed to : {brief_res.provider}/{brief_res.model_used}")
    print(f"  +- Latency   : {brief_res.duration_ms}ms · Cost: ${brief_res.cost_usd:.4f}")

    # ── Summary & Deliverables ───────────────────────────────────────
    total_cost = (minutes_res.cost_usd or 0.0) + (brief_res.cost_usd or 0.0)
    print("\n==================================================================")
    print("   S1 Meeting Brief Pipeline Complete!")
    print("==================================================================")
    print(f"  Total Cost     : ${total_cost:.4f} ({'100% Local $0.00' if minutes_res.is_local else 'Cloud'})")
    print(f"  Minutes File   : {minutes_path}")
    print(f"  Audio Brief    : {brief_path}")
    print()


if __name__ == "__main__":
    main()
