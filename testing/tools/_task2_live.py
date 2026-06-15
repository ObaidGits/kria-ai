#!/usr/bin/env python3
"""Task 2 (Issue #3) live probe: send a single GUI-cognition prompt LIVE through
the same backend path the UI uses and print the per-step verification verdicts so
we can honestly judge execute-vs-flap. NEVER fabricates: prints exactly what the
structured payload reports.

Usage:
    python3 testing/tools/_task2_live.py "Open Chrome, focus the address bar, type kria.ai, and press Enter"
"""
from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:3001"


def token() -> str | None:
    p = Path.home() / ".kria" / "api_token"
    return p.read_text(encoding="utf-8").strip() if p.exists() else None


def send(msg: str, sid: str, timeout: int = 220):
    payload = {
        "message": msg,
        "session_id": sid,
        "manual_profile": {
            "mode_id": "gui_cognition",
            "label": "GUI Cognition",
            "app_lock": "gui_cognition",
            "tool_lock": None,
            "strategy": "routed_within_lock",
        },
        "gui_cognition_test": {
            "execution_mode": "execute_live",
            "workflow": True,
            "hitl_decision_fixture": "approve",
        },
    }
    headers = {"Content-Type": "application/json"}
    tok = token()
    if tok:
        headers["Authorization"] = f"Bearer {tok}"
    req = urllib.request.Request(
        f"{BASE}/api/testing/desktop-chat-command",
        data=json.dumps(payload).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode())


def main() -> int:
    msg = sys.argv[1] if len(sys.argv) > 1 else "Open Chrome, focus the address bar, type kria.ai, and press Enter"
    sid = f"task2-{int(time.time())}"
    print(f"PROMPT: {msg}")
    print(f"SID: {sid}")
    try:
        resp = send(msg, sid)
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code}: {e.read().decode('utf-8', 'replace')[:400]}")
        return 1
    except Exception as e:  # noqa: BLE001
        print(f"ERROR {type(e).__name__}: {e}")
        return 1

    gc = resp.get("response", {}).get("gui_cognition") or resp.get("gui_cognition") or {}
    if not gc:
        print("NO gui_cognition payload. Top-level keys:", list(resp.keys()))
        print(json.dumps(resp, indent=2)[:1500])
        return 1

    print("STATUS:", gc.get("status"))
    print("PLANNER_MODE:", gc.get("planner_mode") or gc.get("plan", {}).get("planner_mode"))
    run = gc.get("workflow_run") or {}
    print("WORKFLOW_STATUS:", run.get("status"))
    steps = run.get("steps") or gc.get("steps") or []
    print(f"STEPS ({len(steps)}):")
    for s in steps:
        sid_ = s.get("step_id")
        st = s.get("step_type") or s.get("action_type")
        status = s.get("status")
        ver = s.get("verification") or {}
        verdict = ver.get("verdict") or ver.get("status") or s.get("verification_status")
        conf = ver.get("confidence")
        print(f"  - {sid_:8} {str(st):16} status={status:12} verify={verdict} conf={conf}")
    # Honest dump of any blocker / reply
    if gc.get("blocker"):
        print("BLOCKER:", json.dumps(gc.get("blocker"))[:300])
    print("REPLY:", (gc.get("reply") or resp.get("response", {}).get("reply") or "")[:300])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
