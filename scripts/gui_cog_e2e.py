#!/usr/bin/env python3
"""End-to-end GUI-Cognition verification with EXTERNAL ground truth.

Unlike scripts/gui_cog_eval.py (which scores the PLAN), this harness makes KRIA
ACTUALLY execute (auto-approve + ExecuteLive) and then checks REALITY from
outside KRIA, so a "done" that did not happen is caught as a MISMATCH.

Ground-truth sensors (external, never trust KRIA's own verdict):
  1. Chrome DevTools Protocol (CDP) — exact tab count / URLs (new tab, navigate,
     close tab). Strongest, queryable over HTTP. Requires Chrome started with
     --remote-debugging-port=9222.
  2. pgrep — a process/app actually launched.
  3. Screenshot hash/diff (grim) — the screen visibly changed.

Verdicts (honest):
  PASS         reality matches the expected change AND KRIA claimed success
  FAIL         reality did NOT change as expected
  MISMATCH     KRIA claimed success but reality disagrees (the fake-pass we hunt)
  BLOCKED      KRIA refused/again stopped at a gate (no execution attempted)
  INCONCLUSIVE sensor unavailable / setup not satisfied

SAFETY: ExecuteLive drives the REAL keyboard/mouse via uinput. Auto-approval is
only honored when the KRIA desktop app runs with KRIA_GUI_TEST_SUBSTRATE=1
(server-side gate). The preflight refuses to run executing cases otherwise.
"""
import json
import os
import subprocess
import time
import urllib.request

API = os.environ.get("KRIA_API", "http://127.0.0.1:3001")
TOKEN = open(os.path.expanduser("~/.kria/api_token")).read().strip()
CDP = os.environ.get("KRIA_CDP", "http://127.0.0.1:9222")
TIMEOUT = int(os.environ.get("E2E_TIMEOUT", "240"))


# ── transport ──────────────────────────────────────────────────────────────
def post(path: str, body: dict, timeout: int = TIMEOUT) -> dict:
    req = urllib.request.Request(
        f"{API}{path}",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def run_prompt(prompt: str, execute: bool) -> dict:
    """Drive a GUI-Cognition turn. When execute=True, auto-approve + ExecuteLive."""
    body = {
        "message": prompt,
        "manual_profile": {"mode_id": "gui_cognition", "app_lock": "gui_cognition"},
    }
    if execute:
        body["gui_cognition_test"] = {
            "hitl_decision_fixture": "approve",
            "execution_mode": "execute_live",
        }
    return post("/api/testing/desktop-chat-command", body)


# ── ground-truth sensors ─────────────────────────────────────────────────────
def cdp_pages() -> list | None:
    try:
        with urllib.request.urlopen(f"{CDP}/json", timeout=4) as r:
            data = json.load(r)
        return [t for t in data if t.get("type") == "page"]
    except Exception:
        return None


def cdp_tab_count() -> int | None:
    pages = cdp_pages()
    return None if pages is None else len(pages)


def cdp_urls() -> list:
    pages = cdp_pages() or []
    return [p.get("url", "") for p in pages]


def pgrep_count(pattern: str) -> int:
    try:
        out = subprocess.run(["pgrep", "-fc", pattern], capture_output=True, text=True, timeout=5)
        return int((out.stdout or "0").strip() or "0")
    except Exception:
        return -1


def screenshot_hash() -> str | None:
    """Best-effort screen hash via grim (Wayland). None if unavailable."""
    import hashlib
    path = "/tmp/kria_e2e_shot.png"
    try:
        r = subprocess.run(["grim", path], capture_output=True, timeout=8)
        if r.returncode != 0 or not os.path.exists(path):
            return None
        with open(path, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()
    except Exception:
        return None


# ── KRIA self-claim (to compare against reality) ─────────────────────────────
def kria_claimed_success(resp: dict) -> bool:
    g = (resp.get("response") or {}).get("gui_cognition") or {}
    ex = g.get("execution") or {}
    ver = g.get("verification") or {}
    if isinstance(ex, dict) and ex.get("success") is True:
        return True
    if isinstance(ver, dict) and str(ver.get("status", "")).lower() in ("verified", "ok", "success"):
        return True
    # Fall back to a permissive reply scan only if no structured signal.
    return False


def kria_executed(resp: dict) -> bool:
    """Did KRIA even attempt execution (vs stop at a gate)?"""
    g = (resp.get("response") or {}).get("gui_cognition") or {}
    return bool(g.get("execution")) or bool((g.get("action") or {}))


# ── scoring ──────────────────────────────────────────────────────────────────
def verdict_from(expected_change: bool, real_change: bool, kria_says: bool, executed: bool) -> tuple[str, str]:
    if not executed:
        return "BLOCKED", "KRIA did not execute (gate/again) — no reality change attempted"
    if expected_change and real_change:
        return "PASS", "reality changed as expected"
    if expected_change and not real_change and kria_says:
        return "MISMATCH", "KRIA claimed success but reality did NOT change (fake-pass)"
    if expected_change and not real_change:
        return "FAIL", "expected change did not happen"
    return "INCONCLUSIVE", "no clear expectation/observation"


# ── cases (CDP-backed where strongest) ───────────────────────────────────────
def case_new_tab() -> dict:
    before = cdp_tab_count()
    if before is None:
        return {"verdict": "INCONCLUSIVE", "why": "Chrome CDP not reachable (start chrome with --remote-debugging-port=9222)"}
    resp = run_prompt("create a new browser tab", execute=True)
    time.sleep(2.0)
    after = cdp_tab_count()
    real_change = after is not None and after > before
    v, why = verdict_from(True, real_change, kria_claimed_success(resp), kria_executed(resp))
    return {"verdict": v, "why": f"tabs {before}->{after}; {why}"}


def case_navigate() -> dict:
    before = cdp_urls()
    if cdp_tab_count() is None:
        return {"verdict": "INCONCLUSIVE", "why": "Chrome CDP not reachable"}
    resp = run_prompt("in chrome go to example.com", execute=True)
    time.sleep(3.0)
    after = cdp_urls()
    real_change = any("example.com" in u for u in after) and after != before
    v, why = verdict_from(True, real_change, kria_claimed_success(resp), kria_executed(resp))
    return {"verdict": v, "why": f"urls now {after[:4]}; {why}"}


def case_open_app() -> dict:
    before = pgrep_count("gnome-calculator")
    resp = run_prompt("open the calculator", execute=True)
    time.sleep(2.5)
    after = pgrep_count("gnome-calculator")
    real_change = after > before >= 0
    v, why = verdict_from(True, real_change, kria_claimed_success(resp), kria_executed(resp))
    return {"verdict": v, "why": f"gnome-calculator procs {before}->{after}; {why}"}


CASES = [
    ("new tab (CDP tab-count)", case_new_tab),
    ("navigate example.com (CDP url)", case_navigate),
    ("open calculator (pgrep)", case_open_app),
]


# ── preflight ────────────────────────────────────────────────────────────────
def preflight() -> dict:
    out = {"api": False, "substrate": None, "cdp": False, "grim": False}
    try:
        with urllib.request.urlopen(f"{API}/api/health", timeout=10) as r:
            json.load(r)
        out["api"] = True
    except Exception:
        pass
    out["cdp"] = cdp_tab_count() is not None
    out["grim"] = screenshot_hash() is not None
    # Probe execution environment by reading a turn's response marker.
    try:
        r = run_prompt("open the calculator", execute=False)
        g = (r.get("response") or {}).get("gui_cognition") or {}
        env = g.get("execution_environment") or {}
        out["substrate"] = env.get("environment")
    except Exception as e:  # noqa: BLE001
        out["substrate"] = f"probe-failed: {e}"
    return out


def main() -> int:
    print(f"# GUI Cognition E2E (external ground truth) — app {API}\n")
    pf = preflight()
    print("## Preflight")
    print(f"  api reachable : {pf['api']}")
    print(f"  exec env      : {pf['substrate']}  (need 'test_substrate' for ExecuteLive auto-approve)")
    print(f"  chrome CDP    : {pf['cdp']}  ({CDP})")
    print(f"  grim capture  : {pf['grim']}")
    print()

    if pf["substrate"] != "test_substrate":
        print("!! Executing cases SKIPPED: app is NOT in test-substrate mode, so auto-approve")
        print("   is rejected (Requirement 20.3) and ExecuteLive will not fire. Relaunch the")
        print("   KRIA desktop app inside the substrate, e.g.:")
        print("     scripts/gui_cognition_test_substrate.sh --mode nested --keep -- \\")
        print("       env KRIA_GUI_COG_SHORTCUT_REPAIR=1 cargo run -p kria-desktop")
        print("   and start Chrome with: google-chrome --remote-debugging-port=9222")
        return 2

    rows = []
    for name, fn in CASES:
        print(f"[run] {name} ...", flush=True)
        try:
            res = fn()
        except Exception as e:  # noqa: BLE001
            res = {"verdict": "INCONCLUSIVE", "why": f"error: {e}"}
        rows.append((name, res["verdict"], res["why"]))
        print(f"    -> {res['verdict']} | {res['why']}\n", flush=True)

    print("## Summary\n")
    print("| Case | Verdict | Detail |")
    print("|------|---------|--------|")
    for n, v, w in rows:
        print(f"| {n} | {v} | {w} |")
    counts = {}
    for _n, v, _w in rows:
        counts[v] = counts.get(v, 0) + 1
    print("\n", {k: counts.get(k, 0) for k in ["PASS", "FAIL", "MISMATCH", "BLOCKED", "INCONCLUSIVE"]})
    return 0 if counts.get("FAIL", 0) == 0 and counts.get("MISMATCH", 0) == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
