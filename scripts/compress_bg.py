"""One-off: compress the homepage background into a bundled, web-optimised asset.

Reads the source PNG, downscales to a sensible max width, and writes a quality-
tuned WebP (with a JPEG fallback) into ui/public/backgrounds so the frontend can
serve it offline. Run: .venv/bin/python scripts/compress_bg.py
"""
from pathlib import Path
from PIL import Image

SRC = Path("/home/obaid/Downloads/dark-theme-night.png")
OUT_DIR = Path("ui/public/backgrounds")
OUT_DIR.mkdir(parents=True, exist_ok=True)

img = Image.open(SRC).convert("RGB")

# Downscale to a max width of 1920 (retina-friendly, far smaller than source need).
MAX_W = 1920
if img.width > MAX_W:
    h = round(img.height * MAX_W / img.width)
    img = img.resize((MAX_W, h), Image.LANCZOS)

webp = OUT_DIR / "dark-theme-night.webp"
jpg = OUT_DIR / "dark-theme-night.jpg"
img.save(webp, "WEBP", quality=72, method=6)
img.save(jpg, "JPEG", quality=78, optimize=True, progressive=True)

for p in (SRC, webp, jpg):
    print(f"{p}: {p.stat().st_size / 1024:.0f} KB  {getattr(Image.open(p), 'size', '')}")
