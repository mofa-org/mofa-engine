#!/usr/bin/env python3
"""S3 Document & Receipt AI: Image/Photo -> VLM Structured JSON Extraction (PRD v3.1 §2.2.1 S3).

Demonstrates Vision-Language Model (VLM) structured understanding:
  1. Accepts receipt / invoice / document image
  2. Passes image to local VLM (detail=low/high for billing tier control)
  3. Extracts merchant, date, line items, totals into structured JSON

Usage:
  python3 examples/doc_ai.py
  python3 examples/doc_ai.py --image path/to/receipt.jpg --detail low
  mofa run doc
"""

import argparse
import os
import sys
from pathlib import Path

# Add mofa-fm SDK to import path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "mofa-fm"))
from mofa_sdk import MofaEngine

SAMPLE_RECEIPT = Path(__file__).parent / "samples" / "sample_receipt.png"


def main():
    parser = argparse.ArgumentParser(description="S3 Document AI: Photo -> VLM Structured JSON Extraction")
    parser.add_argument("--image", default=str(SAMPLE_RECEIPT), help="Path to receipt/document image")
    parser.add_argument("--detail", default="low", choices=["low", "high", "auto"], help="Cloud billing & visual resolution detail tier")
    parser.add_argument("--prefer", default="local", choices=["local", "auto", "cloud"], help="Routing locality preference")
    parser.add_argument("--out", default="output/receipt_data.json", help="Output path for extracted JSON data")
    args = parser.parse_args()

    engine = MofaEngine()
    image_path = Path(args.image)

    print("\n==================================================================")
    print("   Scenario S3: Document & Receipt AI (Multimodal VLM)")
    print("==================================================================")
    print(f"  Target Image : {image_path} (exists: {image_path.exists()})")
    print(f"  Detail Tier  : {args.detail} · Prefer: {args.prefer}")

    question = (
        "Extract merchant name, transaction date, line items (item name, quantity, price), "
        "subtotal, tax, and grand total from this receipt image. "
        "Output ONLY a valid JSON object matching this schema: "
        '{"merchant": str, "date": str, "items": [{"name": str, "qty": int, "price": float}], "subtotal": float, "tax": float, "total": float}'
    )

    print("\nSending multimodal request to MoFA VLM router...")
    try:
        result = engine.understand(
            images=[str(image_path)] if image_path.exists() else [],
            question=question,
            detail=args.detail,
            prefer=args.prefer,
        )
        
        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(result.text or "{}", encoding="utf-8")

        result.show()

        print("\n==================================================================")
        print("   S3 Document AI Extraction Complete!")
        print("==================================================================")
        print(f"  Saved JSON     : {out_path}")
        print(f"  Provider/Model : {result.provider}/{result.model_used}")
        print(f"  Latency        : {result.duration_ms}ms · Cost: ${result.cost_usd:.4f}")
        print()
    except Exception as e:
        print(f"\n[ERROR] VLM extraction failed: {e}")
        print("[TIP]   Run 'mofa doctor' to inspect your local Ollama VLM models (e.g. llava, qwen2.5-vl).")
        sys.exit(1)


if __name__ == "__main__":
    main()
