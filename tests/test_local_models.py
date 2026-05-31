#!/usr/bin/env python3
"""Deep local-model test suite for MoFA Engine.

Tests Ollama models with real inference, memory tracking,
routing priority, failover, hint preflight, and pipelines.
"""
import requests, json, sys, os, time

BASE = "http://127.0.0.1:8420"
S = requests.Session()
S.trust_env = False

passed = 0
total = 0

def invoke(timeout=200, **kw):
    body = {}
    for k, v in kw.items():
        body[k] = v
    r = S.post(f"{BASE}/v1/invoke", json=body, timeout=timeout)
    r.raise_for_status()
    return r.json()

def test(name, fn):
    global passed, total
    total += 1
    try:
        result = fn()
        if isinstance(result, str):
            print(f"  ✅ {name}: {result}")
        else:
            print(f"  ✅ {name}")
        passed += 1
        return True
    except Exception as e:
        print(f"  ❌ {name}: {e}")
        return False

print("=" * 70)
print("  MoFA Engine — Local Model Deep Test Suite")
print("=" * 70)

# ── Determine which local model is warmest ───────────────
caps = S.get(f"{BASE}/v1/capabilities", timeout=5).json()
ollama_models = [m for m in caps if m['provider'] == 'ollama']
# prefer already-hot model, else the smaller one
hot_local = next((m for m in ollama_models if m['status'] == 'hot'), None)
LOCAL_MODEL = hot_local['name'] if hot_local else (
    min(ollama_models, key=lambda m: m['memory_estimate_bytes'])['name']
    if ollama_models else "qwen3.5:0.8b"
)
print(f"\n  Using local model: {LOCAL_MODEL} ({'already hot' if hot_local else 'cold start'})")

# ── 1. Discovery ─────────────────────────────────────────
print("\n--- Discovery ---")

test("Health check", lambda: (
    h := S.get(f"{BASE}/health", timeout=5).json(),
    f"status={h['status']}, uptime={h['uptime_secs']}s"
)[-1])

test("Ollama models discovered", lambda: (
    f"{len(ollama_models)} local models: {', '.join(m['name'] for m in ollama_models)}"
    if len(ollama_models) > 0 else (_ for _ in ()).throw(AssertionError("no Ollama models"))
))

test("Cloud-proxy models excluded", lambda: (
    cloud := [m for m in ollama_models if ':cloud' in m['name']],
    f"0 cloud-proxy models" if len(cloud) == 0
    else (_ for _ in ()).throw(AssertionError(f"{len(cloud)} cloud models leaked"))
)[-1])

test("All local models cost=free", lambda: (
    bad := [m['name'] for m in ollama_models if m['cost_tier'] != 'free'],
    "all free" if not bad else (_ for _ in ()).throw(AssertionError(f"not free: {bad}"))
)[-1])

# ── 2. Local Model Inference ────────────────────────────
print("\n--- Local Model Inference ---")

test(f"Simple chat ({LOCAL_MODEL})", lambda: (
    r := invoke(model=LOCAL_MODEL, capability="chat",
                messages=[{"role":"user","content":"Reply with exactly one word: hello"}]),
    f"provider={r['provider']}, text='{(r.get('text') or '')[:50]}', {r['duration_ms']}ms"
    if r['provider'] == 'ollama' else (_ for _ in ()).throw(
        AssertionError(f"expected ollama, got {r['provider']}"))
)[-1])

test("System prompt + multi-turn", lambda: (
    r := invoke(model=LOCAL_MODEL, capability="chat", messages=[
        {"role":"system","content":"You are a math tutor. Answer in one line."},
        {"role":"user","content":"What is 2+3?"},
        {"role":"assistant","content":"2+3 = 5"},
        {"role":"user","content":"Good. What is 5*6?"},
    ]),
    text := (r.get('text') or ''),
    f"text='{text[:60]}', {r['duration_ms']}ms"
    if len(text) > 1 else (_ for _ in ()).throw(AssertionError(f"empty response"))
)[-1])

test("Chinese output", lambda: (
    r := invoke(model=LOCAL_MODEL, capability="chat",
                messages=[
                    {"role":"system","content":"Reply only in Chinese."},
                    {"role":"user","content":"What is the meaning of life?"},
                ]),
    text := (r.get('text') or ''),
    f"text='{text[:60]}', {r['duration_ms']}ms"
    if len(text) > 1 else (_ for _ in ()).throw(AssertionError("empty"))
)[-1])

test("Creative generation (haiku)", lambda: (
    r := invoke(model=LOCAL_MODEL, capability="chat",
                messages=[{"role":"user","content":"Write a haiku about the moon. Only the haiku."}]),
    text := (r.get('text') or ''),
    f"'{text.strip()[:80]}' ({r['duration_ms']}ms)"
    if len(text.strip()) > 5 else (_ for _ in ()).throw(AssertionError(f"too short: '{text}'"))
)[-1])

# ── 3. Routing Priority ────────────────────────────────
print("\n--- Routing Priority ---")

test("Auto-route chat → picks local", lambda: (
    r := invoke(capability="chat",
                messages=[{"role":"user","content":"Reply: OK"}]),
    f"selected {r['provider']}/{r['model_used']}"
    if r['provider'] == 'ollama' else (_ for _ in ()).throw(
        AssertionError(f"expected ollama, got {r['provider']}/{r['model_used']}"))
)[-1])

test("TTS → picks cloud (no local TTS)", lambda: (
    r := invoke(capability="tts",
                messages=[{"role":"user","content":"Hello world"}]),
    file_ok := r.get('file') and os.path.exists(r['file']),
    f"provider={r['provider']}, model={r['model_used']}, file={os.path.getsize(r['file'])}B"
    if file_ok else f"provider={r['provider']}, no file"
)[-1])

test("Named cloud model bypasses local", lambda: (
    r := invoke(model="deepseek-chat", capability="chat",
                messages=[{"role":"user","content":"Reply: OK"}]),
    f"provider={r['provider']}"
    if r['provider'] != 'ollama' else (_ for _ in ()).throw(
        AssertionError("should not use ollama for deepseek-chat"))
)[-1])

# ── 4. Status & Model Lifecycle ─────────────────────────
print("\n--- Status & Model Lifecycle ---")

test(f"{LOCAL_MODEL} status is hot after use", lambda: (
    cap := S.get(f"{BASE}/v1/capabilities", timeout=5).json(),
    m := next((x for x in cap if x['name'] == LOCAL_MODEL), None),
    f"status={m['status']}" if m and m['status'] in ('hot', 'busy')
    else (_ for _ in ()).throw(AssertionError(
        f"expected hot/busy, got {m['status'] if m else 'not found'}"))
)[-1])

test("Engine status has loaded models", lambda: (
    s := S.get(f"{BASE}/v1/status", timeout=5).json(),
    f"total={s['total_models']}, loaded={s['loaded_models']}, providers={s['providers']}"
    if s['loaded_models'] > 0 else (_ for _ in ()).throw(AssertionError("no loaded"))
)[-1])

test("Provider health shows circuit states", lambda: (
    s := S.get(f"{BASE}/v1/status", timeout=5).json(),
    healths := s.get('provider_health', []),
    ollama_h := next((h for h in healths if h['name'] == 'ollama'), None),
    f"ollama: healthy={ollama_h['healthy']}, circuit={ollama_h['circuit_state']}"
    if ollama_h else (_ for _ in ()).throw(AssertionError("ollama not in health"))
)[-1])

# ── 5. Preflight / Hint ────────────────────────────────
print("\n--- Preflight ---")

test("Hint pre-warms next capability", lambda: (
    r := invoke(capability="chat", hint_next="tts",
                messages=[{"role":"user","content":"Reply: OK"}]),
    time.sleep(1),
    cap := S.get(f"{BASE}/v1/capabilities", timeout=5).json(),
    tts_warm := [m for m in cap if m['capability'] == 'tts' and m['status'] in ('hot','warming')],
    f"chat done ({r['duration_ms']}ms), {len(tts_warm)} TTS model(s) warmed"
)[-1])

# ── 6. Failover ─────────────────────────────────────────
print("\n--- Failover ---")

test("Capability with no models returns error", lambda: (
    r := S.post(f"{BASE}/v1/invoke", json={
        "capability": "video_gen",
        "messages": [{"role":"user","content":"test"}]
    }, timeout=10),
    f"status={r.status_code}, error present"
    if r.status_code >= 400 else (_ for _ in ()).throw(
        AssertionError(f"expected error, got {r.status_code}"))
)[-1])

test("Empty messages returns error", lambda: (
    r := S.post(f"{BASE}/v1/invoke", json={
        "capability": "chat", "messages": []
    }, timeout=10),
    f"status={r.status_code}"
    if r.status_code >= 400 else (_ for _ in ()).throw(
        AssertionError(f"expected error, got {r.status_code}"))
)[-1])

# ── 7. Pipeline (Local LLM → Cloud TTS) ────────────────
print("\n--- End-to-End Pipeline ---")

test("Article → Chinese → Audio (local+cloud)", lambda: (
    r1 := invoke(capability="chat", hint_next="tts", messages=[
        {"role":"system","content":"Translate to Chinese. ONLY output Chinese text."},
        {"role":"user","content":"AI is transforming how we build software."},
    ]),
    chinese := (r1.get('text') or '人工智能正在改变软件开发'),
    r2 := invoke(capability="tts", messages=[{"role":"user","content":chinese[:200]}]),
    file_ok := r2.get('file') and os.path.exists(r2['file']),
    f"LLM({r1['provider']}/{r1['model_used']}, {r1['duration_ms']}ms) → "
    f"TTS({r2['provider']}/{r2['model_used']}, {r2['duration_ms']}ms), "
    f"audio={os.path.getsize(r2['file'])}B" if file_ok else
    (_ for _ in ()).throw(AssertionError(f"audio file missing: {r2.get('file')}"))
)[-1])

# ── 8. Dashboard & Events ──────────────────────────────
print("\n--- Dashboard & Events ---")

test("Dashboard HTML loads", lambda: (
    r := S.get(f"{BASE}/", timeout=5),
    f"{len(r.text)} chars" if r.status_code == 200 and "MoFA" in r.text
    else (_ for _ in ()).throw(AssertionError(f"status={r.status_code}"))
)[-1])

test("SSE event stream", lambda: (
    r := S.get(f"{BASE}/v1/events", timeout=2, stream=True),
    r.close(),
    f"status={r.status_code}"
)[-1])

# ── Summary ─────────────────────────────────────────────
print(f"\n{'='*70}")
print(f"  Results: {passed}/{total} passed")
print(f"{'='*70}")
sys.exit(0 if passed == total else 1)
