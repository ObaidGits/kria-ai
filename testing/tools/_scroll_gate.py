#!/usr/bin/env python3
"""Phase 4 (Issue #5 scroll) live gate. Opens a long-content Text Editor, focuses
it via the GNOME extension, then runs the C8_scroll family (#36-40) against that
scrollable surface (no reset between prompts; scroll keeps focus). Scores
PASS/PARTIAL/FAIL + leaks via the production audit scorer."""
from __future__ import annotations
import subprocess, sys, time, uuid
from pathlib import Path

REPO = Path("/media/obaid/SSD/KRIA")
sys.path.insert(0, str(REPO))
from testing.tools.gui_cognition_capability_audit import (  # noqa: E402
    BASE_URL, detect_leaks, gui_of, health, judge, send, token,
)
from testing.tools.heldout_prompt_set import HeldoutPrompt, load_prompts  # noqa: E402

TOKEN_EXT = (Path.home() / ".kria" / "gui_ext_token").read_text().strip()
DEST = ["--session", "--dest", "ai.kria.ActiveWindow",
        "--object-path", "/ai/kria/ActiveWindow", "--method"]


def ext(method: str, *args: str) -> str:
    return subprocess.run(["gdbus", "call", *DEST, f"ai.kria.ActiveWindow.{method}", *args],
                          capture_output=True, text=True).stdout


def setup_scrollable() -> bool:
    f = Path("/tmp/kria_scroll_test.txt")
    f.write_text("\n".join(f"line {i}" for i in range(1, 501)))
    subprocess.Popen(["gio", "launch",
                      "/usr/share/applications/org.gnome.TextEditor.desktop", str(f)],
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(4)
    lw = ext("ListWindows", TOKEN_EXT)
    import re
    m = re.findall(r'\{[^{}]*TextEditor[^{}]*\}', lw)
    if not m:
        return False
    wid = re.search(r'"id":"(\d+)"', m[0])
    if not wid:
        return False
    ext("ActivateWindow", TOKEN_EXT, wid.group(1))
    time.sleep(1)
    return True


def verdict(score: float) -> str:
    return "PASS" if score >= 1.0 else ("PARTIAL" if score >= 0.5 else "FAIL")


def main() -> int:
    if not health(BASE_URL):
        print("FATAL: desktop /api/health not healthy", file=sys.stderr)
        return 2
    if not setup_scrollable():
        print("FATAL: could not open/focus a scrollable Text Editor", file=sys.stderr)
        return 2
    ps = load_prompts()
    scroll = [(i, p) for i, p in enumerate(ps, 1) if p.cap == "C8_scroll"]
    tok = token()
    counts = {"PASS": 0, "PARTIAL": 0, "FAIL": 0}
    leaks_all = []
    for n, (num, p) in enumerate(scroll, 1):
        # Re-focus the editor before each scroll (prior scroll kept focus, but a
        # prompt's own perception/vision must not have stolen it).
        lw = ext("ListWindows", TOKEN_EXT)
        import re
        m = re.findall(r'\{[^{}]*TextEditor[^{}]*\}', lw)
        if m:
            wid = re.search(r'"id":"(\d+)"', m[0])
            if wid:
                ext("ActivateWindow", TOKEN_EXT, wid.group(1))
                time.sleep(0.6)
        sid = f"scroll-{num:03d}-{uuid.uuid4().hex[:8]}"
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
        print(f"[{n}/{len(scroll)}] #{num:<3d} {v:7s} {dt:5.1f}s "
              f"verify={sc.signals.get('verify')} exec={sc.signals.get('exec_status')}"
              f"{terr}  :: {p.text[:48]}", flush=True)
    print(f"\n=== SCROLL GATE DONE ===")
    print(f"PASS {counts['PASS']} | PARTIAL {counts['PARTIAL']} | FAIL {counts['FAIL']} | leaks {len(leaks_all)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
