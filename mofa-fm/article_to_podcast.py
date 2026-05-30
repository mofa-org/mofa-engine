#!/usr/bin/env python3
"""mofa-fm: Article to Podcast demo using MoFA Engine."""

import requests
import sys
import json
import os
import subprocess

ENGINE = "http://127.0.0.1:8420"

ARTICLE = """
Artificial intelligence is transforming how we build software. Large language models
can now write code, debug issues, and even architect entire systems. But the real
revolution isn't in replacing developers — it's in augmenting them. Tools like
Claude Code let developers work at a higher level of abstraction, focusing on what
to build rather than how to build it. The future of software development is a
partnership between human creativity and AI capability.
"""

def check_engine():
    try:
        r = requests.get(f"{ENGINE}/health", timeout=5)
        r.raise_for_status()
        print("✅ Engine is healthy")
        return True
    except Exception as e:
        print(f"❌ Engine not available: {e}")
        return False

def show_capabilities():
    r = requests.get(f"{ENGINE}/v1/capabilities")
    caps = r.json()
    print(f"\n📋 Available models ({len(caps['models'])}):")
    for m in caps["models"]:
        print(f"   {m['name']:20s} | {m['model_type']:5s} | {m['backend']:8s} | {m['status']}")

def article_to_podcast(article: str) -> str:
    print("\n🔄 Step 1: Translating & rewriting to Chinese spoken style...")
    resp = requests.post(f"{ENGINE}/v1/run", json={
        "type": "llm",
        "input": {
            "text": article,
            "prompt": "You are a podcast script writer. Take this English article and rewrite it as a natural, engaging Chinese podcast script. Use conversational tone. Keep it concise (under 200 characters). Output ONLY the Chinese script, nothing else."
        },
        "hint": {"next": "tts"}
    }, timeout=120)
    resp.raise_for_status()
    result = resp.json()
    chinese_text = result["output"]["text"]
    print(f"   Model: {result['model_used']} ({result['backend']}) in {result['duration_ms']}ms")
    print(f"   Script: {chinese_text[:100]}...")

    print("\n🔄 Step 2: Generating speech with TTS...")
    resp = requests.post(f"{ENGINE}/v1/run", json={
        "type": "tts",
        "input": {"text": chinese_text}
    }, timeout=120)
    resp.raise_for_status()
    result = resp.json()
    audio_path = result["output"]["file"]
    print(f"   Model: {result['model_used']} ({result['backend']}) in {result['duration_ms']}ms")
    print(f"   Audio: {audio_path}")

    # Copy to a more accessible location
    out_path = os.path.expanduser("~/Desktop/mofa_podcast.mp3")
    import shutil
    shutil.copy2(audio_path, out_path)
    print(f"\n🎧 Podcast saved to: {out_path}")
    
    return out_path

def main():
    print("=" * 60)
    print("  mofa-fm: Article → Chinese Podcast")
    print("  Powered by MoFA Engine")
    print("=" * 60)

    if not check_engine():
        sys.exit(1)

    show_capabilities()

    article = ARTICLE.strip()
    if len(sys.argv) > 1:
        article = " ".join(sys.argv[1:])

    audio = article_to_podcast(article)

    print("\n" + "=" * 60)
    print("  ✅ Pipeline complete!")
    print("=" * 60)

if __name__ == "__main__":
    main()
