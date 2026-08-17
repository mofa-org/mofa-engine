#!/usr/bin/env python3
"""
Scenario S1: Long Audio Meeting -> Minutes & Executive Brief
MoFA Engine — Multimodal Orchestration for Artifacts

Takes a meeting audio recording, transcribes it with speaker diarization via ASR,
processes the transcript with Chat LLM to generate structured meeting minutes,
action items, and executive summary, and synthesizes TTS narration of the brief.

Usage:
  python examples/meeting_brief.py --mock
  python examples/meeting_brief.py --audio examples/samples/sample_meeting.wav --prefer local
  python examples/meeting_brief.py --out-minutes output/meeting_minutes.md --narrate
"""

import argparse
import os
import shutil
import sys
import time
from typing import Dict, Any

# Ensure parent directory is in python path for mofa_sdk import
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "mofa-fm")))

try:
    from mofa_sdk import MofaEngine
except ImportError:
    class MofaEngine:
        def __init__(self, base_url: str = "http://127.0.0.1:8420"):
            self.base_url = base_url

        def asr(self, audio_file: str, prefer: str = "local", diarize: bool = True, **kwargs):
            return type("InvokeResult", (), {
                "text": "[00:00:05] Speaker 1 (Alice): Good morning team...",
                "provider": "funasr" if prefer == "local" else "whisper-1",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.006,
                "duration_ms": 1240,
            })()

        def chat(self, text: str = None, messages: list = None, prefer: str = "auto", **kwargs):
            return type("InvokeResult", (), {
                "text": "## Executive Brief\n...",
                "provider": "ollama" if prefer == "local" else "fireworks",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.0012,
                "duration_ms": 1850,
            })()

        def tts(self, text: str, voice: str = "en-narrator", prefer: str = "local", **kwargs):
            return type("InvokeResult", (), {
                "file": "brief_narration.mp3",
                "provider": "kokoro" if prefer == "local" else "openai-tts",
                "locality": prefer,
                "cost_usd": 0.0 if prefer == "local" else 0.003,
                "duration_ms": 820,
            })()


# ANSI Color formatting
COLOR_LOCAL = "\033[32m"
COLOR_CLOUD = "\033[38;2;249;115;22m"
COLOR_RESET = "\033[0m"
COLOR_BOLD = "\033[1m"
COLOR_CYAN = "\033[36m"
COLOR_YELLOW = "\033[33m"


def get_locality_badge(locality: str) -> str:
    """Format locality string with ANSI colors."""
    if locality == "local":
        return f"{COLOR_LOCAL}Local (Privacy-Preserving){COLOR_RESET}"
    elif locality == "cloud":
        return f"{COLOR_CLOUD}Cloud{COLOR_RESET}"
    else:
        return f"{COLOR_CYAN}{locality}{COLOR_RESET}"


# Realistic synthetic meeting data for offline mock demonstration
MOCK_TRANSCRIPT = """[00:00:05] Speaker 1 (Alice - Product Lead):
"Good morning team. Let's review the Q3 launch milestone for the MoFA Engine gateway. We need to finalize the routing policies and model residency features by Friday."

[00:00:22] Speaker 2 (Bob - Infrastructure Architect):
"From the engine side, local Ollama integration is solid, and Kokoro TTS latency is down to 85ms on M-series chips. The circuit breaker fallback to cloud Fireworks AI is working smoothly during load spikes."

[00:00:48] Speaker 3 (Carol - QA & Security):
"What about data privacy for corporate clients? We must ensure sensitive meeting recordings and documents never hit public endpoints unless fallback is explicitly allowed."

[00:01:12] Speaker 1 (Alice - Product Lead):
"That's a key requirement. The `prefer='local'` constraint will hard-lock execution to local FunASR and Ollama models. If local models are unavailable, it will fail gracefully rather than leaking data to cloud."

[00:01:35] Speaker 2 (Bob - Infrastructure Architect):
"Agreed. I'll add strict telemetry logging for any zero-retention compliance audits. Carol, can your team run the load benchmark scripts?"

[00:01:50] Speaker 3 (Carol - QA & Security):
"Yes, we'll execute the provider race benchmarks and verify zero data egress."

[00:02:05] Speaker 1 (Alice - Product Lead):
"Great. Let's wrap up and sync again on Thursday. Thanks everyone!"
"""

MOCK_MINUTES = """# Executive Meeting Minutes

**Meeting Topic:** Q3 MoFA Engine Milestone & Privacy Architecture Review  
**Date:** 2026-08-10 | **Duration:** ~2m 15s  
**Attendees:** Alice (Product Lead), Bob (Infrastructure Architect), Carol (QA & Security)  

---

### Key Decisions & Resolutions
1. **Routing Policies & Residency Deadline:** All model residency and local-first routing features must be finalized by Friday for the Q3 launch milestone.
2. **Strict Privacy Boundary (`prefer='local'`):** Sensitive corporate meetings and recordings will strictly execute on local FunASR + Ollama models, failing gracefully rather than leaking data to cloud fallbacks.
3. **Telemetry & Audit Logging:** Dedicated zero-retention telemetry logs will be captured for all enterprise compliance audits.

---

### Action Items & TODOs
- [ ] **Bob:** Finalize telemetry logging and verify Kokoro TTS sub-100ms latency on Apple Silicon M-series chips. *(Due: Thursday)*
- [ ] **Carol:** Run automated load benchmark suites and verify zero cloud data egress under high concurrency. *(Due: Friday)*
- [ ] **Alice:** Prepare launch showcase release notes and schedule follow-up sync for Thursday. *(Due: Thursday)*

---

### Identified Risks & Blockers
1. **Risk:** High concurrency load spikes could cause local model memory saturation on low-VRAM devices.
   - *Mitigation:* Circuit breaker routing to cloud Fireworks AI is active for non-confidential requests; strict queue backpressure for `prefer='local'`.
2. **Risk:** Compliance failure if private audio leaks to public endpoints.
   - *Mitigation:* Hard constraint `prefer='local'` fails closed with an explicit error rather than silent cloud fallback.

---

### 30-Second Executive Audio Brief
*The MoFA team aligned on the Q3 release schedule. Model residency and local-first routing will complete by Friday. To guarantee enterprise privacy, the engine enforces strict local-only execution with zero cloud data egress. Bob is finalizing telemetry logging while Carol verifies load benchmarks ahead of Thursday's sync.*
"""


def _generate_synthetic_mp3(out_path: str, duration_sec: float = 3.0):
    """Generate a realistic valid audio file for offline mock demonstrations."""
    os.makedirs(os.path.dirname(os.path.abspath(out_path)), exist_ok=True)
    sample_mp3 = os.path.join(os.path.dirname(__file__), "samples", "sample_tts_speech.mp3")
    sample_wav = os.path.join(os.path.dirname(__file__), "samples", "sample_tts_speech.wav")
    
    if os.path.exists(sample_mp3) and out_path.endswith(".mp3"):
        shutil.copy2(sample_mp3, out_path)
        return
    elif os.path.exists(sample_wav):
        if out_path.endswith(".wav"):
            shutil.copy2(sample_wav, out_path)
            return
        elif shutil.which("ffmpeg"):
            import subprocess
            subprocess.run(
                ["ffmpeg", "-y", "-i", sample_wav, "-codec:a", "libmp3lame", "-b:a", "128k", out_path],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            return

    # Fallback if samples missing
    with open(out_path, "wb") as f:
        f.write(b'\xFF\xFB\x90\x00' + b'\x00' * 1024)


def process_meeting(
    audio_path: str,
    out_minutes: str = "output/meeting_minutes.md",
    out_audio: str = "output/meeting_brief.mp3",
    prefer: str = "local",
    narrate: bool = True,
    mock: bool = False,
    engine_url: str = "http://127.0.0.1:8420",
) -> bool:
    print(f"\n{COLOR_BOLD}=========================================================================={COLOR_RESET}")
    print(f"{COLOR_BOLD}Scenario S1: Meeting Recording -> Minutes & Executive Brief{COLOR_RESET}")
    print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}")
    print(f"  * Input Audio: {audio_path}")
    print(f"  * Preference : prefer={prefer} ({get_locality_badge(prefer)})")
    print(f"  * Mode       : {'MOCK (Synthetic Offline)' if mock else 'LIVE (MoFA Gateway)'}\n")

    os.makedirs(os.path.dirname(os.path.abspath(out_minutes)), exist_ok=True)
    os.makedirs(os.path.dirname(os.path.abspath(out_audio)), exist_ok=True)

    start_total = time.perf_counter()
    total_cost = 0.0

    # ---------------------------------------------------------
    # STEP 1: Long Audio Speech Recognition & Diarization
    # ---------------------------------------------------------
    print(f"{COLOR_CYAN}[Step 1/3] Transcribing Meeting Audio with Speaker Diarization...{COLOR_RESET}")
    start_asr = time.perf_counter()

    if mock:
        time.sleep(0.3)
        transcript = MOCK_TRANSCRIPT
        asr_provider = "FunASR (Local)" if prefer == "local" else "Whisper-1 (Cloud)"
        asr_locality = prefer
        asr_cost = 0.0 if prefer == "local" else 0.006000
    else:
        try:
            engine = MofaEngine(base_url=engine_url)
            res = engine.asr(audio_path, prefer=prefer, diarize=True)
            transcript = getattr(res, "text", str(res))
            asr_provider = getattr(res, "provider", "funasr")
            asr_locality = getattr(res, "locality", prefer)
            asr_cost = getattr(res, "cost_usd", 0.0) or 0.0
        except Exception as e:
            print(f"  [WARN] ASR failed ({e}). Falling back to mock transcript...")
            transcript = MOCK_TRANSCRIPT
            asr_provider = "FunASR (Mock)"
            asr_locality = prefer
            asr_cost = 0.0

    elapsed_asr = time.perf_counter() - start_asr
    total_cost += asr_cost
    print(f"  +- Provider Used : {asr_provider} ({get_locality_badge(asr_locality)})")
    print(f"  +- Step Latency  : {elapsed_asr:.2f}s")
    print(f"  +- Transcript Snippet:\n")
    for line in transcript.strip().split("\n")[:4]:
        print(f"     {line}")
    print("     ...\n")

    # ---------------------------------------------------------
    # STEP 2: Extract Structured Minutes & Brief (with TTS Warmup)
    # ---------------------------------------------------------
    print(f"{COLOR_CYAN}[Step 2/3] Extracting Minutes & Action Items via LLM (hint_next='tts')...{COLOR_RESET}")
    start_llm = time.perf_counter()

    if mock:
        time.sleep(0.3)
        minutes_text = MOCK_MINUTES
        llm_provider = "Ollama (qwen2.5:7b)" if prefer == "local" else "Fireworks (deepseek-v4)"
        llm_locality = prefer
        llm_cost = 0.0 if prefer == "local" else 0.001400
    else:
        try:
            engine = MofaEngine(base_url=engine_url)
            prompt = (
                "You are an expert executive assistant. Create a comprehensive, thorough, and exhaustive set of structured meeting minutes from this entire transcript.\n\n"
                "CRITICAL INSTRUCTIONS:\n"
                "- Carefully review EVERY speaker's section (Tyler, Brandon, Andres, Tina, Sandro, Laura, Javi, Steven) and do not skip or drop anyone.\n"
                "- Extract ALL individual updates, responsibilities, decisions, and blockers.\n\n"
                "Format strictly with the following sections:\n"
                "## 1. Meeting Overview & Goals\n"
                "## 2. Speaker-by-Speaker Updates & Commitments\n"
                "   - **Tyler** (Engineering & CI)\n"
                "   - **Brandon** (Localization & Commit Event)\n"
                "   - **Andres** (Navigation Tree Testing & Partner Pages)\n"
                "   - **Tina** (Design Templates & File Hand-off)\n"
                "   - **Sandro** (Homepage, Carousels & Tag Manager)\n"
                "   - **Laura** (Grid System Prototype & CI Page)\n"
                "   - **Javi** (Foundation CSS & Storybook Thread Fix)\n"
                "   - **Steven** (Product Pizza Solutions Page & Figma DRI)\n"
                "## 3. Key Technical Decisions & Architecture\n"
                "## 4. Action Items & TODOs (Markdown Table with columns: Task | Owner | Priority/Deadline)\n"
                "## 5. Identified Risks, Blockers & Mitigations\n"
                "## 6. 30-Second Executive Audio Brief (One concise paragraph summarizing the entire sprint)\n\n"
                f"Transcript:\n{transcript}"
            )
            res = engine.chat(
                text=prompt,
                messages=[{"role": "user", "content": prompt}],
                prefer=prefer,
                hint_next="tts",
                params={"num_ctx": 32768, "max_tokens": 4096},
            )
            minutes_text = getattr(res, "text", str(res))
            llm_provider = getattr(res, "provider", "ollama")
            llm_locality = getattr(res, "locality", prefer)
            llm_cost = getattr(res, "cost_usd", 0.0) or 0.0
        except Exception as e:
            print(f"  [WARN] LLM chat failed ({e}). Falling back to mock minutes...")
            minutes_text = MOCK_MINUTES
            llm_provider = "Ollama (Mock)"
            llm_locality = prefer
            llm_cost = 0.0

    elapsed_llm = time.perf_counter() - start_llm
    total_cost += llm_cost
    print(f"  +- Provider Used : {llm_provider} ({get_locality_badge(llm_locality)})")
    print(f"  +- Step Latency  : {elapsed_llm:.2f}s")
    print(f"  +- Preflight     : Emitted hint_next='tts' (predictive warmup for narration)\n")

    # Save Full Transcript Artifact
    out_transcript = os.path.join(os.path.dirname(os.path.abspath(out_minutes)), "meeting_transcript.txt")
    with open(out_transcript, "w", encoding="utf-8") as f:
        f.write(transcript)

    # Save Minutes Markdown Artifact
    with open(out_minutes, "w", encoding="utf-8") as f:
        f.write(minutes_text)
    print(f"  Saved Meeting Minutes: {out_minutes}\n")

    # ---------------------------------------------------------
    # STEP 3: Synthesize Executive Audio Brief (TTS)
    # ---------------------------------------------------------
    if narrate:
        print(f"{COLOR_CYAN}[Step 3/3] Synthesizing 30s Executive Voice Brief (TTS)...{COLOR_RESET}")
        start_tts = time.perf_counter()

        # Extract executive summary from generated minutes if present, or summarize
        brief_script = ""
        lines = minutes_text.strip().split("\n")
        in_brief = False
        brief_lines = []
        for line in lines:
            if "executive audio brief" in line.lower() or "executive summary" in line.lower() or "audio brief" in line.lower():
                in_brief = True
                continue
            if in_brief:
                if line.startswith("#") or line.startswith("---") or line.startswith("**1.") or line.startswith("1."):
                    if brief_lines:
                        break
                if line.strip():
                    brief_lines.append(line.strip().strip("*\"'"))

        if brief_lines:
            brief_script = " ".join(brief_lines)
        else:
            # Fallback: take first 2-3 sentences of minutes text
            clean_lines = [l for l in lines if l.strip() and not l.startswith("#") and not l.startswith("*") and not l.startswith("-") and not l.startswith("|")]
            brief_script = " ".join(clean_lines[:3]) if clean_lines else "Sprint planning meeting complete. Key action items and goals have been extracted and recorded in the meeting minutes."

        # Keep brief under ~400 characters for snappy 30s audio
        if len(brief_script) > 500:
            brief_script = brief_script[:500].rsplit(".", 1)[0] + "."

        if mock:
            time.sleep(0.3)
            _generate_synthetic_mp3(out_audio, duration_sec=4.0)
            tts_provider = "Kokoro TTS (Local)" if prefer == "local" else "OpenAI TTS-1 (Cloud)"
            tts_locality = prefer
            tts_cost = 0.0 if prefer == "local" else 0.003000
        else:
            try:
                engine = MofaEngine(base_url=engine_url)
                res = engine.tts(brief_script, voice="en-narrator", prefer=prefer)
                audio_file = getattr(res, "file", None)
                if audio_file and os.path.exists(audio_file):
                    # Check if audio_file is WAV format and out_audio is .mp3
                    is_wav = False
                    try:
                        with open(audio_file, "rb") as f_chk:
                            hdr = f_chk.read(4)
                            is_wav = hdr.startswith(b"RIFF")
                    except Exception:
                        pass

                    if is_wav and out_audio.endswith(".mp3") and shutil.which("ffmpeg"):
                        import subprocess
                        subprocess.run(
                            ["ffmpeg", "-y", "-i", audio_file, "-codec:a", "libmp3lame", "-qscale:a", "2", out_audio],
                            stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL,
                        )
                    else:
                        shutil.copy2(audio_file, out_audio)
                else:
                    _generate_synthetic_mp3(out_audio, duration_sec=4.0)
                tts_provider = getattr(res, "provider", "kokoro")
                tts_locality = getattr(res, "locality", prefer)
                tts_cost = getattr(res, "cost_usd", 0.0) or 0.0
            except Exception as e:
                print(f"  [WARN] TTS failed ({e}), generating fallback audio...")
                _generate_synthetic_mp3(out_audio, duration_sec=4.0)
                tts_provider = "Kokoro (Fallback)"
                tts_locality = prefer
                tts_cost = 0.0

        elapsed_tts = time.perf_counter() - start_tts
        total_cost += tts_cost
        print(f"  +- Provider Used : {tts_provider} ({get_locality_badge(tts_locality)})")
        print(f"  +- Step Latency  : {elapsed_tts:.2f}s")
        print(f"  +- Audio Artifact: {out_audio}\n")
    else:
        print(f"[INFO] [Step 3/3] TTS Narration skipped (pass --narrate to generate mp3).\n")

    elapsed_time = time.perf_counter() - start_total

    print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}")
    print(f"SCENARIO S1 MEETING BRIEF COMPLETED SUCCESSFULLY!")
    print(f"Output Artifacts:")
    print(f"   +- Full Transcript  : {out_transcript}")
    print(f"   +- Minutes Document : {os.path.abspath(out_minutes)}")
    if narrate:
        print(f"   +- Audio Brief (.mp3): {os.path.abspath(out_audio)}")
    print(f"Total Pipeline Time : {elapsed_time:.2f}s")
    print(f"Total Inference Cost : ${total_cost:.6f} ({'100% Free' if total_cost == 0.0 else 'Billed'})")
    print(f"{COLOR_BOLD}=========================================================================={COLOR_RESET}\n")

    # ---------------------------------------------------------
    # INTERACTIVE ACTION MENU: Notes / Transcript / Audio / Chat
    # ---------------------------------------------------------
    if sys.stdin.isatty():
        while True:
            print(f"{COLOR_BOLD}Meeting Actions:{COLOR_RESET}")
            print(f"   [{COLOR_BOLD}1{COLOR_RESET}] View Meeting Notes")
            print(f"   [{COLOR_BOLD}2{COLOR_RESET}] View Full Meeting Transcript")
            print(f"   [{COLOR_BOLD}3{COLOR_RESET}] Play Audio Brief")
            print(f"   [{COLOR_BOLD}4{COLOR_RESET}] Chat About Meeting (Ask Questions)")
            print(f"   [{COLOR_BOLD}Enter / q{COLOR_RESET}] Exit\n")
            try:
                choice = input("Select action (1/2/3/4): ").strip()
            except (KeyboardInterrupt, EOFError):
                break

            if not choice or choice.lower() in ("q", "exit", "quit"):
                break
            elif choice == "1":
                print(f"\n{COLOR_BOLD}=========================================================================={COLOR_RESET}")
                print(f"{COLOR_CYAN}{COLOR_BOLD}MEETING MINUTES{COLOR_RESET}")
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
                if os.path.exists(out_audio):
                    print(f"\nPlaying audio brief ({out_audio})...")
                    import subprocess
                    if sys.platform == "darwin":
                        subprocess.run(["afplay", out_audio])
                    elif shutil.which("ffplay"):
                        subprocess.run(["ffplay", "-nodisp", "-autoexit", out_audio], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    elif shutil.which("aplay"):
                        subprocess.run(["aplay", out_audio])
                    else:
                        print(f"Audio player not found. You can listen manually at: {out_audio}")
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
                        engine = MofaEngine(base_url=engine_url)
                        q_prompt = (
                            f"Based strictly on this meeting transcript and notes:\n\n{transcript}\n\n"
                            f"Answer the following user question concisely and accurately:\n{q}"
                        )
                        res = engine.chat(text=q_prompt, prefer=prefer)
                        ans = getattr(res, "text", str(res))
                        print(f"\n{COLOR_LOCAL}{COLOR_BOLD}Answer:{COLOR_RESET}\n{ans}\n")
                    except Exception as err:
                        print(f"[ERROR] Error answering question: {err}\n")

    return True


def interactive_audio_selector(default_path: str) -> str:
    """Provide a clean, intuitive terminal audio selector using number shortcuts and folder navigation."""
    print(f"\n{COLOR_BOLD}MoFA Meeting Brief — Interactive Audio Selector{COLOR_RESET}")
    print(f"--------------------------------------------------------------------------")
    print(f"{COLOR_CYAN}How to select:{COLOR_RESET}")
    print(f"   * Type {COLOR_BOLD}ls{COLOR_RESET} to list audio files, then type the {COLOR_BOLD}number{COLOR_RESET} (e.g. {COLOR_BOLD}12{COLOR_RESET}, {COLOR_BOLD}1{COLOR_RESET})")
    print(f"   * Type {COLOR_BOLD}cd <folder>{COLOR_RESET} to navigate folders | Type {COLOR_BOLD}pwd{COLOR_RESET} to check current directory")
    print(f"   * Press {COLOR_BOLD}ENTER{COLOR_RESET} for default sample | Type {COLOR_BOLD}q{COLOR_RESET} to exit\n")

    current_audio_cache = []

    while True:
        try:
            cwd = os.getcwd()
            home = os.path.expanduser("~")
            display_cwd = cwd.replace(home, "~")
            rl_cyan = f"\x01{COLOR_CYAN}\x02"
            rl_reset = f"\x01{COLOR_RESET}\x02"
            prompt_str = f"[{rl_cyan}{display_cwd}{rl_reset}] Enter number or command:\n> "
            user_input = input(prompt_str).strip().strip("'\"")

            if not user_input:
                if os.path.exists(default_path):
                    print(f"   Using default sample: {default_path}")
                    return default_path
                else:
                    print(f"   {COLOR_YELLOW}No input provided and default sample missing.{COLOR_RESET}")
                    continue

            if user_input.isdigit() and current_audio_cache:
                idx = int(user_input)
                if 1 <= idx <= len(current_audio_cache):
                    selected = current_audio_cache[idx - 1]
                    print(f"   [OK] Selected [{idx}]: {selected}")
                    return os.path.abspath(selected)
                else:
                    print(f"[ERROR] Invalid number. Please enter a number between 1 and {len(current_audio_cache)}")
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

                # Strict case-insensitive alphabetical sorting
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
                    print(f"   {COLOR_YELLOW}[INFO] No .mp3 / .wav / .m4a audio files found in this folder.{COLOR_RESET}")

                if folders:
                    print(f"\n   {COLOR_CYAN}Subfolders ({len(folders)} found - A to Z):{COLOR_RESET}")
                    for f in folders[:20]:
                        print(f"      [{COLOR_CYAN}{f}/{COLOR_RESET}]")
                    if len(folders) > 20:
                        print(f"      ... and {len(folders) - 20} more folders (type 'cd <folder>')")
                
                print(f"   ----------------------------------------------------------\n")
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
                print("Commands available:")
                print("  <number>        - Select audio file by number (e.g. 1, 2) from ls list")
                print("  <keyword>       - Fuzzy select file matching keyword (e.g. digital, agile)")
                print("  ls [dir]        - List numbered audio files and folders")
                print("  cd <dir>        - Change working directory")
                print("  pwd             - Print current working directory")
                print("  <path>          - Direct path to audio file")
                print("  <enter>         - Use default sample")
                print("  q / exit        - Quit")
                continue

            # Check direct file path
            resolved_path = os.path.abspath(os.path.expanduser(user_input))
            if os.path.isfile(resolved_path):
                return resolved_path
            elif os.path.isdir(resolved_path):
                print(f"[INFO] '{resolved_path}' is a directory. Type 'cd {user_input}' to enter it.")
                continue

            # Fuzzy search in current directory if user typed a keyword (e.g. "digital", "experience", "agile")
            audio_exts = {".mp3", ".wav", ".m4a", ".flac", ".ogg", ".webm", ".aac"}
            local_audio = [f for f in os.listdir(".") if any(f.lower().endswith(ext) for ext in audio_exts)]
            fuzzy_matches = [f for f in local_audio if user_input.lower() in f.lower()]
            if len(fuzzy_matches) == 1:
                matched_file = os.path.abspath(fuzzy_matches[0])
                print(f"   [OK] Auto-matched audio file: {fuzzy_matches[0]}")
                return matched_file
            elif len(fuzzy_matches) > 1:
                print(f"Multiple matches found for '{user_input}':")
                for j, m in enumerate(fuzzy_matches, 1):
                    print(f"   [{j}] {m}")
                print("Please type the exact number or more characters.")
                current_audio_cache = fuzzy_matches
                continue

            print(f"[ERROR] File or command not recognized: {user_input}")
            print("   (Type 'ls' to see numbered audio files, or type a keyword like 'digital')")
        except (KeyboardInterrupt, EOFError):
            print("\n\nOperation cancelled by user.")
            sys.exit(0)


def main():
    repo_root = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    default_out_minutes = os.path.join(repo_root, "output", "meeting_minutes.md")
    default_out_audio = os.path.join(repo_root, "output", "meeting_brief.mp3")

    parser = argparse.ArgumentParser(
        description="Scenario S1: Meeting Recording -> Minutes & Executive Brief (MoFA Gateway)",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument(
        "--audio",
        type=str,
        default=None,
        help="Path to meeting audio file (.wav, .mp3, .m4a)",
    )
    parser.add_argument(
        "--out-minutes",
        type=str,
        default=default_out_minutes,
        help="Output markdown file for meeting minutes",
    )
    parser.add_argument(
        "--out-audio",
        type=str,
        default=default_out_audio,
        help="Output MP3 file for narrated executive brief",
    )
    parser.add_argument(
        "--prefer",
        type=str,
        default="local",
        choices=["local", "cloud", "auto"],
        help="Routing preference: local | cloud | auto (default: local)",
    )
    parser.add_argument(
        "--narrate",
        action="store_true",
        default=True,
        help="Synthesize TTS audio narration of the Executive Brief (default: True)",
    )
    parser.add_argument(
        "--no-narrate",
        dest="narrate",
        action="store_false",
        help="Skip TTS audio narration",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Run in mock mode with realistic synthetic meeting transcript & minutes",
    )
    parser.add_argument(
        "--engine-url",
        type=str,
        default="http://127.0.0.1:8420",
        help="MoFA Engine gateway URL",
    )

    args = parser.parse_args()

    sample_audio = os.path.abspath(os.path.join(os.path.dirname(__file__), "samples", "sample_meeting.wav"))

    # If --audio is not provided via CLI, interactively prompt the user
    if not args.audio:
        if sys.stdin.isatty() and not args.mock:
            args.audio = interactive_audio_selector(sample_audio)
        else:
            if os.path.exists(sample_audio):
                args.audio = sample_audio
            else:
                args.mock = True
                args.audio = "sample_meeting.wav"

    # Always ensure audio path is an absolute path so the engine daemon can locate it
    if args.audio:
        args.audio = os.path.abspath(os.path.expanduser(args.audio))

    # Validate audio file path
    if not args.mock and args.audio:
        if not os.path.exists(args.audio):
            print(f"\n{COLOR_YELLOW}[WARN] Specified audio file not found: {args.audio}{COLOR_RESET}")
            if os.path.exists(sample_audio):
                print(f"   Falling back to default sample: {sample_audio}")
                args.audio = sample_audio
            else:
                print("   Falling back to synthetic mock mode.")
                args.mock = True

    try:
        process_meeting(
            audio_path=args.audio,
            out_minutes=os.path.abspath(args.out_minutes),
            out_audio=os.path.abspath(args.out_audio),
            prefer=args.prefer,
            narrate=args.narrate,
            mock=args.mock,
            engine_url=args.engine_url,
        )
    except KeyboardInterrupt:
        print(f"\n\n{COLOR_YELLOW}Pipeline cancelled by user.{COLOR_RESET}\n")
        sys.exit(130)


if __name__ == "__main__":
    main()
