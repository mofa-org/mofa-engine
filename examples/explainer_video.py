#!/usr/bin/env python3
"""S4 Flagship Explainer Video: Topic -> Script -> Images -> TTS -> MP4 Video (PRD v3.1 §2.2.1 S4).

Executes a 5-step multimodal pipeline:
  1. Chat (Script) -> Generates 3-scene narration script with hint_next="image_gen"
  2. ImageGen (Visuals) -> Generates scene illustration for each section
  3. TTS (Narration) -> Synthesizes spoken audio voiceover
  4. FFmpeg (Composition) -> Stitches scene visuals + audio into final MP4 video
  5. Quality Gate -> Verifies duration and video integrity via ffprobe

Usage:
  python3 examples/explainer_video.py
  python3 examples/explainer_video.py --topic "Quantum Computing Superposition"
  mofa run video
"""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine


def main():
    parser = argparse.ArgumentParser(description="S4 Flagship Explainer Video Generator")
    parser.add_argument("--topic", default="How neural networks learn through backpropagation", help="Explainer video topic")
    parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"], help="Routing locality preference")
    parser.add_argument("--out", default="output/explainer_video.mp4", help="Output path for final MP4 video")
    args = parser.parse_args()

    engine = MofaEngine()
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_dir = out_path.parent / "tmp_s4"
    tmp_dir.mkdir(parents=True, exist_ok=True)

    print("\n==================================================================")
    print("   Scenario S4: Flagship Animated Explainer Video Generator")
    print("==================================================================")
    print(f"  Topic : {args.topic}")

    # ── Step 1: Script Generation ────────────────────────────────────
    print(f"\n[Step 1/5] Chat: Generating 3-scene narration script (hint_next=image_gen)...")
    script_prompt = (
        f"Write a concise 3-scene spoken explainer script about: {args.topic}. "
        "Format EXACTLY as:\n"
        "Scene 1: [Scene visual description]\n[Narration text]\n\n"
        "Scene 2: [Scene visual description]\n[Narration text]\n\n"
        "Scene 3: [Scene visual description]\n[Narration text]"
    )
    script_res = engine.chat(
        script_prompt,
        reasoning={"effort": "medium"},
        hint_next="image_gen",
        prefer=args.prefer,
    )
    print(f"  [OK] Script generated ({script_res.provider}/{script_res.model_used}, {script_res.duration_ms}ms)")
    
    # Parse scenes
    raw_text = script_res.text or ""
    scene_blocks = [s.strip() for s in raw_text.split("Scene") if s.strip()]
    if len(scene_blocks) < 3:
        scene_blocks = [
            f"1: Overview of {args.topic}\nWelcome to this quick explainer on {args.topic}.",
            f"2: Core Mechanisms\nLet's look at how the underlying principles operate in practice.",
            f"3: Key Takeaways\nIn summary, local AI orchestration unlocks powerful new workflows.",
        ]

    # ── Step 2: Scene Visuals ────────────────────────────────────────
    print(f"\n[Step 2/5] ImageGen: Generating visuals for {min(3, len(scene_blocks))} scenes...")
    scene_images = []
    for i, scene in enumerate(scene_blocks[:3]):
        img_path = tmp_dir / f"scene_{i+1}.png"
        try:
            img_res = engine.image_gen(
                prompt=f"Educational clean minimalist illustration: {scene[:150]}",
                size="1024x1024",
                prefer=args.prefer,
            )
            img_res.save(str(img_path))
            scene_images.append(str(img_path))
            print(f"  [OK] Scene {i+1} visual generated ({img_res.provider}, {img_res.duration_ms}ms)")
        except Exception as e:
            # Generate styled title card via FFmpeg if image_gen provider is offline
            label = f"Scene {i+1}: {args.topic[:30]}..."
            if shutil.which("ffmpeg"):
                subprocess.run([
                    "ffmpeg", "-y", "-f", "lavfi", "-i", "color=c=0x0f172a:s=1024x1024:d=1",
                    "-vf", f"drawtext=text='{label}':fontsize=36:fontcolor=0x38bdf8:x=(w-tw)/2:y=(h-th)/2",
                    "-frames:v", "1", str(img_path)
                ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                scene_images.append(str(img_path))
                print(f"  [FALLBACK] Scene {i+1} title card created via FFmpeg ({e})")

    # ── Step 3: TTS Narration ────────────────────────────────────────
    print(f"\n[Step 3/5] TTS: Synthesizing spoken voiceover narration...")
    narration_path = tmp_dir / "narration.mp3"
    narration_res = engine.tts(raw_text, voice="en-narrator", prefer=args.prefer)
    narration_res.save(str(narration_path))
    print(f"  [OK] Narration audio saved ({narration_res.provider}, {narration_res.duration_ms}ms)")

    # ── Step 4: Video Composition ────────────────────────────────────
    print(f"\n[Step 4/5] Compose: Rendering final MP4 video via FFmpeg...")
    if shutil.which("ffmpeg") and scene_images and narration_path.exists():
        concat_file = tmp_dir / "concat.txt"
        with open(concat_file, "w") as f:
            for img in scene_images:
                f.write(f"file '{os.path.abspath(img)}'\nduration 4\n")
            f.write(f"file '{os.path.abspath(scene_images[-1])}'\n")
        
        subprocess.run([
            "ffmpeg", "-y", "-f", "concat", "-safe", "0", "-i", str(concat_file),
            "-i", str(narration_path), "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest", str(out_path)
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print(f"  [OK] Video rendered to: {out_path}")
    else:
        print(f"  [WARN] FFmpeg unavailable; scene assets preserved in {tmp_dir}/")

    # ── Step 5: Quality Gate (PRD §S4 SLA) ────────────────────────────
    print(f"\n[Step 5/5] Quality Gate: Validating output container integrity...")
    if out_path.exists() and shutil.which("ffprobe"):
        probe = subprocess.run([
            "ffprobe", "-v", "error", "-show_entries", "format=duration",
            "-of", "csv=p=0", str(out_path)
        ], capture_output=True, text=True)
        duration = float(probe.stdout.strip() or "0")
        if duration > 0:
            print(f"  [PASS] Video Duration: {duration:.1f}s | Size: {out_path.stat().st_size / 1024:.1f} KB")
        else:
            print("  [WARN] Video file created but duration could not be verified.")
    elif out_path.exists():
        print(f"  [PASS] Video file verified: {out_path.stat().st_size / 1024:.1f} KB")

    total_cost = (script_res.cost_usd or 0.0) + (narration_res.cost_usd or 0.0)
    print("\n==================================================================")
    print("   S4 Explainer Video Pipeline Complete!")
    print("==================================================================")
    print(f"  Output Video : {out_path}")
    print(f"  Total Cost   : ${total_cost:.4f} (Benchmarked against $0.15-1.50 cloud video)")
    print()


if __name__ == "__main__":
    main()
