#!/usr/bin/env python3
"""Kokoro TTS Bridge — exposes local Kokoro as an OpenAI-compatible TTS endpoint.

Usage:
    source .kokoro-venv/bin/activate
    python3 kokoro_tts_server.py
"""

import json, io, os, tempfile
from http.server import HTTPServer, BaseHTTPRequestHandler

_pipeline = None

from pathlib import Path

def get_pipeline():
    global _pipeline
    if _pipeline is None:
        print("[kokoro] Loading ONNX pipeline...")

        from kokoro_onnx import Kokoro

        model_dir = Path(__file__).parent / ".kokoro-models"

        _pipeline = Kokoro(
            str(model_dir / "kokoro-v1.0.fp16.onnx"),
            str(model_dir / "voices-v1.0.bin"),
        )

        print("[kokoro] Pipeline ready!")

    return _pipeline


class KokoroHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/models":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "data": [{"id": "kokoro", "object": "model"}]
            }).encode())
        else:
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b'{"status":"ok"}')

    def do_POST(self):
        if self.path != "/v1/audio/speech":
            self.send_response(404)
            self.end_headers()
            return

        content_length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(content_length)) if content_length else {}
        text = body.get("input", "Hello from Kokoro")
        raw_voice = body.get("voice", "heart")
        # Map OpenAI voice names to Kokoro format (af_ prefix)
        voice = raw_voice if raw_voice.startswith("af_") else f"af_{raw_voice}"

        print(f"[kokoro] Generating: {text[:80]}... ({len(text)} chars)")
        try:
            import soundfile as sf
            import numpy as np
            import re
            pipeline = get_pipeline()

            # Kokoro has a 510 phoneme limit. Chinese chars expand to ~15 phonemes
            # each, so max ~30 chars. Use 25 for safety margin.
            MAX_CHARS = 25
            # Split on ANY punctuation (Chinese + English)
            parts = re.split(r'(?<=[。！？.!?\n，,、；;：:）)」』])', text)
            chunks = []
            current = ""
            for p in parts:
                if not p:
                    continue
                if len(current) + len(p) > MAX_CHARS and current:
                    chunks.append(current)
                    # If single part is still too long, force-split it
                    while len(p) > MAX_CHARS:
                        chunks.append(p[:MAX_CHARS])
                        p = p[MAX_CHARS:]
                    current = p
                else:
                    current += p
            if current:
                chunks.append(current)
            if not chunks:
                chunks = [text[:MAX_CHARS]]

            print(f"[kokoro] Processing {len(chunks)} chunk(s)...")
            all_samples = []
            sample_rate = 24000
            for i, chunk in enumerate(chunks):
                chunk = chunk.strip()
                if not chunk:
                    continue
                samples, sr = pipeline.create(chunk, voice=voice, speed=1.0)
                all_samples.append(samples)
                sample_rate = sr
                print(f"[kokoro]   chunk {i+1}/{len(chunks)}: '{chunk[:20]}' -> {len(samples)} samples")

            if not all_samples:
                raise RuntimeError("Kokoro returned no audio")

            full_audio = np.concatenate(all_samples)

            buf = io.BytesIO()
            sf.write(buf, full_audio, sample_rate, format="WAV")
            audio_bytes = buf.getvalue()

            tmp = tempfile.NamedTemporaryFile(suffix=".wav", delete=False, dir="/tmp")
            tmp.write(audio_bytes)
            tmp.close()
            print(f"[kokoro] Done: {tmp.name} ({len(audio_bytes):,} bytes)")

            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("X-Audio-File", tmp.name)
            self.send_header("Content-Length", str(len(audio_bytes)))
            self.end_headers()
            self.wfile.write(audio_bytes)
        except Exception as e:
            print(f"[kokoro] ERROR: {e}")
            import traceback; traceback.print_exc()
            self.send_response(500)
            self.end_headers()
            self.wfile.write(json.dumps({"error": str(e)}).encode())

    def log_message(self, *a): pass

if __name__ == "__main__":
    port = int(os.environ.get("KOKORO_PORT", "8421"))
    print(f"[kokoro] Kokoro TTS bridge on http://127.0.0.1:{port}")
    print(f"[kokoro] POST /v1/audio/speech")
    get_pipeline()  # pre-warm
    HTTPServer(("127.0.0.1", port), KokoroHandler).serve_forever()
