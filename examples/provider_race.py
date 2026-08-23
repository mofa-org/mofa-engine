#!/usr/bin/env python3
"""S7 Provider Race: Same Prompt -> Multi-Vendor Benchmark (PRD v3.1 §2.2.1 S7).

Demonstrates multi-vendor access and dual-track latency/cost benchmarking:
  1. Queries gateway capabilities to discover all active chat providers
  2. Sends identical test prompt concurrently or sequentially
  3. Displays comparative telemetry: Latency, Cost ($), and Locality ([LOCAL $0.00] vs [CLOUD])

Usage:
  python3 examples/01_provider_race.py
  mofa run race
"""

import os
import sys
import time
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine

GREEN = "\033[32m"
CYAN = "\033[36m"
YELLOW = "\033[33m"
BOLD = "\033[1m"
DIM = "\033[2m"
RESET = "\033[0m"


def main():
    engine = MofaEngine()
    prompt = "Explain quantum entanglement in exactly 2 concise sentences."

    print("\n==================================================================")
    print("   Scenario S7: Multi-Provider Race & Dual-Track Benchmark")
    print("==================================================================")
    print(f"  Benchmark Prompt : \"{prompt}\"")

    # Discover chat-capable providers from engine gateway
    try:
        caps = engine.capabilities()
    except Exception as e:
        print(f"\n[ERROR] Cannot reach MoFA Engine on port 8420 ({e})")
        print("[TIP]   Run 'mofa doctor' to inspect gateway services.")
        sys.exit(1)

    chat_providers = [c for c in caps if isinstance(c, dict) and c.get("capability") == "chat"]
    if not chat_providers:
        # If capabilities schema is simplified, query available models
        chat_providers = [{"provider": "ollama", "model": "gemma3:4b"}]

    print(f"  Active Providers : {len(chat_providers)} discovered\n")
    print(f"{'PROVIDER':<20} {'MODEL':<24} {'LATENCY':>10} {'COST':>12} {'LOCALITY':>12}")
    print("─" * 80)

    results = []
    seen = set()
    for cap in chat_providers:
        provider = cap.get("provider", "unknown")
        model_name = cap.get("name") or cap.get("model", "default")
        model_id = cap.get("id") or f"{provider}/{model_name}"
        
        if model_id in seen:
            continue
        seen.add(model_id)

        try:
            t0 = time.time()
            res = engine.chat(prompt, model=model_id)
            duration_ms = int((time.time() - t0) * 1000)
            loc_badge = f"{GREEN}LOCAL{RESET}" if res.is_local else f"{YELLOW}CLOUD{RESET}"
            cost_str = f"${res.cost_usd:.4f}"
            print(f"{provider:<20} {res.model_used:<24} {duration_ms:>8}ms {cost_str:>12} [{loc_badge}]")
            results.append({
                "provider": provider,
                "model": res.model_used,
                "ms": duration_ms,
                "cost": res.cost_usd,
                "is_local": res.is_local,
                "answer": (res.text or "").strip(),
            })
        except Exception as e:
            err_msg = str(e).split("\n")[0][:30]
            print(f"{provider:<20} {model_name:<24} {'FAILED':>10} {'—':>12} {DIM}[{err_msg}]{RESET}")

    if results:
        local_runs = [r for r in results if r["is_local"]]
        cloud_runs = [r for r in results if not r["is_local"]]
        print("\n==================================================================")
        print("   Dual-Track Benchmark Results")
        print("==================================================================")
        if local_runs:
            fastest_local = min(local_runs, key=lambda x: x["ms"])
            print(f"  Fastest Local Provider : {fastest_local['provider']}/{fastest_local['model']} ({fastest_local['ms']}ms, ${fastest_local['cost']:.4f})")
        if cloud_runs:
            fastest_cloud = min(cloud_runs, key=lambda x: x["ms"])
            print(f"  Fastest Cloud Provider : {fastest_cloud['provider']}/{fastest_cloud['model']} ({fastest_cloud['ms']}ms, ${fastest_cloud['cost']:.4f})")

        print("\n==================================================================")
        print("   Generated Model Answers (Comparison)")
        print("==================================================================")
        for r in results:
            loc_tag = f"{GREEN}[LOCAL]{RESET}" if r["is_local"] else f"{YELLOW}[CLOUD]{RESET}"
            print(f"\n{BOLD}▶ {r['provider']}/{r['model']}{RESET} {loc_tag} ({r['ms']}ms):")
            print(f"  \"{r['answer']}\"")
        print()

        # Save to output/provider_race.md
        out_dir = Path("output")
        out_dir.mkdir(parents=True, exist_ok=True)
        report_lines = [
            "# 🏎️ S7 Provider Race & Dual-Track Benchmark Report",
            f"\n**Prompt:** *\"{prompt}\"*",
            "\n## Telemetry Comparison\n",
            "| Provider | Model | Latency (ms) | Cost ($) | Locality |",
            "| :--- | :--- | :--- | :--- | :--- |",
        ]
        for r in results:
            loc = "Local ($0.00)" if r["is_local"] else f"Cloud (${r['cost']:.6f})"
            report_lines.append(f"| {r['provider']} | {r['model']} | {r['ms']}ms | ${r['cost']:.6f} | {loc} |")
        
        report_lines.append("\n## Generated Answers\n")
        for r in results:
            report_lines.append(f"### {r['provider']}/{r['model']} ({'Local' if r['is_local'] else 'Cloud'}, {r['ms']}ms)")
            report_lines.append(f"> {r['answer']}\n")

        (out_dir / "provider_race.md").write_text("\n".join(report_lines), encoding="utf-8")
        print(f"  💾 Full Benchmark Report Saved : output/provider_race.md\n")


if __name__ == "__main__":
    main()
