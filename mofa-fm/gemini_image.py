#!/usr/bin/env python3
"""Topic-Specific AI Image Generation Engine for MoFA.

Generates relevant, high-quality visuals for ANY topic using AI image generation.
Priority: AI-generated topic-specific images > Fallback card
"""

import os
import sys
import json
import argparse
import re
import hashlib
import urllib.request
import urllib.parse
import urllib.error
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont, ImageEnhance
import io

Image.MAX_IMAGE_PIXELS = None


def clean_search_keywords(prompt: str) -> str:
    """Extract clean search terms from any user prompt."""
    clean = re.sub(r'^(Clean|Modern|Technical|Visual|Infographic|Diagram|Illustrating|Scene \d+:?|Part \d+:?|\s|-)+', '', prompt, flags=re.IGNORECASE).strip()
    clean = re.sub(r'^(High-resolution|realistic|photograph|of|about|showing|illustrating|diagram):?\s*', '', clean, flags=re.IGNORECASE).strip()
    clean = re.sub(r'\[.*?\]', '', clean).strip()
    clean = clean.split('.')[0].strip()
    words = [w for w in re.findall(r'\b[a-zA-Z]{3,}\b', clean) if w.lower() not in {"and", "the", "for", "with", "from", "that", "this", "visual", "diagram", "clean", "scene", "part", "high", "resolution", "realistic", "photograph", "about"}]
    if len(words) >= 2:
        return " ".join(words[:5])
    elif words:
        return words[0]
    return "technology science"


def standardize_and_save_image(raw_bytes: bytes, out_path: str) -> bool:
    """Crop to 1280x720, enhance, and save."""
    try:
        img = Image.open(io.BytesIO(raw_bytes))
        img = img.convert("RGB")
        target_w, target_h = 1280, 720
        src_w, src_h = img.size
        src_aspect = src_w / src_h
        target_aspect = target_w / target_h
        if src_aspect > target_aspect:
            new_w = int(src_h * target_aspect)
            offset = (src_w - new_w) // 2
            img = img.crop((offset, 0, offset + new_w, src_h))
        else:
            new_h = int(src_w / target_aspect)
            offset = (src_h - new_h) // 2
            img = img.crop((0, offset, src_w, offset + new_h))
        img = img.resize((target_w, target_h), Image.Resampling.LANCZOS)
        enhancer = ImageEnhance.Sharpness(img)
        img = enhancer.enhance(1.1)
        out = Path(out_path)
        out.parent.mkdir(parents=True, exist_ok=True)
        img.save(str(out), "PNG")
        return True
    except Exception as e:
        print(f"[WARN] standardize error: {e}", file=sys.stderr)
        return False


def generate_ai_image(prompt: str, out_path: str, seed_offset: int = 0) -> bool:
    """Generate a topic-specific AI image using Pollinations.
    
    Creates architectural, technical, diagrammatic visuals — NOT random stock photos.
    """
    clean_topic = clean_search_keywords(prompt)
    
    # Build a prompt that creates technical/architectural visuals, not portraits
    enhanced_prompt = (
        f"clean minimalist technical illustration of {clean_topic}, "
        f"modern flat design, architectural diagram style, "
        f"dark background with glowing neon accents, "
        f"professional infographic, no people, no faces, no portraits, "
        f"technology visualization, blueprint aesthetic, 4k"
    )
    safe_prompt = urllib.parse.quote(enhanced_prompt)
    seed = (int(hashlib.md5(clean_topic.encode('utf-8')).hexdigest(), 16) + seed_offset) % 100000
    
    url = f"https://image.pollinations.ai/prompt/{safe_prompt}?width=1280&height=720&nologo=true&seed={seed}"
    
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"}
    )
    
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            data = resp.read()
            if len(data) > 10000:
                if standardize_and_save_image(data, out_path):
                    print(f"[OK] AI image generated for \"{clean_topic}\" ({len(data)} bytes): {out_path}")
                    return True
    except Exception as e:
        print(f"[WARN] AI image generation failed ({e})", file=sys.stderr)
    
    return False


def generate_fallback_card(prompt: str, out_path: str) -> bool:
    """Generate a styled text card as absolute fallback."""
    width, height = 1280, 720
    img = Image.new("RGB", (width, height), "#0a0a1a")
    draw = ImageDraw.Draw(img)
    
    # Draw grid pattern
    for x in range(0, width, 40):
        draw.line([(x, 0), (x, height)], fill="#1a1a3a", width=1)
    for y in range(0, height, 40):
        draw.line([(0, y), (width, y)], fill="#1a1a3a", width=1)
    
    try:
        font = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 36)
        font_small = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 18)
    except Exception:
        font = ImageFont.load_default()
        font_small = font
    
    topic = clean_search_keywords(prompt)[:40]
    draw.text((width//2, height//2 - 20), topic, fill="#e0e0ff", anchor="mm", font=font)
    draw.text((width//2, height//2 + 30), "MoFA Engine · AI Orchestration", fill="#6060a0", anchor="mm", font=font_small)
    
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    img.save(str(out), "PNG")
    return True


def generate_image(prompt: str, out_path: str, model: str = "gemini-2.5-flash-image", api_key: str = None) -> bool:
    # 1. AI-generated topic-specific technical/architectural image (primary)
    if generate_ai_image(prompt, out_path):
        return True

    # 2. Fallback: styled text card
    return generate_fallback_card(prompt, out_path)


def main():
    parser = argparse.ArgumentParser(description="MoFA AI Image Engine")
    parser.add_argument("--prompt", required=True, help="Prompt description")
    parser.add_argument("--output", required=True, help="Output image path")
    parser.add_argument("--model", default="gemini-2.5-flash-image")
    parser.add_argument("--key", default=None)
    args = parser.parse_args()
    ok = generate_image(args.prompt, args.output, model=args.model, api_key=args.key)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
