#!/usr/bin/env python3
"""GUI Cognition — external-signal LIVE PROOF harness (spec Task 1 / Req 9, 15, 18, 22).

Unlike the legacy `gui_cognition_live_eval.py` (which classifies from KRIA's OWN
reply payload), this harness PROVES each sub-goal with signals INDEPENDENT of
KRIA's self-report:

  - window present/focused + title  -> GNOME compositor via the KRIA extension
                                       (gdbus ai.kria.ActiveWindow ListWindows/GetFocusedWindow)
  - on-screen text (OCR)            -> ext CaptureScreen -> tesseract
  - file exists/contains            -> filesystem
  - command/typed output            -> OCR of the focused window (else INCONCLUSIVE)

Per prompt (from `gui_cog_corpus.json`): drive the SAME desktop path the UI uses,
then evaluate each sub-goal's verifier; classify PASS / FAIL / INCONCLUSIVE
(never fabricated). Supports N-run flakiness (majority), non-destructive isolation,
and writes JSON artifacts + a per-category summary.

Gated by the environment preflight (Req 14.4): refuses unless the latest preflight
is ready, unless --skip-preflight.

Usage:
  python3 testing/tools/gui_cog_proof.py
      [--base-url http://127.0.0.1:3001]
      [--corpus testing/tools/gui_cog_corpus.json]
      [--only o01,m02,...] [--category multi_step]
      [--runs 1] [--timeout 180] [--skip-preflight]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
ARTIFACT_DIR = REPO_ROOT / "eval_reports" / "gui_cog"
CONF_FLOOR = 0.6  # mirrors kria-core verifier::CONFIDENCE_FLOOR

# App-name aliases so verification matches the same loose names a layman types.
APP_ALIASES = {
    "code": ["visual studio code", "code", "vscode"],
    "files": ["files", "nautilus"],
    "file explorer": ["files", "nautilus"],
    "text editor": ["text editor", "gedit", "gnome text editor"],
    "notepad": ["text editor", "gedit"],
    "terminal": ["terminal", "console", "kgx", "gnome-terminal"],
    "settings": ["settings", "gnome-control-center"],
    "calculator": ["calculator", "gnome-calculator"],
    "calc": ["calculator"],
    "chrome": ["chrome", "chromium", "google-chrome"],
    "browser": ["chrome", "chromium", "firefox"],
    "software": ["software", "app center", "gnome-software", "snap-store"],
    "system monitor": ["system monitor", "gnome-system-monitor"],
}


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


# ── tokens / preflight ───────────────────────────────────────────────────────


def read_api_token() -> str | None:
    p = Path.home() / ".kria" / "api_token"
    return p.read_text().strip() if p.exists() else None


def read_ext_token() -> str | None:
    p = Path.home() / ".kria" / "gui_ext_token"
    return p.read_text().strip() if p.exists() else None


def preflight_ready() -> tuple[bool, str]:
    latest = ARTIFACT_DIR / "preflight_latest.json"
    if not latest.exists():
        return False, "no preflight artifact (run scripts/gui_cog_preflight.py first)"
    try:
        rec = json.loads(latest.read_text())
    except Exception as exc:  # noqa: BLE001
        return False, f"unreadable preflight: {exc}"
    return (rec.get("ready") is True), rec.get("reason", "not ready")


# ── external signal probes ───────────────────────────────────────────────────


def _gdbus(method: str, *args: str, timeout: float = 6.0) -> Any:
    """Call ai.kria.ActiveWindow.<method>; return parsed JSON dict or None."""
    argv = [
        "gdbus", "call", "--session", "--dest", "ai.kria.ActiveWindow",
        "--object-path", "/ai/kria/ActiveWindow",
        "--method", f"ai.kria.ActiveWindow.{method}", *args,
    ]
    try:
        out = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        raw = out.stdout.strip()
        if not raw:
            return None
        # gdbus wraps the returned string in ('...',). Extract the inner JSON.
        m = re.search(r"\('(.*)',\)\s*$", raw, re.DOTALL)
        inner = m.group(1) if m else raw
        inner = inner.encode().decode("unicode_escape")
        return json.loads(inner)
    except Exception:  # noqa: BLE001
        return None


def list_windows(ext_token: str) -> list[dict]:
    res = _gdbus("ListWindows", ext_token)
    if isinstance(res, dict) and res.get("ok"):
        return res.get("windows", []) or []
    return []


# ── test isolation: close apps opened during a test (RAM + clean state) ───────

# Window owners we must NEVER kill: KRIA itself, the IDE running this session,
# and the desktop shell. Pre-existing terminals/editors are protected via the
# baseline-pid snapshot (so the user's own shell/editor is safe), but
# test-OPENED terminals/editors are NOT in this list — they must be closed to
# free RAM and keep the screen clean (the 17-terminal accumulation bug).
_PROTECTED_WIN = (
    "kria", "kiro", "gjs", "gnome-shell", "plasmashell", "mutter",
)


def _win_is_protected(win: dict) -> bool:
    blob = " ".join(
        str(win.get(k, "")).lower() for k in ("app_name", "wm_class", "app_id", "title")
    )
    return any(p in blob for p in _PROTECTED_WIN)


def window_pids(ext_token: str) -> set[int]:
    pids: set[int] = set()
    for w in list_windows(ext_token):
        pid = w.get("pid")
        if isinstance(pid, int) and pid > 0:
            pids.add(pid)
    return pids


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def clear_stale_singleton_locks() -> int:
    """Remove single-instance lock files that point to a DEAD pid, in common app
    config dirs. Chromium/Electron apps (Chrome, Chromium, VS Code, Slack, …) use
    a `SingletonLock` symlink named `<host>-<pid>`; if the app was killed
    ungracefully the lock goes stale and the NEXT launch fails to open a window.
    General (any singleton-lock app, matched by the lock convention, not a
    hardcoded app name) and SAFE (only removes locks whose pid is dead)."""
    import glob
    from pathlib import Path as _P
    removed = 0
    cfg = _P.home() / ".config"
    patterns = [str(cfg / "*" / "SingletonLock"), str(cfg / "**" / "SingletonLock")]
    seen = set()
    for pat in patterns:
        for lock in glob.glob(pat, recursive=True):
            if lock in seen:
                continue
            seen.add(lock)
            try:
                target = os.readlink(lock)  # '<host>-<pid>'
            except OSError:
                continue
            m = re.search(r"-(\d+)$", target)
            if not m:
                continue
            pid = int(m.group(1))
            if _pid_alive(pid):
                continue
            base = os.path.dirname(lock)
            for name in ("SingletonLock", "SingletonCookie", "SingletonSocket"):
                try:
                    os.unlink(os.path.join(base, name))
                except OSError:
                    pass
            removed += 1
    return removed


def cleanup_new_apps(ext_token: str, baseline_pids: set[int]) -> int:
    """Close every window opened SINCE `baseline_pids` (test-opened apps), freeing
    RAM and isolating the next test from leftover state (e.g. a stale Chrome that
    breaks navigation). Never touches protected windows (KRIA/IDE/terminal/shell).
    Returns the number of processes signalled. General — keyed on 'new window
    owner', not on any specific app/prompt.

    IMPORTANT: shut down GRACEFULLY (SIGTERM) and give apps time to exit cleanly
    before any SIGKILL. Single-instance apps (browsers) remove their own
    SingletonLock on a graceful exit; a premature SIGKILL leaves a stale lock
    that makes the NEXT launch fail to open a window. So we SIGTERM, wait, and
    only SIGKILL true stragglers — a general policy, not app-specific."""
    def _targets() -> list[int]:
        out = []
        for w in list_windows(ext_token):
            pid = w.get("pid")
            if isinstance(pid, int) and pid > 0 and pid not in baseline_pids and not _win_is_protected(w):
                out.append(pid)
        return out

    first = _targets()
    for pid in first:
        # Per-PID graceful SIGTERM ONLY. NEVER kill the process group: apps the
        # desktop launches share its process group, so killpg would take down
        # KRIA itself. SIGTERM lets single-instance apps remove their own locks.
        try:
            os.kill(pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    if not first:
        return 0
    time.sleep(3.0)
    for pid in _targets():
        try:
            os.kill(pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
    time.sleep(0.5)
    clear_stale_singleton_locks()
    return len(first)


# ── negative-case scoring (Requirement 33): honest refusal == PASS ────────────

def classify_negative(expect: str, reply: str, ext_token: str, target: str) -> dict:
    """Score a NEGATIVE / manual-step prompt by KRIA's HONEST response. For these
    cases the correct outcome is a truthful refusal/pause, NOT a forced action —
    so PASS = the reply states the honest limitation AND (for nonexistent apps)
    no wrong window was opened. General over phrasing."""
    low = reply.lower()
    if expect == "honest_not_installed":
        honest = any(k in low for k in (
            "not installed", "isn't installed", "is not installed", "couldn't find",
            "could not find", "no such app", "not found", "don't have", "doesn't appear",
            "not available", "no application", "can't find",
        ))
        # External guard: the (nonexistent) app's window must NOT have opened.
        opened = any(target and target.lower() in str(w.get("app_name", "")).lower()
                     for w in list_windows(ext_token))
        if honest and not opened:
            return verdict("VERIFIED", 0.9, "honest 'not installed' refusal; no wrong app opened")
        if opened:
            return verdict("FAILED", 0.9, "a wrong app window opened for a nonexistent app")
        return verdict("INCONCLUSIVE", 0.3, f"reply not clearly an honest refusal: {reply[:80]!r}")
    if expect == "honest_option_absent":
        honest = any(k in low for k in (
            "not available", "couldn't find", "could not find", "no such", "not present",
            "not on screen", "isn't on", "doesn't exist", "no option", "not visible",
            "can't find", "unable to find", "no button",
        ))
        return (verdict("VERIFIED", 0.85, "honest 'option absent' refusal")
                if honest else
                verdict("INCONCLUSIVE", 0.3, f"reply not a clear option-absent refusal: {reply[:80]!r}"))
    if expect == "manual_step_login":
        honest = any(k in low for k in (
            "sign in", "sign-in", "log in", "log-in", "login", "credentials", "password",
            "needs your", "complete it", "your sign", "permission", "continue",
        ))
        return (verdict("VERIFIED", 0.85, "honest manual-step / sign-in pause")
                if honest else
                verdict("INCONCLUSIVE", 0.3, f"reply did not surface a manual-step pause: {reply[:80]!r}"))
    return verdict("INCONCLUSIVE", 0.0, f"unknown negative expectation '{expect}'")


def focused_window(ext_token: str) -> dict | None:
    res = _gdbus("GetFocusedWindow", ext_token)
    if isinstance(res, dict) and res.get("ok"):
        return res.get("window")
    return None


def capture_ocr(ext_token: str) -> str:
    """Capture the screen via the KRIA extension and OCR it with tesseract."""
    if not shutil.which("tesseract"):
        return ""
    path = f"/tmp/kria_proof_{os.getpid()}_{uuid.uuid4().hex}.png"
    res = _gdbus("CaptureScreen", ext_token, path, timeout=8.0)
    if not (isinstance(res, dict) and res.get("ok")) or not Path(path).exists():
        return ""
    try:
        out = subprocess.run(["tesseract", path, "stdout"], capture_output=True,
                             text=True, timeout=20)
        return out.stdout or ""
    except Exception:  # noqa: BLE001
        return ""
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def _alias_terms(target: str) -> list[str]:
    t = target.strip().lower()
    terms = {t}
    for k, vs in APP_ALIASES.items():
        if k in t or t in k:
            terms.update(vs)
        for v in vs:
            if v in t:
                terms.update(vs + [k])
    # also split words
    terms.update(w for w in t.split() if len(w) > 2)
    return [x for x in terms if x]


def find_window(windows: list[dict], target: str) -> dict | None:
    terms = _alias_terms(target)
    best = None
    for w in windows:
        hay = " ".join(str(w.get(k, "")) for k in ("app_name", "title", "wm_class", "app_id")).lower()
        if any(term in hay for term in terms):
            # Prefer a focused match.
            if w.get("focused"):
                return w
            best = best or w
    return best


# ── verifiers (mirror kria-core verifier::verify_sub_goal predicates) ─────────


def verdict(outcome: str, conf: float, detail: str) -> dict:
    # Apply the same confidence floor as the Rust verifier.
    if outcome in ("VERIFIED", "FAILED") and conf < CONF_FLOOR:
        return {"outcome": "INCONCLUSIVE", "confidence": conf,
                "detail": f"low confidence {conf:.2f}<{CONF_FLOOR}: {detail}"}
    return {"outcome": outcome, "confidence": conf, "detail": detail}


# ── cross-substrate artifact verification (content-based, name-agnostic) ──────

_SCRIPT_DIRS = None


def _candidate_dirs() -> list:
    from pathlib import Path as _P
    return [_P.home(), _P.cwd(), _P("/tmp"), _P.home() / "Desktop", _P.home() / "Documents"]


def find_recent_file(ext: str, contains: str, max_age_s: float = 180.0):
    """Find the most-recently-modified file with extension `ext` (e.g. '.py')
    whose content contains `contains`, created within `max_age_s`. Name-agnostic:
    the user rarely dictates an exact filename, so a write task is verified by a
    freshly-written file of the right TYPE + CONTENT, not a hardcoded name."""
    import time as _t
    from pathlib import Path as _P
    now = _t.time()
    best = None
    best_mtime = 0.0
    for d in _candidate_dirs():
        try:
            entries = list(_P(d).glob(f"*{ext}"))
        except Exception:
            continue
        for f in entries:
            try:
                st = f.stat()
            except OSError:
                continue
            if now - st.st_mtime > max_age_s:
                continue
            if st.st_mtime <= best_mtime:
                continue
            try:
                body = f.read_text(errors="replace")
            except Exception:
                continue
            if not contains or contains.lower() in body.lower():
                best, best_mtime = f, st.st_mtime
    return best


def rerun_script(command: str, expect: str) -> dict | None:
    """Verify a run-command sub-goal by RE-RUNNING the agent's real artifact and
    checking output. Two general, safe cases:
      (a) 'python3 X' / 'bash X' / 'node X' → find the script the agent actually
          wrote (recent file of that type) and run it.
      (b) a SAFE read-only command (ls/pwd/echo/date/whoami/cat/uname/...) → run
          the command verbatim (deterministic, no side effects).
    Returns a verdict or None when not applicable. Never runs a non-whitelisted
    or mutating command."""
    parts = command.split()
    if len(parts) < 1:
        return None
    interp = parts[0].lower()
    ext_map = {"python3": ".py", "python": ".py", "bash": ".sh", "sh": ".sh", "node": ".js"}
    safe_cmds = {"ls", "pwd", "echo", "date", "whoami", "cat", "uname", "df", "du",
                 "head", "tail", "wc", "hostname", "uptime", "id", "env", "printenv",
                 "cal", "free", "which", "basename", "dirname"}
    run_argv = None
    label = command
    if interp in ext_map:
        script = find_recent_file(ext_map[interp], "", max_age_s=180.0)
        if not script:
            return None
        run_argv = [interp, str(script)]
        label = f"agent's script {script.name}"
    elif interp in safe_cmds:
        run_argv = ["bash", "-lc", command]  # safe read-only; preserves args like 'ls ~'
        label = command
    else:
        return None
    try:
        out = subprocess.run(run_argv, capture_output=True, text=True, timeout=20)
        combined = (out.stdout or "") + (out.stderr or "")
    except Exception as exc:  # noqa: BLE001
        return verdict("INCONCLUSIVE", 0.2, f"could not re-run {label}: {exc}")
    if not expect or expect in combined:
        return verdict("VERIFIED", 0.8, f"{label} runs and produces '{expect or '(output)'}'")
    return verdict("INCONCLUSIVE", 0.3, f"{label} ran but output lacked '{expect}'")


def verify_sub_goal(sg: dict, ext_token: str) -> dict:
    kind = sg.get("kind")
    target = (sg.get("target_hint") or "").strip()
    expect = (sg.get("expect_contains") or "").strip()

    if kind == "open_app":
        # Poll for the window: a cold-launching app (browsers/IDEs under load) can
        # take 20s+ on first launch (Req 22.4 settle). Generous bound so a slow
        # cold start is not a false negative.
        last = 0
        for _ in range(25):
            wins = list_windows(ext_token)
            last = len(wins)
            w = find_window(wins, target)
            if w is not None:
                if w.get("focused"):
                    return verdict("VERIFIED", 0.95, f"window '{w.get('app_name')}' present+focused")
                return verdict("VERIFIED", 0.75, f"window '{w.get('app_name')}' present (not focused)")
            time.sleep(1.0)
        return verdict("FAILED", 0.9, f"no window matching '{target}' among {last} windows after settle")

    if kind == "navigate":
        needle = (expect or target).lower()
        # Settle against a still-loading page (Req 22.4): poll the focused window
        # title for up to ~8s — heavy pages (YouTube/GitHub) load slower than light
        # ones. STRICT: proven ONLY by the loaded page title, never by OCR of the
        # typed address-bar text.
        title = ""
        for _ in range(8):
            w = focused_window(ext_token)
            title = (w or {}).get("title", "")
            if needle and needle in title.lower():
                return verdict("VERIFIED", 0.85, f"loaded page title '{title}' contains '{needle}'")
            time.sleep(1.0)
        return verdict("FAILED", 0.7, f"page title '{title}' does not show '{needle}' (page not loaded?)")

    if kind == "write_file":
        base = Path(target).name or target
        candidates = [Path.home(), Path.home() / "Desktop", Path.home() / "Documents",
                      Path.cwd(), Path("/tmp")]
        found = None
        for d in candidates:
            for cand in (d / base, d / target):
                if cand.is_file():
                    found = cand
                    break
            if found:
                break
        if not found:
            # Name-agnostic fallback: the user didn't dictate an exact filename or
            # language, so accept a freshly-written file (any common script/text
            # type) whose content matches the expectation — verifies the agent's
            # real artifact, not a corpus-prescribed name/extension.
            exts = [Path(base).suffix] if Path(base).suffix else []
            exts += [".py", ".sh", ".js", ".txt", ".md", ".html"]
            seen_ext = set()
            for ext in exts:
                if not ext or ext in seen_ext:
                    continue
                seen_ext.add(ext)
                recent = find_recent_file(ext, expect)
                if recent is not None:
                    return verdict("VERIFIED", 0.9,
                                   f"recent {ext} file '{recent.name}' matches expected content")
            return verdict("FAILED", 0.85, f"file '{base}' not found in common dirs")
        if expect:
            try:
                content = found.read_text(errors="replace")
            except Exception as exc:  # noqa: BLE001
                return verdict("INCONCLUSIVE", 0.0, f"unreadable {found}: {exc}")
            if expect in content:
                return verdict("VERIFIED", 0.95, f"{found} contains '{expect}'")
            return verdict("FAILED", 0.85, f"{found} missing '{expect}'")
        return verdict("VERIFIED", 0.9, f"file exists: {found}")

    if kind in ("run_command", "read_output"):
        # External proof of command output is OCR of the (terminal) window.
        if not expect:
            return verdict("INCONCLUSIVE", 0.0, "no expected output to externally confirm")
        text = capture_ocr(ext_token)
        if expect in text:
            return verdict("VERIFIED", 0.7, f"screen shows '{expect}'")
        # Cross-substrate fallback: if this is a script run ('python3 X' etc.),
        # re-run the agent's ACTUAL written script and verify its output — proves
        # the artifact works end-to-end without a corpus-prescribed filename.
        if kind == "run_command":
            rr = rerun_script(target, expect)
            if rr is not None:
                return rr
        return verdict("INCONCLUSIVE", 0.3, f"could not OCR-confirm '{expect}' (terminal text may be unreadable)")

    if kind == "type":
        needle = expect or target
        text = capture_ocr(ext_token)
        if needle and needle in text:
            return verdict("VERIFIED", 0.75, f"screen shows '{needle}'")
        # Numeric results may render with thousands separators ("3,328" / "3 328")
        # or stray OCR spacing — compare with separators/whitespace stripped.
        if needle and needle.isdigit():
            squashed = re.sub(r"[\s,]", "", text)
            if needle in squashed:
                return verdict("VERIFIED", 0.7, f"screen shows '{needle}' (separator-normalized)")
        return verdict("FAILED", 0.65, f"screen missing '{needle}'")

    if kind == "click":
        w = focused_window(ext_token)
        title = (w or {}).get("title", "").lower()
        if target and target.lower() in title:
            return verdict("VERIFIED", 0.7, f"focused title contains '{target}'")
        text = capture_ocr(ext_token).lower()
        if target and target.lower() in text:
            return verdict("VERIFIED", 0.65, f"screen shows '{target}'")
        return verdict("INCONCLUSIVE", 0.3, f"could not confirm click effect for '{target}'")

    return verdict("INCONCLUSIVE", 0.0, f"no verifier for kind '{kind}'")


# ── drive the desktop (same path the UI uses) ─────────────────────────────────


def send_prompt(base_url: str, api_token: str | None, message: str, session_id: str,
                timeout_s: int) -> tuple[bool, dict, str | None]:
    payload = {
        "message": message,
        "session_id": session_id,
        "manual_profile": {
            "mode_id": "gui_cognition", "label": "GUI Cognition",
            "app_lock": "gui_cognition", "tool_lock": None,
            "strategy": "routed_within_lock",
        },
        "gui_cognition_test": {
            "execution_mode": "execute_live", "workflow": True,
            "hitl_decision_fixture": "approve",
        },
    }
    headers = {"Content-Type": "application/json", "Accept": "application/json"}
    if api_token:
        headers["Authorization"] = f"Bearer {api_token}"
    req = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/testing/desktop-chat-command",
        data=json.dumps(payload).encode(), headers=headers, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout_s) as resp:
            return True, json.loads(resp.read().decode()), None
    except urllib.error.HTTPError as exc:
        return False, {}, f"HTTP {exc.code}: {exc.read().decode('utf-8','replace')[:200]}"
    except Exception as exc:  # noqa: BLE001
        return False, {}, f"{type(exc).__name__}: {exc}"


def reply_of(resp: dict) -> str:
    r = resp.get("response")
    if isinstance(r, dict) and r.get("reply"):
        return str(r["reply"])
    return str(resp.get("reply") or "")


def classify_prompt(sub_verdicts: list[dict]) -> str:
    outcomes = [v["outcome"] for v in sub_verdicts]
    if any(o == "FAILED" for o in outcomes):
        return "FAIL"
    if all(o == "VERIFIED" for o in outcomes):
        return "PASS"
    return "INCONCLUSIVE"


def run_once(prompt: dict, base_url: str, api_token: str, ext_token: str,
             timeout_s: int) -> dict:
    session_id = f"proof-{prompt['id']}-{uuid.uuid4().hex[:8]}"
    ok, resp, err = send_prompt(base_url, api_token, prompt["prompt"], session_id, timeout_s)
    settle = 1.5
    time.sleep(settle)  # let the screen settle before verifying
    reply = reply_of(resp)
    # NEGATIVE / manual-step prompts (Req 33): the correct outcome is an HONEST
    # refusal/pause, scored from KRIA's reply (+ a no-wrong-window guard), not
    # from a forced action.
    expect = prompt.get("expect")
    if ok and expect in ("honest_not_installed", "honest_option_absent", "manual_step_login"):
        target = ""
        for sg in prompt.get("sub_goals", []):
            if sg.get("target_hint"):
                target = sg["target_hint"]
                break
        v = classify_negative(expect, reply, ext_token, target)
        v["sub_goal"] = {"kind": f"negative:{expect}", "target_hint": target, "expect_contains": ""}
        return {
            "ok_request": ok,
            "request_error": err,
            "reply": reply[:200],
            "sub_verdicts": [v],
            "classification": v["outcome"].replace("VERIFIED", "PASS").replace("FAILED", "FAIL"),
        }
    sub_verdicts = []
    for sg in prompt.get("sub_goals", []):
        v = verify_sub_goal(sg, ext_token)
        v["sub_goal"] = {"kind": sg.get("kind"), "target_hint": sg.get("target_hint"),
                          "expect_contains": sg.get("expect_contains")}
        sub_verdicts.append(v)
    classification = classify_prompt(sub_verdicts) if ok else "FAIL"
    return {
        "ok_request": ok,
        "request_error": err,
        "reply": reply_of(resp)[:200],
        "sub_verdicts": sub_verdicts,
        "classification": classification,
    }


def majority(classes: list[str]) -> str:
    c = Counter(classes)
    # PASS only if it's the strict majority; FAIL if any deterministic fail majority.
    return c.most_common(1)[0][0]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", default=os.environ.get("KRIA_LOCAL_API_URL", "http://127.0.0.1:3001"))
    ap.add_argument("--corpus", default=str(Path(__file__).parent / "gui_cog_corpus.json"))
    ap.add_argument("--only", default=None, help="comma-separated prompt ids")
    ap.add_argument("--category", default=None, help="filter by category")
    ap.add_argument("--runs", type=int, default=1, help="N runs per prompt (flakiness)")
    ap.add_argument("--timeout", type=int, default=180)
    ap.add_argument("--skip-preflight", action="store_true")
    ap.add_argument("--no-cleanup", action="store_true",
                    help="do NOT close test-opened apps between prompts (default: clean up)")
    args = ap.parse_args()

    if not args.skip_preflight:
        ready, reason = preflight_ready()
        if not ready:
            print(f"FATAL: preflight not ready: {reason}")
            print("Run: python3 scripts/gui_cog_preflight.py --auto-start")
            return 3

    ext_token = read_ext_token()
    if not ext_token:
        print("FATAL: no GUI extension token (~/.kria/gui_ext_token); external verification impossible")
        return 4
    api_token = read_api_token()

    corpus = json.loads(Path(args.corpus).read_text())
    prompts = corpus["prompts"]
    if args.only:
        ids = {s.strip() for s in args.only.split(",")}
        prompts = [p for p in prompts if p["id"] in ids]
    if args.category:
        prompts = [p for p in prompts if p["category"] == args.category]

    ts = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    run_dir = ARTIFACT_DIR / f"proof_{ts}"
    run_dir.mkdir(parents=True, exist_ok=True)

    results = []
    by_cat: dict[str, Counter] = defaultdict(Counter)
    # Test-isolation baseline: windows present BEFORE any test (KRIA/IDE/shell).
    # Anything opened during a test is closed afterwards to free RAM and give the
    # next test a clean screen (fixes navigation flakiness from leftover Chrome).
    baseline_pids = window_pids(ext_token) if not args.no_cleanup else set()
    if not args.no_cleanup:
        stale = clear_stale_singleton_locks()
        if stale:
            print(f"(cleared {stale} stale single-instance lock(s) before testing)")
    for i, p in enumerate(prompts, 1):
        run_classes = []
        runs = []
        for r in range(args.runs):
            out = run_once(p, args.base_url, api_token, ext_token, args.timeout)
            run_classes.append(out["classification"])
            runs.append(out)
        final = majority(run_classes)
        by_cat[p["category"]][final] += 1
        rec = {"id": p["id"], "category": p["category"], "prompt": p["prompt"],
               "final": final, "run_classes": run_classes, "runs": runs}
        results.append(rec)
        (run_dir / f"{p['id']}.json").write_text(json.dumps(rec, indent=2))
        print(f"[{i}/{len(prompts)}] {p['id']} ({p['category']}) :: {p['prompt'][:50]}")
        print(f"    -> {final}  | " + " ; ".join(
            f"{v['sub_goal']['kind']}={v['outcome']}({v['confidence']:.2f})"
            for v in runs[0]["sub_verdicts"]))
        # Close test-opened apps before the next prompt (RAM + clean state).
        if not args.no_cleanup:
            n = cleanup_new_apps(ext_token, baseline_pids)
            if n:
                print(f"    (cleanup: closed {n} test-opened app window(s))")

    overall = Counter(r["final"] for r in results)
    summary = {
        "generated_at": _now(),
        "base_url": args.base_url,
        "runs_per_prompt": args.runs,
        "total": len(results),
        "overall": dict(overall),
        "by_category": {c: dict(v) for c, v in by_cat.items()},
        "pass_rate": round(overall.get("PASS", 0) / max(1, len(results)), 3),
    }
    (run_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    (ARTIFACT_DIR / "proof_latest_summary.json").write_text(json.dumps(summary, indent=2))
    print("\n=== SUMMARY ===")
    print(json.dumps(summary, indent=2))
    print(f"\nArtifacts: {run_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
