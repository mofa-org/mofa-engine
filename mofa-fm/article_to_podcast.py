#!/usr/bin/env python3
"""mofa-fm: Article → Chinese Podcast via MoFA Engine.

Demonstrates the engine's multi-provider routing and pipeline capability:
1. LLM translates English article to Chinese podcast script
2. TTS synthesizes the script into an audio file
"""

import sys, os, shutil
sys.path.insert(0, os.path.dirname(__file__))
from mofa_sdk import MofaEngine

ARTICLE = """
Artificial intelligence is transforming how we build software. Large language models
can now write code, debug issues, and even architect entire systems. But the real
revolution isn't in replacing developers — it's in augmenting them. Tools like
Claude Code let developers work at a higher level of abstraction, focusing on what
to build rather than how to build it.
""".strip()

def main():
    engine = MofaEngine()
    print("=" * 60)
    print("  mofa-fm: Article → Chinese Podcast")
    print("=" * 60)

    h = engine.health()
    print(f"\n  Engine: {h['status']} (uptime {h['uptime_secs']}s)")

    caps = engine.capabilities()
    providers = set(m["provider"] for m in caps)
    print(f"  Models: {len(caps)} across {len(providers)} providers ({', '.join(sorted(providers))})")

    article = " ".join(sys.argv[1:]) if len(sys.argv) > 1 else ARTICLE

    print(f"\n1. Translating to Chinese podcast script...")
    r1 = engine.chat(
        article,
        messages=[
            {"role": "system", "content": "Rewrite this as a natural, engaging Chinese podcast script. Conversational tone. Under 200 chars. Output ONLY the Chinese text."},
            {"role": "user", "content": article},
        ],
        hint_next="tts",
    )
    print(f"   [{r1.provider}/{r1.model_used}] {r1.duration_ms}ms")
    print(f"   Script: {(r1.text or '')[:80]}...")

    print(f"\n2. Synthesizing speech...")
    r2 = engine.tts(r1.text or "人工智能正在改变软件开发")
    print(f"   [{r2.provider}/{r2.model_used}] {r2.duration_ms}ms")

    if r2.file and os.path.exists(r2.file):
        out = os.path.expanduser("~/Desktop/mofa_podcast.mp3")
        shutil.copy2(r2.file, out)
        size = os.path.getsize(out)
        print(f"   Audio: {out} ({size:,} bytes)")
    else:
        print(f"   Audio file: {r2.file}")

    print(f"\n{'='*60}")
    print(f"  Done! Total pipeline: {r1.duration_ms + r2.duration_ms}ms")
    print(f"{'='*60}")

if __name__ == "__main__":
    main()
