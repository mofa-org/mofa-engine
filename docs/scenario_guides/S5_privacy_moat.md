# Scenario S5: Privacy Moat (Air-Gapped Local Inference)

**100% Local Execution · Zero Data Egress · Data Flow Compliance Audit**

---

## Overview
Enterprise scenarios frequently handle sensitive, proprietary, or regulated data (e.g. source code, financial records, medical transcripts, client contracts). In such environments, accidentally leaking data to cloud endpoints is a critical compliance violation.

MoFA Engine provides a strict **Privacy Moat** via the `prefer="local"` routing constraint and **Data Flow Audit** ledger.

---

## Key Capabilities
1. **Hard Data Boundary (`prefer="local"`)**:
 - Forces routing strictly through locally resident models (`ollama/gemma3:4b`, `kokoro`, `funasr`).
 - If a local model is unavailable or encounters an error, the engine **fails safely** rather than falling back to cloud APIs.
2. **Data Classification Tagging**:
 - Requests tagged with `data_class="sensitive"` are validated by routing middleware to block cloud endpoints.
3. **Data Flow Audit Ledger**:
 - Real-time audit trail in Web Studio (`Observability → Data Flow Audit`) logging timestamp, capability, model, locality (`local` vs `cloud`), and violations count.

---

## Python SDK Usage

```python
from mofa_sdk import MofaEngine, Pipeline

engine = MofaEngine("http://127.0.0.1:8420")

# Confidential pipeline with hard local constraint
pipeline = (
 Pipeline(engine)
 .chat(
 "Analyze this confidential security audit and list top 3 vulnerabilities:

{text}",
 prefer="local", # 100% Local model (0% cloud leak)
 data_class="sensitive", # Blocks any cloud failover
 hint_next="tts" # Pre-warms local Kokoro voice in background
 )
 .tts(voice="Alloy", prefer="local")
)

result = pipeline.run(text="Confidential: API Key rotation failed on internal database cluster...")
result.steps[-1].save("output/confidential_brief.wav")
print(" Local confidential analysis complete!")
```

---

## Web Studio Verification
1. Open **`http://localhost:3000`**.
2. Click **S5 Privacy Moat** preset card.
3. Enter confidential query text.
4. Click **"Analyze Confidentially"**.
5. Switch to **"Observability" → "Data Flow Audit"** to view the green `Local` badge and verify **Violations: 0**.
