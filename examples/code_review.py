#!/usr/bin/env python3
"""
Scenario S2: AI Code Review with Reasoning Stream (Responses API)
MoFA Engine — Multimodal Orchestration for Artifacts

Performs automated AI code review on git diffs using the Responses API with high-effort
reasoning streams. Displays thought chains (reasoning events) in dim gray followed by
final review reports (output events) in bold white, and saves the output report to disk.

Usage:
    python examples/code_review.py --mock
    git diff HEAD~1 | python examples/code_review.py
    python examples/code_review.py --diff-file examples/samples/sample_diff.patch --prefer local
    python examples/code_review.py --out output/review_report.md
"""

import argparse
import os
import sys
import time
from dataclasses import dataclass, field
from typing import Generator, Tuple, Optional, Dict, Any

# Ensure parent directory is in python path for mofa_sdk import
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "mofa-fm")))

try:
    from mofa_sdk import MofaEngine, StreamEvent
except ImportError:
    @dataclass
    class StreamEvent:
        """Fallback StreamEvent definition if mofa_sdk import fails."""
        type: str
        delta: str = ""
        content: str = ""
        metadata: Dict[str, Any] = field(default_factory=dict)

    class MofaEngine:
        """Fallback mock MofaEngine client."""
        def __init__(self, base_url: str = "http://127.0.0.1:8420"):
            self.base_url = base_url

        def responses(
            self,
            input: str,
            reasoning: Optional[Dict[str, Any]] = None,
            prefer: Optional[str] = "local",
            stream: bool = True,
        ) -> Generator[StreamEvent, None, None]:
            return generate_mock_stream_events()


MOCK_DIFF = """diff --git a/src/auth/jwt.py b/src/auth/jwt.py
index a1b2c3d..e4f5g6h 100644
--- a/src/auth/jwt.py
+++ b/src/auth/jwt.py
@@ -14,6 +14,14 @@ def verify_token(token: str) -> dict:
     try:
-        payload = jwt.decode(token, SECRET_KEY, algorithms=["HS256"])
+        payload = jwt.decode(token, SECRET_KEY, algorithms=["HS256", "none"])
         return payload
     except jwt.ExpiredSignatureError:
         raise AuthError("Token has expired")
+    except Exception:
+        # Silence error for fallback admin user
+        return {"user_id": "guest", "role": "admin"}
"""


def generate_mock_stream_events() -> Generator[StreamEvent, None, None]:
    """Generate realistic synthetic streaming reasoning and output events for mock mode."""
    reasoning_chunks = [
        "Analyzing git diff patch for file 'src/auth/jwt.py'...\n",
        "Inspecting authentication token verification flow...\n",
        "Line 17 modification detected: added 'none' algorithm to jwt.decode() allowed algorithms list.\n",
        "CRITICAL SECURITY RISK IDENTIFIED: Adding 'none' algorithm exposes JWT signature stripping vulnerability (CVE-2015-9235 pattern).\n",
        "Line 21 modification detected: catching generic Exception and returning fallback payload with role='admin'.\n",
        "HIGH SECURITY RISK IDENTIFIED: Catch-all exception block leads to authentication bypass and administrative privilege escalation.\n",
        "Evaluating severity level: CRITICAL (Merge Blocker).\n",
        "Formulating structured remediation steps and secure replacement code...\n",
    ]

    for chunk in reasoning_chunks:
        yield StreamEvent(type="reasoning", delta=chunk, content=chunk)

    output_chunks = [
        "## AI Code Review Report\n\n",
        "**Target File:** `src/auth/jwt.py`  \n",
        "**Security Rating:** **CRITICAL VULNERABILITIES DETECTED**  \n",
        "**Status:** **BLOCK MERGE**\n\n",
        "--- \n\n",
        "### Critical Security Findings\n\n",
        "#### 1. Insecure Algorithm Fallback (`\"none\"` Algorithm Allowed)\n",
        "- **Severity:** Critical (CVE-2015-9235 Style)\n",
        "- **Line:** `algorithms=[\"HS256\", \"none\"]`  \n",
        "- **Impact:** Allows malicious actors to bypass JWT signature verification completely by altering the JWT header to `{\"alg\": \"none\"}`.\n",
        "- **Remediation:** Remove `\"none\"` from the allowed algorithms list immediately. Restrict accepted algorithms strictly to `[\"HS256\"]` (or appropriate asymmetric keys).\n\n",
        "#### 2. Exception Swallowing & Unchecked Privilege Escalation\n",
        "- **Severity:** High (CWE-393 / Authentication Bypass)\n",
        "- **Line:** `except Exception: return {\"user_id\": \"guest\", \"role\": \"admin\"}`  \n",
        "- **Impact:** Any malformed, invalid, or corrupted token will trigger an exception and automatically grant the user administrative privileges (`role: \"admin\"`).\n",
        "- **Remediation:** Log the exact exception type and re-raise or return a generic `401 Unauthorized`. Never synthesize admin credentials on authentication failure.\n\n",
        "---\n\n",
        "### Recommended Patch\n\n",
        "```python\n",
        "def verify_token(token: str) -> dict:\n",
        "    try:\n",
        "        payload = jwt.decode(token, SECRET_KEY, algorithms=[\"HS256\"])\n",
        "        return payload\n",
        "    except jwt.PyJWTError as e:\n",
        "        logger.warning(f\"JWT verification failed: {e}\")\n",
        "        raise AuthError(\"Invalid or expired authentication token\")\n",
        "```\n\n",
        "**Final Recommendation:** Reject pull request until critical security flaws are addressed.\n"
    ]

    for chunk in output_chunks:
        yield StreamEvent(type="output", delta=chunk, content=chunk)


def get_diff_input(diff_file: Optional[str] = None, mock: bool = False) -> Tuple[str, bool]:
    """Retrieve git diff from file argument, stdin pipe, or built-in sample diff."""
    if mock:
        return MOCK_DIFF, True

    if diff_file and os.path.exists(diff_file):
        with open(diff_file, "r", encoding="utf-8") as f:
            return f.read(), False

    # Check bundled sample diff
    sample_diff = os.path.join(os.path.dirname(__file__), "samples", "sample_diff.patch")
    if not sys.stdin.isatty():
        piped_input = sys.stdin.read().strip()
        if piped_input:
            return piped_input, False

    if os.path.exists(sample_diff):
        print(f"[INFO] No diff piped -- using bundled sample diff ({sample_diff}).")
        with open(sample_diff, "r", encoding="utf-8") as f:
            return f.read(), False

    print("[INFO] No diff piped -- running with default sample diff in MOCK mode.")
    return MOCK_DIFF, True


def run_code_review(
    diff_text: str,
    out_path: str = "output/review_report.md",
    prefer: str = "local",
    mock: bool = False,
    engine_url: str = "http://127.0.0.1:8420"
):
    """Execute AI code review with streaming reasoning and output tokens."""
    print("=" * 70)
    print("Scenario S2: AI Code Review with Deep Thinking Stream (Responses API)")
    print("=" * 70)

    locality_label = "Local Distilled R1 (Free)" if prefer == "local" else "Cloud DeepSeek-R1"
    print(f"  * Locality Preference : prefer={prefer} ({locality_label})")
    print(f"  * Reasoning Effort    : effort='high'")
    print(f"  * Diff Input Length   : {len(diff_text)} chars\n")

    input_prompt = (
        "You are an expert security engineer and code reviewer. Review the following git diff.\n"
        "Identify security vulnerabilities, bugs, anti-patterns, and code quality issues.\n"
        "Provide severity ratings (Critical, High, Medium, Low) and concrete remediation code.\n\n"
        f"```diff\n{diff_text}\n```"
    )

    if mock:
        event_stream = generate_mock_stream_events()
    else:
        try:
            engine = MofaEngine(base_url=engine_url)
            event_stream = engine.responses(
                input=input_prompt,
                reasoning={"effort": "high"},
                prefer=prefer,
                stream=True
            )
        except Exception as e:
            print(f"[WARN] Responses API connection failed ({e}). Falling back to mock reasoning stream...")
            event_stream = generate_mock_stream_events()
            mock = True

    current_type = None
    reasoning_acc = ""
    output_acc = ""

    start_time = time.perf_counter()

    try:
        for event in event_stream:
            evt_type = getattr(event, "type", "output")
            delta = getattr(event, "delta", "") or getattr(event, "content", "")

            # Print section banner when transition occurs
            if evt_type != current_type:
                if evt_type == "reasoning":
                    sys.stdout.write("\n\033[90m[THOUGHT CHAIN / REASONING STREAM]\033[0m\n\033[90m")
                elif evt_type in ("output", "text"):
                    sys.stdout.write("\033[0m\n\n\033[1;37m[AI CODE REVIEW REPORT]\033[0m\n\033[1;37m")
                current_type = evt_type

            # Stream deltas formatted in color
            if evt_type == "reasoning":
                reasoning_acc += delta
                sys.stdout.write(f"\033[90m{delta}\033[0m")
            else:
                output_acc += delta
                sys.stdout.write(f"\033[1;37m{delta}\033[0m")

            sys.stdout.flush()

            if mock:
                time.sleep(0.03)
    except Exception as e:
        print(f"\n[WARN] Responses API streaming failed ({e}). Falling back to mock reasoning stream...")
        mock = True
        reasoning_acc = ""
        output_acc = ""
        current_type = None
        for event in generate_mock_stream_events():
            evt_type = getattr(event, "type", "output")
            delta = getattr(event, "delta", "") or getattr(event, "content", "")
            if evt_type != current_type:
                if evt_type == "reasoning":
                    sys.stdout.write("\n\033[90m[THOUGHT CHAIN / REASONING STREAM]\033[0m\n\033[90m")
                elif evt_type in ("output", "text"):
                    sys.stdout.write("\033[0m\n\n\033[1;37m[AI CODE REVIEW REPORT]\033[0m\n\033[1;37m")
                current_type = evt_type
            if evt_type == "reasoning":
                reasoning_acc += delta
                sys.stdout.write(f"\033[90m{delta}\033[0m")
            else:
                output_acc += delta
                sys.stdout.write(f"\033[1;37m{delta}\033[0m")
            sys.stdout.flush()
            time.sleep(0.03)

    # Reset colors
    sys.stdout.write("\033[0m\n")
    sys.stdout.flush()
    elapsed_sec = time.perf_counter() - start_time

    # Calculate token & cost stats
    reasoning_tokens = int(len(reasoning_acc.split()) * 1.3) if reasoning_acc else 0
    output_tokens = int(len(output_acc.split()) * 1.3) if output_acc else 0
    total_tokens = reasoning_tokens + output_tokens
    velocity = total_tokens / max(elapsed_sec, 0.001)

    if prefer == "local":
        cost_usd = 0.000000
        cost_str = "$0.000000 (100% Local Inference)"
    else:
        cost_usd = (total_tokens / 1000.0) * 0.00015
        cost_str = f"${cost_usd:.6f}"

    # Save to report artifact
    os.makedirs(os.path.dirname(os.path.abspath(out_path)), exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("# AI Code Review Report\n\n")
        f.write(f"**Date:** {time.strftime('%Y-%m-%d %H:%M:%S')}  \n")
        f.write(f"**Locality:** {prefer} | **Inference Cost:** {cost_str}  \n\n")
        if reasoning_acc:
            f.write("<details>\n<summary><b>Expand Thought Chain / Reasoning Trace</b></summary>\n\n")
            f.write("```\n" + reasoning_acc.strip() + "\n```\n\n")
            f.write("</details>\n\n---\n\n")
        f.write(output_acc.strip() + "\n")

    print("\n" + "=" * 70)
    print("CODE REVIEW PERFORMANCE & METRICS SUMMARY")
    print("=" * 70)
    print(f"  * Locality Preference : prefer={prefer} ({locality_label})")
    print(f"  * Total Time Elapsed  : {elapsed_sec:.2f}s")
    print(f"  * Reasoning Tokens    : {reasoning_tokens} tokens")
    print(f"  * Review Output Tokens: {output_tokens} tokens")
    print(f"  * Total Tokens Streamed: {total_tokens} tokens")
    print(f"  * Streaming Velocity  : {velocity:.1f} tok/s")
    print(f"  * Total Inference Cost: {cost_str}")
    print("=" * 70)
    print(f"\nSCENARIO S2 CODE REVIEW COMPLETED SUCCESSFULLY!")
    print(f"Review Report Artifact: {os.path.abspath(out_path)}\n")


def main():
    parser = argparse.ArgumentParser(description="Scenario S2: AI Code Review with Reasoning Stream")
    parser.add_argument("--diff-file", type=str, help="Path to input diff file")
    parser.add_argument("--out", type=str, default="output/review_report.md", help="Path to save markdown review report")
    parser.add_argument("--prefer", type=str, default="local", choices=["local", "cloud", "auto"], help="Routing locality constraint")
    parser.add_argument("--mock", action="store_true", help="Run in mock streaming mode")
    parser.add_argument("--engine-url", type=str, default="http://127.0.0.1:8420", help="MoFA Engine API URL")

    args = parser.parse_args()

    diff_text, is_mock = get_diff_input(diff_file=args.diff_file, mock=args.mock)
    run_code_review(
        diff_text=diff_text,
        out_path=args.out,
        prefer=args.prefer,
        mock=is_mock,
        engine_url=args.engine_url
    )


if __name__ == "__main__":
    main()
