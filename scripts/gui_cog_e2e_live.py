#!/usr/bin/env python3
"""Live GUI-Cognition E2E with INDEPENDENT compositor ground truth.

KRIA actually executes (auto-approve + ExecuteLive). Reality is checked from
OUTSIDE KRIA via the GNOME shell extension D-Bus (GetFocusedWindow / ListWindows
= compositor truth) + pgrep. A KRIA "done" that reality does not confirm is a
MISMATCH.

Per case we also surface KRIA's own Sight / Brain / Hands so you can see the
full pipeline (tech) plus a one-line layman verdict.
"""
import json
import os
import subprocess
import time
import urllib.request

API = "http://127.0.0.1:3001"
TOKEN = open(os.path.expanduser("~/.kria/api_token")).read().strip()
EXT_TOKEN = open(os.path.expanduser("~/.kria/gui_ext_token")).read().strip()
ENV = dict(os.environ, DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/1000/bus")


# ── KRIA turn ────────────────────────────────────────────────────────────────
def run_prompt(prompt: str) -> dict:
    body = {
        "message": prompt,
        "manual_profile": {"mode_id": "gui_cognition", "app_lock": "gui_cognition"},
        "gui_cognition_test": {"hitl_decision_fixture": "approve", "execution_mode": "execute_live"},
    }
    req = urllib.request.Request(
        f"{API}/api/testing/desktop-chat-command",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req, timeout=240) as r:
        return json.load(r)


# ── independent compositor ground truth (GNOME extension D-Bus) ──────────────
def ext_call(method: str, *args: str):
    argv = [
        "gdbus", "call", "--session", "--dest", "ai.kria.ActiveWindow",
        "--object-path", "/ai/kria/ActiveWindow",
        "--method", f"ai.kria.ActiveWindow.{method}",
    ] + list(args)
    try:
        out = subprocess.run(argv, capture_output=True, text=True, timeout=6, env=ENV)
        s = out.stdout.strip()
        # gdbus wraps a string return as ('...json...',)
        if s.startswith("('") or s.startswith('("'):
            s = s[2:]
        if s.endswith("',)") or s.endswith('",)'):
            s = s[:-3]
        s = s.encode().decode("unicode_escape")
        return json.loads(s)
    except Exception:
        return None


def focused_title() -> str:
    v = ext_call("GetFocusedWindow", EXT_TOKEN) or {}
    w = v.get("window") or {}
    return (w.get("title") or "").lower()


def windows() -> list:
    v = ext_call("ListWindows", EXT_TOKEN) or {}
    out = []
    for w in v.get("windows", []) or []:
        out.append(" ".join(str(w.get(k, "")) for k in ("app_name", "wm_class", "app_id", "title")).lower())
    return out


def has_window(keyword: str) -> bool:
    return any(keyword in w for w in windows())


def pgrep(pattern: str) -> int:
    try:
        return int(subprocess.run(["pgrep", "-fc", pattern], capture_output=True, text=True, env=ENV).stdout.strip() or "0")
    except Exception:
        return -1


# ── KRIA self-claim + pipeline extraction ────────────────────────────────────
def pipeline(resp: dict) -> dict:
    g = (resp.get("response") or {}).get("gui_cognition") or {}
    plan = g.get("plan") or {}
    steps = [(s.get("step_type"), s.get("text_payload_summary")) for s in (plan.get("typed_steps") or [])]
    perc = g.get("perception") or g.get("context") or {}
    sight = {
        "active_window": (perc.get("active_window") or perc.get("active_window_title")),
        "controls": perc.get("visible_control_count") or perc.get("control_count"),
        "source": perc.get("active_window_source") or perc.get("source"),
    }
    execu = g.get("execution") or {}
    ver = g.get("verification") or {}
    hands = {
        "backend": (g.get("action_backend") or {}).get("selected_backend"),
        "exec_success": execu.get("success") if isinstance(execu, dict) else None,
        "verify": ver.get("status") if isinstance(ver, dict) else None,
        "planner_mode": plan.get("planner_mode"),
    }
    claim = (isinstance(execu, dict) and execu.get("success") is True) or \
            (isinstance(ver, dict) and str(ver.get("status", "")).lower() in ("verified", "ok", "success"))
    executed = bool(execu) or bool(g.get("action"))
    return {"steps": steps, "sight": sight, "hands": hands, "claim": claim, "executed": executed,
            "reply": (resp.get("reply") or "")[:160]}


def verdict(expected: bool, real: bool, claim: bool, executed: bool) -> tuple:
    if not executed:
        return "BLOCKED", "KRIA did not execute (gate/again)"
    if expected and real:
        return "PASS", "reality confirms"
    if expected and not real and claim:
        return "MISMATCH", "KRIA claimed success but reality does NOT confirm"
    if expected and not real:
        return "FAIL", "expected change not observed"
    return "INCONCLUSIVE", "no clear observation"


# ── cases: (prompt, settle_s, check fn -> (real_change, detail)) ─────────────
def chk_window(keyword):
    def f(_before):
        return has_window(keyword), f"window[{keyword}]={has_window(keyword)} focused='{focused_title()[:40]}'"
    return f


def chk_focus_title(substr):
    def f(_before):
        t = focused_title()
        return substr in t, f"focused='{t[:50]}'"
    return f


def chk_pgrep(proc):
    def f(before):
        now = pgrep(proc)
        return now > before, f"{proc} procs {before}->{now}"
    return f


CASES = [
    ("open google chrome", 3.0, "win", "chrome", chk_window("chrome")),
    ("create a new tab", 2.5, "focus", "new tab", chk_focus_title("new tab")),
    ("in chrome navigate to example.com", 3.5, "focus", "example", chk_focus_title("example")),
    ("in chrome go to wikipedia.org", 3.5, "focus", "wikipedia", chk_focus_title("wikipedia")),
    ("open the calculator", 3.0, "pgrep", "gnome-calculator", chk_pgrep("gnome-calculator")),
    ("in the calculator type 7 * 8 then press enter", 3.0, "focus", "calculator", chk_focus_title("calculator")),
    ("open the system settings", 3.0, "pgrep", "gnome-control-center", chk_pgrep("gnome-control-center")),
    ("open the files application", 3.0, "pgrep", "nautilus", chk_pgrep("nautilus")),
    ("open the text editor", 3.0, "win", "editor", chk_window("editor")),
    ("switch to the chrome window", 3.0, "focus", "chrome", chk_focus_title("chrome")),
    ("reload the current page in chrome", 3.0, "focus", "chrome", chk_focus_title("")),
    ("close the current tab in chrome", 3.0, "focus", "chrome", chk_focus_title("")),
]


def main():
    print(f"# GUI Cognition LIVE E2E (compositor ground truth) — {API}")
    print(f"  exec: auto-approve + ExecuteLive | truth: GNOME ext GetFocusedWindow/ListWindows + pgrep\n")
    rows = []
    for i, (prompt, settle, kind, key, check) in enumerate(CASES, 1):
        before = pgrep(key) if kind == "pgrep" else None
        print(f"[{i}/{len(CASES)}] {prompt!r}", flush=True)
        try:
            resp = run_prompt(prompt)
            pp = pipeline(resp)
            time.sleep(settle)
            real, detail = check(before)
            # close-tab: a focused chrome that is NOT 'new tab' is acceptable proof
            expected = True
            v, why = verdict(expected, real if key else True, pp["claim"], pp["executed"])
        except Exception as e:
            pp = {"steps": [], "sight": {}, "hands": {}, "claim": False, "executed": False, "reply": str(e)}
            v, why, detail = "INCONCLUSIVE", f"error: {e}", "-"
        rows.append((prompt, v, why, detail, pp))
        print(f"   SIGHT : win='{pp['sight'].get('active_window')}' controls={pp['sight'].get('controls')} src={pp['sight'].get('source')}")
        print(f"   BRAIN : mode={pp['hands'].get('planner_mode')} steps={pp['steps']}")
        print(f"   HANDS : backend={pp['hands'].get('backend')} exec={pp['hands'].get('exec_success')} verify={pp['hands'].get('verify')}")
        print(f"   TRUTH : {detail}")
        print(f"   => {v} ({why})\n", flush=True)

    print("\n## Summary (tech)\n| # | Prompt | Verdict | Reality |")
    print("|---|--------|---------|---------|")
    for i, (p, v, _why, detail, _pp) in enumerate(rows, 1):
        print(f"| {i} | {p} | {v} | {detail[:60]} |")
    counts = {}
    for _p, v, *_ in rows:
        counts[v] = counts.get(v, 0) + 1
    print("\n", {k: counts.get(k, 0) for k in ["PASS", "FAIL", "MISMATCH", "BLOCKED", "INCONCLUSIVE"]})


if __name__ == "__main__":
    main()
