#!/usr/bin/env python3
"""Scenario S1: Meeting Audio -> Minutes + 30s Audio Brief (PRD v3.1 §2.2.1 S1).

Executes a 3-step multimodal pipeline:
  1. ASR (Speech-to-Text) -> Transcribes meeting recording with speaker diarization
  2. Chat (LLM Extraction) -> Extracts Decisions, Action Items, and Risks
  3. TTS (Speech Synthesis) -> Generates a 30s executive audio brief (.mp3)

Features:
  - Interactive Terminal Audio Selector with file browsing & fuzzy search
  - Clean spoken audio generation (strips asterisks and markdown)
  - Post-run Interactive Action Menu (View Minutes, Transcript, Play Audio, Chat Q&A)

Usage:
  python3 examples/meeting_brief.py
  python3 examples/meeting_brief.py --audio examples/samples/sample_meeting.wav --prefer local
  mofa run meeting
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine

# ANSI Color formatting
COLOR_LOCAL = "\033[32m"
COLOR_CLOUD = "\033[38;2;249;115;22m"
COLOR_RESET = "\033[0m"
COLOR_BOLD = "\033[1m"
COLOR_CYAN = "\033[36m"
COLOR_YELLOW = "\033[33m"

SAMPLE_AUDIO = Path(__file__).parent / "samples" / "sample_meeting.wav"
SAMPLE_TRANSCRIPT = Path(__file__).parent / "samples" / "sample_transcript.txt"


def get_locality_badge(locality: str) -> str:
    """Format locality string with ANSI colors."""
    if locality == "local":
        return f"{COLOR_LOCAL}Local (Privacy-Preserving){COLOR_RESET}"
    elif locality == "cloud":
        return f"{COLOR_CLOUD}Cloud{COLOR_RESET}"
    else:
        return f"{COLOR_CYAN}{locality}{COLOR_RESET}"


def clean_text_for_speech(text: str) -> str:
    """Remove markdown artifacts, asterisks, brackets, and headers for speech synthesis."""
    text = re.sub(r'\*+', '', text)                     # Remove asterisks
    text = re.sub(r'\[.*?\]', '', text)                 # Remove [brackets]
    text = re.sub(r'#+\s*', '', text)                   # Remove headers
    text = re.sub(r'`[^`]*`', '', text)                 # Remove inline code
    text = re.sub(r'\(.*?\)', '', text)                  # Remove parentheses
    text = re.sub(r'[_~>|]', '', text)                  # Remove markdown symbols
    text = re.sub(r'\n{2,}', '. ', text)                # Collapse newlines
    text = re.sub(r'\n', ' ', text)
    text = re.sub(r'\s{2,}', ' ', text)                 # Collapse multiple spaces
    text = text.strip()
    if text and not text[-1] in '.!?':
        text += '.'
    return text


def interactive_audio_selector(default_path: Path) -> Path:
    """Interactive terminal audio selector with number shortcuts, folder navigation, and fuzzy matching."""
    print(f"\n{COLOR_BOLD}MoFA Meeting Brief — Interactive Audio Selector{COLOR_RESET}")
    print("--------------------------------------------------------------------------")
    print(f"{COLOR_CYAN}How to select:{COLOR_RESET}")
    print(f"   * Type {COLOR_BOLD}ls{COLOR_RESET} to list audio files, then type the {COLOR_BOLD}number{COLOR_RESET} (e.g. {COLOR_BOLD}1{COLOR_RESET}, {COLOR_BOLD}2{COLOR_RESET})")
    print(f"   * Type {COLOR_BOLD}cd <folder>{COLOR_RESET} to navigate folders | Type {COLOR_BOLD}pwd{COLOR_RESET} to check directory")
    print(f"   * Press {COLOR_BOLD}ENTER{COLOR_RESET} for default sample | Type {COLOR_BOLD}q{COLOR_RESET} to exit\n")

    current_audio_cache = []

    while True:
        try:
            cwd = os.getcwd()
            home = os.path.expanduser("~")
            display_cwd = cwd.replace(home, "~")
            prompt_str = f"[{COLOR_CYAN}{display_cwd}{COLOR_RESET}] Enter file path, number, or command:\n> "
            user_input = input(prompt_str).strip().strip("'\"")

            if not user_input:
                if default_path.exists():
                    print(f"   Using default sample: {default_path}")
                    return default_path
                else:
                    print(f"   {COLOR_YELLOW}No input provided and default sample missing.{COLOR_RESET}")
                    continue

            if user_input.isdigit() and current_audio_cache:
                idx = int(user_input)
                if 1 <= idx <= len(current_audio_cache):
                    selected = Path(current_audio_cache[idx - 1]).resolve()
                    print(f"   [OK] Selected [{idx}]: {selected}")
                    return selected
                else:
                    print(f"[ERROR] Invalid number. Enter 1 to {len(current_audio_cache)}")
                    continue

            cmd_parts = user_input.split(maxsplit=1)
            cmd = cmd_parts[0].lower()
            arg = cmd_parts[1] if len(cmd_parts) > 1 else ""

            if cmd in ("q", "exit", "quit"):
                print("Exiting.")
                sys.exit(0)
            elif cmd == "pwd":
                print(f"Current directory: {os.getcwd()}")
                continue
            elif cmd == "ls":
                target_dir = os.path.expanduser(arg) if arg else "."
                if not os.path.isdir(target_dir):
                    print(f"[ERROR] Not a directory: {target_dir}")
                    continue
                try:
                    items = sorted(os.listdir(target_dir), key=str.lower)
                except Exception as e:
                    print(f"[ERROR] Cannot read directory: {e}")
                    continue

                audio_exts = {".mp3", ".wav", ".m4a", ".flac", ".ogg", ".webm", ".aac"}
                folders = []
                audio_files = []

                for item in items:
                    if item.startswith("."):
                        continue
                    full_p = os.path.join(target_dir, item)
                    if os.path.isdir(full_p):
                        folders.append(item)
                    elif any(item.lower().endswith(ext) for ext in audio_exts):
                        audio_files.append(item)

                audio_files = sorted(audio_files, key=str.lower)
                folders = sorted(folders, key=str.lower)
                current_audio_cache = [os.path.join(target_dir, a) for a in audio_files]

                print(f"\nAudio & Folders in {COLOR_CYAN}{os.path.abspath(target_dir)}{COLOR_RESET}:")
                if audio_files:
                    print(f"\n   {COLOR_LOCAL}Audio Files ({len(audio_files)} found - type number to select):{COLOR_RESET}")
                    for i, a in enumerate(audio_files, 1):
                        try:
                            size_mb = os.path.getsize(os.path.join(target_dir, a)) / (1024 * 1024)
                            print(f"      {COLOR_BOLD}[{i}]{COLOR_RESET} {COLOR_LOCAL}{a}{COLOR_RESET}  ({size_mb:.1f} MB)")
                        except OSError:
                            print(f"      {COLOR_BOLD}[{i}]{COLOR_RESET} {COLOR_LOCAL}{a}{COLOR_RESET}")
                else:
                    print(f"   {COLOR_YELLOW}[INFO] No audio files found in this folder.{COLOR_RESET}")

                if folders:
                    print(f"\n   {COLOR_CYAN}Subfolders ({len(folders)} found):{COLOR_RESET}")
                    for f in folders[:20]:
                        print(f"      [{COLOR_CYAN}{f}/{COLOR_RESET}]")
                    if len(folders) > 20:
                        print(f"      ... and {len(folders) - 20} more folders (type 'cd <folder>')")
                print("   ----------------------------------------------------------\n")
                continue
            elif cmd == "cd":
                target_dir = os.path.expanduser(arg) if arg else os.path.expanduser("~")
                try:
                    os.chdir(target_dir)
                    current_audio_cache = []
                    print(f"Changed directory to: {os.getcwd()}")
                except Exception as e:
                    print(f"[ERROR] Cannot cd to {target_dir}: {e}")
                continue
            elif cmd in ("help", "?"):
                print("Commands:")
                print("  <number>    - Select audio by number from 'ls' list")
                print("  <keyword>   - Fuzzy match file in current directory")
                print("  ls [dir]    - List numbered audio files and subfolders")
                print("  cd <dir>    - Change working directory")
                print("  pwd         - Print current working directory")
                print("  <path>      - Direct path to audio file")
                print("  <enter>     - Use default sample audio")
                print("  q / exit    - Quit")
                continue

            # Check direct file path
            resolved = Path(os.path.expanduser(user_input)).resolve()
            if resolved.is_file():
                return resolved
            elif resolved.is_dir():
                print(f"[INFO] '{resolved}' is a directory. Type 'cd {user_input}' to enter.")
                continue

            # Fuzzy search in current directory
            audio_exts = {".mp3", ".wav", ".m4a", ".flac", ".ogg", ".webm", ".aac"}
            local_audio = [f for f in os.listdir(".") if any(f.lower().endswith(ext) for ext in audio_exts)]
            fuzzy_matches = [f for f in local_audio if user_input.lower() in f.lower()]
            if len(fuzzy_matches) == 1:
                matched = Path(fuzzy_matches[0]).resolve()
                print(f"   [OK] Auto-matched audio file: {fuzzy_matches[0]}")
                return matched
            elif len(fuzzy_matches) > 1:
                print(f"Multiple matches found for '{user_input}':")
                for j, m in enumerate(fuzzy_matches, 1):
                    print(f"   [{j}] {m}")
                current_audio_cache = fuzzy_matches
                continue

            print(f"[ERROR] File or command not recognized: {user_input}")
            print("   (Type 'ls' to see numbered audio files or press Enter for default sample)")
        except (KeyboardInterrupt, EOFError):
            print("\n\nOperation cancelled by user.")
            sys.exit(0)


def interactive_post_menu(minutes_text: str, transcript: str, out_audio: Path, engine: MofaEngine, prefer: str):
    """Interactive post-run menu for exploring notes, transcripts, playback, and Q&A."""
    if not sys.stdin.isatty():
        return

    while True:
        print(f"\n{COLOR_BOLD}Meeting Actions:{COLOR_RESET}")
        print(f"   [{COLOR_BOLD}1{COLOR_RESET}] View Meeting Minutes")
        print(f"   [{COLOR_BOLD}2{COLOR_RESET}] View Full Meeting Transcript")
        print(f"   [{COLOR_BOLD}3{COLOR_RESET}] Play Audio Brief")
        print(f"   [{COLOR_BOLD}4{COLOR_RESET}] Chat About Meeting (Interactive Q&A)")
        print(f"   [{COLOR_BOLD}Enter / q{COLOR_RESET}] Exit\n")
        try:
            choice = input("Select action (1/2/3/4): ").strip()
        except (KeyboardInterrupt, EOFError):
            break

        if not choice or choice.lower() in ("q", "exit", "quit"):
            break
        elif choice == "1":
            print(f"\n{COLOR_BOLD}=========================================================================={COLOR_RESET}")
            print(f"{COLOR_CYAN}{COLOR_BOLD}EXTRACTED MEETING MINUTES{COLOR_RESET}")
            print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}")
            print(minutes_text.strip())
            print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}\n")
        elif choice == "2":
            print(f"\n{COLOR_BOLD}=========================================================================={COLOR_RESET}")
            print(f"{COLOR_CYAN}{COLOR_BOLD}FULL MEETING TRANSCRIPT ({len(transcript.split())} words){COLOR_RESET}")
            print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}")
            print(transcript.strip())
            print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}\n")
        elif choice == "3":
            play_target = None
            for cand in [out_audio.with_suffix(".mp3"), out_audio.with_suffix(".wav"), out_audio]:
                if cand.exists() and cand.stat().st_size > 0:
                    play_target = cand
                    break

            if play_target:
                print(f"\nPlaying audio brief ({play_target})...")
                played = False
                if sys.platform == "darwin":
                    target_to_play = play_target
                    try:
                        with open(play_target, "rb") as f:
                            magic = f.read(4)
                        if magic == b"RIFF" and play_target.suffix.lower() != ".wav":
                            # WAV data inside .mp3 named file; play as .wav for CoreAudio
                            wav_path = play_target.with_suffix(".wav")
                            if not wav_path.exists() or wav_path.stat().st_size != play_target.stat().st_size:
                                shutil.copy(play_target, wav_path)
                            target_to_play = wav_path
                    except Exception:
                        pass

                    res = subprocess.run(["afplay", str(target_to_play)], capture_output=True, text=True)
                    if res.returncode == 0:
                        played = True
                    elif shutil.which("ffplay"):
                        subprocess.run(["ffplay", "-nodisp", "-autoexit", str(target_to_play)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                        played = True
                elif shutil.which("ffplay"):
                    subprocess.run(["ffplay", "-nodisp", "-autoexit", str(play_target)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    played = True
                elif shutil.which("aplay"):
                    subprocess.run(["aplay", str(play_target)])
                    played = True

                if not played:
                    print(f"Audio player not found. Listen at: {play_target}")
                print("[OK] Playback finished.\n")
            else:
                print(f"[WARN] Audio file not found at {out_audio}\n")
        elif choice == "4":
            print(f"\n{COLOR_CYAN}Chat: Ask any question about this meeting (type 'back' or 'q' to return):{COLOR_RESET}")
            while True:
                try:
                    q = input("Question: ").strip()
                except (KeyboardInterrupt, EOFError):
                    break
                if not q or q.lower() in ("back", "q", "exit"):
                    print()
                    break
                print(f"Querying MoFA Engine ({prefer})...")
                try:
                    q_prompt = (
                        f"Based strictly on this meeting transcript and notes:\n\n{transcript}\n\n"
                        f"Answer the following user question concisely and accurately:\n{q}"
                    )
                    res = engine.chat(q_prompt, prefer=prefer)
                    ans = res.text or "No response generated."
                    print(f"\n{COLOR_LOCAL}{COLOR_BOLD}Answer:{COLOR_RESET}\n{ans}\n")
                except Exception as err:
                    print(f"[ERROR] Error answering question: {err}\n")


def main():
    parser = argparse.ArgumentParser(description="S1 Meeting Brief: Audio -> Minutes + 30s Audio Brief")
    parser.add_argument("--audio", default=None, help="Path to meeting audio recording (leave empty for interactive selector)")
    parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"], help="Routing locality preference")
    parser.add_argument("--out", default="output", help="Output directory for generated artifacts")
    parser.add_argument("--no-narrate", action="store_true", help="Skip TTS audio brief generation")
    args = parser.parse_args()

    # If --audio not supplied on CLI and stdin is a TTY, launch interactive selector
    if args.audio:
        audio_path = Path(os.path.expanduser(args.audio)).resolve()
    elif sys.stdin.isatty():
        audio_path = interactive_audio_selector(SAMPLE_AUDIO)
    else:
        audio_path = SAMPLE_AUDIO.resolve()

    if not audio_path.exists():
        print(f"{COLOR_YELLOW}[WARN] Audio file not found: {audio_path}. Falling back to default sample.{COLOR_RESET}")
        audio_path = SAMPLE_AUDIO.resolve()

    engine = MofaEngine()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    print("\n==================================================================")
    print("   Scenario S1: Meeting Audio -> Minutes + Audio Brief")
    print("==================================================================")
    print(f"  * Input Audio : {audio_path}")
    print(f"  * Locality    : {get_locality_badge(args.prefer)}")
    print(f"  * Engine Gateway: {engine.base_url}\n")

    # ── Step 1: ASR (Speech-to-Text) ──────────────────────────────────
    print(f"[Step 1/3] ASR: Transcribing meeting audio ({audio_path.name})...")
    transcript = ""
    try:
        asr_res = engine.asr(str(audio_path), prefer=args.prefer)
        transcript = asr_res.text or ""
        print(f"  [OK] Transcribed ({asr_res.provider}, {asr_res.duration_ms}ms)")
        # Print short transcript snippet
        lines = [l.strip() for l in transcript.split("\n") if l.strip()]
        for l in lines[:3]:
            print(f"       {l[:80]}")
        if len(lines) > 3:
            print("       ...")
    except Exception as e:
        print(f"  [FALLBACK] Gateway ASR service unavailable ({e}); reading sample transcript...")
        if SAMPLE_TRANSCRIPT.exists():
            transcript = SAMPLE_TRANSCRIPT.read_text(encoding="utf-8")
        else:
            transcript = (
                "Speaker 1 (Alice): We must lock enterprise data to local models by Friday.\n"
                "Speaker 2 (Bob): Agreed, Kokoro TTS achieves 85ms on Apple Silicon.\n"
                "Speaker 3 (Carol): What about privacy? We will enforce prefer='local'."
            )

    # ── Step 2: Chat (Structured Minutes Extraction) ──────────────────
    print("\n[Step 2/3] Chat: Extracting structured meeting minutes (hint_next=tts)...")
    minutes_prompt = (
        "Extract executive meeting minutes from the following meeting transcript.\n"
        "Include strictly:\n"
        "## 1. Key Decisions\n"
        "## 2. Action Items (with assignees and deadlines)\n"
        "## 3. Risks & Blockers\n"
        "## 4. Executive Audio Brief (2-3 concise sentences summarizing the key outcome)\n\n"
        "Transcript:\n" + transcript
    )
    minutes_res = engine.chat(
        minutes_prompt,
        hint_next="tts",
        prefer=args.prefer,
    )
    minutes_path = out / "meeting_minutes.md"
    minutes_res.save(str(minutes_path))
    minutes_text = minutes_res.text or ""
    print(f"  [OK] Minutes saved to: {minutes_path}")
    print(f"  +- Routed to : {minutes_res.provider}/{minutes_res.model_used}")
    print(f"  +- Latency   : {minutes_res.duration_ms}ms · Cost: ${minutes_res.cost_usd or 0.0:.4f}\n")

    print("┌" + "─" * 68 + "┐")
    print("│                     EXTRACTED MEETING MINUTES                      │")
    print("├" + "─" * 68 + "┤")
    for line in minutes_text.split("\n")[:15]:
        print(f"  {line}")
    if len(minutes_text.split("\n")) > 15:
        print("  ... (full minutes saved to file)")
    print("└" + "─" * 68 + "┘")

    # ── Step 3: TTS (30s Audio Brief) ─────────────────────────────────
    brief_path = out / "meeting_brief.mp3"
    brief_res = None
    if not args.no_narrate:
        print("\n[Step 3/3] TTS: Synthesizing executive audio brief...")
        # Extract executive summary section or first clean sentences
        brief_text = ""
        if "Executive Audio Brief" in minutes_text or "executive summary" in minutes_text.lower():
            parts = re.split(r'(?i)##\s*4\.\s*Executive Audio Brief|Executive Summary', minutes_text)
            if len(parts) > 1:
                brief_text = parts[1].strip().split("\n\n")[0]
        if not brief_text:
            brief_text = minutes_text[:350]

        # Clean all markdown/asterisks so TTS does not read formatting aloud
        spoken_brief = clean_text_for_speech(brief_text)
        print(f"  [INFO] Narration text: \"{spoken_brief[:80]}...\"")
        
        try:
            brief_res = engine.tts(spoken_brief, voice="alloy", prefer=args.prefer)
            src_ext = Path(brief_res.file).suffix.lower() if brief_res.file else ".wav"
            brief_path = out / f"meeting_brief{src_ext}"
            brief_res.save(str(brief_path))
            print(f"  [OK] Audio brief saved to: {brief_path}")
            print(f"  +- Routed to : {brief_res.provider}/{brief_res.model_used}")
            print(f"  +- Latency   : {brief_res.duration_ms}ms · Cost: ${brief_res.cost_usd or 0.0:.4f}")
        except Exception as err:
            print(f"  [WARN] TTS generation failed: {err}")
    else:
        print("\n[Step 3/3] TTS: Skipped (--no-narrate)")

    # ── Summary & Deliverables ───────────────────────────────────────
    total_cost = (minutes_res.cost_usd or 0.0) + ((brief_res.cost_usd or 0.0) if brief_res else 0.0)
    print("\n==================================================================")
    print("   S1 Meeting Brief Pipeline Complete!")
    print("==================================================================")
    print(f"  Total Cost     : ${total_cost:.4f} ({'100% Local $0.00' if minutes_res.is_local else 'Cloud'})")
    print(f"  Minutes File   : {minutes_path}")
    if not args.no_narrate and brief_path.exists():
        print(f"  Audio Brief    : {brief_path}")
    print("==================================================================")

    # ── Post-run Interactive Menu ────────────────────────────────────
    interactive_post_menu(minutes_text, transcript, brief_path, engine, args.prefer)


if __name__ == "__main__":
    main()

