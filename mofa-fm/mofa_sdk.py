"""MoFA Engine Python SDK — High Performance Intelligent Gateway Client.

Connects to default daemon on http://127.0.0.1:8420.
Supports locality constraints, streaming SSE, voice alias resolution, and event subscriptions.
"""

import json
import requests
from dataclasses import dataclass
from typing import Dict, List, Optional, Generator, Callable, Any

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
    text: Optional[str]
    file: Optional[str]
    model_used: str
    provider: str
    duration_ms: int
    request_id: str
    tokens_used: Optional[int] = None
    cost_usd: float = 0.0
    locality: str = "local"


class MofaEngine:
    """Client for the MoFA Engine API Gateway daemon."""

    def __init__(self, base_url: str = "http://127.0.0.1:8420"):
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        self.session.trust_env = False

    def health(self) -> dict:
        """Check engine daemon health status."""
        return self.session.get(f"{self.base_url}/health", timeout=5).json()

    def status(self) -> dict:
        """Get live engine status, model residency, and memory gauge state."""
        return self.session.get(f"{self.base_url}/v1/status", timeout=10).json()

    def capabilities(self) -> list:
        """List active capabilities available on connected providers."""
        return self.session.get(f"{self.base_url}/v1/capabilities", timeout=10).json()

    def invoke(
        self,
        *,
        capability: Optional[str] = None,
        model: Optional[str] = None,
        text: Optional[str] = None,
        messages: Optional[List[Dict[str, Any]]] = None,
        input_file: Optional[str] = None,
        hint_next: Optional[str] = None,
        locality: Optional[str] = None,
        fallback_policy: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        params: Optional[Dict[str, Any]] = None,
        timeout: int = 300,
    ) -> InvokeResult:
        """Invoke a capability on the engine."""
        body: Dict[str, Any] = {}
        if capability:
            body["capability"] = capability
        if model:
            body["model"] = model
        if hint_next:
            body["hint_next"] = hint_next
        if input_file:
            body["input_file"] = input_file
        if locality:
            body["locality"] = locality
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
            model_used=d.get("model_used", d.get("model", "unknown")),
            provider=d.get("provider", "unknown"),
            duration_ms=d.get("duration_ms", 0),
            request_id=d.get("request_id", ""),
            tokens_used=d.get("tokens_used"),
            cost_usd=d.get("cost_usd", 0.0),
            locality=d.get("locality", "local"),
        )

    def chat(
        self,
        text: str,
        *,
        model: Optional[str] = None,
        hint_next: Optional[str] = None,
        locality: Optional[str] = None,
        reasoning: Optional[Dict[str, Any]] = None,
        **kw,
    ) -> InvokeResult:
        """Convenience method for chat inference."""
        return self.invoke(
            capability="chat",
            model=model,
            text=text,
            hint_next=hint_next,
            locality=locality,
            reasoning=reasoning,
            **kw,
        )

    def tts(
        self,
        text: str,
        *,
        voice: str = "zh-female-1",
        model: Optional[str] = None,
        speed: float = 1.0,
    ) -> InvokeResult:
        """Convenience method for text-to-speech audio synthesis."""
        resolved_voice = VOICE_ALIASES.get(voice, voice)
        return self.invoke(
            capability="tts",
            model=model,
            text=text,
            params={"voice": resolved_voice, "speed": speed},
        )

    def asr(self, file_path: str, *, model: Optional[str] = None) -> InvokeResult:
        """Convenience method for speech recognition ASR."""
        return self.invoke(capability="asr", model=model, input_file=file_path)

    def chat_stream(
        self,
        text: str,
        *,
        model: Optional[str] = None,
        locality: Optional[str] = None,
    ) -> Generator[str, None, None]:
        """Stream tokens in real-time from the engine via SSE stream."""
        body = {
            "capability": "chat",
            "messages": [{"role": "user", "content": text}],
        }
        if model:
            body["model"] = model
        if locality:
            body["locality"] = locality

        response = self.session.post(
            f"{self.base_url}/v1/invoke/stream",
            json=body,
            stream=True,
            timeout=300,
        )
        response.raise_for_status()

        for line in response.iter_lines(decode_unicode=True):
            if line and line.startswith("data: "):
                raw_data = line[6:]
                if raw_data == "[DONE]":
                    break
                try:
                    chunk = json.loads(raw_data)
                    if chunk.get("type") == "text":
                        yield chunk.get("content", "")
                except json.JSONDecodeError:
                    yield raw_data
