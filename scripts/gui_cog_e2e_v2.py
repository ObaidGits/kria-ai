#!/usr/bin/env python3
"""A5 — V2 live eval (held-out prompts) with external compositor ground truth.

Targets the V2 pipeline (KRIA_GUI_COG_V2=1). Parses the V2 response shape
(engine/status/steps), executes for real (auto-approve + ExecuteLive in a
test_substrate), and verifies REALITY from outside KRIA via the GNOME extension
(GetFocusedWindow / ListWindows) + pgrep.

SAFETY: run the SAFE held-out set only when the app is in test_substrate (real
HOME + auto-approve would let a Red action run, so destructive prompts are NOT
in this set — the destructive gate is covered by A3 unit tests, and a
deny-probe is run separately in normal mode). Held-out = different phrasings from
the prompts used while tuning.
"""
import json
import os
import subprocess
import time
import urllib.request

API = "http://127.0.0.1:3001"
TOKEN = open(os.path.expanduser("~/.kria/api_token")).read().strip()
ETOK = open(os.path.expanduser("~/.kria/gui_ext_token")).read().strip()
ENV = dict(os.environ, DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/1000/bus")


def run_prompt(prompt: str, execute: bool) -> dict:
    body = {
        "message": prompt,
        "manual_profile": {"mode_id": "gui_cognition", "app_lock": "gui_cognition"},
    }
    if execute:
        body["gui_cognition_test"] = {"hitl_decision_fixture": "approve", "execution_mode": "execute_live"}
    req = urllib.request.Request(
        f"{API}/api/testing/desktop-chat-command",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req, timeout=320) as r:
        return json.load(r)


def v2(resp: dict) -> dict:
    g = (resp.get("response") or {}).get("gui_cognition") or {}
    steps = g.get("steps") or []
    return {
        "engine": g.get("engine"),
        "status": g.get("status"),
        "actions": [s.get("action") for s in steps],
        "executed": any(s.get("ok") for s in steps),
        "reply": (resp.get("reply") or "")[:140],
    }


def ext(method: str, *args: str):
    argv = ["gdbus", "call", "--session", "--dest", "ai.kria.ActiveWindow",
            "--object-path", "/ai/kria/ActiveWindow", "--method", f"ai.kria.ActiveWindow.{method}"] + list(args)
    try:
        s = subprocess.run(argv, capture_output=True, text=True, timeout=6, env=ENV).stdout.strip()
        if s.startswith("('") or s.startswith('("'):
            s = s[2:]
        if s.endswith("',)") or s.endswith('",)'):
            s = s[:-3]
        return json.loads(s.encode().decode("unicode_escape"))
    except Exception:
        return None


def focused() -> str:
    v = ext("GetFocusedWindow", ETOK) or {}
    return ((v.get("window") or {}).get("title") or "").lower()


def pgrep(p: str) -> int:
    try:
        return int(subprocess.run(["pgrep", "-fc", p], capture_output=True, text=True, env=ENV).stdout.strip() or "0")
    except Exception:
        return -1


# Held-out SAFE cases: (prompt, settle, kind, key, expect_substr_or_proc)
CASES = [
    ("launch the calculator app", 3.0, "pgrep", "gnome-calculator"),
    ("open the system settings", 3.0, "focus", "settings"),
    ("open the files manager", 3.0, "pgrep", "nautilus"),
    ("open google chrome then reload the page", 4.0, "focus", "chrome"),
    ("open chrome and open a new tab", 4.0, "focus", "new tab"),
    ("open chrome and close the current tab", 4.0, "focus", "chrome"),
]


def main():
    # Preflight: require V2 engine. (Substrate/auto-approve is set at launch via
    # KRIA_GUI_TEST_SUBSTRATE=1; the V2 response does not echo it, so we trust the
    # launch and detect non-execution per-case as BLOCKED rather than gating here.)
    pf = v2(run_prompt("open the calculator", execute=False))
    print(f"# A5 V2 live eval — engine={pf['engine']}\n")
    if pf["engine"] != "v2":
        print("!! Not in V2 mode (set KRIA_GUI_COG_V2=1). Aborting."); return

    rows = []
    for prompt, settle, kind, key in CASES:
        before = pgrep(key) if kind == "pgrep" else None
        print(f"[run] {prompt!r}", flush=True)
        try:
            r = v2(run_prompt(prompt, execute=True))
            time.sleep(settle)
            if kind == "pgrep":
                now = pgrep(key)
                real = now > before
                detail = f"{key} {before}->{now}"
            else:
                t = focused()
                real = key in t
                detail = f"focused='{t[:45]}'"
            verdict = "PASS" if (real and r["executed"]) else ("MISMATCH" if r["status"] == "completed" and not real else ("BLOCKED" if not r["executed"] else "FAIL"))
        except Exception as e:
            r = {"status": "error", "actions": [], "executed": False, "reply": str(e)}
            verdict, detail = "INCONCLUSIVE", str(e)
        rows.append((prompt, verdict, r["status"], r["actions"], detail))
        print(f"   status={r['status']} actions={r['actions']}")
        print(f"   reality: {detail} -> {verdict}\n", flush=True)

    print("## A5 Summary\n| Prompt | Verdict | V2 status | actions | reality |\n|--|--|--|--|--|")
    for p, v, st, acts, d in rows:
        print(f"| {p} | {v} | {st} | {'+'.join(acts)} | {d[:40]} |")
    counts = {}
    for _p, v, *_ in rows:
        counts[v] = counts.get(v, 0) + 1
    print("\n", {k: counts.get(k, 0) for k in ["PASS", "FAIL", "MISMATCH", "BLOCKED", "INCONCLUSIVE"]})


if __name__ == "__main__":
    main()
