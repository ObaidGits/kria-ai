#!/usr/bin/env python3
"""
Idempotent setup for GUI Cognition V2 — Sight (OmniParser) layer.

Installs the vision-sidecar dependencies (CPU torch + ultralytics + mss +
huggingface_hub, and optionally a Florence-2 caption stack), downloads the
OmniParser v2 weights, and verifies everything — all idempotently, so re-running
is safe and fast. This is the automation core: the same script powers a
developer spike today and the user-facing first-run provisioning later.

Usage:
    python scripts/setup_gui_cognition.py            # install + download + verify
    python scripts/setup_gui_cognition.py --check    # verify only (no changes)
    python scripts/setup_gui_cognition.py --no-caption  # skip Florence-2 caption deps

Honest behavior: each step checks "already done?" and skips if so. Nothing is
faked — a missing/unverifiable piece is reported as PENDING/FAIL with the reason.
Exit code 0 = ready, 1 = pending/failed.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
VENV_DIR = REPO_ROOT / "sidecars" / "kria-vision" / "venv"
VENV_PY = VENV_DIR / "bin" / "python"
VENV_PIP = [str(VENV_PY), "-m", "pip"]

OMNI_REPO = "microsoft/OmniParser-v2.0"
OMNI_DIR = Path(os.environ.get("KRIA_OMNIPARSER_DIR", str(Path.home() / ".kria" / "models" / "omniparser")))

# Required runtime imports for the OmniParser Sight backend.
CORE_MODULES = ["mss", "ultralytics", "torch", "huggingface_hub"]
CAPTION_MODULES = ["transformers", "timm", "einops"]
# Florence-2 (OmniParser caption) is incompatible with the transformers 5.x line;
# pin a known-good version so caption labels are semantic, not generic "icon".
CAPTION_TRANSFORMERS_VERSION = "4.49.0"


def transformers_caption_compatible() -> bool:
    """True when the installed transformers can load Florence-2 for captioning."""
    r = subprocess.run(
        [str(VENV_PY), "-c",
         "import transformers,sys;"
         "v=tuple(int(x) for x in transformers.__version__.split('.')[:2]);"
         "sys.exit(0 if (4,38) <= v < (4,52) else 1)"],
        capture_output=True,
    )
    return r.returncode == 0


def log(msg: str):
    print(f"[setup-gui-cog] {msg}", flush=True)


def have_module(mod: str) -> bool:
    r = subprocess.run([str(VENV_PY), "-c", f"import {mod}"], capture_output=True)
    return r.returncode == 0


def run(cmd, desc: str) -> bool:
    log(f"-> {desc}")
    r = subprocess.run(cmd)
    if r.returncode != 0:
        log(f"   FAILED: {desc}")
        return False
    return True


def ensure_venv() -> bool:
    if VENV_PY.exists():
        return True
    log("vision-sidecar venv missing; creating it")
    return run([sys.executable, "-m", "venv", str(VENV_DIR)], "create venv")


def find_icon_detector() -> Path | None:
    if not OMNI_DIR.exists():
        return None
    # OmniParser v2 ships the icon detector as a YOLO .pt under icon_detect/.
    preferred = OMNI_DIR / "icon_detect" / "model.pt"
    if preferred.exists():
        return preferred
    pts = sorted(OMNI_DIR.rglob("*.pt"))
    return pts[0] if pts else None


def find_caption_dir() -> Path | None:
    if not OMNI_DIR.exists():
        return None
    for name in ("icon_caption", "icon_caption_florence", "icon_caption_blip2"):
        d = OMNI_DIR / name
        if d.is_dir() and any(d.iterdir()):
            return d
    return None


# ---------------------------------------------------------------------------
# Install steps (idempotent)
# ---------------------------------------------------------------------------

def install_deps(with_caption: bool) -> bool:
    ok = True
    # 1. CPU torch first — small (~200MB) and keeps OmniParser detection off the
    #    6GB GPU so the resident LLM keeps its VRAM. ultralytics then reuses it.
    if not have_module("torch"):
        ok &= run(
            VENV_PIP + ["install", "torch", "torchvision",
                        "--index-url", "https://download.pytorch.org/whl/cpu"],
            "install CPU torch + torchvision",
        )
    else:
        log("torch already installed — skip")

    # 2. Core sight deps.
    missing = [m for m in ["ultralytics", "mss", "huggingface_hub"] if not have_module(m)]
    if missing:
        ok &= run(VENV_PIP + ["install", *missing], f"install {', '.join(missing)}")
    else:
        log("ultralytics/mss/huggingface_hub already installed — skip")

    # 3. Optional caption stack (better element labels). Florence-2 needs a
    #    COMPATIBLE transformers (the 5.x line broke Florence-2:
    #    'Florence2LanguageConfig has no attribute forced_bos_token_id'), so we
    #    pin a known-good version. Idempotent: only (re)install when missing or
    #    the installed transformers is outside the supported range.
    if with_caption:
        need_install = False
        if not all(have_module(m) for m in CAPTION_MODULES):
            need_install = True
        elif not transformers_caption_compatible():
            log("transformers present but incompatible with Florence-2 caption — pinning")
            need_install = True
        if need_install:
            ok &= run(
                VENV_PIP + ["install", f"transformers=={CAPTION_TRANSFORMERS_VERSION}", "timm", "einops"],
                f"install caption deps (transformers=={CAPTION_TRANSFORMERS_VERSION})",
            )
        else:
            log("caption deps already installed (compatible) — skip")
    return ok


def download_weights() -> bool:
    if find_icon_detector() is not None:
        log(f"OmniParser weights already present at {OMNI_DIR} — skip")
        return True
    OMNI_DIR.mkdir(parents=True, exist_ok=True)
    log(f"downloading {OMNI_REPO} -> {OMNI_DIR} (resumable)")
    code = (
        "from huggingface_hub import snapshot_download;"
        f"snapshot_download(repo_id='{OMNI_REPO}', local_dir=r'{OMNI_DIR}',"
        "allow_patterns=['icon_detect/*','icon_caption*/*','*.md'])"
    )
    return run([str(VENV_PY), "-c", code], "snapshot_download OmniParser v2")


# ---------------------------------------------------------------------------
# Verify
# ---------------------------------------------------------------------------

def verify(with_caption: bool) -> bool:
    ok = True
    log("verify: python deps")
    for m in CORE_MODULES:
        present = have_module(m)
        log(f"   {m}: {'OK' if present else 'MISSING'}")
        ok &= present
    if with_caption:
        for m in CAPTION_MODULES:
            log(f"   {m}: {'OK' if have_module(m) else 'MISSING (caption optional)'}")

    log("verify: weights")
    det = find_icon_detector()
    if det:
        log(f"   icon detector: {det}")
    else:
        log("   icon detector: MISSING")
        ok = False
    cap = find_caption_dir()
    log(f"   caption dir: {cap if cap else 'none (labels = detector class names)'}")

    # Load the YOLO weights to confirm they are valid + loadable (cheap).
    if det and have_module("ultralytics"):
        log("verify: loading icon detector with ultralytics (CPU)")
        chk = subprocess.run(
            [str(VENV_PY), "-c",
             f"import os;os.environ['CUDA_VISIBLE_DEVICES']='';"
             f"from ultralytics import YOLO;YOLO(r'{det}');print('YOLO_OK')"],
            capture_output=True, text=True,
        )
        if "YOLO_OK" in chk.stdout:
            log("   detector loads OK")
        else:
            log(f"   detector load FAILED: {chk.stderr.strip()[-300:]}")
            ok = False

    log("verify: system screenshot tool (grim, for Wayland capture)")
    grim = subprocess.run(["bash", "-c", "command -v grim"], capture_output=True, text=True)
    if grim.stdout.strip():
        log(f"   grim: {grim.stdout.strip()}")
    else:
        log("   grim: MISSING — install with:  sudo apt install -y grim")
        # grim is only needed for the manual /parse capture test, not the
        # sidecar itself (it uses mss), so this does NOT fail readiness.

    return ok


def print_run_block():
    det = find_icon_detector()
    cap = find_caption_dir()
    log("to run the sidecar with the OmniParser backend (detection on CPU):")
    print("\n  cd sidecars/kria-vision")
    print("  CUDA_VISIBLE_DEVICES=\"\" \\")
    print("  KRIA_VISION_MODEL=omniparser \\")
    print(f"  KRIA_OMNIPARSER_WEIGHTS={det if det else '<icon_detect model.pt>'} \\")
    if cap:
        print(f"  KRIA_OMNIPARSER_CAPTION={cap} \\")
    print("  KRIA_VISION_PORT=8080 \\")
    print("  venv/bin/python main.py\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="verify only, no install/download")
    ap.add_argument("--no-caption", action="store_true", help="skip Florence-2 caption deps")
    args = ap.parse_args()
    with_caption = not args.no_caption

    if not VENV_PY.exists() and not ensure_venv():
        log("FATAL: no usable vision-sidecar venv")
        return 1

    if not args.check:
        if not install_deps(with_caption):
            log("dependency install reported errors (see above)")
        if not download_weights():
            log("weights download reported errors (see above)")

    ready = verify(with_caption)
    print()
    if ready:
        log("RESULT: READY ✅  — OmniParser Sight deps + weights present and valid.")
        print_run_block()
        return 0
    else:
        log("RESULT: PENDING ❌ — see MISSING/FAILED items above. Re-run to resume.")
        return 1


if __name__ == "__main__":
    sys.exit(main())
