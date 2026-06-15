#!/usr/bin/env python3
"""Scoped live gate runner (temp). Runs a named phase subset of the user prompt
list through the running desktop and scores PASS/PARTIAL/FAIL + leaks.

Usage: python3 testing/tools/_phase_live.py <phase>
"""
from __future__ import annotations

import sys
import time
import uuid
from pathlib import Path

REPO = Path("/media/obaid/SSD/KRIA")
sys.path.insert(0, str(REPO))

from testing.tools.gui_cognition_capability_audit import (  # noqa: E402
    BASE_URL, detect_leaks, gui_of, health, judge, send, token,
)
from testing.tools.heldout_prompt_set import HeldoutPrompt  # noqa: E402

PHASES: dict[str, list[tuple[int, str, str, str]]] = {
    # Phase 1 (Issue #2 verification predicate): open-app + app-launch-then-act
    "1": [
        (1, "C1_open_app", "action", "Open Chrome"),
        (2, "C1_open_app", "action", "Open Firefox"),
        (3, "C1_open_app", "action", "Open the file manager"),
        (4, "C1_open_app", "action", "Open the terminal"),
        (5, "C1_open_app", "action", "Open the text editor"),
        (6, "C1_open_app", "action", "Open the calculator"),
        (41, "C13_multistep", "action", "Open Chrome and search for weather"),
        (42, "C13_multistep", "action", "Open Chrome and search for KRIA"),
        (46, "C13_multistep", "action", "Open file manager and select the Downloads folder"),
        (47, "C13_multistep", "action", "Open file manager and select the Documents folder"),
        (70, "C13_multistep", "action", "Open terminal, clear the screen"),
        (80, "C13_multistep", "action", "Open the text editor, create a short shopping list, and save it"),
        (62, "C16_read_visible", "action", "Open Chrome, search for weather, and wait for results"),
        (63, "C16_read_visible", "action", "Open Chrome, search for KRIA, and summarize the visible results"),
        (64, "C16_read_visible", "action", "Open Chrome, search for Python documentation, and open the first result"),
        (65, "C16_read_visible", "action", "Open Chrome, search for today's weather, and read the visible temperature"),
        (73, "C15_fm_select", "action", "Open file manager, go to Downloads, select the newest file, and show its name"),
        (74, "C15_fm_select", "action", "Open file manager, go to Documents, select the first PDF, and show its name"),
        (78, "C16_read_visible", "action", "Open Chrome, search for nearby restaurants, and summarize the visible results"),
        (79, "C16_read_visible", "action", "Open Chrome, search for a news topic, and summarize only what is visible"),
        (97, "C20_verify_stop", "action", "Open Chrome and search for weather, then stop after results are visible"),
        (98, "C20_verify_stop", "action", "Open Chrome and search for KRIA, then stop before opening any result"),
    ],
}


# Phase 2 (Issue #3 auto-prerequisite for bare primitives): built by 1-based
# index from the frozen set (105 prompts). #110 in the spec does not exist in the
# frozen set and is dropped. Context-free primitives (key-press / scroll) legitimately
# route to AskClarification when no app is inferable — scored as ASK, not FAIL.
def _build_phase2() -> list[tuple[int, str, str, str]]:
    from testing.tools.heldout_prompt_set import load_prompts  # noqa: E402
    ps = load_prompts()
    idx = (list(range(12, 26)) + list(range(32, 41))
           + [43, 44, 52, 53, 59, 60, 61, 67, 68, 69])
    out = []
    for i in idx:
        if i - 1 < len(ps):
            p = ps[i - 1]
            out.append((i, p.cap, p.kind, p.text))
    return out


PHASES["2"] = _build_phase2()


def _build_phase4() -> list[tuple[int, str, str, str]]:
    """Phase 4 (Issue #5 scroll): the C8_scroll family (#36-40)."""
    from testing.tools.heldout_prompt_set import load_prompts  # noqa: E402
    ps = load_prompts()
    out = []
    for i in range(36, 41):
        if i - 1 < len(ps):
            p = ps[i - 1]
            out.append((i, p.cap, p.kind, p.text))
    return out


PHASES["4"] = _build_phase4()


def _build_phase3() -> list[tuple[int, str, str, str]]:
    """Phase 3 (Issue #1 window activation): the C2_switch_window family."""
    from testing.tools.heldout_prompt_set import load_prompts  # noqa: E402
    ps = load_prompts()
    out = []
    for i, p in enumerate(ps, 1):
        if p.cap == "C2_switch_window":
            out.append((i, p.cap, p.kind, p.text))
    return out


PHASES["3"] = _build_phase3()


def verdict(score: float) -> str:
    return "PASS" if score >= 1.0 else ("PARTIAL" if score >= 0.5 else "FAIL")


def reset_apps() -> None:
    """Close launcher-spawned GUI apps so each prompt starts from a clean desktop
    (each True-Test prompt is independent; this prevents a prior prompt's app from
    being 'already open' and confounding the per-prompt verdict). Never touches the
    IDE/test host."""
    import subprocess
    for pat in (
        "gnome-calculator", "gnome-text-editor", "gedit",
        "nautilus", "gnome-terminal", "org.gnome.Console", "kgx",
    ):
        subprocess.run(["pkill", "-f", pat], capture_output=True)
    # NOTE: deliberately do NOT pkill chrome/chromium/firefox — the IDE/test host
    # is Electron (chromium-based) and a broad match would kill this session.
    time.sleep(1.5)


def main() -> int:
    phase = sys.argv[1] if len(sys.argv) > 1 else "1"
    prompts = PHASES[phase]
    if not health(BASE_URL):
        print("FATAL: desktop /api/health not healthy", file=sys.stderr)
        return 2
    tok = token()
    counts = {"PASS": 0, "PARTIAL": 0, "FAIL": 0}
    leaks_all = []
    fam: dict[str, list[float]] = {}
    t0 = time.time()
    n = len(prompts)
    for i, (num, cap, kind, text) in enumerate(prompts, 1):
        if phase != "3":
            reset_apps()  # phase 3 (switch) needs the target apps to stay open
        p = HeldoutPrompt(cap=cap, name=cap, text=text, kind=kind)
        sid = f"phase{phase}-{num:03d}-{uuid.uuid4().hex[:8]}"
        ts = time.time()
        try:
            ok, resp, err = send(text, sid, tok, base_url=BASE_URL, timeout=170)
        except Exception as e:  # noqa: BLE001
            ok, resp, err = False, {}, repr(e)
        dt = time.time() - ts
        g = gui_of(resp) if ok else {}
        sc = judge(p, g, environment="real_session", approved=False)
        v = verdict(sc.score)
        counts[v] += 1
        fam.setdefault(cap, []).append(sc.score)
        leaks = detect_leaks(p, g, approved=False, run_index=1) if ok else []
        leaks_all += leaks
        mark = "  !!LEAK!!" if leaks else ""
        terr = "" if ok else f" transport={err}"
        rung = (g.get("planner") or {}).get("ladder_rung") if ok else None
        print(f"[{i:2d}/{n}] #{num:<3d} {v:7s} {cap:16s} {dt:5.1f}s label={sc.label:22s} "
              f"verify={sc.signals.get('verify')} exec={sc.signals.get('exec_status')} "
              f"rung={rung}{mark}{terr}  :: {text[:52]}", flush=True)
    el = time.time() - t0
    print(f"\n=== PHASE {phase} DONE {el/60:.1f}min ===")
    print(f"PASS {counts['PASS']} | PARTIAL {counts['PARTIAL']} | FAIL {counts['FAIL']} | leaks {len(leaks_all)}")
    for cap, scs in fam.items():
        pct = 100.0 * sum(scs) / len(scs)
        print(f"  {cap:16s} {len(scs):2d} prompts  {pct:5.0f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
