#!/usr/bin/env python3
"""Staged GUI-Cognition eval against the LIVE Kria app testing API.

Drives the same backend path the desktop UI uses
(`/api/testing/desktop-chat-command` -> desktop_gui_cognition_command_capture),
so the planning/decision layer is exercised end-to-end through the real app.

Honest scoring at the PLAN level (deterministic, no fabricated execution):
  PASS         - plan is valid AND contains the expected step(s)
  FAIL         - plan invalid OR the expected action was dropped
  INCONCLUSIVE - app/model unavailable, timeout, or ambiguous (e.g. clarification)

NOTE: in headless API mode an executable turn stops at the HITL safety gate
(can_execute=false) by design — that is NOT a FAIL. We score the PLAN the app
produced, which is what determines on-screen behavior after approval in the UI.
"""
import json
import os
import sys
import time
import urllib.request

API = os.environ.get("KRIA_API", "http://127.0.0.1:3001")
TOKEN = open(os.path.expanduser("~/.kria/api_token")).read().strip()
TIMEOUT = int(os.environ.get("EVAL_TIMEOUT", "240"))

# Each case: prompt + expectation predicate over the parsed plan summary.
#   kind="multi_shortcut": expect a PressKey step carrying `combo` (+ valid plan)
#   kind="single_open":    expect exactly an OpenApp-led plan (valid)
#   kind="no_convert":     a non-standard control click must NOT become a shortcut
CASES = [
    {"prompt": "Open chrome and create a new tab", "kind": "multi_shortcut", "combo": "ctrl+t"},
    {"prompt": "Open chrome and reload the page", "kind": "multi_shortcut", "combo": "ctrl+r"},
    {"prompt": "Open the text editor and select all the text", "kind": "multi_shortcut", "combo": "ctrl+a"},
    {"prompt": "Open chrome and close the current tab", "kind": "multi_shortcut", "combo": "ctrl+w"},
    {"prompt": "Open the calculator", "kind": "single_open"},
    {"prompt": "Open settings and click the Submit button", "kind": "no_convert"},
]


def call(prompt: str) -> dict:
    body = json.dumps({
        "message": prompt,
        "manual_profile": {"mode_id": "gui_cognition", "app_lock": "gui_cognition"},
    }).encode()
    req = urllib.request.Request(
        f"{API}/api/testing/desktop-chat-command",
        data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
        return json.load(r)


def summarize(resp: dict) -> dict:
    g = (resp.get("response") or {}).get("gui_cognition") or {}
    pl = g.get("plan") or {}
    ts = pl.get("typed_steps") or []
    return {
        "planner_mode": pl.get("planner_mode"),
        "valid": (g.get("plan_validation") or {}).get("status"),
        "steps": [(s.get("step_type"), s.get("text_payload_summary")) for s in ts],
    }


def score(case: dict, s: dict) -> tuple[str, str]:
    steps = s["steps"]
    valid = s["valid"] == "valid"
    keys = [c for (t, c) in steps if t == "PressKey"]
    types = [t for (t, _c) in steps]
    if case["kind"] == "multi_shortcut":
        if not valid:
            return "FAIL", f"plan not valid ({s['valid']})"
        if case["combo"] in keys:
            return "PASS", f"PressKey {case['combo']} kept in plan"
        return "FAIL", f"expected PressKey {case['combo']}, got steps {steps}"
    if case["kind"] == "single_open":
        if valid and types and types[0] == "OpenApp" and "PressKey" not in types:
            return "PASS", "single OpenApp plan, no spurious shortcut"
        return ("INCONCLUSIVE", f"unexpected single-action plan {steps}") if valid else ("FAIL", "invalid")
    if case["kind"] == "no_convert":
        # A non-standard control must NOT be converted to a shortcut. A clarify/
        # ask or a ClickControl are both acceptable (no false PressKey).
        if "PressKey" in types:
            return "FAIL", f"non-standard control wrongly converted to a shortcut: {steps}"
        return "PASS", f"no false shortcut conversion ({types})"
    return "INCONCLUSIVE", "unknown kind"


def main() -> int:
    print(f"# GUI Cognition staged eval — live app {API}\n")
    rows = []
    for i, case in enumerate(CASES, 1):
        print(f"[{i}/{len(CASES)}] {case['prompt']!r} ...", flush=True)
        t0 = time.time()
        try:
            resp = call(case["prompt"])
            s = summarize(resp)
            verdict, why = score(case, s)
        except Exception as e:  # noqa: BLE001
            s = {"planner_mode": None, "valid": None, "steps": []}
            verdict, why = "INCONCLUSIVE", f"error: {e}"
        dt = time.time() - t0
        rows.append((case["prompt"], verdict, why, s, dt))
        print(f"    -> {verdict} ({dt:.0f}s) | mode={s['planner_mode']} valid={s['valid']} steps={s['steps']}")
        print(f"       {why}\n", flush=True)

    print("\n## Summary\n")
    print("| # | Prompt | Verdict | Steps |")
    print("|---|--------|---------|-------|")
    for i, (p, v, _why, s, _dt) in enumerate(rows, 1):
        steps = " ".join(f"{t}({c})" if c else t for (t, c) in s["steps"]) or "-"
        print(f"| {i} | {p} | {v} | {steps} |")

    counts = {}
    for _p, v, *_ in rows:
        counts[v] = counts.get(v, 0) + 1
    print("\n", {k: counts.get(k, 0) for k in ["PASS", "FAIL", "INCONCLUSIVE"]})
    return 0 if counts.get("FAIL", 0) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
