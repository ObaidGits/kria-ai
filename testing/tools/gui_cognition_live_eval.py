#!/usr/bin/env python3
"""GUI Cognition live evaluation probe.

Runs a batch of real, daily GUI-action prompts through the SAME backend path the
desktop UI uses (POST /api/testing/desktop-chat-command, manual_profile =
gui_cognition), in execute_live + workflow mode, and classifies each result from
the structured `response.gui_cognition.*` payload.

It does NOT touch the manifest/inventory; it is an exploratory live probe whose
output is a markdown report + per-prompt raw JSON dumps.

Usage:
    python3 testing/tools/gui_cognition_live_eval.py \
        [--mode execute_live|execute_fixture|safety_only] \
        [--base-url http://127.0.0.1:3001] \
        [--out planning_docs/gui_cognition_live_eval_report.md]
"""
from __future__ import annotations

import argparse
import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass
class Prompt:
    pid: str
    text: str
    expect: str  # human-readable expected outcome
    may_be_absent: bool = False  # app might not be installed (negative-ok)


PROMPTS: list[Prompt] = [
    Prompt("p01", "Open the Calculator app", "Calculator window opens"),
    Prompt("p02", "Open the Files manager", "File manager (Nautilus) opens"),
    Prompt("p03", "Open the Text Editor", "Text editor opens"),
    Prompt("p04", "Open the Terminal", "Terminal opens"),
    Prompt("p05", "Open the Settings app", "Settings opens"),
    Prompt("p06", "Open Google Chrome", "Chrome opens"),
    Prompt("p07", "Open Firefox", "Firefox opens"),
    Prompt("p08", "Open the Calculator and type 5 + 5", "Calc opens, 5 + 5 typed"),
    Prompt("p09", "Open Google Chrome and search for the latest Ubuntu version",
           "Chrome opens, search runs"),
    Prompt("p10", "Open Google Chrome and open Gmail", "Chrome opens Gmail"),
    Prompt("p11", "Open Google Chrome and go to youtube.com", "Chrome navigates to YouTube"),
    Prompt("p12", "Open Firefox and search for weather today", "Firefox search runs"),
    Prompt("p13", "Open the Files manager and go to the Downloads folder",
           "Nautilus opens Downloads"),
    Prompt("p14", "Open the Settings and go to Wi-Fi", "Settings opens Wi-Fi pane"),
    Prompt("p15", "Open the Terminal and run ls", "Terminal runs ls"),
    Prompt("p16", "Open the Text Editor and type Hello World", "Editor types text"),
    Prompt("p17", "Open Google Chrome and open a new tab", "Chrome new tab"),
    Prompt("p18", "Open the Screenshot tool", "Screenshot tool opens"),
    Prompt("p19", "Open Google Chrome and go to github.com", "Chrome navigates to GitHub"),
    Prompt("p20", "Open Google Chrome, go to google.com and search for lofi beats",
           "Chrome searches"),
    Prompt("p21", "Open the Calculator and compute 256 times 13", "Calc computes"),
    Prompt("p22", "Open the Brave browser", "Brave opens", may_be_absent=True),
    Prompt("p23", "Open Spotify", "Spotify opens", may_be_absent=True),
    Prompt("p24", "Open Google Chrome and search for news today", "Chrome searches news"),
    Prompt("p25", "Focus the K.R.I.A. window", "KRIA window focused"),
]


@dataclass
class Result:
    prompt: Prompt
    ok_http: bool = False
    elapsed_ms: int = 0
    reply: str = ""
    error: str | None = None
    classification: str = "UNKNOWN"
    detail: str = ""
    facts: dict[str, Any] = field(default_factory=dict)
    raw_path: str = ""


def read_token() -> str | None:
    path = Path.home() / ".kria" / "api_token"
    if path.exists():
        return path.read_text(encoding="utf-8").strip()
    return None


def health(base_url: str) -> bool:
    try:
        with urllib.request.urlopen(f"{base_url.rstrip('/')}/api/health", timeout=10) as resp:
            return resp.status == 200
    except Exception:
        return False


def send(base_url: str, token: str | None, message: str, session_id: str, mode: str,
         timeout_seconds: int) -> tuple[bool, dict[str, Any], str | None]:
    payload = {
        "message": message,
        "session_id": session_id,
        "manual_profile": {
            "mode_id": "gui_cognition",
            "label": "GUI Cognition",
            "app_lock": "gui_cognition",
            "tool_lock": None,
            "strategy": "routed_within_lock",
        },
        "gui_cognition_test": {
            "execution_mode": mode,
            "workflow": True,
            "hitl_decision_fixture": "approve",
        },
    }
    body = json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/testing/desktop-chat-command",
        data=body, headers=headers, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout_seconds) as resp:
            raw = resp.read().decode("utf-8")
            return True, json.loads(raw), None
    except urllib.error.HTTPError as exc:
        return False, {}, f"HTTP {exc.code}: {exc.read().decode('utf-8', 'replace')[:300]}"
    except Exception as exc:  # noqa: BLE001
        return False, {}, f"{type(exc).__name__}: {exc}"


def gui_of(response: dict[str, Any]) -> dict[str, Any]:
    r = response.get("response")
    if isinstance(r, dict) and isinstance(r.get("gui_cognition"), dict):
        return r["gui_cognition"]
    if isinstance(response.get("gui_cognition"), dict):
        return response["gui_cognition"]
    return {}


def reply_of(response: dict[str, Any]) -> str:
    r = response.get("response")
    if isinstance(r, dict) and r.get("reply"):
        return str(r["reply"])
    return str(response.get("reply") or "")


def classify(prompt: Prompt, g: dict[str, Any], reply: str) -> tuple[str, str, dict[str, Any]]:
    """Return (classification, detail, facts)."""
    facts: dict[str, Any] = {}
    plan = g.get("plan") or {}
    tr = g.get("target_resolution") or {}
    execu = g.get("execution") or {}
    verif = g.get("verification") or {}
    wf = g.get("workflow_run") or {}
    blocker = g.get("blocker") or {}

    facts["intent"] = g.get("intent")
    facts["risk_level"] = g.get("risk_level")
    facts["requires_approval"] = g.get("requires_approval")
    facts["plan_id"] = plan.get("plan_id")
    facts["target_status"] = tr.get("status")
    facts["action_type"] = execu.get("action_type")
    facts["exec_status"] = execu.get("status")
    facts["exec_error_code"] = execu.get("error_code")
    facts["exec_error"] = execu.get("safe_error_summary")
    facts["backend_used"] = execu.get("backend_used")
    facts["verification_status"] = verif.get("status")
    facts["blocker_kind"] = blocker.get("kind")
    facts["blocker_reason"] = blocker.get("reason")
    if wf:
        steps = wf.get("steps") or wf.get("step_states") or []
        facts["workflow_step_count"] = wf.get("step_count") or (len(steps) if isinstance(steps, list) else None)
        facts["workflow_status"] = wf.get("status")

    # Empty / garbage
    if not g and not reply:
        return "UNEXPECTED", "Empty gui_cognition payload and empty reply.", facts

    exec_status = execu.get("status")
    verif_status = verif.get("status")
    wf_status = (wf.get("status") if isinstance(wf, dict) else None)

    # Hard executor bug: action kind leaked as app name.
    err = (execu.get("safe_error_summary") or "")
    if "is not found in the installed app registry" in err and prompt and not prompt.may_be_absent:
        # If the not-found name equals the action type, that's the OpenApp-name bug.
        atype = str(execu.get("action_type") or "")
        if atype and f"'{atype}'" in err:
            return "BUG", f"Executor passed action kind '{atype}' as the app name → {err}", facts
        return "PARTIAL", f"App not found: {err}", facts

    succeeded = exec_status in ("succeeded", "completed")

    # Whole workflow completed and verified.
    if wf_status == "completed" and verif_status in ("verified", None):
        return "PASS", "Workflow completed and verified.", facts
    if succeeded and verif_status == "verified":
        return "PASS", "Executed and verified.", facts
    if succeeded and verif_status in ("verification_failed", "inconclusive", None):
        # Action ran but post-state not confirmed (often a later workflow step).
        if blocker:
            return "PARTIAL_PROGRESS", (
                f"First action ran; workflow then blocked: {blocker.get('reason')}"), facts
        return "PARTIAL", f"Executed but verification={verif_status}.", facts
    if exec_status == "failed":
        if prompt.may_be_absent and ("not installed" in err or "registry" in err):
            return "EXPECTED_ABSENT", f"App likely not installed: {err}", facts
        return "PARTIAL", f"Execution failed: {err or execu.get('error_code')}", facts

    # No execution attempted.
    if blocker:
        if prompt.may_be_absent and "registry" in (blocker.get("reason") or ""):
            return "EXPECTED_ABSENT", f"App likely not installed: {blocker.get('reason')}", facts
        return "BLOCKED", f"Blocked ({blocker.get('kind')}): {blocker.get('reason')}", facts
    if tr.get("status") == "resolved":
        return "PARTIAL", "Resolved but no execution result captured.", facts
    if plan.get("plan_id"):
        return "PARTIAL", "Planned but not resolved/executed.", facts
    return "UNEXPECTED", f"Pipeline produced no plan. reply={reply[:80]!r}", facts


def preflight_ready() -> tuple[bool, str]:
    """Read the latest environment-preflight artifact (spec Task 0 / Req 14.4).
    Returns (ready, reason). Missing artifact → not ready (run preflight first)."""
    latest = Path("eval_reports/gui_cog/preflight_latest.json")
    if not latest.exists():
        return False, "no preflight artifact (run scripts/gui_cog_preflight.py first)"
    try:
        rec = json.loads(latest.read_text())
    except Exception as exc:  # noqa: BLE001
        return False, f"unreadable preflight artifact: {exc}"
    if rec.get("ready") is True:
        return True, "ready"
    return False, rec.get("reason", "preflight not ready")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", default="execute_live",
                    choices=["execute_live", "execute_fixture", "safety_only"])
    ap.add_argument("--base-url", default="http://127.0.0.1:3001")
    ap.add_argument("--timeout", type=int, default=180)
    ap.add_argument("--out", default="planning_docs/gui_cognition_live_eval_report.md")
    ap.add_argument("--raw-dir", default="planning_docs/gui_cognition_live_eval_raw")
    ap.add_argument("--only", default=None, help="comma-separated prompt ids to run")
    ap.add_argument("--skip-preflight", action="store_true",
                    help="bypass the environment preflight gate (NOT recommended)")
    args = ap.parse_args()

    # Preflight gate (spec Req 14.4): refuse to run unless the latest environment
    # preflight is ready, so a half-up stack never yields fabricated results.
    if not args.skip_preflight:
        ok, reason = preflight_ready()
        if not ok:
            print(f"FATAL: environment preflight not ready: {reason}")
            print("Run: python3 scripts/gui_cog_preflight.py --auto-start  (or pass --skip-preflight)")
            return 3

    if not health(args.base_url):
        print(f"FATAL: desktop API not healthy at {args.base_url}")
        return 2

    token = read_token()
    raw_dir = Path(args.raw_dir)
    raw_dir.mkdir(parents=True, exist_ok=True)

    only = set(s.strip() for s in args.only.split(",")) if args.only else None
    prompts = [p for p in PROMPTS if (only is None or p.pid in only)]

    results: list[Result] = []
    for i, p in enumerate(prompts, 1):
        sid = f"live-eval-{p.pid}-{int(time.time())}"
        print(f"[{i}/{len(prompts)}] {p.pid} :: {p.text}", flush=True)
        t0 = time.time()
        ok, resp, err = send(args.base_url, token, p.text, sid, args.mode, args.timeout)
        elapsed = int((time.time() - t0) * 1000)
        res = Result(prompt=p, ok_http=ok, elapsed_ms=elapsed)
        if not ok:
            res.classification = "UNEXPECTED"
            res.error = err
            res.detail = err or "request failed"
            print(f"    -> ERROR {err}", flush=True)
        else:
            g = gui_of(resp)
            reply = reply_of(resp)
            res.reply = reply
            cls, detail, facts = classify(p, g, reply)
            res.classification = cls
            res.detail = detail
            res.facts = facts
            raw_path = raw_dir / f"{p.pid}.json"
            raw_path.write_text(json.dumps(resp, indent=2, default=str), encoding="utf-8")
            res.raw_path = str(raw_path)
            print(f"    -> {cls} | exec={facts.get('exec_status')} "
                  f"verify={facts.get('verification_status')} | {detail[:90]}", flush=True)
        results.append(res)
        time.sleep(1.0)

    write_report(Path(args.out), args.mode, args.base_url, results)
    print(f"\nReport: {args.out}")

    counts: dict[str, int] = {}
    for r in results:
        counts[r.classification] = counts.get(r.classification, 0) + 1
    print("Summary:", json.dumps(counts))
    return 0


def write_report(path: Path, mode: str, base_url: str, results: list[Result]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    counts: dict[str, int] = {}
    for r in results:
        counts[r.classification] = counts.get(r.classification, 0) + 1
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")

    lines: list[str] = []
    lines.append("# GUI Cognition Live Evaluation Report")
    lines.append("")
    lines.append(f"- Generated: {now}")
    lines.append(f"- Mode: `{mode}`  ·  Endpoint: `{base_url}/api/testing/desktop-chat-command`")
    lines.append(f"- Path: same as UI (`send_manual_tool_message`, manual_profile=`gui_cognition`, workflow=true, hitl=approve)")
    lines.append(f"- Prompts: {len(results)}")
    lines.append("")
    lines.append("## Summary")
    lines.append("")
    lines.append("| Outcome | Count |")
    lines.append("|---|---|")
    for k in sorted(counts):
        lines.append(f"| {k} | {counts[k]} |")
    lines.append("")
    lines.append("Legend: **PASS** executed+verified · **PARTIAL** ran but failed/unverified · "
                 "**BUG** concrete defect · **BLOCKED** stopped before execution · "
                 "**EXPECTED_ABSENT** app not installed · **UNEXPECTED** crash/empty/no-plan.")
    lines.append("")
    lines.append("## Results")
    lines.append("")
    lines.append("| ID | Prompt | Outcome | exec | verify | Detail |")
    lines.append("|---|---|---|---|---|---|")
    for r in results:
        f = r.facts
        detail = r.detail.replace("|", "\\|")[:120]
        lines.append(
            f"| {r.prompt.pid} | {r.prompt.text} | {r.classification} | "
            f"{f.get('exec_status')} | {f.get('verification_status')} | {detail} |"
        )
    lines.append("")
    lines.append("## Per-prompt detail")
    lines.append("")
    for r in results:
        f = r.facts
        lines.append(f"### {r.prompt.pid} — {r.prompt.text}")
        lines.append("")
        lines.append(f"- Expected: {r.prompt.expect}")
        lines.append(f"- Outcome: **{r.classification}** ({r.elapsed_ms} ms)")
        lines.append(f"- Detail: {r.detail}")
        if r.reply:
            lines.append(f"- Reply: {r.reply[:200]}")
        if r.error:
            lines.append(f"- Error: {r.error}")
        if f:
            keys = ["intent", "risk_level", "requires_approval", "plan_id", "target_status",
                    "action_type", "exec_status", "exec_error_code", "exec_error",
                    "backend_used", "verification_status", "workflow_status",
                    "workflow_step_count", "blocker_kind", "blocker_reason"]
            lines.append("- Signals:")
            for k in keys:
                if f.get(k) is not None:
                    lines.append(f"  - `{k}` = {f.get(k)}")
        if r.raw_path:
            lines.append(f"- Raw: `{r.raw_path}`")
        lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
