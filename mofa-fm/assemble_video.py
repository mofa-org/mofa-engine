#!/usr/bin/env python3
"""Assemble scene images + narration audio into a final MP4 video.

Usage:
  python3 mofa-fm/assemble_video.py --images img1.png img2.png img3.png --audio narration.wav --output video.mp4
"""

import argparse
import os
import subprocess
import shutil
import sys
import tempfile
from pathlib import Path


def get_audio_duration(audio_path: str) -> float:
    if not shutil.which("ffprobe"):
        return 12.0
    result = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "csv=p=0", audio_path],
        capture_output=True, text=True
    )
    try:
        return float(result.stdout.strip() or "12.0")
    except ValueError:
        return 12.0


def assemble_video(image_paths: list, audio_path: str, output_path: str) -> bool:
    if not shutil.which("ffmpeg"):
        print("[ERROR] ffmpeg not found", file=sys.stderr)
        return False

    valid_images = [p for p in image_paths if Path(p).exists()]
    if not valid_images:
        print(f"[ERROR] No valid image files. Checked: {image_paths}", file=sys.stderr)
        return False

    if not Path(audio_path).exists():
        print(f"[ERROR] Audio not found: {audio_path}", file=sys.stderr)
        return False

    print(f"[INFO] Images: {valid_images}", file=sys.stderr)
    print(f"[INFO] Audio: {audio_path} ({Path(audio_path).stat().st_size} bytes)", file=sys.stderr)

    audio_dur = get_audio_duration(audio_path)
    scene_dur = max(3.0, audio_dur / len(valid_images))
    print(f"[INFO] Audio duration: {audio_dur:.1f}s, Scene duration: {scene_dur:.1f}s", file=sys.stderr)

    # Create concat file
    concat_file = Path(tempfile.mktemp(suffix=".txt"))
    with open(concat_file, "w") as f:
        for img in valid_images:
            abs_path = os.path.abspath(img)
            f.write(f"file '{abs_path}'\nduration {scene_dur:.2f}\n")
        # FFmpeg concat demuxer needs last file repeated without duration
        f.write(f"file '{os.path.abspath(valid_images[-1])}'\n")

    Path(output_path).parent.mkdir(parents=True, exist_ok=True)

    # FFmpeg: concat images + audio → MP4
    # -vf scale ensures all images are exactly 1280x720
    # Audio is re-encoded to AAC for MP4 compatibility
    cmd = [
        "ffmpeg", "-y",
        "-f", "concat", "-safe", "0", "-i", str(concat_file),
        "-i", audio_path,
        "-vf", "scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2:color=black",
        "-c:v", "libx264", "-pix_fmt", "yuv420p",
        "-r", "24",
        "-c:a", "aac", "-b:a", "192k", "-ar", "44100", "-ac", "2",
        "-map", "0:v:0", "-map", "1:a:0",
        "-shortest",
        output_path
    ]

    print(f"[INFO] Running: {' '.join(cmd)}", file=sys.stderr)
    result = subprocess.run(cmd, capture_output=True, text=True)

    concat_file.unlink(missing_ok=True)

    if result.returncode != 0:
        print(f"[ERROR] FFmpeg failed (exit {result.returncode}):", file=sys.stderr)
        print(result.stderr[-500:], file=sys.stderr)
        return False

    out = Path(output_path)
    if out.exists() and out.stat().st_size > 0:
        final_dur = get_audio_duration(output_path)
        print(f"[OK] Video: {output_path} ({final_dur:.1f}s, {out.stat().st_size / 1024:.0f}KB)")
        return True

    return False


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--images", nargs="+", required=True)
    parser.add_argument("--audio", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    ok = assemble_video(args.images, args.audio, args.output)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
