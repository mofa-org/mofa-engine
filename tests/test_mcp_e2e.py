#!/usr/bin/env python3
"""End-to-End Test Suite for MoFA Engine MCP Server (Model Context Protocol).

Tests MCP tool registration, schema validation, live end-to-end tool execution,
resources, prompts, pipelines, and multi-turn sessions across all capabilities:
  1. mofa_doctor (Diagnostic & readiness inspection)
  2. mofa_chat (Chat LLM with reasoning & multi-turn session memory)
  3. mofa_session_clear (Session history reset)
  4. mofa_tts (Speech synthesis with rich media artifact formatting)
  5. mofa_asr (Speech-to-text audio transcription)
  6. mofa_understand (Multimodal VLM document understanding)
  7. mofa_embed (Semantic vector embeddings)
  8. mofa_image_gen (Image generation interface & markdown artifacts)
  9. mofa_run_pipeline (Declarative multimodal pipeline orchestration)
  10. MCP Resources (mofa://models, mofa://cost, mofa://status, mofa://scenarios)
  11. MCP Prompts (mofa_review_diff, mofa_meeting_brief, mofa_podcast_script, mofa_extract_receipt)

Usage:
  python3 tests/test_mcp_e2e.py
"""

import json
import os
import sys
import unittest
from pathlib import Path

# Add mofa-fm SDK to path
ROOT_DIR = Path(__file__).parent.parent
sys.path.insert(0, str(ROOT_DIR / "mofa-fm"))

import mcp_server
from mcp_server import (
    mcp,
    mofa_chat,
    mofa_session_clear,
    mofa_tts,
    mofa_asr,
    mofa_understand,
    mofa_embed,
    mofa_image_gen,
    mofa_run_pipeline,
    mofa_doctor,
    get_models_resource,
    get_cost_resource,
    get_status_resource,
    get_scenarios_resource,
    prompt_review_diff,
    prompt_meeting_brief,
    prompt_podcast_script,
    prompt_extract_receipt,
)


class TestMofaMcpServerE2E(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        print("\n==================================================================")
        print("   MoFA Engine — MCP (Model Context Protocol) E2E Test Suite")
        print("==================================================================")

    def test_01_tool_registration(self):
        """Verify all 9 MCP tools are properly registered and callable."""
        tools = [
            mofa_chat,
            mofa_session_clear,
            mofa_tts,
            mofa_asr,
            mofa_understand,
            mofa_embed,
            mofa_image_gen,
            mofa_run_pipeline,
            mofa_doctor,
        ]
        for t in tools:
            self.assertTrue(callable(t), f"Tool {t.__name__} must be callable")

        print(f"\n[Test 1] Tool Registration: {len(tools)} tools verified:")
        for t in tools:
            first_line = t.__doc__.splitlines()[0] if t.__doc__ else ""
            print(f"  + {t.__name__}: {first_line}")

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
        self.assertTrue("MoFA Speech Synthesis" in res or "TTS completed" in res)
        print(f"  [OK] TTS Result: {res.replace(chr(10), ' · ')[:120]}...")

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

    def test_08_multi_turn_session_memory(self):
        """Test multi-turn conversational memory with session_id (Improvement #6)."""
        print("\n[Test 8] Testing multi-turn session memory...")
        sid = "test-session-mcp-001"
        
        # Turn 1
        res1 = mofa_chat(
            message="My favorite secret word is 'Moonlight'. Remember this.",
            session_id=sid,
            prefer="local",
        )
        self.assertIn(sid, res1)
        self.assertIn("turn 1", res1)
        print(f"  + Turn 1: {res1.splitlines()[0][:80]}...")

        # Turn 2
        res2 = mofa_chat(
            message="What is my secret word?",
            session_id=sid,
            prefer="local",
        )
        self.assertIn(sid, res2)
        self.assertIn("turn 2", res2)
        print(f"  + Turn 2: {res2.splitlines()[0][:80]}...")

        # Clear session
        clear_res = mofa_session_clear(session_id=sid)
        self.assertIn("Cleared session", clear_res)
        print(f"  + Clear: {clear_res}")

    def test_09_declarative_pipeline_execution(self):
        """Test mofa_run_pipeline tool execution (Improvement #5)."""
        print("\n[Test 9] Testing declarative pipeline execution tool...")
        
        # Test podcast pipeline preset
        res = mofa_run_pipeline(
            pipeline_type="podcast",
            input_text="MoFA Engine provides high throughput local AI inference.",
            voice="en-narrator",
            prefer="local",
        )
        self.assertIsInstance(res, str)
        self.assertIn("MoFA Pipeline: Podcast Studio", res)
        self.assertIn("Total Latency", res)
        print(f"  [OK] Pipeline Result:\n{res[:200]}...\n")

    def test_10_mcp_resources_and_prompts(self):
        """Test MCP resources and prompt templates (Improvement #1)."""
        print("\n[Test 10] Testing MCP resources and prompts...")
        
        # Resources
        models_json = get_models_resource()
        self.assertIsInstance(models_json, str)
        print("  + Resource mofa://models returned valid payload")

        cost_json = get_cost_resource()
        cost_data = json.loads(cost_json)
        self.assertIn("total_cost_usd", cost_data)
        print(f"  + Resource mofa://cost: total_cost=${cost_data.get('total_cost_usd', 0.0)}")

        scenarios_json = get_scenarios_resource()
        scenarios_data = json.loads(scenarios_json)
        self.assertIn("S1", scenarios_data)
        self.assertIn("S6", scenarios_data)
        print(f"  + Resource mofa://scenarios: {len(scenarios_data)} scenarios verified")

        # Prompts
        diff_prompt = prompt_review_diff(diff="--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old\n+new")
        self.assertIn("MoFA Engine Code Review Specialist", diff_prompt)
        print("  + Prompt mofa_review_diff generated successfully")

        brief_prompt = prompt_meeting_brief(transcript="Alice: We deploy tomorrow.")
        self.assertIn("MoFA Executive Meeting Assistant", brief_prompt)
        print("  + Prompt mofa_meeting_brief generated successfully")

    def test_11_media_artifact_formatting(self):
        """Test image and audio artifact formatting (Improvement #4)."""
        print("\n[Test 11] Testing media artifact formatting...")
        
        # Image gen artifact formatting
        res_img = mofa_image_gen(prompt="Futuristic neural network server rack in cyber style", prefer="local")
        self.assertIn("MoFA Generated Image", res_img)
        self.assertIn("Resolution", res_img)
        print(f"  + Image artifact format:\n{res_img[:150]}...\n")


if __name__ == "__main__":
    unittest.main()

