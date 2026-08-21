#!/usr/bin/env python3
"""S2 Code / PR Review Agent: Git Diff -> Deep Reasoning -> Review Report (PRD v3.1 §2.2.1 S2).

Demonstrates the Responses API deep-thinking stream:
  1. Accepts a unified git diff (.patch file or staged git changes)
  2. Streams thought-chain reasoning tokens in real-time (effort=high)
  3. Formats and saves structured review report with severity annotations (.md)

Usage:
  python3 examples/code_review.py
  python3 examples/code_review.py --staged
  python3 examples/code_review.py --diff path/to/patch.diff --effort high
  mofa run review
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine

SAMPLE_DIFF = Path(__file__).parent / "samples" / "sample_diff.patch"


def main():
    parser = argparse.ArgumentParser(description="S2 Code Review Agent: Git Diff -> Deep Reasoning -> Report")
    parser.add_argument("--diff", default=None, help="Path to .patch / .diff file to review")
    parser.add_argument("--staged", action="store_true", help="Review staged git changes directly from current repo")
    parser.add_argument("--effort", default="high", choices=["low", "medium", "high"], help="Reasoning effort tier")
    parser.add_argument("--out", default="output/code_review_report.md", help="Output file path for review report")
    args = parser.parse_args()

    # ── Resolve Diff Input ───────────────────────────────────────────
    diff_text = ""
    source_label = ""
    if args.staged:
        try:
            diff_text = subprocess.run(["git", "diff", "--cached"], capture_output=True, text=True).stdout
            source_label = "staged git changes"
        except Exception:
            diff_text = ""
    elif args.diff:
        diff_path = Path(args.diff)
        if diff_path.exists():
            diff_text = diff_path.read_text(encoding="utf-8")
            source_label = str(diff_path)
    
    if not diff_text.strip():
        if SAMPLE_DIFF.exists():
            diff_text = SAMPLE_DIFF.read_text(encoding="utf-8")
            source_label = f"sample diff ({SAMPLE_DIFF.name})"
        else:
            diff_text = "diff --git a/auth/jwt.py b/auth/jwt.py\n- exp = now + timedelta(hours=1)\n+ pass # Temporary bypass"
            source_label = "inline fallback diff"

    print("\n==================================================================")
    print("   Scenario S2: Code / PR Review Deep Thinking Agent")
    print("==================================================================")
    print(f"  Input Diff : {source_label} ({len(diff_text)} chars)")
    print(f"  Effort Tier: {args.effort}")
    print("\nStreaming deep thought chain and annotated review...\n")

    engine = MofaEngine()
    prompt = (
        "Perform a rigorous security and performance code review on the following git diff. "
        "Annotate issues with severity (BLOCKER / MAJOR / MINOR) and provide concrete fix suggestions:\n\n"
        + diff_text
    )

    report_chunks = []
    try:
        for ev in engine.responses(input=prompt, reasoning={"effort": args.effort}, stream=True):
            if ev.type == "reasoning":
                # Print thought-chain in dim/italic text
                print(f"\033[2m{ev.delta}\033[0m", end="", flush=True)
            else:
                report_chunks.append(ev.delta)
                print(ev.delta, end="", flush=True)
    except Exception as e:
        print(f"\n[INFO] Streaming responses fallback ({e}); requesting standard chat...")
        res = engine.chat(prompt, reasoning={"effort": args.effort})
        report_chunks.append(res.text or "")
        print(res.text or "")

    full_report = "".join(report_chunks)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(full_report, encoding="utf-8")

    print("\n\n==================================================================")
    print("   S2 Code Review Complete!")
    print("==================================================================")
    print(f"  Review Report Saved : {out_path} ({len(full_report)} chars)")
    print()


if __name__ == "__main__":
    main()
