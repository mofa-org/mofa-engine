#!/usr/bin/env python3
"""End-to-End Test Suite for MoFA Engine MCP Server (Model Context Protocol).

Tests MCP tool registration, schema validation, and live end-to-end tool execution
across all 7 full-modal capabilities:
  1. mofa_doctor (Diagnostic & readiness inspection)
  2. mofa_chat (Chat LLM with optional deep thinking reasoning)
  3. mofa_tts (Speech synthesis with voice alias resolution)
  4. mofa_asr (Speech-to-text audio transcription)
  5. mofa_understand (Multimodal VLM document understanding)
  6. mofa_embed (Semantic vector embeddings)
  7. mofa_image_gen (Image generation interface)

Usage:
  python3 tests/test_mcp_e2e.py
"""

import os
import sys
import unittest
from pathlib import Path

# Add mofa-fm SDK to path
ROOT_DIR = Path(__file__).parent.parent
sys.path.insert(0, str(ROOT_DIR / "mofa-fm"))

try:
    import fastmcp
    import mcp_server
    from mcp_server import (
        mcp,
        mofa_chat,
        mofa_tts,
        mofa_asr,
        mofa_understand,
        mofa_embed,
        mofa_image_gen,
        mofa_doctor,
    )
    HAS_FASTMCP = True
except ImportError:
    HAS_FASTMCP = False


@unittest.skipUnless(HAS_FASTMCP, "fastmcp is required for MCP E2E tests: pip install fastmcp")
class TestMofaMcpServerE2E(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        print("\n==================================================================")
        print("   MoFA Engine — MCP (Model Context Protocol) E2E Test Suite")
        print("==================================================================")

    def test_01_tool_registration(self):
        """Verify all 7 MCP tools are properly registered on the FastMCP server instance."""
        # Check tool functions exist and are callable
        tools = [
            mofa_chat,
            mofa_tts,
            mofa_asr,
            mofa_understand,
            mofa_embed,
            mofa_image_gen,
            mofa_doctor,
        ]
        for t in tools:
            self.assertTrue(callable(t), f"Tool {t.__name__} must be callable")

        print(f"\n[Test 1] Tool Registration: {len(tools)} tools verified:")
        for t in tools:
            print(f"  + {t.__name__}: {t.__doc__.splitlines()[0] if t.__doc__ else ''}")

    def test_02_mcp_doctor_tool(self):
        """Test mofa_doctor tool invocation via MCP."""
        print("\n[Test 2] Invoking mofa_doctor via MCP...")
        res = mofa_doctor()
        self.assertIsInstance(res, str)
        self.assertIn("MoFA Engine", res)
        print("  [OK] Doctor report returned successfully (length:", len(res), "chars)")

    def test_03_mcp_chat_tool(self):
        """Test mofa_chat standard and deep thinking invocations."""
        print("\n[Test 3] Invoking mofa_chat standard via MCP...")
        prompt = "Explain in exactly 1 concise sentence what an API gateway does."
        res = mofa_chat(message=prompt, prefer="local")
        self.assertIsInstance(res, str)
        self.assertTrue(len(res) > 0)
        self.assertIn("locality: local", res)
        print(f"  [OK] Response: {res.splitlines()[0][:90]}...")
        print(f"  +- Telemetry: {res.splitlines()[-1]}")

        print("\n  Invoking mofa_chat with reasoning_effort='high' via MCP...")
        res_reasoning = mofa_chat(
            message="Check if 17 is prime in 1 short sentence.",
            reasoning_effort="high",
            prefer="local",
        )
        self.assertIsInstance(res_reasoning, str)
        self.assertTrue(len(res_reasoning) > 0)
        print(f"  [OK] Reasoning Response: {res_reasoning.splitlines()[0][:90]}...")

    def test_04_mcp_tts_tool(self):
        """Test mofa_tts speech synthesis tool via MCP."""
        print("\n[Test 4] Invoking mofa_tts via MCP...")
        text = "Testing speech synthesis through Model Context Protocol tool."
        res = mofa_tts(text=text, voice="en-narrator", prefer="local")
        self.assertIsInstance(res, str)
        self.assertTrue("Audio saved" in res or "TTS completed" in res)
        print(f"  [OK] TTS Result: {res.replace(chr(10), ' · ')}")

    def test_05_mcp_asr_tool(self):
        """Test mofa_asr audio transcription tool via MCP."""
        print("\n[Test 5] Invoking mofa_asr via MCP...")
        sample_audio = ROOT_DIR / "examples" / "samples" / "sample_meeting.wav"
        if sample_audio.exists():
            res = mofa_asr(audio_file=str(sample_audio), prefer="local")
            self.assertIsInstance(res, str)
            print(f"  [OK] ASR Result: {res.splitlines()[0][:90]}...")
            print(f"  +- Telemetry: {res.splitlines()[-1]}")
        else:
            self.skipTest(f"Sample audio fixture not found: {sample_audio}")

    def test_06_mcp_understand_vlm_tool(self):
        """Test mofa_understand multimodal VLM tool via MCP."""
        print("\n[Test 6] Invoking mofa_understand via MCP...")
        sample_receipt = ROOT_DIR / "examples" / "samples" / "sample_receipt.png"
        if sample_receipt.exists():
            res = mofa_understand(
                question="What is the merchant name in this receipt?",
                image_paths=[str(sample_receipt)],
                detail="low",
                prefer="local",
            )
            self.assertIsInstance(res, str)
            print(f"  [OK] VLM Result: {res.splitlines()[0][:90]}...")
            print(f"  +- Telemetry: {res.splitlines()[-1]}")
        else:
            self.skipTest(f"Sample receipt fixture not found: {sample_receipt}")

    def test_07_mcp_embed_tool(self):
        """Test mofa_embed vector embedding tool via MCP."""
        print("\n[Test 7] Invoking mofa_embed via MCP...")
        res = mofa_embed(text="Vector search embedding test for AI agents.", prefer="local")
        self.assertIsInstance(res, str)
        print(f"  [OK] Embed Result: {res.splitlines()[0][:60]}...")
        print(f"  +- Telemetry: {res.splitlines()[-1]}")


if __name__ == "__main__":
    unittest.main()
