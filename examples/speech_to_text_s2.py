#!/usr/bin/env python3
"""Scenario S2: Multilingual Speech-to-Text & Subtitle Generator."""
import sys
import os
from pathlib import Path

# Add mofa-fm to path
sys.path.insert(0, "mofa-fm")
from mofa_sdk import MofaEngine

import re

def generate_srt(transcript: str, duration_secs: float = 10.0, out_path: str = "subtitles.srt"):
    """Format transcript into SubRip (.srt) subtitle file."""
    clean_text = re.sub(r"\[\d{2}:\d{2}\.\d{3}\s*-->\s*\d{2}:\d{2}\.\d{3}\]", "", transcript)
    lines = [s.strip() for s in clean_text.split(".") if s.strip()]
    num_lines = max(1, len(lines))
    segment_duration = duration_secs / num_lines

    srt_content = ""
    for i, line in enumerate(lines):
        start_sec = i * segment_duration
        end_sec = (i + 1) * segment_duration
        
        start_time = f"00:00:{int(start_sec):02d},000"
        end_time = f"00:00:{int(end_sec):02d},000"

        srt_content += f"{i+1}\n{start_time} --> {end_time}\n{line}.\n\n"

    with open(out_path, "w") as f:
        f.write(srt_content)
    return out_path


def generate_vtt(transcript: str, duration_secs: float = 10.0, out_path: str = "subtitles.vtt"):
    """Format transcript into WebVTT (.vtt) subtitle file."""
    clean_text = re.sub(r"\[\d{2}:\d{2}\.\d{3}\s*-->\s*\d{2}:\d{2}\.\d{3}\]", "", transcript)
    lines = [s.strip() for s in clean_text.split(".") if s.strip()]
    num_lines = max(1, len(lines))
    segment_duration = duration_secs / num_lines

    vtt_content = "WEBVTT\n\n"
    for i, line in enumerate(lines):
        start_sec = i * segment_duration
        end_sec = (i + 1) * segment_duration

        start_time = f"00:00:{int(start_sec):02d}.000"
        end_time = f"00:00:{int(end_sec):02d}.000"

        vtt_content += f"{start_time} --> {end_time}\n{line}.\n\n"


        vtt_content += f"{start_time} --> {end_time}\n{line}.\n\n"

    with open(out_path, "w") as f:
        f.write(vtt_content)
    return out_path


def main():
    print("=" * 65)
    print("🎙️ Scenario S2: Speech-to-Text & Subtitle Generator")
    print("=" * 65)

    audio_file = "scenario_s1_response.mp3"
    if not os.path.exists(audio_file):
        audio_file = "narration.mp3"

    print(f"\n📂 Input Audio File: {audio_file}")
    print("\n⏳ [Step 1/3] Routing ASR transcription request to MoFA Engine...")
    engine = MofaEngine(base_url="http://127.0.0.1:8420")

    try:
        asr_res = engine.asr(audio_file)
        transcript = getattr(asr_res, "text", str(asr_res))
    except Exception as e:
        print(f"❌ ASR transcription failed: {e}")
        print("   Ensure the MoFA Engine is running: cargo run --release -p mofa-engine -- -c mofa_hybrid.toml")
        sys.exit(1)

    print(f"\n📝 Extracted Transcript:\n   \"{transcript}\"")

    # Step 2: Generate SRT Subtitle
    print("\n⏳ [Step 2/3] Generating SubRip (.srt) Subtitle File...")
    srt_file = generate_srt(transcript, duration_secs=10.0, out_path="output_subtitles.srt")
    print(f"     └─ Created SRT Subtitle: {srt_file}")

    # Step 3: Generate WebVTT Subtitle
    print("\n⏳ [Step 3/3] Generating WebVTT (.vtt) Subtitle File...")
    vtt_file = generate_vtt(transcript, duration_secs=10.0, out_path="output_subtitles.vtt")
    print(f"     └─ Created VTT Subtitle: {vtt_file}")

    print("\n" + "=" * 65)
    print("🎉 SCENARIO S2 SPEECH-TO-TEXT COMPLETED SUCCESSFULLY!")
    print(f"📁 Output Artifacts:\n   ├─ SRT: {os.path.abspath(srt_file)}\n   └─ VTT: {os.path.abspath(vtt_file)}")
    print("=" * 65)


if __name__ == "__main__":
    main()
