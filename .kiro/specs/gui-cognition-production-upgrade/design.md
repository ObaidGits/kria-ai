# Design Document

## Overview

Upgrade KRIA's **GUI Cognition Tool Mode** from ~28% to production-grade so the full True-GUI
prompt set works end-to-end, verified with **real live tests** through the same endpoint the UI
uses — and never endangers the user's real data.

Principles:
- **Selected-mode only.** No normal-mode auto-routing. Preserve the pipeline
  `observe → goal contract → plan → validate → resolve → safety gate → (HITL) → execute → verify → (recover)`.
- **Intelligence first, hardcoding last.** Raise the planner's reasoning and thread real data.
- **Every change live-verified**, bounded, abortable, and recorded in an audit ledger.
- **Safety is structural**, not optional: destructive/approval tests run only in an isolated
  substrate; the real session is never put at risk.

Four root causes block ~70% today (planner intelligence, per-step re-observe, Wayland focus,
plan-step completeness). This design also closes the hardening gaps: strict JSON parsing,
compositor-native focus, browser-content targeting reality, clipboard safety, streaming event
channel, runaway/abort + kill-switch, test isolation, audit ledger, verification contract,
preconditions/degraded mode, and abuse resistance.

## Architecture

Pipeline (unchanged shape, hardened internals + bounded loop + abort):

```
prompt (gui_cognition mode)
  → preconditions health-gate (R25)            # uinput/AT-SPI/focus/DISPLAY
  → observe (perception)
  → goal contract                              # intent, app/control hints, payload
  → plan (LLM constrained-JSON → schema-validate → 1 repair-retry → deterministic fallback)
  → validate (readiness, completeness)
  ┌─ loop (bounded by max_steps / watchdog / cancel / GlobalSafetyHalt) ─────────────┐
  │   → resolve target (against FRESH observation)        ← Task 2                    │
  │   → safety gate + HITL (tiers; auto-approve only in substrate)                    │
  │   → execute (compositor-native focus / uinput input)  ← Task 3                    │
  │   → verify (per-action contract, bounded wait, confidence)  ← Task 4/Task 8       │
  │   → ledger.record(action)                              ← Task (audit)             │
  │   → if state-changing: re-observe; else next step                                 │
  └───────────────────────────────────────────────────────────────────────────────┘
  → events stream to UI incrementally via mpsc channel    ← Task 9
```

Backend: `crates/kria-core/src/agent/gui_cognition/*`; desktop wiring:
`crates/kria-desktop/src/commands/{chat.rs,gui_cognition.rs,local_api.rs}`. Frontend:
`ui/src/stores/guiCognitionSession.ts`, `components/GuiCognitionPanel.tsx`,
`lib/guiCognitionSummary.ts`, `stores/app.ts`, `components/ChatView.tsx`.

## Components and Interfaces

| Component | File(s) | Change |
|---|---|---|
| Preconditions | desktop `commands/gui_cognition.rs` | Health-gate before execute_live; degraded mode |
| Goal extraction | `gui_cognition/goal_contract.rs` | Extract + thread app/control/payload |
| Planner | `gui_cognition/llm_planner.rs` | Constrained JSON + **strict schema-validate + one repair-retry**; richer deterministic typed steps for ALL primitives; per-step `verification_strategy` + payload |
| Validation | `llm_planner.rs` (`validate_*`) | Stop blocking well-formed primitive steps |
| Workflow runtime | `gui_cognition/workflow_runtime.rs` | Re-observe hook; **max_steps/watchdog/cancel/halt**; idempotency-aware retry |
| Target resolve | `gui_cognition/mod.rs`, target resolver | Resolve against fresh context; DPI/multi-monitor aware bounds |
| Window focus | new focus abstraction + desktop wiring | Compositor-native activate-by-identity; Alt+Tab last resort + verify |
| Browser targeting | perception + (new) browser bridge | Chrome-UI via a11y; **page content via DOM/CDP** or scoped-out (see §Browser) |
| Clipboard | desktop clipboard helper | Save → use → restore; serialized access |
| Safety + HITL | `gui_cognition/safety_hitl.rs` | Classify new primitives; reject auto-approve outside substrate |
| Verify / Recover | `gui_cognition/verifier.rs`, `recovery.rs` | Per-action contract; idempotency-aware single retry |
| Abort / Halt | runtime + desktop + GlobalSafetyHalt | Cancel token; halt wiring; UI stop |
| Audit ledger | new `gui_cognition/audit_ledger` (or reuse safety/audit) | Append-only sanitized action record |
| Event streaming | runtime → desktop via mpsc | Emit envelopes during the turn |
| Desktop entry | `commands/chat.rs`, `commands/gui_cognition.rs` | Non-blocking spawn (done); incremental emit; cancel API |
| Frontend | `guiCognitionSession.ts`, `GuiCognitionPanel.tsx`, `guiCognitionSummary.ts`, `ChatView.tsx` | Streaming render; layered output (done); Stop button; sequential-turn rendering |
| Live harness | `testing/tools/gui_cognition_capability_audit.py`, harness driver | Held-out set; 3× runs; substrate; per-action verification |

Key interfaces:
- **Planner action vocabulary** (typed): `OpenApp, SwitchWindow, FocusField, TypeText, ClearField,
  SelectAll, Copy, Paste, PressKey, Scroll, ClickControl, SetCheckbox, CloseDialog, InAppSearch,
  WaitForState, VerifyState, SummarizeVisibleContent, AskClarification, RequireApproval`.
- **Re-observe hook**: runtime requests a fresh `GuiContext` from a desktop-supplied provider after
  each state-changing step; bounded by caps.
- **WindowFocus trait**: `focus_window(identity) -> Result { backend_used }`; backends ordered
  `GnomeBridge → Portal → UinputAltTab(verify) → X11Wmctrl(x11 only)`.
- **CancelToken / Watchdog**: cooperative cancel checked before each action; watchdog aborts on
  budget breach.
- **Event channel**: `mpsc::Sender<GuiCognitionEvent>` passed into the runtime so the desktop layer
  forwards envelopes live.
- **AuditLedger**: `record(ActionRecord)` append-only, sanitized.

## Data Models

- **GuiTypedPlanStep** (extended use): `step_type, target_app_hint, target_window_hint,
  target_control_hint, text_payload_summary, text_payload_hash, expected_precondition,
  expected_postcondition, verification_strategy, risk_level, requires_approval, idempotent: bool`.
  Every emitted step MUST have a non-empty `verification_strategy`; payload steps MUST have a
  payload or be `AskClarification`.
- **GuiExecutionMode**: `SafetyOnly | ExecuteFixture | ExecuteLive` (interactive default
  ExecuteLive; programmatic/test default SafetyOnly).
- **ExecutionEnvironment** (new): `RealSession | TestSubstrate { scratch_dir, restore_clipboard }`.
  Auto-approve + destructive live tests require `TestSubstrate`.
- **WindowFocusBackend**: `{ GnomeBridge | Portal | UinputAltTab | X11Wmctrl }` + `backend_used`.
- **TurnBudget**: `{ max_steps, turn_watchdog_ms, step_resolve_ms, step_verify_ms, max_reobserve }`.
- **CancelState**: `{ requested: bool, reason }`.
- **ActionRecord** (ledger): `{ ts, action_kind, target_label(sanitized), result, verification,
  backend_used, step_index }` — no secrets/raw payloads.
- **VerificationOutcome**: `verified | inconclusive | failed` + `confidence` + evidence source.
- **Capability audit record**: `{ capability, prompt, kind(action|ask|boundary), score 0..1, label,
  signals{...}, run_index }` over the held-out set.
- **Event envelope** (`gui_cognition:event`): versioned, ordered, sanitized; emitted incrementally.

## Browser-content targeting (resolving the a11y/OCR contradiction)

Real web page elements are frequently **not** in the a11y tree, and OCR-only targets are blocked
by policy. Design decision for v1:
- **Browser chrome** (address bar, tabs, reload/back, in-page find bar) IS targetable via a11y and
  is in scope (covers "type kria.ai in address bar", "press Enter", "open new tab").
- **Page content** (links/buttons inside the rendered page) is OUT of scope for click/type in v1
  **unless** a browser DOM/CDP bridge is added. "Open the first result" / clicking page links is
  explicitly deferred or routed through a CDP path (tracked task).
- **Read/summarize** of page content uses OCR/page text as **data only** (Requirement 9.2) — never
  as a target authority. This removes the contradiction: we summarize visible page text but do not
  click OCR-only page elements.

## Correctness Properties

### Property 1: No action-kind leakage
The executor never receives the action type as a target name.
**Validates: Requirements 1.4, 5.1**

### Property 2: Fresh-context resolution
For any step N>1 after a state-changing step, target resolution uses an observation captured after
step N-1.
**Validates: Requirements 2.1, 2.2, 6.1**

### Property 3: Plan completeness
Every executable step has a payload (when required) and a `verification_strategy`; otherwise it is
`AskClarification`, never an invalid step.
**Validates: Requirements 1.1, 4.1, 4.2**

### Property 4: Safety monotonicity
RED/BLACK actions never execute without an approved, matching HITL decision; deny ⇒ no execution;
auto-approve only in TestSubstrate.
**Validates: Requirements 10.1, 10.3, 10.4, 15.1**

### Property 5: No-guess
Ambiguous target ⇒ pause+ask; never execute.
**Validates: Requirements 11.1, 11.2**

### Property 6: Boundary respect
A prompt boundary ⇒ no destructive/state-changing action.
**Validates: Requirements 12.1, 12.2**

### Property 7: Privacy and injection resistance
No raw prompt/OCR/screenshot/clipboard/secret in events, logs, or UI; password value never logged;
OCR/page text never influences planner/executor decisions.
**Validates: Requirements 5.10, 9.2, 15.3, 26.2**

### Property 8: Verification truth
`ActionCompleted` = backend success; only `verification = verified` (above confidence threshold)
means the expected state was confirmed; ambiguous evidence ⇒ `inconclusive`.
**Validates: Requirements 2.3, 13.1, 23.2, 23.3**

### Property 9: Boundedness and abort
Every turn terminates within budget; cancel/GlobalSafetyHalt halts before the next action; no
unbounded re-observe loop.
**Validates: Requirements 19.2, 21.1, 21.2, 21.3**

### Property 10: Idempotent-only retry
Auto-retry occurs only for idempotent actions; non-idempotent actions are never silently repeated.
**Validates: Requirements 14.1, 14.2**

### Property 11: Data-loss safety
Destructive/approval live tests run only in TestSubstrate; the user clipboard is saved/restored.
**Validates: Requirements 7.2, 10.4, 20.1, 20.2, 20.3**

## Error Handling

- **Planner failure/timeout:** schema-validate → one repair-retry → deterministic fallback
  (complete); truthful `llm_status`; never emit an invalid plan; never lenient-scrape prose.
- **Target absent after re-observe:** stop safely with plain reason; no blind retry.
- **Focus backend unavailable:** clear actionable error (not generic "backend failed").
- **Verification failed/inconclusive:** one safe retry only if idempotent; else stop and report.
- **Unexpected dialog:** stop and report what is visible.
- **HITL denied/expired/mismatch:** no execution; record non-authorizing decision.
- **Budget breach / cancel / halt:** safe abort with reason; ledger records the abort.
- **Precondition missing:** degrade to observe/plan-only with a clear reason.
- **Clipboard race:** serialize; on restore failure, report (never silently drop user clipboard).

## Concurrency

- A single GUI Cognition turn runs at a time per session; the command dispatches non-blocking and
  the UI shows a busy state.
- A new prompt while a turn is active is rejected with a user-visible "busy" message (no silent
  drop), or optionally queued — behavior is explicit and tested (Requirement 16.3).
- The runtime checks the CancelToken before each action; cancellation is cooperative and prompt.

## Testing Strategy

Four tiers; a stage passes only when its live gate (median of 3 runs on the held-out set) is met,
prior green suites stay green, and zero destructive-leak occurred.

- **T1 Unit (Rust):** planner validity/completeness/repair-retry, validator, executor request
  building, re-observe loop + caps, focus backend selection, idempotency classification,
  ledger record. `cargo test -p kria-core ...`.
- **T2 Integration (in-process, deterministic fixtures):** full pipeline per capability;
  safety/HITL/boundary/ambiguity/verify/recovery; abort/cancel/halt; clipboard save/restore.
  `cargo test -p kria-core --test gui_cognition_*`. This tier is CI-safe (no display).
- **T3 Live same-path (authoritative gate):** real prompts via
  `POST /api/testing/desktop-chat-command`, `mode_id=gui_cognition`, `execute_live`, asserting on
  `response.gui_cognition.*`. Non-destructive read/observe MAY run on the real session;
  destructive/approval prompts run ONLY in the **TestSubstrate** (Xvfb/dedicated seat/scratch user)
  with scratch files and saved-restored clipboard. Scored by the held-out capability audit, 3×.
- **T4 E2E UI (Playwright):** rendered webview (or mocked bridge) — prompt renders, streaming
  progress, layered result, sequential turns, Stop aborts. Runs in CI on the isolated substrate.

**Verification contract per action type** (Requirement 23) defines the predicate + evidence +
bounded wait + confidence used by T3 to confirm the *real* outcome (not assumptions).

**Regression gates each stage:** `cd ui && npm run test:run`; gui_cognition core suites (planner,
safety_hitl, backend_route, workflow_runtime, verifier, recovery, target_resolver,
checkpoint_resume); broad `desktop_command` suite; E2E suite; `git diff --check`.

**Flake control:** each live gate is the median of 3 runs with a recorded variance band; a family
that swings across the gate boundary is treated as not-yet-passing.

## Rollout, flags, and rollback

- Each task lands behind an explicit feature flag (named in tasks.md), default OFF, flipped ON only
  after its live gate passes. Steps 1–12 behavior is preserved while flags are OFF.
- Per-stage rollback = revert the flag; "do-not-merge until green" enforced.
- Production exit: held-out audit overall ≥ 90%, every family ≥ 80%, 0 BROKEN, 0 destructive-leak;
  all tiers + Steps 1–12 + desktop_command + UI + E2E suites green; `git diff --check` clean;
  final report with before/after matrix + ledger evidence per family.
