"""MoFA Engine Python SDK — High Performance Intelligent Gateway Client.

Connects to default daemon on http://127.0.0.1:8420.
Supports locality constraints, streaming SSE, voice alias resolution,
multimodal vision, image generation, reasoning API, and event subscriptions.

PRD v3.1 §6.3 — Full Python SDK surface.
"""

import json
import os
import base64
import shutil
import subprocess
import time
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Generator, AsyncGenerator, Any, Union
from pathlib import Path

try:
    import requests
    HAS_REQUESTS = True
except ImportError:
    HAS_REQUESTS = False
    import urllib.request
    import urllib.error
    import urllib.parse

    class SimpleResponse:
        def __init__(self, status_code: int, content: bytes, headers: dict = None):
            self.status_code = status_code
            self._content = content
            self.headers = headers or {}

        def json(self):
            return json.loads(self._content.decode("utf-8"))

        @property
        def text(self):
            return self._content.decode("utf-8")

        def raise_for_status(self):
            if self.status_code >= 400:
                raise RuntimeError(f"HTTP {self.status_code}: {self.text}")

        def iter_lines(self, decode_unicode=True):
            for line in self._content.splitlines():
                if decode_unicode:
                    yield line.decode("utf-8")
                else:
                    yield line

    class SimpleSession:
        def __init__(self):
            self.trust_env = False

        def get(self, url: str, headers: dict = None, params: dict = None, timeout: float = 30, stream: bool = False):
            if params:
                query = urllib.parse.urlencode(params)
                url = f"{url}?{query}"
            req = urllib.request.Request(url, headers=headers or {})
            try:
                with urllib.request.urlopen(req, timeout=timeout) as resp:
                    return SimpleResponse(resp.status, resp.read(), dict(resp.headers))
            except urllib.error.HTTPError as e:
                return SimpleResponse(e.code, e.read(), dict(e.headers))

        def post(self, url: str, json: dict = None, data: dict = None, files: dict = None, headers: dict = None, timeout: float = 300, stream: bool = False):
            headers = headers or {}
            body_bytes = b""
            if json is not None:
                headers["Content-Type"] = "application/json"
                body_bytes = __import__("json").dumps(json).encode("utf-8")
            elif files or data:
                # Basic multipart or form encoding
                boundary = "----MofaSdkBoundary" + os.urandom(8).hex()
                headers["Content-Type"] = f"multipart/form-data; boundary={boundary}"
                chunks = []
                if data:
                    for k, v in data.items():
                        chunks.append(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n".encode("utf-8"))
                if files:
                    for k, (fname, fobj, ftype) in files.items():
                        fdata = fobj.read() if hasattr(fobj, "read") else fobj
                        chunks.append(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"; filename=\"{fname}\"\r\nContent-Type: {ftype}\r\n\r\n".encode("utf-8") + fdata + b"\r\n")
                chunks.append(f"--{boundary}--\r\n".encode("utf-8"))
                body_bytes = b"".join(chunks)

            req = urllib.request.Request(url, data=body_bytes, headers=headers)
            try:
                with urllib.request.urlopen(req, timeout=timeout) as resp:
                    return SimpleResponse(resp.status, resp.read(), dict(resp.headers))
            except urllib.error.HTTPError as e:
                return SimpleResponse(e.code, e.read(), dict(e.headers))

VOICE_ALIASES: Dict[str, str] = {
    "zh-female-1": "af_heart",
    "zh-male-1": "af_alloy",
    "en-narrator": "af_alloy",
    "af_alloy": "af_alloy",
    "af_heart": "af_heart",
    "alloy": "af_alloy",
}


@dataclass
class InvokeResult:
    """Result of an engine invocation."""

    text: Optional[str] = None
    file: Optional[str] = None
    url: Optional[str] = None
    model_used: str = "unknown"
    provider: str = "unknown"
    duration_ms: int = 0
    request_id: str = ""
    tokens_used: Optional[int] = None
    cost_usd: float = 0.0
    locality: str = "local"
    words: Optional[List[Dict[str, Any]]] = None

    # ─── Ergonomic Response Helpers ──────────────────────────────────

    def save(self, path: str) -> str:
        """Save the result artifact (audio/image/markdown text) to a local file.

        Handles server-side file copies, remote URLs, or text content automatically.
        """
        target = Path(path)
        target.parent.mkdir(parents=True, exist_ok=True)

        if self.file and os.path.exists(self.file):
            shutil.copy(self.file, target)
            return str(target)

        if self.url:
            try:
                import urllib.request
                urllib.request.urlretrieve(self.url, str(target))
                return str(target)
            except Exception:
                pass

        if self.text is not None:
            target.write_text(self.text, encoding="utf-8")
            return str(target)

        if self.file:
            # If server path is relative/unreachable directly, record as text
            target.write_text(f"Artifact location: {self.file}", encoding="utf-8")
            return str(target)

        raise ValueError("No text, file, or url available to save.")

    def play(self) -> bool:
        """Play audio artifacts using system audio player (macOS `afplay` or Linux `aplay`)."""
        audio_path = None
        if self.file and os.path.exists(self.file):
            audio_path = self.file

        if not audio_path:
            return False

        import subprocess
        for player in ["afplay", "paplay", "aplay"]:
            if shutil.which(player):
                try:
                    subprocess.run([player, audio_path], check=False)
                    return True
                except Exception:
                    pass
        return False

    def to_bytes(self) -> bytes:
        """Return raw binary bytes of the artifact file or encoded text."""
        if self.file and os.path.exists(self.file):
            return Path(self.file).read_bytes()
        if self.text:
            return self.text.encode("utf-8")
        return b""

    def to_markdown(self) -> str:
        """Format response as a clean Markdown report with telemetry metadata."""
        body = self.text or f"Artifact generated: `{self.file or self.url}`"
        return (
            f"{body}\n\n"
            f"---\n"
            f"*Generated by **{self.provider}/{self.model_used}** "
            f"({self.locality.upper()}) in {self.duration_ms}ms · "
            f"Cost: ${self.cost_usd:.6f}*"
        )

    def show(self) -> None:
        """Print result to terminal with formatted badges and telemetry."""
        loc_badge = "\033[32m[LOCAL $0.00]\033[0m" if self.is_local else f"\033[33m[CLOUD ${self.cost_usd:.4f}]\033[0m"
        print(f"\n{loc_badge} \033[1m{self.provider}/{self.model_used}\033[0m ({self.duration_ms}ms)")
        if self.text:
            print(f"\n{self.text}\n")
        if self.file:
            print(f"Artifact: {self.file}")

    @property
    def is_local(self) -> bool:
        """True if invocation was served on-device with zero egress cost."""
        if self.locality:
            return self.locality.lower() == "local"
        return self.provider.lower() in ("ollama", "funasr", "kokoro", "local-tts", "local-asr", "local-sd")

    @property
    def savings_vs_cloud(self) -> float:
        """Estimated USD saved by routing locally instead of commercial cloud APIs."""
        # Industry baseline ~$0.003/1K tokens or ~$0.015/min audio
        if not self.is_local:
            return 0.0
        tokens = self.tokens_used or 500
        return max(0.001, (tokens / 1000.0) * 0.005)



@dataclass
class StreamEvent:
    """A single event from a streaming response (SSE)."""

    type: str  # "text", "reasoning", "started", "completed", "error"
    delta: str = ""
    content: str = ""
    metadata: Dict[str, Any] = field(default_factory=dict)


class MofaEngine:
    """Client for the MoFA Engine API Gateway daemon.

    Provides unified access to Chat, TTS, ASR, VLM, ImageGen,
    and Reasoning capabilities via a single SDK interface.
    """

    def __init__(self, base_url: str = "http://127.0.0.1:8420"):
        self.base_url = base_url.rstrip("/")
        if HAS_REQUESTS:
            self.session = requests.Session()
            self.session.trust_env = False
        else:
            self.session = SimpleSession()

    # ─── Health & Status ──────────────────────────────────────────────

    def health(self) -> dict:
        """Check engine daemon health status."""
        return self.session.get(f"{self.base_url}/health", timeout=5).json()

    def status(self) -> dict:
        """Get live engine status, model residency, and memory gauge state."""
        return self.session.get(f"{self.base_url}/v1/status", timeout=10).json()

    def capabilities(self) -> list:
        """List active capabilities available on connected providers."""
        return self.session.get(f"{self.base_url}/v1/capabilities", timeout=10).json()

    def cost(self) -> dict:
        """Get accumulated cost and token usage metrics."""
        try:
            resp = self.session.get(f"{self.base_url}/v1/cost", timeout=5)
            if resp.status_code == 200:
                return resp.json()
        except Exception:
            pass

        # Parse from Prometheus /metrics text format
        total_cost = 0.0
        local_reqs = 0
        cloud_reqs = 0
        try:
            m_resp = self.session.get(f"{self.base_url}/metrics", timeout=5)
            if m_resp.status_code == 200:
                for line in m_resp.text.splitlines():
                    if line.startswith("mofa_cost_usd_total"):
                        parts = line.rsplit(None, 1)
                        if len(parts) == 2:
                            try:
                                total_cost += float(parts[1])
                            except ValueError:
                                pass
                    elif line.startswith("mofa_requests_total"):
                        if 'locality="local"' in line:
                            local_reqs += 1
                        elif 'locality="cloud"' in line:
                            cloud_reqs += 1
        except Exception:
            pass

        return {
            "total_cost_usd": total_cost,
            "local_requests": local_reqs,
            "cloud_requests": cloud_reqs,
            "estimated_savings_usd": max(0.0, local_reqs * 0.003),
        }

    # ─── Core Invoke ──────────────────────────────────────────────────

    def invoke(
        self,
        *,
        capability: Optional[str] = None,
        model: Optional[str] = None,
        text: Optional[str] = None,
        messages: Optional[List[Dict[str, Any]]] = None,
        input_file: Optional[str] = None,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        fallback_policy: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        params: Optional[Dict[str, Any]] = None,
        timeout: int = 300,
    ) -> InvokeResult:
        """Invoke a capability on the engine.

        Args:
            capability: One of chat, tts, asr, vlm, image_gen, video_gen, embedding.
            model: Specific model name to route to (optional).
            text: Text input (converted to messages automatically).
            messages: Chat messages list [{role, content}].
            input_file: Path to input file (for ASR).
            hint_next: Next capability hint for cross-capability warmup.
            prefer: Routing preference — "local", "cloud", or "auto" (PRD §4.1).
            fallback_policy: Fallback behavior on failure.
            reasoning: Reasoning config {effort: low|medium|high} (PRD §3.4).
            params: Capability-specific parameters passed through to backend.
            timeout: HTTP request timeout in seconds.

        Returns:
            InvokeResult with text, file, provider, cost, and metadata.
        """
        body: Dict[str, Any] = {}
        if capability:
            body["capability"] = capability
        if model:
            body["model"] = model
        if hint_next:
            body["hint_next"] = hint_next
        if input_file:
            body["input_file"] = input_file
        if prefer:
            body["prefer"] = prefer
        if fallback_policy:
            body["fallback_policy"] = fallback_policy
        if reasoning:
            body["reasoning"] = reasoning

        # Voice alias resolution for TTS
        request_params = params.copy() if params else {}
        if capability == "tts" and "voice" in request_params:
            requested_voice = request_params["voice"]
            request_params["voice"] = VOICE_ALIASES.get(requested_voice, requested_voice)
        if request_params:
            body["params"] = request_params

        if messages:
            body["messages"] = messages
        elif text:
            body["messages"] = [{"role": "user", "content": text}]

        r = self.session.post(f"{self.base_url}/v1/invoke", json=body, timeout=timeout)
        r.raise_for_status()
        d = r.json()

        provider_name = d.get("provider", "unknown")
        detected_locality = d.get("locality")
        if not detected_locality:
            detected_locality = "cloud" if provider_name.lower() in ("gemini", "openai", "deepseek", "anthropic", "fireworks") else "local"

        return InvokeResult(
            text=d.get("text"),
            file=d.get("file"),
            url=d.get("url"),
            model_used=d.get("model_used", d.get("model", "unknown")),
            provider=provider_name,
            duration_ms=d.get("duration_ms", 0),
            request_id=d.get("request_id", ""),
            tokens_used=d.get("tokens_used"),
            cost_usd=float(d.get("cost_usd") if d.get("cost_usd") is not None else 0.0),
            locality=detected_locality,
            words=d.get("words"),
        )

    # ─── Chat ─────────────────────────────────────────────────────────

    def chat(
        self,
        text: str,
        *,
        model: Optional[str] = None,
        messages: Optional[List[Dict[str, Any]]] = None,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        **kw,
    ) -> InvokeResult:
        """Chat inference with optional reasoning and locality control.

        Args:
            text: User message text.
            model: Target model (optional, engine routes automatically).
            messages: Full message list (overrides text if provided).
            hint_next: Next capability hint for warmup (e.g., "tts", "image_gen").
            prefer: "local" | "cloud" | "auto" routing preference.
            reasoning: {effort: "low"|"medium"|"high"} for deep thinking.
        """
        return self.invoke(
            capability="chat",
            model=model,
            text=text,
            messages=messages,
            hint_next=hint_next,
            prefer=prefer,
            reasoning=reasoning,
            **kw,
        )

    # ─── Chat Stream (SSE) ────────────────────────────────────────────

    def chat_stream(
        self,
        text: str,
        *,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> Generator[StreamEvent, None, None]:
        """Stream chat tokens in real-time via SSE.

        Yields StreamEvent objects with type="text" for content deltas.
        """
        body: Dict[str, Any] = {
            "capability": "chat",
            "messages": [{"role": "user", "content": text}],
        }
        if model:
            body["model"] = model
        if prefer:
            body["prefer"] = prefer

        response = self.session.post(
            f"{self.base_url}/v1/invoke/stream",
            json=body,
            stream=True,
            timeout=300,
        )
        response.raise_for_status()

        for line in response.iter_lines(decode_unicode=True):
            if not line or not line.startswith("data: "):
                continue
            raw_data = line[6:]
            if raw_data == "[DONE]":
                break
            try:
                chunk = json.loads(raw_data)
                yield StreamEvent(
                    type=chunk.get("type", "text"),
                    delta=chunk.get("content", chunk.get("delta", "")),
                    content=chunk.get("content", ""),
                    metadata=chunk,
                )
            except json.JSONDecodeError:
                yield StreamEvent(type="text", delta=raw_data, content=raw_data)

    # ─── Responses API (Reasoning Stream) ─────────────────────────────

    def responses(
        self,
        input: str,
        *,
        reasoning: Optional[Dict[str, Any]] = None,
        stream: bool = True,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> Generator[StreamEvent, None, None]:
        """Responses API for deep thinking with reasoning stream (PRD §3.1/S2).

        Streams reasoning and output increments separately, enabling
        collapsible thought chain display and compliance audit.

        Args:
            input: The input text (e.g., git diff, contract text).
            reasoning: {effort: "low"|"medium"|"high"} tier routing.
            stream: Whether to stream (default True).
            model: Specific model override.
            prefer: "local" | "cloud" routing preference.

        Yields:
            StreamEvent with type="reasoning" (thought chain) or type="output" (final report).
        """
        body: Dict[str, Any] = {
            "capability": "chat",
            "messages": [{"role": "user", "content": input}],
        }
        if reasoning:
            body["reasoning"] = reasoning
        if model:
            body["model"] = model
        if prefer:
            body["prefer"] = prefer

        if stream:
            response = self.session.post(
                f"{self.base_url}/v1/invoke/stream",
                json=body,
                stream=True,
                timeout=600,
            )
            response.raise_for_status()

            for line in response.iter_lines(decode_unicode=True):
                if not line or not line.startswith("data: "):
                    continue
                raw_data = line[6:]
                if raw_data == "[DONE]":
                    break
                try:
                    chunk = json.loads(raw_data)
                    yield StreamEvent(
                        type=chunk.get("type", "output"),
                        delta=chunk.get("content", chunk.get("delta", "")),
                        content=chunk.get("content", ""),
                        metadata=chunk,
                    )
                except json.JSONDecodeError:
                    yield StreamEvent(type="output", delta=raw_data, content=raw_data)
        else:
            result = self.invoke(
                capability="chat",
                text=input,
                reasoning=reasoning,
                model=model,
                prefer=prefer,
            )
            yield StreamEvent(type="output", delta=result.text or "", content=result.text or "")

    # ─── TTS ──────────────────────────────────────────────────────────

    def tts(
        self,
        text: str,
        *,
        voice: str = "zh-female-1",
        model: Optional[str] = None,
        speed: float = 1.0,
        prefer: Optional[str] = None,
        hint_next: Optional[str] = None,
    ) -> InvokeResult:
        """Text-to-speech audio synthesis (PRD §3.2).

        Args:
            text: Script/text to synthesize.
            voice: Voice alias (e.g., "zh-female-1", "en-narrator").
            speed: Playback speed (0.5–2.0).
            prefer: "local" for Crane/Kokoro, "cloud" for tts-1.
        """
        resolved_voice = VOICE_ALIASES.get(voice, voice)
        try:
            return self.invoke(
                capability="tts",
                model=model,
                text=text,
                prefer=prefer,
                hint_next=hint_next,
                params={"voice": resolved_voice, "speed": speed},
            )
        except Exception as e:
            import tempfile
            import subprocess
            if shutil.which("say"):
                t0 = time.time()
                tmp_out = Path(tempfile.gettempdir()) / f"mofa_tts_{int(time.time()*1000)}.aiff"
                subprocess.run(["say", "-o", str(tmp_out), text[:400]], check=False)
                if tmp_out.exists():
                    duration_ms = int((time.time() - t0) * 1000)
                    return InvokeResult(
                        file=str(tmp_out),
                        model_used="system-voice",
                        provider="macos-say",
                        duration_ms=duration_ms,
                        cost_usd=0.0,
                        locality="local",
                    )
            raise e

    # ─── ASR (with file upload) ───────────────────────────────────────

    def asr(
        self,
        audio_file: str,
        *,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
        diarize: bool = False,
        language: Optional[str] = None,
        hint_next: Optional[str] = None,
    ) -> InvokeResult:
        """Speech recognition / transcription (PRD §3.3/S1).

        Supports both local file path and multipart HTTP upload.

        Args:
            audio_file: Path to audio file (.wav, .mp3, .m4a).
            model: ASR model override.
            prefer: "local" for FunASR, "cloud" for whisper-1.
            diarize: Enable speaker diarization (speaker attribution).
            language: Language hint (auto-detect if not specified).
        """
        file_path = Path(audio_file)
        if file_path.exists() and file_path.stat().st_size < 25 * 1024 * 1024:
            # Multipart upload for files < 25MB
            with open(file_path, "rb") as f:
                files = {"file": (file_path.name, f, "audio/wav")}
                data: Dict[str, Any] = {"capability": "asr"}
                if model:
                    data["model"] = model
                if prefer:
                    data["prefer"] = prefer
                if diarize:
                    data["diarize"] = "true"
                if language:
                    data["language"] = language
                try:
                    r = self.session.post(
                        f"{self.base_url}/v1/audio/transcriptions",
                        files=files,
                        data=data,
                        timeout=600,
                    )
                    r.raise_for_status()
                    d = r.json()
                    return InvokeResult(
                        text=d.get("text"),
                        model_used=d.get("model_used", d.get("model", "unknown")),
                        provider=d.get("provider", "unknown"),
                        duration_ms=d.get("duration_ms", 0),
                        request_id=d.get("request_id", ""),
                        tokens_used=d.get("tokens_used"),
                        cost_usd=d.get("cost_usd", 0.0),
                        locality=d.get("locality", "local"),
                        words=d.get("words"),
                    )
                except Exception:
                    pass  # Fall through to invoke-based approach

        # Fallback: use invoke with input_file path
        params: Dict[str, Any] = {}
        if diarize:
            params["diarize"] = True
        if language:
            params["language"] = language
        return self.invoke(
            capability="asr",
            model=model,
            input_file=audio_file,
            prefer=prefer,
            params=params if params else None,
        )

    # ─── Image Generation ─────────────────────────────────────────────

    def image_gen(
        self,
        prompt: str,
        *,
        size: str = "1024x1024",
        style: Optional[str] = None,
        n: int = 1,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        """Generate images from text prompts (PRD §3.6/S4).

        Args:
            prompt: Text description of the image to generate.
            size: Output image size (e.g., "1024x1024", "512x512").
            style: Style preset (e.g., "vivid", "natural").
            n: Number of images to generate.
            model: Model override (e.g., "dall-e-3", "stable-diffusion").
            prefer: "local" for SD/SDXL, "cloud" for DALL-E/FLUX.

        Returns:
            InvokeResult with url or file pointing to generated image.
        """
        params: Dict[str, Any] = {"prompt": prompt, "size": size, "n": n}
        if style:
            params["style"] = style
        return self.invoke(
            capability="image_gen",
            model=model,
            text=prompt,
            prefer=prefer,
            params=params,
        )

    # ─── Video Generation (Skeleton — PRD §8.6 scope boundary) ────────

    def video_gen(
        self,
        prompt: str,
        *,
        duration_secs: int = 10,
        resolution: str = "720p",
        style: Optional[str] = None,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
        cancel_token: Optional[str] = None,
    ) -> InvokeResult:
        """Generate video from text prompt (PRD §3.6 — interface skeleton only).

        NOTE: True video generation is a scope boundary (PRD §8.6).
        The engine will return UnsupportedOperation until a VideoGen
        backend is mounted. This skeleton ensures SDK surface parity
        and allows downstream code to type-check against the interface.

        Args:
            prompt: Text description of the video to generate.
            duration_secs: Target video duration in seconds (default 10).
            resolution: Output resolution — "480p", "720p", "1080p".
            style: Style preset (e.g., "cinematic", "animation").
            model: Model override (future: "sora", "runway-gen3").
            prefer: "local" | "cloud" routing preference.
            cancel_token: Client-supplied cancellation token for async abort.

        Returns:
            InvokeResult with task status or UnsupportedOperation error.

        Raises:
            requests.HTTPError: 400 UnsupportedOperation from engine.
        """
        params: Dict[str, Any] = {
            "prompt": prompt,
            "duration_secs": duration_secs,
            "resolution": resolution,
        }
        if style:
            params["style"] = style
        if cancel_token:
            params["cancel_token"] = cancel_token
        return self.invoke(
            capability="video_gen",
            model=model,
            text=prompt,
            prefer=prefer,
            params=params,
        )

    # ─── Multimodal Understanding (Vision / VLM) ──────────────────────

    def understand(
        self,
        *,
        images: Optional[List[str]] = None,
        question: str = "",
        detail: str = "auto",
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        """Vision / multimodal understanding (PRD §3.5/S3).

        Send images + question to VLM for structured data extraction.

        Args:
            images: List of image paths, URLs, or base64 strings.
            question: Question to ask about the image(s).
            detail: Cloud billing tier — "low", "high", or "auto".
            model: VLM model override.
            prefer: "local" for local VLM, "cloud" for GPT-4o/Qwen-VL.

        Returns:
            InvokeResult with extracted text/structured data.
        """
        image_list: List[str] = []
        for img in (images or []):
            img_path = Path(img)
            if img_path.exists():
                with open(img_path, "rb") as f:
                    b64 = base64.b64encode(f.read()).decode("utf-8")
                    ext = img_path.suffix.lstrip(".").lower()
                    mime = {"jpg": "jpeg", "jpeg": "jpeg", "png": "png", "gif": "gif", "webp": "webp"}.get(ext, "jpeg")
                    image_list.append(f"data:image/{mime};base64,{b64}")
            else:
                image_list.append(img)

        text_prompt = question or "Describe and extract structured information from this image."
        messages = [{"role": "user", "content": text_prompt, "images": image_list}]
        params: Dict[str, Any] = {"detail": detail}
        return self.invoke(
            capability="vlm",
            model=model,
            messages=messages,
            prefer=prefer,
            params=params,
        )

    # ─── Embedding / Vectorization (PRD §3.7) ─────────────────────────

    def embed(
        self,
        texts: Union[str, List[str]],
        *,
        model: Optional[str] = None,
        dimensions: Optional[int] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        """Generate text embeddings for semantic search, vector databases, and RAG (PRD §3.7).

        Args:
            texts: Single string or list of strings to generate embeddings for.
            model: Specific embedding model override (e.g., 'nomic-embed-text', 'text-embedding-3-small').
            dimensions: Desired output vector dimensions.
            prefer: 'local' for on-device Ollama embeddings, 'cloud' for commercial provider.

        Returns:
            InvokeResult with JSON string containing vector embeddings and telemetry.
        """
        input_texts = [texts] if isinstance(texts, str) else texts
        params: Dict[str, Any] = {"input": input_texts}
        if dimensions:
            params["dimensions"] = dimensions
        return self.invoke(
            capability="embedding",
            model=model,
            prefer=prefer,
            params=params,
        )

    # ─── Subscriptions & Events ───────────────────────────────────────

    def subscribe(
        self,
        capabilities: List[str],
        *,
        ttl_secs: int = 300,
    ) -> dict:
        """Subscribe to keep models warm for specified capabilities (PRD §4.3).

        Args:
            capabilities: List of capabilities to keep warm (e.g., ["vlm", "tts"]).
            ttl_secs: Time-to-live for the subscription in seconds.

        Returns:
            Subscription acknowledgment response.
        """
        body = {"capabilities": capabilities, "subscription_ttl": ttl_secs}
        r = self.session.post(f"{self.base_url}/v1/subscriptions", json=body, timeout=30)
        r.raise_for_status()
        return r.json()

    def events(
        self,
        *,
        last_event_id: Optional[str] = None,
        event_filter: Optional[List[str]] = None,
    ) -> Generator[Dict[str, Any], None, None]:
        """Subscribe to real-time engine events via SSE (PRD §5.4).

        Args:
            last_event_id: Resume from last received event ID (disconnect recovery).
            event_filter: List of event types to filter (e.g., ["inference_complete", "warmup"]).

        Yields:
            Parsed event dictionaries.
        """
        headers: Dict[str, str] = {}
        if last_event_id:
            headers["Last-Event-ID"] = last_event_id

        params: Dict[str, Any] = {}
        if event_filter:
            params["filter"] = ",".join(event_filter)

        response = self.session.get(
            f"{self.base_url}/v1/events",
            headers=headers,
            params=params,
            stream=True,
            timeout=None,
        )
        response.raise_for_status()

        for line in response.iter_lines(decode_unicode=True):
            if not line or not line.startswith("data: "):
                continue
            raw_data = line[6:]
            if raw_data == "[DONE]":
                break
            try:
                yield json.loads(raw_data)
            except json.JSONDecodeError:
                yield {"type": "raw", "content": raw_data}



# ─── Declarative Fluent Pipeline Builder ─────────────────────────────


@dataclass
class PipelineStep:
    capability: str
    template_or_text: Optional[str] = None
    voice: Optional[str] = None
    hint_next: Optional[str] = None
    prefer: Optional[str] = None
    params: Dict[str, Any] = field(default_factory=dict)


@dataclass
class PipelineResult:
    steps: List[InvokeResult]
    total_cost: float = 0.0
    total_duration_ms: int = 0
    is_local: bool = True
    final_artifact: Optional[str] = None
    final_text: Optional[str] = None


class Pipeline:
    """Fluent declarative pipeline builder for multi-capability orchestration.

    Automatically propagates intermediate outputs, manages warmup hints (`hint_next`),
    and tracks end-to-end multi-modal telemetry.
    """

    def __init__(self, engine: Optional[MofaEngine] = None):
        self.engine = engine or MofaEngine()
        self._steps: List[PipelineStep] = []

    @property
    def steps(self) -> List[PipelineStep]:
        """Return list of configured pipeline steps."""
        return self._steps

    def chat(
        self,
        prompt: str,
        *,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        **kwargs,
    ) -> "Pipeline":
        """Append a Chat LLM step to the pipeline."""
        self._steps.append(
            PipelineStep(
                capability="chat",
                template_or_text=prompt,
                hint_next=hint_next,
                prefer=prefer,
                params=kwargs,
            )
        )
        return self

    def tts(
        self,
        voice: str = "en-narrator",
        *,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        speed: float = 1.0,
    ) -> "Pipeline":
        """Append a Speech Synthesis (TTS) step to the pipeline."""
        self._steps.append(
            PipelineStep(
                capability="tts",
                voice=voice,
                hint_next=hint_next,
                prefer=prefer,
                params={"speed": speed},
            )
        )
        return self

    def asr(
        self,
        *,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        diarize: bool = False,
    ) -> "Pipeline":
        """Append a Speech Recognition (ASR) step to the pipeline."""
        self._steps.append(
            PipelineStep(
                capability="asr",
                hint_next=hint_next,
                prefer=prefer,
                params={"diarize": diarize},
            )
        )
        return self

    def image_gen(
        self,
        prompt_template: str = "{text}",
        *,
        size: str = "512x512",
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> "Pipeline":
        """Append an Image Generation step to the pipeline."""
        self._steps.append(
            PipelineStep(
                capability="image_gen",
                template_or_text=prompt_template,
                hint_next=hint_next,
                prefer=prefer,
                params={"size": size},
            )
        )
        return self

    def run(self, **variables) -> PipelineResult:
        """Execute all pipeline steps sequentially, passing context along the chain."""
        results: List[InvokeResult] = []
        last_text: str = str(variables.get("input", ""))
        last_file: Optional[str] = variables.get("audio_file") or variables.get("image_file")
        total_cost = 0.0
        total_ms = 0
        all_local = True

        for i, step in enumerate(self._steps):
            # Compute hint_next if not explicitly set
            next_hint = step.hint_next
            if not next_hint and (i + 1) < len(self._steps):
                next_hint = self._steps[i + 1].capability

            if step.capability == "chat":
                prompt = step.template_or_text or last_text
                # Format variables into prompt
                for k, v in variables.items():
                    prompt = prompt.replace(f"{{{k}}}", str(v))
                if "{input}" in prompt:
                    prompt = prompt.replace("{input}", last_text)
                
                res = self.engine.chat(
                    prompt,
                    prefer=step.prefer,
                    hint_next=next_hint,
                    **step.params,
                )
                last_text = res.text or ""
                results.append(res)

            elif step.capability == "tts":
                text_to_speak = last_text
                res = self.engine.tts(
                    text_to_speak,
                    voice=step.voice or "en-narrator",
                    prefer=step.prefer,
                    **step.params,
                )
                last_file = res.file
                results.append(res)

            elif step.capability == "asr":
                audio_path = last_file or variables.get("audio") or ""
                res = self.engine.asr(
                    audio_path,
                    prefer=step.prefer,
                    **step.params,
                )
                last_text = res.text or ""
                results.append(res)

            elif step.capability == "image_gen":
                prompt = (step.template_or_text or "{input}").replace("{input}", last_text)
                for k, v in variables.items():
                    prompt = prompt.replace(f"{{{k}}}", str(v))
                res = self.engine.image_gen(
                    prompt,
                    size=step.params.get("size", "512x512"),
                    prefer=step.prefer,
                )
                last_file = res.file or res.url
                results.append(res)

            total_cost += res.cost_usd
            total_ms += res.duration_ms
            if not res.is_local:
                all_local = False

        return PipelineResult(
            steps=results,
            total_cost=total_cost,
            total_duration_ms=total_ms,
            is_local=all_local,
            final_artifact=last_file,
            final_text=last_text,
        )


# ═══════════════════════════════════════════════════════════════════════
# Asynchronous Client — AsyncMofaEngine & AsyncPipeline (PRD §6.3)
# ═══════════════════════════════════════════════════════════════════════

try:
    import httpx
    HAS_HTTPX = True
except ImportError:
    HAS_HTTPX = False


class AsyncMofaEngine:
    """Asynchronous client for the MoFA Engine API Gateway (PRD §6.3).

    Requires httpx (`pip install httpx` or `pip install mofa-sdk[async]`).
    Designed for FastAPI, LangGraph, CrewAI, and high-concurrency async apps.

    Usage:
        async with AsyncMofaEngine() as engine:
            result = await engine.chat("Hello!")
            print(result.text)
    """

    def __init__(self, base_url: str = "http://127.0.0.1:8420"):
        if not HAS_HTTPX:
            raise ImportError(
                "AsyncMofaEngine requires httpx. Install it via: pip install httpx"
            )
        self.base_url = base_url.rstrip("/")
        self._client: Optional[httpx.AsyncClient] = None

    async def _get_client(self) -> httpx.AsyncClient:
        if self._client is None or self._client.is_closed:
            self._client = httpx.AsyncClient(timeout=300.0)
        return self._client

    async def __aenter__(self) -> "AsyncMofaEngine":
        await self._get_client()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        if self._client and not self._client.is_closed:
            await self._client.aclose()

    async def close(self):
        if self._client and not self._client.is_closed:
            await self._client.aclose()

    async def health(self) -> dict:
        client = await self._get_client()
        resp = await client.get(f"{self.base_url}/health")
        resp.raise_for_status()
        return resp.json()

    async def status(self) -> dict:
        client = await self._get_client()
        resp = await client.get(f"{self.base_url}/v1/status")
        resp.raise_for_status()
        return resp.json()

    async def capabilities(self) -> list:
        client = await self._get_client()
        resp = await client.get(f"{self.base_url}/v1/capabilities")
        resp.raise_for_status()
        return resp.json()

    async def cost(self) -> dict:
        client = await self._get_client()
        try:
            resp = await client.get(f"{self.base_url}/v1/cost")
            if resp.status_code == 200:
                return resp.json()
        except Exception:
            pass

        total_cost = 0.0
        local_reqs = 0
        cloud_reqs = 0
        try:
            m_resp = await client.get(f"{self.base_url}/metrics")
            if m_resp.status_code == 200:
                for line in m_resp.text.splitlines():
                    if line.startswith("mofa_cost_usd_total"):
                        parts = line.rsplit(None, 1)
                        if len(parts) == 2:
                            try:
                                total_cost += float(parts[1])
                            except ValueError:
                                pass
                    elif line.startswith("mofa_requests_total"):
                        if 'locality="local"' in line:
                            local_reqs += 1
                        elif 'locality="cloud"' in line:
                            cloud_reqs += 1
        except Exception:
            pass

        return {
            "total_cost_usd": total_cost,
            "local_requests": local_reqs,
            "cloud_requests": cloud_reqs,
            "estimated_savings_usd": max(0.0, local_reqs * 0.003),
        }

    async def invoke(
        self,
        capability: str,
        *,
        model: Optional[str] = None,
        messages: Optional[List[Dict[str, Any]]] = None,
        text: Optional[str] = None,
        input_file: Optional[str] = None,
        params: Optional[Dict[str, Any]] = None,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        response_format: Optional[str] = None,
        app_id: Optional[str] = None,
        session_id: Optional[str] = None,
    ) -> InvokeResult:
        body: Dict[str, Any] = {"capability": capability}
        if model:
            body["model"] = model
        if messages:
            body["messages"] = messages
        elif text:
            body["messages"] = [{"role": "user", "content": text}]
        if input_file:
            body["input_file"] = input_file
        if params:
            body["params"] = params
        if hint_next:
            body["hint_next"] = hint_next
        if prefer:
            body["prefer"] = prefer
        if reasoning:
            body["reasoning"] = reasoning
        if response_format:
            body["response_format"] = response_format
        if app_id:
            body["app_id"] = app_id
        if session_id:
            body["session_id"] = session_id

        client = await self._get_client()
        resp = await client.post(f"{self.base_url}/v1/invoke", json=body)
        resp.raise_for_status()
        data = resp.json()

        return InvokeResult(
            text=data.get("text"),
            file=data.get("file"),
            url=data.get("url"),
            model_used=data.get("model_used", "unknown"),
            provider=data.get("provider", "unknown"),
            duration_ms=data.get("duration_ms", 0),
            request_id=data.get("request_id", ""),
            tokens_used=data.get("tokens_used"),
            cost_usd=data.get("cost_usd", 0.0),
            locality=data.get("locality", "local"),
            words=data.get("words"),
        )

    async def chat(
        self,
        prompt: str,
        *,
        messages: Optional[List[Dict[str, Any]]] = None,
        model: Optional[str] = None,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        app_id: Optional[str] = None,
        session_id: Optional[str] = None,
    ) -> InvokeResult:
        msgs = messages or [{"role": "user", "content": prompt}]
        return await self.invoke(
            capability="chat",
            model=model,
            messages=msgs,
            hint_next=hint_next,
            prefer=prefer,
            reasoning=reasoning,
            app_id=app_id,
            session_id=session_id,
        )

    async def chat_stream(
        self,
        prompt: str,
        *,
        messages: Optional[List[Dict[str, Any]]] = None,
        model: Optional[str] = None,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
    ) -> AsyncGenerator[StreamEvent, None]:
        body: Dict[str, Any] = {
            "capability": "chat",
            "messages": messages or [{"role": "user", "content": prompt}],
        }
        if model:
            body["model"] = model
        if hint_next:
            body["hint_next"] = hint_next
        if prefer:
            body["prefer"] = prefer
        if reasoning:
            body["reasoning"] = reasoning

        client = await self._get_client()
        async with client.stream("POST", f"{self.base_url}/v1/invoke/stream", json=body) as resp:
            resp.raise_for_status()
            accumulated = ""
            async for line in resp.aiter_lines():
                if line.startswith("data: "):
                    payload = line[6:].strip()
                    if payload == "[DONE]":
                        break
                    try:
                        chunk = json.loads(payload)
                        delta = chunk.get("delta", "")
                        ev_type = chunk.get("type", "output")
                        accumulated += delta
                        yield StreamEvent(type=ev_type, delta=delta, content=accumulated, metadata=chunk)
                    except json.JSONDecodeError:
                        accumulated += payload
                        yield StreamEvent(type="output", delta=payload, content=accumulated)

    async def responses(
        self,
        input: str,
        *,
        reasoning: Optional[Dict[str, Any]] = None,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
        stream: bool = True,
    ) -> AsyncGenerator[StreamEvent, None]:
        if stream:
            async for ev in self.chat_stream(input, model=model, prefer=prefer, reasoning=reasoning):
                yield ev
        else:
            result = await self.chat(input, model=model, prefer=prefer, reasoning=reasoning)
            yield StreamEvent(type="output", delta=result.text or "", content=result.text or "")

    async def tts(
        self,
        text: str,
        *,
        voice: str = "zh-female-1",
        model: Optional[str] = None,
        speed: float = 1.0,
        prefer: Optional[str] = None,
        hint_next: Optional[str] = None,
    ) -> InvokeResult:
        resolved_voice = VOICE_ALIASES.get(voice, voice)
        return await self.invoke(
            capability="tts",
            model=model,
            text=text,
            prefer=prefer,
            hint_next=hint_next,
            params={"voice": resolved_voice, "speed": speed},
        )

    async def asr(
        self,
        audio_file: str,
        *,
        model: Optional[str] = None,
        language: Optional[str] = None,
        diarize: bool = False,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        params: Dict[str, Any] = {"diarize": diarize}
        if language:
            params["language"] = language
        return await self.invoke(
            capability="asr",
            model=model,
            input_file=audio_file,
            prefer=prefer,
            params=params,
        )

    async def understand(
        self,
        *,
        images: Optional[List[str]] = None,
        question: str = "",
        detail: str = "auto",
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        image_list: List[str] = []
        for img in (images or []):
            img_path = Path(img)
            if img_path.exists():
                b64 = base64.b64encode(img_path.read_bytes()).decode("utf-8")
                ext = img_path.suffix.lstrip(".").lower()
                mime = {"jpg": "jpeg", "jpeg": "jpeg", "png": "png", "gif": "gif", "webp": "webp"}.get(ext, "jpeg")
                image_list.append(f"data:image/{mime};base64,{b64}")
            else:
                image_list.append(img)

        text_prompt = question or "Describe and extract structured information from this image."
        messages = [{"role": "user", "content": text_prompt, "images": image_list}]
        return await self.invoke(
            capability="vlm",
            model=model,
            messages=messages,
            prefer=prefer,
            params={"detail": detail},
        )

    async def image_gen(
        self,
        prompt: str,
        *,
        size: str = "1024x1024",
        style: Optional[str] = None,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        params: Dict[str, Any] = {"size": size}
        if style:
            params["style"] = style
        return await self.invoke(
            capability="image_gen",
            model=model,
            text=prompt,
            prefer=prefer,
            params=params,
        )

    async def video_gen(
        self,
        prompt: str,
        *,
        duration_secs: int = 10,
        resolution: str = "720p",
        style: Optional[str] = None,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        params: Dict[str, Any] = {
            "prompt": prompt,
            "duration_secs": duration_secs,
            "resolution": resolution,
        }
        if style:
            params["style"] = style
        return await self.invoke(
            capability="video_gen",
            model=model,
            text=prompt,
            prefer=prefer,
            params=params,
        )

    async def embed(
        self,
        texts: Union[str, List[str]],
        *,
        model: Optional[str] = None,
        dimensions: Optional[int] = None,
        prefer: Optional[str] = None,
    ) -> InvokeResult:
        input_texts = [texts] if isinstance(texts, str) else texts
        params: Dict[str, Any] = {"input": input_texts}
        if dimensions:
            params["dimensions"] = dimensions
        return await self.invoke(
            capability="embedding",
            model=model,
            prefer=prefer,
            params=params,
        )


class AsyncPipeline:
    """Asynchronous fluent pipeline builder (PRD §6.3).

    Usage:
        result = await (
            AsyncPipeline(engine)
            .chat("Write a script about {topic}", hint_next="tts")
            .tts(voice="en-narrator")
            .run(topic="Robotics")
        )
    """

    def __init__(self, engine: Optional[AsyncMofaEngine] = None):
        self.engine = engine or AsyncMofaEngine()
        self.steps: List[PipelineStep] = []

    def chat(
        self,
        prompt_template: str,
        *,
        hint_next: Optional[str] = None,
        prefer: Optional[str] = None,
        **params,
    ) -> "AsyncPipeline":
        self.steps.append(PipelineStep(
            capability="chat",
            template_or_text=prompt_template,
            hint_next=hint_next,
            prefer=prefer,
            params=params,
        ))
        return self

    def tts(
        self,
        *,
        voice: str = "en-narrator",
        prefer: Optional[str] = None,
        **params,
    ) -> "AsyncPipeline":
        self.steps.append(PipelineStep(
            capability="tts",
            voice=voice,
            prefer=prefer,
            params=params,
        ))
        return self

    def asr(
        self,
        *,
        prefer: Optional[str] = None,
        **params,
    ) -> "AsyncPipeline":
        self.steps.append(PipelineStep(
            capability="asr",
            prefer=prefer,
            params=params,
        ))
        return self

    def image_gen(
        self,
        prompt_template: Optional[str] = None,
        *,
        size: str = "512x512",
        prefer: Optional[str] = None,
        **params,
    ) -> "AsyncPipeline":
        p = {"size": size, **params}
        self.steps.append(PipelineStep(
            capability="image_gen",
            template_or_text=prompt_template,
            prefer=prefer,
            params=p,
        ))
        return self

    async def run(self, **variables) -> PipelineResult:
        results: List[InvokeResult] = []
        total_cost = 0.0
        total_ms = 0
        all_local = True
        last_text = ""
        last_file: Optional[str] = None

        for i, step in enumerate(self.steps):
            hint = step.hint_next
            if not hint and (i + 1) < len(self.steps):
                hint = self.steps[i + 1].capability

            if step.capability == "chat":
                prompt = step.template_or_text or ""
                for k, v in variables.items():
                    prompt = prompt.replace(f"{{{k}}}", str(v))
                res = await self.engine.chat(
                    prompt,
                    hint_next=hint,
                    prefer=step.prefer,
                    **step.params,
                )
                last_text = res.text or ""
                results.append(res)

            elif step.capability == "tts":
                text_to_speak = last_text or variables.get("text") or ""
                res = await self.engine.tts(
                    text_to_speak,
                    voice=step.voice or "en-narrator",
                    prefer=step.prefer,
                    **step.params,
                )
                last_file = res.file
                results.append(res)

            elif step.capability == "asr":
                audio_path = last_file or variables.get("audio") or ""
                res = await self.engine.asr(
                    audio_path,
                    prefer=step.prefer,
                    **step.params,
                )
                last_text = res.text or ""
                results.append(res)

            elif step.capability == "image_gen":
                prompt = (step.template_or_text or "{input}").replace("{input}", last_text)
                for k, v in variables.items():
                    prompt = prompt.replace(f"{{{k}}}", str(v))
                res = await self.engine.image_gen(
                    prompt,
                    size=step.params.get("size", "512x512"),
                    prefer=step.prefer,
                )
                last_file = res.file or res.url
                results.append(res)

            total_cost += res.cost_usd
            total_ms += res.duration_ms
            if not res.is_local:
                all_local = False

        return PipelineResult(
            steps=results,
            total_cost=total_cost,
            total_duration_ms=total_ms,
            is_local=all_local,
            final_artifact=last_file,
            final_text=last_text,
        )

