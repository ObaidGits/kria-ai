#!/usr/bin/env python3
"""Live gate for a capability family that needs a focused editable/scrollable
surface (C7_key_press, C8_scroll, C6_clipboard). Opens a long-content Text
Editor, focuses it via the GNOME extension before EACH prompt, runs the family,
scores PASS/PARTIAL/FAIL + leaks via the production audit scorer.

Usage: python3 testing/tools/_focused_gate.py <CAP>   e.g. C7_key_press
"""
from __future__ import annotations
import re, subprocess, sys, time, uuid
from pathlib import Path

REPO = Path("/media/obaid/SSD/KRIA")
sys.path.insert(0, str(REPO))
from testing.tools.gui_cognition_capability_audit import (  # noqa: E402
    BASE_URL, detect_leaks, gui_of, health, judge, send, token,
)
from testing.tools.heldout_prompt_set import load_prompts  # noqa: E402

TOKEN_EXT = (Path.home() / ".kria" / "gui_ext_token").read_text().strip()
DEST = ["--session", "--dest", "ai.kria.ActiveWindow",
        "--object-path", "/ai/kria/ActiveWindow", "--method"]


def ext(method: str, *args: str) -> str:
    return subprocess.run(["gdbus", "call", *DEST, f"ai.kria.ActiveWindow.{method}", *args],
                          capture_output=True, text=True).stdout


def focus_editor() -> bool:
    lw = ext("ListWindows", TOKEN_EXT)
    m = re.findall(r'\{[^{}]*TextEditor[^{}]*\}', lw)
    if not m:
        return False
    wid = re.search(r'"id":"(\d+)"', m[0])
    if not wid:
        return False
    ext("ActivateWindow", TOKEN_EXT, wid.group(1))
    time.sleep(0.7)
    return True


def open_editor() -> None:
    f = Path("/tmp/kria_scroll_test.txt")
    f.write_text("\n".join(f"line {i}" for i in range(1, 501)))
    subprocess.Popen(["gio", "launch",
                      "/usr/share/applications/org.gnome.TextEditor.desktop", str(f)],
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(4)


def verdict(score: float) -> str:
    return "PASS" if score >= 1.0 else ("PARTIAL" if score >= 0.5 else "FAIL")


def main() -> int:
    cap = sys.argv[1] if len(sys.argv) > 1 else "C7_key_press"
    if not health(BASE_URL):
        print("FATAL: desktop /api/health not healthy", file=sys.stderr)
        return 2
    open_editor()
    if not focus_editor():
        print("FATAL: could not open/focus Text Editor", file=sys.stderr)
        return 2
    ps = load_prompts()
    fam = [(i, p) for i, p in enumerate(ps, 1) if p.cap == cap]
    tok = token()
    counts = {"PASS": 0, "PARTIAL": 0, "FAIL": 0}
    leaks_all = []
    for n, (num, p) in enumerate(fam, 1):
        focus_editor()
        sid = f"{cap}-{num:03d}-{uuid.uuid4().hex[:8]}"
        ts = time.time()
        try:
            ok, resp, err = send(p.text, sid, tok, base_url=BASE_URL, timeout=170)
        except Exception as e:  # noqa: BLE001
            ok, resp, err = False, {}, repr(e)
        dt = time.time() - ts
        g = gui_of(resp) if ok else {}
        sc = judge(p, g, environment="real_session", approved=False)
        v = verdict(sc.score)
        counts[v] += 1
        leaks = detect_leaks(p, g, approved=False, run_index=1) if ok else []
        leaks_all += leaks
        terr = "" if ok else f" transport={err}"
        print(f"[{n}/{len(fam)}] #{num:<3d} {v:7s} {dt:5.1f}s label={sc.label:22s} "
              f"verify={sc.signals.get('verify')} exec={sc.signals.get('exec_status')}"
              f"{terr}  :: {p.text[:46]}", flush=True)
    print(f"\n=== {cap} GATE DONE ===")
    print(f"PASS {counts['PASS']} | PARTIAL {counts['PARTIAL']} | FAIL {counts['FAIL']} | leaks {len(leaks_all)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
