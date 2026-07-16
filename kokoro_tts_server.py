#!/usr/bin/env python3
"""Kokoro TTS Bridge — exposes local Kokoro as an OpenAI-compatible TTS endpoint.

Uses kokoro_onnx for synthesis and misaki for Chinese phonemization.

Usage:
    source .kokoro-venv/bin/activate
    python3 kokoro_tts_server.py
"""

import json, io, os, tempfile, re
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path

_kokoro = None
_zh_g2p = None

# Voice name mapping: friendly names → Kokoro voice IDs
VOICE_MAP = {
    # Chinese voices
    "xiaoxiao": "zf_xiaoxiao", "yunxi": "zm_yunxi", "xiaoni": "zf_xiaoni",
    "xiaoyi": "zf_xiaoyi", "xiaobei": "zf_xiaobei",
    "yunjian": "zm_yunjian", "yunyang": "zm_yunyang", "yunxia": "zm_yunxia",
    # English voices
    "alloy": "af_alloy", "nova": "af_nova", "echo": "am_echo",
    "fable": "bm_fable", "onyx": "am_onyx", "shimmer": "af_bella",
    "heart": "af_heart",
}

# Chinese voice prefixes
CHINESE_PREFIXES = ("zf_", "zm_")


def get_kokoro():
    global _kokoro
    if _kokoro is None:
        print("[kokoro] Loading ONNX pipeline...")
        from kokoro_onnx import Kokoro
        model_dir = Path(__file__).parent / ".kokoro-models"
        _kokoro = Kokoro(
            str(model_dir / "kokoro-v1.0.fp16.onnx"),
            str(model_dir / "voices-v1.0.bin"),
        )
        print("[kokoro] ONNX pipeline ready!")
    return _kokoro


def get_zh_g2p():
    global _zh_g2p
    if _zh_g2p is None:
        print("[kokoro] Loading Chinese phonemizer (misaki)...")
        from misaki.zh import ZHG2P
        _zh_g2p = ZHG2P()
        print("[kokoro] Chinese phonemizer ready!")
    return _zh_g2p


def resolve_voice(raw_voice: str) -> str:
    """Resolve a voice name to a Kokoro voice ID."""
    raw = raw_voice.lower().strip()
    if raw in VOICE_MAP:
        return VOICE_MAP[raw]
    if any(raw.startswith(p) for p in ("af_", "am_", "bf_", "bm_", "ef_", "em_", "zf_", "zm_")):
        return raw
    return f"af_{raw}"


def is_chinese_voice(voice_id: str) -> bool:
    return voice_id.startswith(CHINESE_PREFIXES)


def synthesize(text: str, voice: str) -> bytes:
    """Synthesize text to WAV bytes."""
    import soundfile as sf
    import numpy as np

    kokoro = get_kokoro()
    use_chinese = is_chinese_voice(voice)

    if use_chinese:
        # Use misaki to convert Chinese text → phonemes
        g2p = get_zh_g2p()

    # Split text into chunks for processing
    MAX_CHARS = 80 if use_chinese else 200
    parts = re.split(r'(?<=[。！？.!?\n，,；;])', text)
    chunks = []
    current = ""
    for p in parts:
        if not p:
            continue
        if len(current) + len(p) > MAX_CHARS and current:
            chunks.append(current)
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

    print(f"[kokoro] Processing {len(chunks)} chunk(s), chinese={use_chinese}...")
    all_samples = []
    sample_rate = 24000

    for i, chunk in enumerate(chunks):
        chunk = chunk.strip()
        if not chunk:
            continue

        if use_chinese:
            # Convert Chinese text to phonemes, then synthesize
            phonemes = g2p(chunk)
            print(f"[kokoro]   chunk {i+1}/{len(chunks)}: '{chunk[:30]}' -> phonemes: '{phonemes[:40]}'")
            samples, sr = kokoro.create(phonemes, voice=voice, speed=1.0, is_phonemes=True)
        else:
            print(f"[kokoro]   chunk {i+1}/{len(chunks)}: '{chunk[:30]}'")
            samples, sr = kokoro.create(chunk, voice=voice, speed=1.0)

        all_samples.append(samples)
        sample_rate = sr

    if not all_samples:
        raise RuntimeError("Kokoro returned no audio")

    full_audio = np.concatenate(all_samples)

    buf = io.BytesIO()
    sf.write(buf, full_audio, sample_rate, format="WAV")
    return buf.getvalue()


class KokoroHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/v1/models":
            body = json.dumps({"data": [{"id": "kokoro", "object": "model"}]}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)
        else:
            body = b'{"status":"ok"}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

    def do_POST(self):
        if self.path != "/v1/audio/speech":
            self.send_response(404)
            self.end_headers()
            return

        content_length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(content_length)) if content_length else {}
        text = body.get("input", "Hello from Kokoro")
        raw_voice = body.get("voice", "xiaoxiao")
        voice = resolve_voice(raw_voice)

        print(f"[kokoro] Request: voice={voice} text='{text[:60]}...' ({len(text)} chars)")

        try:
            audio_bytes = synthesize(text, voice)

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
            body = json.dumps({"error": str(e)}).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

    def log_message(self, *a): pass


if __name__ == "__main__":
    port = int(os.environ.get("KOKORO_PORT", "8421"))
    print(f"[kokoro] Kokoro TTS bridge on http://127.0.0.1:{port}")
    print(f"[kokoro] POST /v1/audio/speech")
    print(f"[kokoro] Voices: {', '.join(sorted(VOICE_MAP.keys()))}")
    # Pre-warm both the ONNX model and the Chinese phonemizer
    get_kokoro()
    get_zh_g2p()
    print(f"[kokoro] Ready! Listening...")
    HTTPServer(("127.0.0.1", port), KokoroHandler).serve_forever()
