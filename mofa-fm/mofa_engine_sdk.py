"""MoFA Engine Python SDK — thin wrapper over the HTTP API."""

import requests
from typing import Optional
from dataclasses import dataclass

@dataclass
class RunResult:
    output_text: Optional[str]
    output_file: Optional[str]
    model_used: str
    backend: str
    duration_ms: int
    request_id: str

class MofaEngine:
    def __init__(self, base_url: str = "http://127.0.0.1:8420"):
        self.base_url = base_url.rstrip("/")
        self.session = requests.Session()
        self.session.trust_env = False

    def health(self) -> dict:
        return self.session.get(f"{self.base_url}/health", timeout=5).json()

    def capabilities(self) -> dict:
        return self.session.get(f"{self.base_url}/v1/capabilities", timeout=10).json()

    def status(self) -> dict:
        return self.session.get(f"{self.base_url}/v1/status", timeout=10).json()

    def run(
        self,
        model_type: Optional[str] = None,
        model: Optional[str] = None,
        text: Optional[str] = None,
        file: Optional[str] = None,
        prompt: Optional[str] = None,
        hint_next: Optional[str] = None,
        params: Optional[dict] = None,
        timeout: int = 300,
    ) -> RunResult:
        body = {"input": {}}
        if model_type:
            body["type"] = model_type
        if model:
            body["model"] = model
        if text:
            body["input"]["text"] = text
        if file:
            body["input"]["file"] = file
        if prompt:
            body["input"]["prompt"] = prompt
        if params:
            body["input"]["params"] = params
        if hint_next:
            body["hint"] = {"next": hint_next}

        resp = self.session.post(
            f"{self.base_url}/v1/run",
            json=body,
            timeout=timeout,
        )
        resp.raise_for_status()
        data = resp.json()
        output = data["output"]
        return RunResult(
            output_text=output.get("text"),
            output_file=output.get("file"),
            model_used=data["model_used"],
            backend=data["backend"],
            duration_ms=data["duration_ms"],
            request_id=data["request_id"],
        )

    def run_llm(self, text: str, prompt: Optional[str] = None, model: Optional[str] = None, hint_next: Optional[str] = None) -> RunResult:
        return self.run(model_type="llm", model=model, text=text, prompt=prompt, hint_next=hint_next)

    def run_tts(self, text: str, model: Optional[str] = None, voice: str = "alloy") -> RunResult:
        return self.run(model_type="tts", model=model, text=text, params={"voice": voice})

    def run_asr(self, file: str, model: Optional[str] = None) -> RunResult:
        return self.run(model_type="asr", model=model, file=file)
