#!/usr/bin/env python3
"""Google Gemini Native Audio TTS Adapter for MoFA Engine.

Uses `gemini-2.5-flash-preview-tts` to generate high-fidelity speech audio.
"""

import os
import sys
import json
import base64
import argparse
import urllib.request
import urllib.error
from pathlib import Path


def synthesize(text: str, out_path: str, api_key: str = None) -> bool:
    key = api_key or os.environ.get("GEMINI_API_KEY")
    if not key:
        print("[ERROR] GEMINI_API_KEY environment variable is missing", file=sys.stderr)
        return False

    url = f"https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent?key={key}"
    payload = {
        "contents": [{"parts": [{"text": text}]}],
        "generationConfig": {
            "responseModalities": ["AUDIO"]
        }
    }

    req = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"}
    )

    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            candidates = data.get("candidates", [])
            if not candidates:
                print(f"[ERROR] No candidates returned from Gemini: {data}", file=sys.stderr)
                return False

            parts = candidates[0].get("content", {}).get("parts", [])
            for part in parts:
                inline = part.get("inlineData", {})
                b64_audio = inline.get("data")
                if b64_audio:
                    audio_bytes = base64.b64decode(b64_audio)
                    out = Path(out_path)
                    out.parent.mkdir(parents=True, exist_ok=True)
                    
                    # Gemini audio output is raw 16-bit 24kHz PCM — package into standard WAV container
                    import wave
                    with wave.open(str(out), "wb") as wav_file:
                        wav_file.setnchannels(1)       # Mono
                        wav_file.setsampwidth(2)      # 16-bit (2 bytes)
                        wav_file.setframerate(24000)   # 24kHz standard Gemini speech rate
                        wav_file.writeframes(audio_bytes)
                    return True

            print("[ERROR] No inlineData audio found in Gemini response", file=sys.stderr)
            return False
    except Exception as e:
        print(f"[ERROR] Gemini TTS request failed: {e}", file=sys.stderr)
        return False


def main():
    parser = argparse.ArgumentParser(description="Gemini TTS Adapter")
    parser.add_argument("--text", default=None, help="Text to synthesize")
    parser.add_argument("--text-file", default=None, help="File containing text to synthesize")
    parser.add_argument("--output", required=True, help="Path for output audio file")
    parser.add_argument("--key", default=None, help="Gemini API Key")
    args = parser.parse_args()

    text = ""
    if args.text_file and os.path.exists(args.text_file):
        text = Path(args.text_file).read_text(encoding="utf-8").strip()
    elif args.text:
        text = args.text.strip()

    if not text:
        print("[ERROR] No text provided for synthesis", file=sys.stderr)
        sys.exit(1)

    ok = synthesize(text, args.output, args.key)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
