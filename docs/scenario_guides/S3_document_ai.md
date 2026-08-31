# Scenario S3: Document / Screenshot AI

## 1. Scenario Overview
- **What S3 is**: Photo/screenshot → VLM extracts structured data (receipt amounts, dates, categories as JSON)
- **PRD reference**: §3 Scenario S3 (lines 726-801)
- **Priority**: P2 (Must), Week 3 mount (reuses VLM backend)
- **Business context**: Receipt reimbursement, manual Q&A, whiteboard-to-record, form entry

## 2. Pipeline Architecture

```mermaid
graph TD
    A[Image Input] --> B{VLM understand()}
    B -->|prefer=local| C[Local VLM: qwen2.5-vl / llava]
    B -->|prefer=cloud| D[Cloud VLM: gpt-4o]
    C --> E[Structured JSON Output]
    D --> E
    
    subgraph Routing Configuration
    F[detail tier: low/high/auto] -.-> B
    G[Batch Processing mode] -.-> B
    end
```

- **Single-step VLM pipeline**: Passes image and question to the capability engine.
- **Detail Tier**: The `detail` parameter (`low`, `high`, `auto`) directly impacts image resolution and cloud billing.
- **Routing Preference**: `prefer=local` routes to local VLMs to save costs, while cloud falls back to premium models.
- **Batch Capability**: Supports passing multiple images for batch extraction.

## 3. Required Infrastructure

| Component | Local Provider | Cloud Fallback | Port | Notes |
| :--- | :--- | :--- | :--- | :--- |
| MoFA Engine Core | Local Gateway | N/A | `8420` | Core orchestration daemon |
| VLM Model Backend | Ollama | OpenAI | `11434` | Needs `llava:latest` or `qwen2.5-vl:7b` locally |
| API Keys | None required | OpenAI | N/A | Cloud fallback needs `OPENAI_API_KEY` |

## 4. Setup Instructions

```bash
# Start the core engine
bash quickstart.sh

# Pull the required local VLM models via Ollama
ollama pull llava:latest

# Or for better extraction quality:
ollama pull qwen2.5-vl:7b
```

## 5. How to Run

### 5a. Single image extraction
```bash
python3 examples/doc_ai.py --images examples/samples/sample_receipt.png --prefer local
```

### 5b. Batch processing
```bash
python3 examples/doc_ai.py --images img1.png img2.png img3.png --prefer local
```

### 5c. Mock Mode (Fast testing without VLM)
```bash
python3 examples/doc_ai.py --mock
```

## 6. Expected Output

- **Console Output**: Status logs for per-image processing, including the provider used, billing cost, and processing latency.
- **JSON File**: `output/extracted_receipt.json` will be generated containing structured fields such as `amount`, `date`, `category`, and itemized lists.
- **Metrics Dashboard**: Per-image cost and billing tier tracking.

## 7. Technical Detail

- **API Usage**: Relies on `engine.understand(images=[path], question="...", detail="low", prefer="local")`
- **Routing**: The VLM capability delegates to `llava`/`qwen2.5-vl` locally or `gpt-4o` if cloud is preferred or local is unavailable.
- **Detail Parameter**: Controls image resolution and cloud API billing. `detail="low"` is ideal for batch screening (cheaper), while `detail="high"` handles complex, dense text edge cases.

## 8. PRD Acceptance Criteria

| PRD Criterion | Test Method | Expected Result |
| :--- | :--- | :--- |
| Message supports multimodal content | Pass image URL/path in request | VLM successfully reads input |
| Vision understanding supports detail tiers | Extract same image with `low` vs `high` | API bills differently; extraction density changes |
| VLM supports warmup | Run `doc_ai.py` twice | Latency on second run is quantifiably lower |
| Batch local VLM hit rate observable | Run batch via local prefer | Dashboard shows $0 cost savings |

## 9. Troubleshooting

- **VLM not available**: Make sure you have pulled the model (`ollama pull llava:latest`) and Ollama is running.
- **Image format errors**: Ensure inputs are standard formats (PNG, JPG) or valid local paths.
- **Slow processing**: Local VLM models are VRAM-heavy. The initial run includes a cold start (model loading into memory) and will take longer.

## 10. Sample Files
- `sample_receipt.png` (16KB) is available in `examples/samples/` for testing.
