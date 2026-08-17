"""MoFA Engine Python SDK — High Performance Intelligent Gateway Client.

Connects to default daemon on http://127.0.0.1:8420.
Supports locality constraints, streaming SSE, voice alias resolution,
multimodal vision, image generation, reasoning API, and event subscriptions.

PRD v3.1 §6.3 — Full Python SDK surface.
"""

import json
import os
import base64
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Generator, Any, Union
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
        return self.session.get(f"{self.base_url}/v1/cost", timeout=10).json()

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

        return InvokeResult(
            text=d.get("text"),
            file=d.get("file"),
            url=d.get("url"),
            model_used=d.get("model_used", d.get("model", "unknown")),
            provider=d.get("provider", "unknown"),
            duration_ms=d.get("duration_ms", 0),
            request_id=d.get("request_id", ""),
            tokens_used=d.get("tokens_used"),
            cost_usd=float(d.get("cost_usd") if d.get("cost_usd") is not None else 0.0),
            locality=d.get("locality", "local"),
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
    ) -> InvokeResult:
        """Text-to-speech audio synthesis (PRD §3.2).

        Args:
            text: Script/text to synthesize.
            voice: Voice alias (e.g., "zh-female-1", "en-narrator").
            speed: Playback speed (0.5–2.0).
            prefer: "local" for Crane/Kokoro, "cloud" for tts-1.
        """
        resolved_voice = VOICE_ALIASES.get(voice, voice)
        return self.invoke(
            capability="tts",
            model=model,
            text=text,
            prefer=prefer,
            params={"voice": resolved_voice, "speed": speed},
        )

    # ─── ASR (with file upload) ───────────────────────────────────────

    def asr(
        self,
        audio_file: str,
        *,
        model: Optional[str] = None,
        prefer: Optional[str] = None,
        diarize: bool = False,
        language: Optional[str] = None,
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

