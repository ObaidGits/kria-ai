#!/usr/bin/env python3
"""GUI Cognition — Environment Preflight (spec task 0).

Brings up / health-checks the full GUI-cognition stack and emits a machine-readable
JSON record that every live proof run references as its precondition:

    { "ready": bool,
      "components": [ { "name", "ok", "detail", "port", "version" }, ... ],
      "reason": "" }

Exit code is 0 only when every REQUIRED component is healthy (ready == true), else
non-zero with an actionable `reason`. The result is also written to
`eval_reports/gui_cog/preflight_<ts>.json` and `eval_reports/gui_cog/preflight_latest.json`
so the live proof harness can refuse to run unless the latest preflight is ready.

Requirements: 14.1–14.5 of the gui-cognition-intelligence-upgrade spec.

Usage:
    python3 scripts/gui_cog_preflight.py
        [--base-url http://127.0.0.1:3001]      # KRIA desktop local API
        [--vision-url http://127.0.0.1:8080]    # kria-vision sidecar
        [--auto-start]                          # best-effort start of what we can
        [--quiet]                               # only emit the final JSON

Only the Python standard library is used (no extra deps).
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
ARTIFACT_DIR = REPO_ROOT / "eval_reports" / "gui_cog"
DEFAULT_BASE_URL = os.environ.get("KRIA_LOCAL_API_URL", "http://127.0.0.1:3001")
DEFAULT_VISION_URL = os.environ.get("KRIA_OMNIPARSER_ENDPOINT", "http://127.0.0.1:8080")


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _http_get_json(url: str, timeout: float = 4.0) -> tuple[bool, Any, str]:
    """GET a URL; return (ok, parsed_json_or_text, detail)."""
    try:
        req = urllib.request.Request(url, headers={"Accept": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            code = resp.getcode()
            body = resp.read().decode("utf-8", "replace")
            if code != 200:
                return False, None, f"HTTP {code}"
            try:
                return True, json.loads(body), f"HTTP {code}"
            except json.JSONDecodeError:
                return True, body, f"HTTP {code} (non-JSON)"
    except urllib.error.HTTPError as exc:
        return False, None, f"HTTP {exc.code}"
    except (urllib.error.URLError, socket.timeout, ConnectionError) as exc:
        return False, None, f"unreachable: {exc}"
    except Exception as exc:  # noqa: BLE001 — preflight must never throw
        return False, None, f"error: {exc}"


def _port_of(url: str) -> int | None:
    try:
        from urllib.parse import urlparse

        p = urlparse(url)
        if p.port:
            return p.port
        return 443 if p.scheme == "https" else 80
    except Exception:  # noqa: BLE001
        return None


def _uinput_socket_path() -> Path:
    """Mirror kria-core `default_uinput_socket_path()` resolution order."""
    env = os.environ.get("KRIA_UINPUT_SOCKET")
    if env:
        return Path(env)
    rt = os.environ.get("XDG_RUNTIME_DIR")
    if rt:
        return Path(rt) / "kria-uinput.sock"
    cache = os.environ.get("XDG_CACHE_HOME")
    if cache:
        return Path(cache) / "kria" / "kria-uinput.sock"
    home = os.environ.get("HOME")
    if home:
        return Path(home) / ".cache" / "kria" / "kria-uinput.sock"
    return Path("/tmp/kria-uinput.sock")


def _pgrep(pattern: str) -> str | None:
    """Return the first matching process command line, or None."""
    if not shutil.which("pgrep"):
        return None
    try:
        out = subprocess.run(
            ["pgrep", "-af", pattern],
            capture_output=True,
            text=True,
            timeout=4,
        )
        line = out.stdout.strip().splitlines()
        return line[0] if line else None
    except Exception:  # noqa: BLE001
        return None


# ── Component checks ─────────────────────────────────────────────────────────
# Each returns a component dict: { name, ok, detail, port, version, required }.


def check_display() -> dict:
    wayland = os.environ.get("WAYLAND_DISPLAY")
    x11 = os.environ.get("DISPLAY")
    session = os.environ.get("XDG_SESSION_TYPE", "")
    ok = bool(wayland or x11)
    detail = f"session_type={session or '?'} DISPLAY={x11 or '-'} WAYLAND_DISPLAY={wayland or '-'}"
    return {"name": "display", "ok": ok, "detail": detail, "port": None,
            "version": None, "required": True}


def check_uinput() -> dict:
    sock = _uinput_socket_path()
    exists = sock.exists()
    proc = _pgrep("kria-uinput-daemon")
    ok = exists or bool(proc)
    detail = f"socket={sock} exists={exists}"
    if proc:
        detail += " | daemon running"
    elif not exists:
        detail += " | no socket & no daemon process"
    return {"name": "uinput_daemon", "ok": ok, "detail": detail, "port": None,
            "version": None, "required": True}


def check_desktop(base_url: str) -> dict:
    url = base_url.rstrip("/") + "/api/health"
    ok, body, detail = _http_get_json(url)
    version = None
    if isinstance(body, dict):
        version = body.get("version")
        status = body.get("status")
        detail = f"{detail} status={status}"
    return {"name": "kria_desktop", "ok": ok, "detail": f"{url} -> {detail}",
            "port": _port_of(base_url), "version": version, "required": True}


def check_vision(vision_url: str) -> dict:
    url = vision_url.rstrip("/") + "/health"
    ok, body, detail = _http_get_json(url)
    version = None
    model = None
    if isinstance(body, dict):
        version = body.get("version")
        model = body.get("model") or body.get("model_name")
        if model:
            detail = f"{detail} model={model}"
    return {"name": "kria_vision_sidecar", "ok": ok,
            "detail": f"{url} -> {detail}", "port": _port_of(vision_url),
            "version": version, "required": True}


def check_model_server(base_url: str) -> dict:
    """Best-effort: the planner/text model server (llama-server) is spawned by the
    desktop on a dynamic port. We detect the process; if the desktop is up the model
    may be loaded lazily on first turn, so a missing process is a soft warning when
    the desktop is healthy."""
    proc = _pgrep("llama-server") or _pgrep("llama_server") or _pgrep("llama.cpp")
    ok = bool(proc)
    detail = f"process={'found' if ok else 'not found'}"
    if proc:
        detail = f"running: {proc[:120]}"
    return {"name": "model_server", "ok": ok, "detail": detail, "port": None,
            "version": None, "required": True}


# ── Best-effort auto-start ───────────────────────────────────────────────────


def try_autostart_vision(vision_url: str, quiet: bool) -> None:
    """Start the kria-vision sidecar if it is down and we can find a launcher.
    Best-effort: never throws; the re-check decides health."""
    sidecar = REPO_ROOT / "sidecars" / "kria-vision"
    main_py = sidecar / "main.py"
    if not main_py.exists():
        return
    port = _port_of(vision_url) or 8080
    # Prefer the sidecar venv if present.
    py = sidecar / "venv" / "bin" / "python"
    python = str(py) if py.exists() else sys.executable
    if not quiet:
        print(f"[preflight] auto-start: launching kria-vision sidecar on :{port}", file=sys.stderr)
    try:
        log = ARTIFACT_DIR / "vision_sidecar.log"
        ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
        with open(log, "ab") as fh:
            subprocess.Popen(
                [python, "-m", "uvicorn", "main:app", "--host", "127.0.0.1",
                 "--port", str(port)],
                cwd=str(sidecar),
                stdout=fh,
                stderr=fh,
                start_new_session=True,
            )
        # Give uvicorn a moment to bind.
        time.sleep(2.5)
    except Exception as exc:  # noqa: BLE001
        if not quiet:
            print(f"[preflight] auto-start vision failed: {exc}", file=sys.stderr)


def run_preflight(base_url: str, vision_url: str, auto_start: bool, quiet: bool) -> dict:
    # First pass.
    components = [
        check_display(),
        check_uinput(),
        check_desktop(base_url),
        check_vision(vision_url),
        check_model_server(base_url),
    ]

    # Best-effort auto-start for what we can, then re-check those.
    if auto_start:
        vision = next(c for c in components if c["name"] == "kria_vision_sidecar")
        if not vision["ok"]:
            try_autostart_vision(vision_url, quiet)
            idx = components.index(vision)
            components[idx] = check_vision(vision_url)

    missing = [c["name"] for c in components if c["required"] and not c["ok"]]
    ready = len(missing) == 0
    reason = "" if ready else (
        "Unhealthy required component(s): " + ", ".join(missing) +
        ". Fix or start them (try --auto-start), then re-run preflight."
    )
    return {
        "ready": ready,
        "generated_at": _now_iso(),
        "base_url": base_url,
        "vision_url": vision_url,
        "components": components,
        "reason": reason,
    }


def write_artifacts(record: dict) -> Path:
    ARTIFACT_DIR.mkdir(parents=True, exist_ok=True)
    ts = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    path = ARTIFACT_DIR / f"preflight_{ts}.json"
    path.write_text(json.dumps(record, indent=2))
    latest = ARTIFACT_DIR / "preflight_latest.json"
    latest.write_text(json.dumps(record, indent=2))
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description="GUI Cognition environment preflight")
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--vision-url", default=DEFAULT_VISION_URL)
    parser.add_argument("--auto-start", action="store_true")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    record = run_preflight(args.base_url, args.vision_url, args.auto_start, args.quiet)
    path = write_artifacts(record)

    print(json.dumps(record, indent=2))
    if not args.quiet:
        print(f"[preflight] artifact: {path}", file=sys.stderr)
        print(f"[preflight] ready={record['ready']}", file=sys.stderr)
    return 0 if record["ready"] else 1


if __name__ == "__main__":
    sys.exit(main())
