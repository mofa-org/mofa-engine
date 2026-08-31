#!/usr/bin/env python3
"""Offline Unit Test Suite for MoFA Engine Python SDK (MofaEngine).

Tests MofaEngine methods, voice alias resolution, parameter passing,
and responses using unittest.mock.
"""

import sys
import os
import unittest
from unittest.mock import MagicMock, patch

# Ensure mofa-fm directory is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "mofa-fm")))

from mofa_sdk import MofaEngine, InvokeResult, StreamEvent, VOICE_ALIASES


class TestMofaEngineSDK(unittest.TestCase):
    def setUp(self):
        self.engine = MofaEngine(base_url="http://127.0.0.1:8420")
        self.engine.session = MagicMock()

    def test_voice_aliases_mapping(self):
        """Verify standard PRD voice alias resolutions."""
        self.assertEqual(VOICE_ALIASES.get("zh-female-1"), "af_heart")
        self.assertEqual(VOICE_ALIASES.get("zh-male-1"), "af_alloy")
        self.assertEqual(VOICE_ALIASES.get("en-narrator"), "af_alloy")

    def test_health_check(self):
        """Test health() endpoint client call."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {"status": "ok", "uptime_secs": 120}
        self.engine.session.get.return_value = mock_resp

        res = self.engine.health()
        self.assertEqual(res["status"], "ok")
        self.engine.session.get.assert_called_with("http://127.0.0.1:8420/health", timeout=5)

    def test_status(self):
        """Test status() endpoint call."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {"total_models": 12, "loaded_models": 2}
        self.engine.session.get.return_value = mock_resp

        res = self.engine.status()
        self.assertEqual(res["total_models"], 12)
        self.engine.session.get.assert_called_with("http://127.0.0.1:8420/v1/status", timeout=10)

    def test_invoke_chat(self):
        """Test invoke() method with capability='chat'."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {
            "text": "Hello world",
            "model_used": "qwen2.5:7b",
            "provider": "ollama",
            "duration_ms": 150,
            "cost_usd": 0.0,
            "locality": "local"
        }
        self.engine.session.post.return_value = mock_resp

        result = self.engine.chat("Hi there", prefer="local")
        self.assertIsInstance(result, InvokeResult)
        self.assertEqual(result.text, "Hello world")
        self.assertEqual(result.provider, "ollama")
        self.assertEqual(result.locality, "local")

    def test_tts_voice_alias_resolution(self):
        """Test tts() method resolves voice aliases properly."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {
            "file": "/tmp/output.mp3",
            "model_used": "kokoro-tts",
            "provider": "kokoro",
            "duration_ms": 400
        }
        self.engine.session.post.return_value = mock_resp

        result = self.engine.tts("Hello voice test", voice="zh-female-1")
        self.assertEqual(result.file, "/tmp/output.mp3")

        # Verify posted JSON contained resolved voice 'af_heart'
        _, kwargs = self.engine.session.post.call_args
        self.assertEqual(kwargs["json"]["params"]["voice"], "af_heart")

    def test_understand_vlm(self):
        """Test understand() vision extraction call."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {
            "text": '{"total": 42.0}',
            "model_used": "qwen2.5-vl:7b",
            "provider": "ollama"
        }
        self.engine.session.post.return_value = mock_resp

        result = self.engine.understand(question="Extract total amount", prefer="local")
        self.assertEqual(result.text, '{"total": 42.0}')

    def test_subscribe(self):
        """Test subscribe() pre-warming subscription call."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {"status": "subscribed", "ttl": 300}
        self.engine.session.post.return_value = mock_resp

        res = self.engine.subscribe(capabilities=["vlm", "tts"], ttl_secs=300)
        self.assertEqual(res["status"], "subscribed")
        _, kwargs = self.engine.session.post.call_args
        self.assertEqual(kwargs["json"]["capabilities"], ["vlm", "tts"])

    def test_embed(self):
        """Test embed() method for vectorization (PRD §3.7)."""
        mock_resp = MagicMock()
        mock_resp.json.return_value = {
            "text": "[0.12, -0.45, 0.78]",
            "model_used": "nomic-embed-text",
            "provider": "ollama",
            "locality": "local",
        }
        self.engine.session.post.return_value = mock_resp

        result = self.engine.embed("Hello embeddings", dimensions=384, prefer="local")
        self.assertEqual(result.text, "[0.12, -0.45, 0.78]")
        self.assertTrue(result.is_local)
        _, kwargs = self.engine.session.post.call_args
        self.assertEqual(kwargs["json"]["capability"], "embedding")
        self.assertEqual(kwargs["json"]["params"]["dimensions"], 384)

    def test_invoke_result_helpers(self):
        """Test InvokeResult ergonomic properties and markdown conversion."""
        res_local = InvokeResult(text="Local summary", provider="ollama", model_used="qwen2.5:7b", duration_ms=120, locality="local", cost_usd=0.0)
        self.assertTrue(res_local.is_local)
        self.assertGreater(res_local.savings_vs_cloud, 0.0)
        md = res_local.to_markdown()
        self.assertIn("Local summary", md)
        self.assertIn("LOCAL", md)

        res_cloud = InvokeResult(text="Cloud summary", provider="fireworks", model_used="llama-3.3-70b", duration_ms=450, locality="cloud", cost_usd=0.002)
        self.assertFalse(res_cloud.is_local)
        self.assertEqual(res_cloud.savings_vs_cloud, 0.0)

    def test_pipeline_chaining(self):
        """Test Pipeline builder chaining and automatic warmup propagation."""
        from mofa_sdk import Pipeline
        pipeline = Pipeline(self.engine)
        pipeline.chat("Write a script about {topic}", hint_next="tts")
        pipeline.tts(voice="en-narrator")

        self.assertEqual(len(pipeline.steps), 2)
        self.assertEqual(pipeline.steps[0].capability, "chat")
        self.assertEqual(pipeline.steps[0].hint_next, "tts")
        self.assertEqual(pipeline.steps[1].capability, "tts")


if __name__ == "__main__":
    unittest.main()

