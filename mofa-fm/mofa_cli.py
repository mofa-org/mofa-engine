#!/usr/bin/env python3
"""MoFA Engine — Unified CLI Entrypoint.

Single command interface for all consumer-facing MoFA operations.

Usage:
  mofa doctor                          # Diagnose environment
  mofa demo                            # 30-second multimodal golden path demo
  mofa status                          # Engine health + loaded models
  mofa models                          # List all discovered models
  mofa cost                            # Session cost summary

  mofa run meeting  --audio file.wav   # S1 Meeting Brief
  mofa run review   --staged           # S2 Code Review
  mofa run doc      --image photo.png  # S3 Document AI
  mofa run video    --topic "..."      # S4 Explainer Video
  mofa run podcast  --url "..."        # S6 Podcast
  mofa run race                        # S7 Provider Race

  mofa chat                            # Interactive terminal chat
"""

import os
import sys
import argparse
import importlib.util
from pathlib import Path

# Resolve project root and ensure SDK is importable
SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent if SCRIPT_DIR.name == "mofa-fm" else SCRIPT_DIR
SDK_DIR = PROJECT_ROOT / "mofa-fm"
EXAMPLES_DIR = PROJECT_ROOT / "examples"

sys.path.insert(0, str(SDK_DIR))

# ANSI styling
GREEN = "\033[32m"
CYAN = "\033[36m"
YELLOW = "\033[33m"
BOLD = "\033[1m"
RESET = "\033[0m"


def cmd_doctor(args):
    """Run environment diagnostic."""
    spec = importlib.util.spec_from_file_location("mofa_doctor", SDK_DIR / "mofa_doctor.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    sys.exit(mod.run_doctor())


def cmd_demo(args):
    """Run the 30-second multimodal golden path demo."""
    demo_script = EXAMPLES_DIR / "quickstart_demo.py"
    if demo_script.exists():
        os.execvp(sys.executable, [sys.executable, str(demo_script)])
    else:
        print(f"{YELLOW}[ERROR]{RESET} Demo script not found at {demo_script}")
        sys.exit(1)


def cmd_status(args):
    """Show engine health, uptime, and provider statuses."""
    try:
        from mofa_sdk import MofaEngine
        engine = MofaEngine()
        health = engine.health()
        status = engine.status()

        print(f"\n{BOLD}{CYAN}MoFA Engine Status{RESET}\n")
        print(f"  Health    : {GREEN}{health.get('status', 'unknown')}{RESET}")
        print(f"  Uptime    : {health.get('uptime_seconds', '?')}s")
        print(f"  Providers : {status.get('providers_online', '?')} online")

        caps = engine.capabilities()
        if caps:
            print(f"\n  {BOLD}Active Capabilities:{RESET}")
            for cap in caps:
                if isinstance(cap, dict):
                    print(f"    +- {cap.get('capability', '?')} via {cap.get('provider', '?')}/{cap.get('model', '?')}")
                else:
                    print(f"    +- {cap}")
        print()
    except Exception as e:
        print(f"\n  {YELLOW}[OFFLINE]{RESET} Cannot reach engine on :8420 ({e})")
        print(f"  {CYAN}[TIP]{RESET}    Run: ./quickstart.sh\n")
        sys.exit(1)


def cmd_models(args):
    """List all discovered models and their capabilities."""
    try:
        from mofa_sdk import MofaEngine
        engine = MofaEngine()
        caps = engine.capabilities()

        print(f"\n{BOLD}{CYAN}Discovered Models{RESET}\n")
        if not caps:
            print(f"  {YELLOW}No models discovered.{RESET}")
            print(f"  {CYAN}[TIP]{RESET} Pull a model: ollama pull qwen2.5:1.5b\n")
            return

        for cap in caps:
            if isinstance(cap, dict):
                locality = cap.get("residency", cap.get("locality", "?"))
                badge = f"{GREEN}LOCAL{RESET}" if "local" in str(locality).lower() else f"{YELLOW}CLOUD{RESET}"
                print(f"  [{badge}] {BOLD}{cap.get('model', '?')}{RESET}")
                print(f"         Provider: {cap.get('provider', '?')} | Capability: {cap.get('capability', '?')}")
            else:
                print(f"  +- {cap}")
        print()
    except Exception as e:
        print(f"\n  {YELLOW}[OFFLINE]{RESET} Cannot reach engine ({e})\n")
        sys.exit(1)


def cmd_cost(args):
    """Show accumulated session cost."""
    try:
        from mofa_sdk import MofaEngine
        engine = MofaEngine()
        cost = engine.cost()

        print(f"\n{BOLD}{CYAN}Cost Summary{RESET}\n")
        total = cost.get("total_cost_usd", 0.0)
        local = cost.get("local_requests", 0)
        cloud = cost.get("cloud_requests", 0)
        print(f"  Total Cost   : {GREEN}${total:.6f}{RESET}")
        print(f"  Local Reqs   : {local}")
        print(f"  Cloud Reqs   : {cloud}")
        saved = cost.get("estimated_savings_usd", 0.0)
        if saved > 0:
            print(f"  Est. Savings : ${saved:.4f} (vs cloud-only)")
        print()
    except Exception as e:
        print(f"\n  {YELLOW}[OFFLINE]{RESET} Cannot reach engine ({e})\n")
        sys.exit(1)


def cmd_chat(args):
    """Interactive terminal chat session with the MoFA engine."""
    try:
        from mofa_sdk import MofaEngine
        engine = MofaEngine()
        engine.health()
    except Exception as e:
        print(f"\n  {YELLOW}[OFFLINE]{RESET} Cannot reach engine ({e})")
        print(f"  {CYAN}[TIP]{RESET}    Run: ./quickstart.sh\n")
        sys.exit(1)

    print(f"\n{BOLD}{CYAN}MoFA Interactive Chat{RESET}")
    print(f"Type your message and press Enter. Type 'quit' or Ctrl+C to exit.\n")

    prefer = args.prefer if hasattr(args, "prefer") else "local"
    session_cost = 0.0

    while True:
        try:
            user_input = input(f"{BOLD}You:{RESET} ").strip()
        except (KeyboardInterrupt, EOFError):
            print(f"\n\n{CYAN}Session cost: ${session_cost:.6f}{RESET}\n")
            break

        if not user_input or user_input.lower() in ("quit", "exit", "q"):
            print(f"\n{CYAN}Session cost: ${session_cost:.6f}{RESET}\n")
            break

        try:
            res = engine.chat(user_input, prefer=prefer)
            session_cost += res.cost_usd
            locality_badge = f"{GREEN}LOCAL{RESET}" if res.is_local else f"{YELLOW}CLOUD{RESET}"
            print(f"\n{BOLD}MoFA [{locality_badge}] ({res.provider}/{res.model_used}, {res.duration_ms}ms):{RESET}")
            print(f"{res.text}\n")
        except Exception as e:
            print(f"\n{YELLOW}[ERROR]{RESET} {e}\n")


# ── Scenario Runners ─────────────────────────────────────────────────

SCENARIO_MAP = {
    "meeting": ("examples/meeting_brief.py", "S1 Meeting Brief"),
    "s1": ("examples/meeting_brief.py", "S1 Meeting Brief"),
    "review": ("examples/code_review.py", "S2 Code Review"),
    "s2": ("examples/code_review.py", "S2 Code Review"),
    "doc": ("examples/doc_ai.py", "S3 Document AI"),
    "s3": ("examples/doc_ai.py", "S3 Document AI"),
    "video": ("examples/explainer_video.py", "S4 Explainer Video"),
    "s4": ("examples/explainer_video.py", "S4 Explainer Video"),
    "privacy": ("examples/meeting_brief.py", "S5 Privacy Moat (Local Meeting)"),
    "s5": ("examples/meeting_brief.py", "S5 Privacy Moat (Local Meeting)"),
    "podcast": ("mofa-fm/article_to_podcast.py", "S6 Podcast Studio"),
    "s6": ("mofa-fm/article_to_podcast.py", "S6 Podcast Studio"),
    "race": ("examples/01_provider_race.py", "S7 Provider Race"),
    "s7": ("examples/01_provider_race.py", "S7 Provider Race"),
}


def cmd_run(args):
    """Run a named scenario."""
    scenario_key = args.scenario.lower()
    if scenario_key not in SCENARIO_MAP:
        print(f"\n{YELLOW}[ERROR]{RESET} Unknown scenario: '{args.scenario}'")
        print(f"\n{BOLD}Available scenarios:{RESET}")
        seen = set()
        for k, (script, name) in SCENARIO_MAP.items():
            if script not in seen:
                print(f"  mofa run {k:<12} # {name}")
                seen.add(script)
        print()
        sys.exit(1)

    script_path, scenario_name = SCENARIO_MAP[scenario_key]
    full_path = PROJECT_ROOT / script_path

    if not full_path.exists():
        print(f"{YELLOW}[ERROR]{RESET} Script not found: {full_path}")
        sys.exit(1)

    print(f"\n{BOLD}{CYAN}Running {scenario_name}...{RESET}\n")

    # Forward remaining args to the scenario script
    extra_args = args.extra_args or []
    os.execvp(sys.executable, [sys.executable, str(full_path)] + extra_args)


# ── Main Entry Point ─────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        prog="mofa",
        description="MoFA Engine — Multimodal Orchestration for Artifacts",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  mofa doctor                    # Check environment readiness
  mofa demo                      # 30-second multimodal golden path
  mofa status                    # Engine health & providers
  mofa models                    # List discovered models
  mofa cost                      # Session cost breakdown
  mofa chat                      # Interactive terminal chat
  mofa run meeting --audio f.wav # Run S1 Meeting Brief
  mofa run review --staged       # Run S2 Code Review
  mofa run video --topic "AI"    # Run S4 Explainer Video
""",
    )

    sub = parser.add_subparsers(dest="command", help="Available commands")

    # doctor
    sub.add_parser("doctor", help="Diagnose environment readiness")

    # demo
    sub.add_parser("demo", help="30-second multimodal golden path demo")

    # status
    sub.add_parser("status", help="Engine health & loaded models")

    # models
    sub.add_parser("models", help="List all discovered models")

    # cost
    sub.add_parser("cost", help="Session cost summary")

    # chat
    chat_parser = sub.add_parser("chat", help="Interactive terminal chat")
    chat_parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"])

    # run
    run_parser = sub.add_parser("run", help="Run a named scenario")
    run_parser.add_argument("scenario", help="Scenario name (meeting, review, doc, video, podcast, race, s1-s7)")
    run_parser.add_argument("extra_args", nargs=argparse.REMAINDER, help="Extra args passed to the scenario script")

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        sys.exit(0)

    dispatch = {
        "doctor": cmd_doctor,
        "demo": cmd_demo,
        "status": cmd_status,
        "models": cmd_models,
        "cost": cmd_cost,
        "chat": cmd_chat,
        "run": cmd_run,
    }

    handler = dispatch.get(args.command)
    if handler:
        handler(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
