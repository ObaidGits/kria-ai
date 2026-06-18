# KRIA Chat System — Test Plan (Phase E)

Tests are gated by severity. Each fix lands with its tests; no fix is "done" until green.

## Unit (frontend, vitest in `ui/`)
- `guiCognitionSession.ts`
  - Accept + start a session from the FIRST V2 lifecycle envelope (`TurnStarted`).
  - Each backend phase event advances lifecycle: observing → planning → safety → executing →
    verifying → completed/failed.
  - `TurnEnded{status}` sets the matching terminal lifecycle and clears the running indicator.
  - Out-of-order / stale `sequence` rejected without clobbering progress.
  - `reason` (thinking) captured + exposed; never contains raw CoT markers.
- `app.ts`
  - `agent:done` / terminal `gui_cognition:event` clears `isThinking`.
  - Optimistic `userMsg` survives a hydration/merge pass (merge-by-id, no blind replace).
  - Wrong-`turn_id`/wrong-`session_id` event is ignored (no cross-session leak).
- Existing suites must stay green: `app.gui-cognition-stream.test.ts`,
  `guiCognitionSession.test.ts`, `app.session-management.test.ts`, `app.tool-choice.test.ts`.

## Unit (backend, `cargo test -p kria-core` / `-p kria-desktop`)
- `gui_cognition_v2` loop emits the full phase sequence (start, observe, decide, gate, execute,
  verify, end) for a fake 2-step turn; `TurnEnded` emitted on EVERY exit (done/ask/cap/no-progress/
  safety/error/cancel).
- Event envelopes carry `version`,`session_id`,`turn_id`,`workflow_id`, monotonic `sequence`,
  and `reason` for decide.
- Desktop glue: `gui_cognition_v2_glue_tests` stays green; new test asserts the desktop runner
  forwards the new lifecycle events (mapping V2 phases → envelope types).

## Integration
- Full V2 turn (fakes) → assert the frontend reducer, fed the real backend envelope sequence,
  reaches `completed` and renders steps. (Contract conformance test: backend-emitted types ⊆
  frontend-accepted types.)

## UI / component
- `ChatView` renders `GuiCognitionPanel` once a turn starts; renders a live assistant bubble that
  updates per phase; finalizes on `TurnEnded`.
- Per-route `ErrorBoundary`: a view that throws shows the fallback and `navigate("home")` still
  works (no wedge).

## Live (desktop, manual + `scripts/gui_cog_e2e_v2.py`)
- "open chrome and create a new tab": panel shows observe→decide(reason)→execute(open_app)→
  execute(new_tab)→done; transcript shows live bubble; external truth (pgrep/wmctrl) PASS.
- Navigate Home→VM Management→Dashboard→Home repeatedly during/after a GUI turn: never stuck.
- Kill the backend mid-turn: `TurnEnded`/watchdog clears thinking ≤ 60s; input recovers.

## Stress / recovery
- Fire 5 GUI prompts back-to-back: queue drains one-at-a-time; no lost/duplicated turns.
- Switch session mid-turn: events do not land in the wrong session (turn_id/session_id guard).
- 200 rapid `gui_cognition:event` envelopes: coalesced render (`flushPendingEvents`), no UI freeze.

## Build gates
`cargo build --workspace` (0 warnings), `cargo test -p kria-core`, `-p kria-desktop`,
`cd ui && npm test`. CI: `kria-ci.yml`.
