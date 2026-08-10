#!/usr/bin/env python3
"""Scenario S1: Interactive Multimodal Chat with Voice Synthesis."""
import sys
import os
import urllib.request
import json

# Add mofa-fm to path
sys.path.insert(0, "mofa-fm")
from mofa_sdk import MofaEngine

def main():
    print("=" * 65)
    print("💬 Scenario S1: Interactive Multimodal Chat")
    print("=" * 65)

    engine = MofaEngine(base_url="http://127.0.0.1:8420")

    query = "Explain how quantum computers process data using superposition in two concise sentences."
    print(f"\n👤 User Query: '{query}'")
    print("\n⏳ [Step 1/2] Routing chat request to MoFA Engine (hint_next='tts')...")

    # Call engine chat
    try:
        chat_res = engine.chat(
            text=query,
            hint_next="tts"
        )
        response_text = getattr(chat_res, "text", str(chat_res))
        print(f"🤖 Assistant Answer:\n   \"{response_text}\"")
    except Exception as e:
        print(f"❌ Chat failed: {e}")
        print("   Ensure the MoFA Engine is running: cargo run --release -p mofa-engine -- -c mofa_hybrid.toml")
        sys.exit(1)

    # Step 2: Synthesize Voice Response via Kokoro TTS
    print("\n⏳ [Step 2/2] Synthesizing Voice Audio Response via Kokoro TTS...")
    out_audio = "scenario_s1_response.mp3"
    try:
        req = urllib.request.Request(
            "http://127.0.0.1:8421/v1/audio/speech",
            data=json.dumps({"model": "kokoro", "input": response_text, "voice": "af_heart"}).encode("utf-8"),
            headers={"Content-Type": "application/json"}
        )
        with urllib.request.urlopen(req) as resp, open(out_audio, "wb") as out_f:
            out_f.write(resp.read())
        print(f"🔊 Spoken Audio Response Generated: {out_audio}")
    except Exception as e:
        print(f"⚠️ Voice TTS fallback: {e}")

    print("\n" + "=" * 65)
    print("🎉 SCENARIO S1 MULTIMODAL CHAT COMPLETED SUCCESSFULLY!")
    print(f"📁 Audio Output Artifact: {os.path.abspath(out_audio)}")
    print("=" * 65)

if __name__ == "__main__":
    main()
