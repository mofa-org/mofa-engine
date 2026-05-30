"""MoFA Engine Python SDK."""

import requests
from dataclasses import dataclass
from typing import Optional

@dataclass
class InvokeResult:
    text: Optional[str]
    file: Optional[str]
    model_used: str
    provider: str
    duration_ms: int
    request_id: str
    tokens_used: Optional[int]

class MofaEngine:
    def __init__(self, base_url: str = "http://127.0.0.1:8420"):
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        self.session.trust_env = False

    def health(self) -> dict:
        return self.session.get(f"{self.base_url}/health", timeout=5).json()

    def capabilities(self) -> list:
        return self.session.get(f"{self.base_url}/v1/capabilities", timeout=10).json()

    def status(self) -> dict:
        return self.session.get(f"{self.base_url}/v1/status", timeout=10).json()

    def invoke(self, *, capability: str = None, model: str = None,
               text: str = None, messages: list = None,
               input_file: str = None, hint_next: str = None,
               params: dict = None, timeout: int = 300) -> InvokeResult:
        body = {}
        if capability: body["capability"] = capability
        if model: body["model"] = model
        if hint_next: body["hint_next"] = hint_next
        if input_file: body["input_file"] = input_file
        if params: body["params"] = params
        if messages:
            body["messages"] = messages
        elif text:
            body["messages"] = [{"role": "user", "content": text}]

        r = self.session.post(f"{self.base_url}/v1/invoke", json=body, timeout=timeout)
        r.raise_for_status()
        d = r.json()
        return InvokeResult(
            text=d.get("text"), file=d.get("file"),
            model_used=d["model_used"], provider=d["provider"],
            duration_ms=d["duration_ms"], request_id=d["request_id"],
            tokens_used=d.get("tokens_used"),
        )

    def chat(self, text: str, *, model: str = None, hint_next: str = None, **kw) -> InvokeResult:
        return self.invoke(capability="chat", model=model, text=text, hint_next=hint_next, **kw)

    def tts(self, text: str, *, voice: str = "alloy", model: str = None) -> InvokeResult:
        return self.invoke(capability="tts", model=model, text=text, params={"voice": voice})

    def asr(self, file_path: str, *, model: str = None) -> InvokeResult:
        return self.invoke(capability="asr", model=model, input_file=file_path)
