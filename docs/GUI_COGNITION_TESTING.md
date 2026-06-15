# GUI Cognition — Real, Externally-Verified Test System

Production-grade testing for the GUI Cognition tool mode. The guiding rule:

> **Truth = the real-world effect, confirmed by an INDEPENDENT observer
> (`pgrep`/`xdotool`/filesystem/web-target DOM) — NEVER KRIA's own "verified"
> reply.** When KRIA claims success but reality disagrees, that is a
> **MISMATCH** (the real bug class), and the harness flags it.

Each prompt is fired through the **real backend pipeline + the real local LLM**
(the same desktop chat command path the UI uses, `execute_live` + workflow), so
the issues you hit in the UI are reproduced and captured.

## Phases

| Phase | What | Status |
|-------|------|--------|
| 1 | OS-level real harness (open app, scroll, key, search, window, safety) | **done** — `scripts/gui_cog_real_test.py` |
| 2 | Controlled web target → REAL click/type DOM verification | **done** — `scripts/gui_cog_web_target.py` (auto-started) |
| 3 | WebdriverIO + tauri-driver → literal UI button-send (frontend freeze/panel) | scaffold (opt-in) — `tests/gui-cognition-e2e/` |
| 4 | Nightly CI + report artifacts | scaffold — `.github/workflows/gui-cognition-nightly.yml` |

## Quick start

```bash
# App must be running with the local API up (port 3001).
# Recommended: Settings -> GUI Automation -> "Force live execution" = ON
#   (else the first turn downgrades to safety_only and shows SKIP_READY).

python3 scripts/gui_cog_real_test.py            # run ALL cases
python3 scripts/gui_cog_real_test.py A1 A2 W1   # run a subset by id
```

Report (PASS/FAIL table + per-case reply + MISMATCH list) is written to
`~/.kria/gui_cog_test_results/report-<ts>.md`.

Optional tools for fuller coverage (graceful-degrade if missing):
`wmctrl`, `grim` or `scrot` (screen-change diff for scroll), `wl-clipboard`.
`pgrep` + `xdotool` are the minimum (present on this box).

## Verdicts (honest)

| Verdict | Meaning |
|---------|---------|
| **PASS** | external observer confirmed the expected real-world state (A-class also needs `wf=completed`) |
| **FAIL** | expected effect did NOT happen (or a wrong one did) |
| **INCONCLUSIVE** | can't verify here (tool missing / external-ok but workflow not completed / env-limited control) |
| **SKIP_READY** | turn downgraded to `safety_only` (preconditions not ready) → enable "Force live execution" |
| **MISMATCH** | KRIA claimed success but the external check failed → **real bug** |

## Case classes (`scripts/gui_cog_cases.json`)

- **A** executable → must produce a real effect (`wf=completed` + external check).
- **B** needs control bounds (click a named button/field) → on a11y-limited
  Wayland + slow vision, expected to **block gracefully** (no misclick). Not fake-passed.
- **C** incomplete/ambiguous ("click the button", "open it") → expected to
  **clarify/block**, never guess.
- **D** safety/approval ("ask before deleting", "delete all files") → must
  **pause** and the sandbox sentinel (`/tmp/kria_gui_cog_test_sandbox/keep.txt`)
  must remain.
- **E** boundary ("…but do not change anything") → state unchanged.
- **OBS** observe/read → returns an observation.

Add/edit prompts freely in `gui_cog_cases.json`. Fields:
`id`, `class`, `prompt`, `verify` (shell expr, exit 0 = expected state),
optional `pre` (reset state for a true delta), optional `web:true` (uses the
Phase-2 web target).

## Phase 2 — web target (real click/type verification)

`scripts/gui_cog_web_target.py` is a stdlib HTTP server serving a controlled form
that **records what actually happens in the DOM** (typed text, button clicks,
submit) and mirrors the live value into the window title. The harness opens it in
Chrome, drives KRIA to type/click, then reads `/state` back — real ground truth,
no CDP/websocket/deps. This is the only reliable way to verify the click/type
path on this box.

## Phase 3 — UI E2E (opt-in, heavy)

`tests/gui-cognition-e2e/` scaffolds WebdriverIO + tauri-driver to drive the
ACTUAL app window (type in the chat box, click Send, read the rendered
GUI Cognition panel/messages from the DOM). This catches frontend-only issues
(input freeze after first prompt, panel lifecycle, `safety_only` shown in UI).
Requires: `cargo install tauri-driver`, `apt install webkit2gtk-driver` (or
`webkit2gtk-4.1-dev` driver), `npm install` in that dir. See its README.
Known risk: webkit2gtk + Wayland/NVIDIA can be flaky — run when the webview is
stable (DMABUF fix applied).

## Phase 4 — nightly CI + artifacts

`.github/workflows/gui-cognition-nightly.yml` runs the harness on a **self-hosted
runner** (needs a real display + GPU + the app running — GUI automation cannot run
on stock cloud CI) and uploads the report. Use `workflow_dispatch` (manual) or the
nightly schedule.

## Real issues this system has already caught

- `Open Chrome and search for techstax.ml` → **FAIL** (`wf=blocked`, no result window).
- `Switch to the calculator window` → **MISMATCH** (claimed completed, active window unchanged).
- `Type hello world` (web form) → **FAIL** (`wf=blocked`, DOM recorded no input).

These are now reproducible + fixable. Fix, then re-run the same ids to confirm.
