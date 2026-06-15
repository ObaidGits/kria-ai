# Implementation Plan: GUI Cognition Production Upgrade

## Overview

Each task is "done" only when: (a) code implemented behind its named flag (default OFF),
(b) T1+T2 (and T4 where noted) pass, (c) the **live capability audit** on the **frozen held-out
set** (≥5 prompts/family, 3 runs, gate on median) hits the task's % gate through the same UI
endpoint (`execute_live`) with **zero destructive-leak**, (d) prior green suites stay green +
`git diff --check` passes, then (e) the flag is flipped ON. Any regression ⇒ revert the flag
(rollback). Baseline before work: **~28% overall**. Destructive/approval live tests run ONLY in the
TestSubstrate; non-destructive read/observe may run on the real session.

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 0, "tasks": ["0"], "parallel": false, "description": "Test foundation & safety harness: held-out audit set, 3x variance, TestSubstrate/Xvfb + scratch sandbox, baseline" },
    { "wave": 1, "tasks": ["1"], "parallel": false, "description": "Runaway control + NFR budgets + cancel/watchdog/GlobalSafetyHalt + preconditions health-gate (foundation before expanding live execution)" },
    { "wave": 2, "tasks": ["2"], "parallel": false, "description": "Planner intelligence: strict JSON validate + repair-retry, vocabulary, richer deterministic, thread data, model-capability validation" },
    { "wave": 3, "tasks": ["3", "4"], "parallel": false, "description": "Per-step re-observe (runtime) then Wayland compositor focus (executor) — serial: both integrate with runtime/executor" },
    { "wave": 4, "tasks": ["5"], "parallel": false, "description": "Plan-step completeness (planner)" },
    { "wave": 5, "tasks": ["6"], "parallel": false, "description": "Primitive coverage hardening + idempotency + password privacy (audit >= 80%)" },
    { "wave": 6, "tasks": ["7", "8"], "parallel": false, "description": "Browser targeting + read/summarize (injection-safe), then combos + clipboard-safe cross-app + fm-select" },
    { "wave": 7, "tasks": ["9"], "parallel": false, "description": "Approval/ambiguity/boundary/verify/recovery + verification contract + audit ledger" },
    { "wave": 8, "tasks": ["10"], "parallel": false, "description": "Frontend prod UX: streaming mpsc, Stop button, sequential turns, layered + E2E UI tier" },
    { "wave": 9, "tasks": ["11"], "parallel": false, "description": "Production acceptance (overall >= 90%, every family >= 80%, 0 BROKEN, 0 destructive-leak)" }
  ]
}
```

```
0 Test+safety harness ─► 1 Runaway/abort/NFR/preconditions ─► 2 Planner intelligence
        ─► 3 Re-observe ─► 4 Wayland focus ─► 5 Step completeness ─► 6 Primitive coverage
        ─► 7 Browser+read ─► 8 Combos+clipboard+fm ─► 9 Approval/verify/ledger
        ─► 10 Frontend UX + E2E ─► 11 Production acceptance
```

## Tasks

- [x] 0. Test foundation & safety harness
  - [x] 0.1 Build the **frozen held-out prompt set** (≥5 prompts/family from the True-GUI set), stored separately and never edited to make a build pass
  - [x] 0.2 Upgrade `gui_cognition_capability_audit.py`: per-family precise live assertions (executed+verified for action families via the verification contract; correct behavior for ask/boundary), 3-run median + variance band, and a **destructive-leak detector** (fail if any unrequested destructive action executes)
  - [x] 0.3 Stand up the **TestSubstrate**: nested compositor / dedicated seat / scratch user (and Xvfb path for CI), with scratch dirs/files and clipboard save-restore; gate auto-approve fixtures to substrate only
  - [x] 0.4 Confirm desktop launch/health/token/restart flow; add a CI-safe deterministic fixture tier (T2) that needs no display
  - [x] 0.5 Record the ~28% baseline (3 runs, variance band) and commit the audit doc
  - [x] 0.6 Gate: audit runs clean end-to-end on real session (non-destructive) + substrate (destructive); baseline reproduced within band
  - _Flag: n/a (test infra)_  _Requirements: 17, 20, 18_

- [x] 1. Runaway control, abort, NFR budgets, preconditions
  - [x] 1.1 Add `TurnBudget` (max_steps, watchdog, per-step resolve/verify timeouts, max_reobserve) — configurable
  - [x] 1.2 Add `CancelToken` checked before each action; cancel API + wire **GlobalSafetyHalt** into the loop
  - [x] 1.3 Abort on budget breach / flapping / repeated verification failure with a clear reason
  - [x] 1.4 Preconditions health-gate before `execute_live` (uinput/AT-SPI/focus/DISPLAY); degraded observe/plan-only with clear reason
  - [x] 1.5 T1/T2: cap/abort/cancel/halt/degraded-mode tests
  - [x] 1.6 Gate: no turn exceeds budget; cancel/halt stop before next action; Steps 1–12 green
  - _Flag: `gui_cog_runtime_guards`_  _Requirements: 19, 21, 25_

- [x] 2. Planner intelligence — valid plans for all primitives + combos (28% → ~55–60%)
  - [x] 2.1 Enforce constrained JSON decoding; **strict schema-validate**; on failure do exactly ONE repair-retry (feed back the error); never lenient-scrape prose
  - [x] 2.2 Validate/select a planner model capable of JSON-grammar output; define accepted deterministic fallback quality bar
  - [x] 2.3 Define the typed action vocabulary + plan schema (OpenApp…RequireApproval), including `idempotent` per step
  - [x] 2.4 Extend `deterministic_typed_steps` so every supported intent maps to a complete valid sequence (app/control hint + payload + `verification_strategy`)
  - [x] 2.5 Thread goal-contract data into all step types; remove any action-kind→target fallback
  - [x] 2.6 Report truthful `planner.mode/llm_status`; persistent `llm_rejected_fallback` on a healthy model = failing health signal
  - [x] 2.7 T1: each intent → valid complete plan; no "prose or non-object"; repair-retry path covered
  - [x] 2.8 T2: pipeline reaches `valid_for_resolution` for all primitives/combos
  - [x] 2.9 Live gate: held-out audit overall ≥ 55%; planner-blocked families no longer "Plan validation blocked"; Steps 1–12 green
  - _Flag: `gui_cog_smart_planner`_  _Requirements: 1, 4_

- [x] 3. Per-step re-observe for multi-step execution
  - [x] 3.1 Add a re-observe hook; desktop supplies a fresh `GuiContext` provider; bounded by Task 1 caps
  - [x] 3.2 After each state-changing step, re-observe and resolve the next target against the fresh context
  - [x] 3.3 Bounded readiness wait (window/app/page) before resolving
  - [x] 3.4 Distinguish "present after change" (continue) from "genuinely absent" (stop); eliminate false "resolved target is no longer present"
  - [x] 3.5 T1/T2: re-observe-between-steps + caps tests
  - [x] 3.6 Live gate: representative combos complete, each step verified; overall ≥ 70%; Multi-step combo family ≥ 80%
  - _Flag: `gui_cog_reobserve`_  _Requirements: 2, 6_

- [x] 4. Wayland-safe window focus / switch
  - [x] 4.1 WindowFocus abstraction; backend order GnomeBridge → Portal → UinputAltTab(verify) → X11Wmctrl(x11 only); select by session
  - [x] 4.2 Route SwitchWindow through it; report `backend_used`; activate-by-window-identity preferred
  - [x] 4.3 Verify by re-observing active window == requested; clear actionable error if no path
  - [x] 4.4 T1: backend selection; T2: SwitchWindow integration
  - [x] 4.5 Live gate: "Switch to the Chrome/terminal/file manager window" executed + verified (no "wmctrl required"); Switch-window family ≥ 80%
  - _Flag: `gui_cog_wayland_focus`_  _Requirements: 3_

- [x] 5. Plan-step completeness (payload + verification)
  - [x] 5.1 Post-process every step to ensure `verification_strategy` set per step type
  - [x] 5.2 Payload steps carry sanitized payload; if truly missing → `AskClarification`, never invalid step
  - [x] 5.3 T1: validator no longer blocks well-formed steps for missing payload/verification
  - [x] 5.4 Live gate: file-manager search / summarize-visible / copy no longer blocked; affected families ≥ 80%
  - _Flag: `gui_cog_step_completeness`_  _Requirements: 4_

- [x] 6. Primitive coverage hardening (visible single actions)
  - [x] 6.1 Verify executors for focus/type/clear/select/copy/paste/key-press/scroll/click/checkbox/dialog-close/in-app-search via the Wayland-capable backend; DPI/multi-monitor-aware bounds
  - [x] 6.2 Password-field focus never logs/echoes value (privacy test)
  - [x] 6.3 Tier-classify each primitive (GREEN/YELLOW) + set `idempotent` correctly
  - [x] 6.4 T1/T2 per primitive
  - [x] 6.5 Live gate: every primitive family ≥ 80%; overall ≥ 80%
  - _Flag: `gui_cog_primitives`_  _Requirements: 5, 15_

- [x] 7. Browser targeting + read/summarize (injection-safe)
  - [x] 7.1 Browser **chrome-UI** (address bar/tabs/back/reload/find) targetable via a11y; in scope
  - [x] 7.2 Page-content click/type scoped-out for v1 OR via a browser DOM/CDP bridge (tracked); document the decision; no OCR-only page targets
  - [x] 7.3 Read/summarize uses OCR/page text as **data only**; never influences planner/executor (injection defense); untrusted text marked
  - [x] 7.4 T2: injection prompt does not alter plan; summary references only observed content
  - [x] 7.5 Live gate: address-bar/tab/navigate + summarize-visible families ≥ 80%
  - _Flag: `gui_cog_browser`_  _Requirements: 5, 9, 26_

- [x] 8. Combos + cross-app clipboard (clipboard-safe) + file-manager select
  - [x] 8.1 Clipboard helper: save → use → restore; serialized access
  - [x] 8.2 Cross-app clipboard combo (copy in browser → switch → paste in editor) end-to-end + re-observe
  - [x] 8.3 File-manager navigate → select newest/first file → show name (non-destructive)
  - [x] 8.4 T2 integration for cross-app + fm + clipboard restore
  - [x] 8.5 Live gate: Cross-app clipboard ≥ 80%; File-manager select ≥ 80%; user clipboard restored
  - _Flag: `gui_cog_crossapp`_  _Requirements: 6, 7, 8_

- [x] 9. Approval / ambiguity / boundary / verify / recovery + verification contract + ledger
  - [x] 9.1 Verification contract per action type (predicate + evidence + bounded wait + confidence); `inconclusive` for low-confidence
  - [x] 9.2 Append-only sanitized **audit ledger** of executed actions; inspectable
  - [x] 9.3 Approval-gated actions pause → execute on approve → never on deny/expired/mismatch; auto-approve only in substrate
  - [x] 9.4 Ambiguity → ask (never guess); boundaries strictly respected; verify-and-stop terminates after verification
  - [x] 9.5 Recovery: idempotent-only single retry on focus-loss; stop+report on unexpected dialog; re-observe+explain on load failure
  - [x] 9.6 T2 for each behavior mode; secret/redaction + ledger tests
  - [x] 9.7 Live gate (substrate for destructive): Approval, Ambiguity, Boundaries, Verify-and-stop, Recovery all ≥ 80%; 0 destructive-leak
  - _Flag: `gui_cog_safety_polish`_  _Requirements: 10, 11, 12, 13, 14, 15, 22, 23_

- [x] 10. Frontend production UX + E2E
  - [x] 10.1 Stream `gui_cognition:event` envelopes via an mpsc channel from the runtime DURING the turn (observe → plan → per-step), not one end batch
  - [x] 10.2 Non-blocking dispatch (done); `thinking` always clears; sequential turns render; explicit "busy" on overlapping prompt
  - [x] 10.3 Visible **Stop/Cancel** button aborts the active turn (wires Task 1 cancel)
  - [x] 10.4 Layered output verified (layman summary + collapsible developer detail; no hashes/IDs/secrets in layman layer)
  - [x] 10.5 T1 (vitest): streaming + sequential-turn + summary + stop tests
  - [x] 10.6 T4 E2E (Playwright on isolated substrate): prompt renders, streaming progress, layered result, sequential turns, Stop aborts
  - [x] 10.7 Gate: `cd ui && npm run test:run` + E2E green; `npm run build` clean
  - _Flag: `gui_cog_stream_ux`_  _Requirements: 16, 24_

- [x] 11. Production acceptance
  - [x] 11.1 Held-out live audit (3 runs, median): overall ≥ 90%, every family ≥ 80%, 0 BROKEN, 0 destructive-leak
  - [x] 11.2 All gui_cognition core suites green; broad `desktop_command` suite green
  - [x] 11.3 UI unit + E2E suites green; `npm run build` clean
  - [x] 11.4 `git diff --check` clean; privacy/no-leak + injection tests green
  - [x] 11.5 All feature flags flipped ON and verified; rollback paths documented
  - [x] 11.6 Final report `planning_docs/gui_cognition_production_upgrade_report.md` with before/after capability matrix + ledger evidence per family
  - _Flag: all ON_  _Requirements: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26_

## Notes

- Selected-mode only; no normal-mode auto-routing; preserve Steps 1–12 behavior while flags OFF.
- Intelligence first, hardcoding last; never execute from raw prompt/OCR/LLM text/coordinates.
- Destructive/approval live tests run ONLY in the TestSubstrate with scratch files + clipboard
  save/restore; auto-approve fixtures are rejected on the real session.
- The held-out capability audit (3-run median, destructive-leak detector) is the authoritative
  acceptance gate at every stage.
- Every task is behind a named flag (default OFF); rollback = revert the flag; do-not-merge until
  green.
