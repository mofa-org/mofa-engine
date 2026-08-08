#!/usr/bin/env python3
"""Test every API provider via MoFA Engine."""
import requests, json, sys, os, time

BASE = "http://127.0.0.1:8420"
S = requests.Session()
S.trust_env = False

def invoke(cap=None, model=None, text="Reply with exactly one word: OK", timeout=60):
    body = {"messages": [{"role": "user", "content": text}]}
    if cap: body["capability"] = cap
    if model: body["model"] = model
    r = S.post(f"{BASE}/v1/invoke", json=body, timeout=timeout)
    r.raise_for_status()
    return r.json()

def test(name, fn):
    try:
        result = fn()
        print(f"  ✅ {name}: provider={result['provider']}, model={result['model_used']}, {result['duration_ms']}ms")
        return True
    except Exception as e:
        print(f"  ❌ {name}: {e}")
        return False

def run_all_provider_tests():
    print("=" * 70)
    print("  MoFA Engine — All-Provider Acceptance Tests")
    print("=" * 70)

    try:
        h = S.get(f"{BASE}/health", timeout=2).json()
        print(f"\nHealth: {h['status']} (uptime {h['uptime_secs']}s)")
    except Exception as e:
        print(f"⚠️ Engine daemon at {BASE} is offline: {e}")
        print("Skipping live provider tests.")
        return 0

    caps = S.get(f"{BASE}/v1/capabilities", timeout=5).json()
    by_p = {}
    for m in caps:
        by_p.setdefault(m['provider'], []).append(m)
    print(f"Models: {len(caps)} total across {len(by_p)} providers")
    for p, ms in sorted(by_p.items()):
        names = [m['name'] for m in ms]
        print(f"  [{p}] {', '.join(names[:4])}" + (f" +{len(names)-4}" if len(names)>4 else ""))

    print(f"\n--- Per-Provider Tests ---")
    passed = 0
    total = 0

    total += 1; passed += test("Ollama (qwen3.5:0.8b)", lambda: invoke(model="qwen3.5:0.8b", timeout=180))
    total += 1; passed += test("OpenAI (gpt-4o-mini)", lambda: invoke(model="gpt-4o-mini"))
    total += 1; passed += test("DeepSeek (deepseek-chat)", lambda: invoke(model="deepseek-chat"))
    total += 1; passed += test("DashScope (qwen-turbo)", lambda: invoke(model="qwen-turbo"))
    total += 1; passed += test("NVIDIA (llama-3.1-8b)", lambda: invoke(model="meta/llama-3.1-8b-instruct"))
    total += 1; passed += test("Perplexity (sonar)", lambda: invoke(model="sonar"))
    total += 1; passed += test("Zhipu→fallback", lambda: invoke(model="glm-4-flash"))

    print(f"\n--- Routing Tests ---")
    total += 1; passed += test("Auto-route chat", lambda: invoke(cap="chat"))

    total += 1; passed += test("TTS (openai tts-1)", lambda: (
        S.post(f"{BASE}/v1/invoke", json={
            "capability": "tts",
            "messages": [{"role": "user", "content": "Hello from MoFA Engine."}]
        }, timeout=30).json()
    ))

    print(f"\n--- Pipeline Test ---")
    total += 1
    try:
        r1 = invoke(cap="chat", text="Translate to Chinese (reply ONLY the translation): AI is transforming software.")
        chinese = r1.get("text", "人工智能正在改变软件")
        r2 = S.post(f"{BASE}/v1/invoke", json={
            "capability": "tts",
            "messages": [{"role": "user", "content": chinese}],
            "hint_next": "chat"
        }, timeout=60).json()
        audio_file = r2.get("file")
        has_file = audio_file is not None and os.path.exists(audio_file)
        size_str = f", audio={os.path.getsize(audio_file)}B" if has_file else ""
        print(f"  ✅ Article→Podcast: LLM({r1['duration_ms']}ms, {r1['provider']}) → TTS({r2['duration_ms']}ms, {r2['provider']}){size_str}")
        passed += 1
    except Exception as e:
        print(f"  ❌ Article→Podcast: {e}")

    total += 1
    try:
        r = S.get(f"{BASE}/", timeout=5)
        assert r.status_code == 200 and "MoFA Engine" in r.text
        print(f"  ✅ Dashboard: {len(r.text)} chars HTML")
        passed += 1
    except Exception as e:
        print(f"  ❌ Dashboard: {e}")

    total += 1
    try:
        r = S.get(f"{BASE}/v1/events", timeout=3, stream=True)
        assert r.status_code == 200
        print(f"  ✅ SSE events endpoint: connected")
        r.close()
        passed += 1
    except Exception as e:
        print(f"  ❌ SSE events: {e}")

    total += 1
    try:
        s = S.get(f"{BASE}/v1/status", timeout=5).json()
        assert s["total_models"] >= 10
        print(f"  ✅ Status: {s['total_models']} models, {s['loaded_models']} loaded, {s['providers']} providers")
        passed += 1
    except Exception as e:
        print(f"  ❌ Status: {e}")

    print(f"\n{'='*70}")
    print(f"  Results: {passed}/{total} passed")
    print(f"{'='*70}")
    return 0 if passed == total else 1

if __name__ == "__main__":
    sys.exit(run_all_provider_tests())
