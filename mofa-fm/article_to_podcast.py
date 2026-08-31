#!/usr/bin/env python3
"""S6 Podcast Studio: Article -> Multi-Voice Audio Podcast Episode (PRD v3.1 §2.2.1 S6).

Demonstrates fluent declarative pipeline composition with automatic preflight warmup:
  1. Chat (LLM Rewrite) -> Converts article text into a conversational 2-host podcast script
  2. Hint Propagation -> Automatically passes hint_next="tts" to warm up speech models
  3. TTS (Speech Synthesis) -> Synthesizes spoken audio episode (.mp3)

Usage:
  python3 mofa-fm/article_to_podcast.py
  python3 mofa-fm/article_to_podcast.py --article examples/samples/sample_article.txt
  mofa run podcast
"""

import argparse
import os
import sys
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mofa_sdk import MofaEngine, Pipeline

SAMPLE_ARTICLE = Path(__file__).parent / ".." / "examples" / "samples" / "sample_article.txt"


def main():
    parser = argparse.ArgumentParser(description="S6 Podcast Studio: Article -> Multi-Voice Audio Podcast")
    parser.add_argument("--article", default=str(SAMPLE_ARTICLE), help="Path to input article (.txt)")
    parser.add_argument("--voice", default="en-narrator", help="Voice alias (e.g. en-narrator, zh-female-1)")
    parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"], help="Routing locality preference")
    parser.add_argument("--out", default="output/podcast_episode.mp3", help="Output path for synthesized audio episode")
    args = parser.parse_args()

    article_path = Path(args.article)
    if article_path.exists():
        article_text = article_path.read_text(encoding="utf-8")
    else:
        article_text = (
            "Artificial intelligence is transforming software engineering. Local-first engines "
            "like MoFA enable developers to orchestrate Chat, Voice, and Vision models directly "
            "on workstation hardware with zero cloud inference cost."
        )

    print("\n==================================================================")
    print("   Scenario S6: Podcast Studio (Declarative Pipeline)")
    print("==================================================================")
    print(f"  Input Article : {article_path.name} ({len(article_text)} chars)")
    print(f"  Voice Alias   : {args.voice} · Locality: {args.prefer}")

    engine = MofaEngine()

    print("\nExecuting fluent Chat -> TTS pipeline with predictive preflight warmup...")
    result = (
        Pipeline(engine)
        .chat(
            "Rewrite this article into a natural, engaging 2-person spoken podcast dialogue. "
            "Keep the response under 250 words:\n\n{article}",
            hint_next="tts",
            prefer=args.prefer,
        )
        .tts(voice=args.voice, prefer=args.prefer)
        .run(article=article_text)
    )

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    result.steps[-1].save(str(out_path))

    print("\n==================================================================")
    print("   S6 Podcast Studio Pipeline Complete!")
    print("==================================================================")
    print(f"  Podcast Audio : {out_path}")
    print(f"  Total Latency : {result.total_duration_ms}ms")
    print(f"  Total Cost    : ${result.total_cost:.4f} ({'100% Local $0.00' if result.is_local else 'Cloud'})")
    print(f"  Steps Passed  : {len(result.steps)}")
    print()


if __name__ == "__main__":
    main()
