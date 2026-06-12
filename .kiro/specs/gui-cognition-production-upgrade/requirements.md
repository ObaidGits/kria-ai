# Requirements Document

## Introduction

KRIA's **GUI Cognition Tool Mode** lets a user issue a natural-language prompt and have KRIA
perform the equivalent **visible GUI action** on screen (open/switch/focus/type/click/scroll/
key-press/copy-paste/in-app-search), one verified step at a time, with safety gating.

A live capability audit (39 prompts, 21 capability families, run through the *same* backend the
UI uses — `execute_live` + workflow) measured **~28% production readiness**. Only single-app
open, settings/calculator combos, and safety/boundary behaviors are reliable. Most primitives
(focus, type-in-focused, clear/select, copy/paste, key-press, scroll, click, checkbox, dialog)
and all multi-step combos fail.

Four root causes block ~70% of capabilities:

1. **Planner intelligence (largest).** The LLM planner is rejected on every prompt
   (`llm_rejected_fallback`, "returned prose or non-object content"), so a thin deterministic
   template planner runs that only handles a few intents.
2. **Per-step re-observe (Blocker #4).** After a state-changing step the next step resolves
   against the stale pre-action screen → "the resolved target is no longer present".
3. **SwitchWindow backend on Wayland.** Window focus uses `wmctrl`, unavailable on Wayland.
4. **Plan-step completeness.** Steps emitted without payload or verification strategy.

This document also incorporates a hardening review (test-isolation safety, runaway/abort control,
audit ledger, browser-content targeting reality, strict planner parsing, clipboard safety,
streaming architecture, concurrency, preconditions, security/abuse) so the spec is production-grade.

**Goal:** Upgrade the GUI Cognition Tool (backend + frontend) to **production grade** so the full
True-GUI prompt set works end-to-end, every capability is verified with **real live tests**, and
no existing suite regresses — without ever endangering the user's real data.

### Definition of production grade (measurable, hard gates)

- Live capability audit on a **frozen held-out prompt set** (≥ 5 prompts/family, run 3×, gate on
  median): **overall ≥ 90%**, **every family ≥ 80%**, and **0 families in BROKEN (<40%)**.
- **Zero destructive-leak**: across the full audit, no unintended destructive action
  (delete/move/rename/submit/install/setting-change) executes without an explicit + approved
  request. Any single occurrence is an automatic fail.
- All Steps 1–12 same-path gui_cognition suites green; broad `desktop_command` suite green; core +
  UI suites green; `git diff --check` clean.
- No raw prompt/OCR/screenshot/clipboard/secret leakage in any event, log, or UI surface.
- Safety gate + HITL still gate every risky action; no normal-mode auto-routing introduced.
- NFR budgets (Requirement 19) met; runaway/abort controls (Requirement 21) verified; audit ledger
  (Requirement 22) present.

## Glossary

| Term | Definition |
|------|-----------|
| **Primitive** | A single visible GUI action: open, switch, focus, type, clear, select, copy, paste, key-press, scroll, click, checkbox, dialog-close, in-app-search. |
| **Combo** | A multi-step prompt composed of several primitives in one turn. |
| **Plan** | Typed, validated sequence of steps produced by the planner. |
| **Re-observe** | Capturing a fresh screen observation between steps before resolving the next target. |
| **Target resolution** | Mapping a plan step to a concrete on-screen control/window/app. |
| **Safety tier** | GREEN (observe/focus/scroll), YELLOW (type/click), RED/BLACK (submit/delete/install/setting-change) → approval required. |
| **HITL** | Human-in-the-loop approval gate. |
| **Live test** | A test issued through `POST /api/testing/desktop-chat-command` (the same path the UI uses), `mode_id = gui_cognition`, `execution_mode = execute_live`, asserting on `response.gui_cognition.*`. |
| **Capability audit** | The repeatable live scorer mapping prompts → capability families → % score. |
| **Held-out set** | A frozen prompt set used only for scoring, never edited to make a build pass. |
| **Test substrate** | An isolated environment (nested compositor / dedicated seat / scratch user / VM) where destructive and approval live tests run without touching the user's real session/files. |
| **Destructive sandbox** | A flagged mode where risky/destructive live tests are confined to scratch apps, scratch files/dirs, and a saved-restored clipboard. |
| **Chrome (browser chrome)** | The browser's own UI (address bar, tabs, buttons) vs. **page content** (DOM inside the web page). |
| **Idempotent action** | An action safe to repeat without changing outcome (focus, observe, scroll-to); non-idempotent: click/submit/type-append. |

## Requirements

### Requirement 1: Intelligent Planner (real reasoning for every primitive and combo)

**User Story:** As a user in GUI Cognition mode, I want my prompt turned into a correct, valid,
ordered plan of visible GUI steps, so that primitives and combos are not rejected at plan
validation.

#### Acceptance Criteria
1. The planner SHALL produce a valid, executable typed plan for every primitive in the supported set.
2. The LLM planner SHALL return schema-valid JSON via grammar/constrained decoding; output SHALL be
   validated against the plan schema. On validation failure the planner SHALL perform exactly ONE
   repair-retry (feeding back the validation error) and otherwise fall back deterministically —
   it SHALL NOT leniently scrape arbitrary text from a prose response.
3. WHEN the LLM planner is unavailable or low-confidence, the deterministic fallback SHALL still
   produce a valid, complete plan for every supported primitive and common combo.
4. The planner SHALL extract the concrete app name, target control hint, and text/query payload
   from the prompt and thread them into the plan steps; the action kind SHALL NEVER be used as a
   target name.
5. `planner.mode`/`planner.llm_status` SHALL be reported truthfully; a persistent
   `llm_rejected_fallback` on a healthy, capability-validated model SHALL be treated as a defect.
6. A single prompt with two actions ("save and close") SHALL produce both steps.

### Requirement 2: Per-step re-observe for multi-step execution

**User Story:** As a user, I want each step of a combo to act on the *current* screen, so opening
an app and then acting inside it works.

#### Acceptance Criteria
1. After any state-changing step, the runtime SHALL capture a fresh observation before resolving
   the next step's target.
2. The next step's target resolution SHALL run against the fresh observation, not the original.
3. WHEN the screen legitimately changed and the target is present, the workflow SHALL continue
   (no false "resolved target is no longer present").
4. WHEN the target is genuinely absent after re-observe, the workflow SHALL stop safely with a
   plain reason.
5. Re-observe SHALL respect timing (bounded readiness wait for app/window/page) and SHALL be
   bounded by the runaway caps in Requirement 21.

### Requirement 3: Wayland-safe window focus / switch

**User Story:** As a user on Wayland, I want "switch to the X window" to actually focus that window.

#### Acceptance Criteria
1. SwitchWindow SHALL NOT depend on `wmctrl` on Wayland sessions.
2. Window focus SHALL prefer a **compositor-native activate-by-window-identity** path (GNOME shell
   bridge / desktop portal). Key-based switching (Alt+Tab via uinput/ydotool) is a last-resort
   fallback and SHALL always be followed by verification.
3. WHEN no focus path is available, the action SHALL fail with a clear, actionable reason (not a
   generic "deterministic action backend failed").
4. SwitchWindow SHALL be verified by re-observing that the requested window is active.

### Requirement 4: Plan-step completeness (payload + verification)

**User Story:** As a user, I want type/search/navigate steps to carry their text and a verification
strategy, so they are not blocked by the validator.

#### Acceptance Criteria
1. Every TypeText/search/navigate step SHALL carry a sanitized text/query payload, OR be emitted as
   `AskClarification` — never a silently-blocked invalid step.
2. Every executable step SHALL carry a `verification_strategy`.
3. The validator SHALL NOT block a well-formed step for "missing payload/verification" when the
   prompt provides the needed information.

### Requirement 5: Full GUI primitive coverage (visible actions)

**User Story:** As a user, I want every basic visible GUI action to work from a prompt, so any
mouse/keyboard action I normally do can be done by prompt.

#### Acceptance Criteria
(each verified live, executed + re-observed)
1. Open app, switch window, focus control (incl. address bar, search box, username, password).
2. Type text, clear field, select-all.
3. Copy, paste.
4. Key press / shortcut (Enter, Escape, Ctrl+L, Ctrl+S).
5. Scroll up/down.
6. Click named buttons (Search, Save, Cancel, OK, Close, Back, Refresh).
7. Check / uncheck a labeled checkbox.
8. Close / handle an active dialog.
9. In-app search (settings, file manager).
10. Password field focus SHALL never log or echo the field value.

### Requirement 6: Multi-step combos (one prompt, several verified steps)

**User Story:** As a user, I want "open Chrome, focus address bar, type kria.ai, press Enter" to run
end-to-end.

#### Acceptance Criteria
1. The combos in the True-GUI prompt set SHALL complete, each step verified before the next.
2. WHEN a step fails verification, the workflow SHALL stop safely (no blind continue).

### Requirement 7: Cross-app clipboard workflows (clipboard-safe)

**User Story:** As a user, I want to copy in one app and paste in another without losing my own
clipboard contents.

#### Acceptance Criteria
1. "Switch to browser, copy the page title, switch to text editor, and paste it" SHALL work via
   real clipboard + window switching + re-observe.
2. The user's existing clipboard SHALL be saved before the operation and restored after (best
   effort), and clipboard access SHALL be serialized to avoid races.

### Requirement 8: File-manager navigate + select (non-destructive)

**User Story:** As a user, I want to navigate folders and select/show a file without changing
anything.

#### Acceptance Criteria
1. "Open file manager, go to Downloads, select the newest file, and show its name" SHALL work.
2. No file SHALL be moved/renamed/deleted unless explicitly requested AND approved.

### Requirement 9: Read / summarize only visible content

**User Story:** As a user, I want KRIA to summarize what is visible on screen.

#### Acceptance Criteria
1. Summaries SHALL be derived only from visible/observed content — never invented.
2. Untrusted OCR/page text SHALL be marked and SHALL be treated as **data only**: it SHALL NEVER
   influence planner or executor decisions (prompt-injection defense).

### Requirement 10: Approval-gated risky actions

**User Story:** As a user, I want risky actions to pause for my approval first.

#### Acceptance Criteria
1. Create folder, rename, move, delete, submit, apply, install, change-setting SHALL require HITL
   approval before execution.
2. WHEN approval is granted, the action SHALL then execute and be verified.
3. WHEN approval is denied/expired/mismatched, no action SHALL execute.
4. Automated approval (test fixtures) SHALL be permitted ONLY in the destructive sandbox /
   test substrate (Requirement 20), NEVER against the user's real session.

### Requirement 11: Ambiguity → ask (never guess)

**User Story:** As a user, I want KRIA to ask when a target is ambiguous instead of guessing.

#### Acceptance Criteria
1. WHEN multiple matching controls exist, the workflow SHALL pause and ask which one.
2. The workflow SHALL NOT execute on an ambiguous target.

### Requirement 12: Boundaries ("show but do not change")

**User Story:** As a user, I want "do not change/delete" boundaries strictly respected.

#### Acceptance Criteria
1. WHEN a prompt includes a boundary, no destructive/state-changing action SHALL occur.
2. Observation/navigation within the boundary is allowed.

### Requirement 13: Verify-and-stop

**User Story:** As a user, I want KRIA to perform, verify, then stop on request.

#### Acceptance Criteria
1. WHEN the prompt says "verify ... and stop", the workflow SHALL verify the expected state and
   terminate without further actions.

### Requirement 14: Recovery / re-focus (idempotency-aware)

**User Story:** As a user, I want safe recovery when focus is lost or a dialog appears.

#### Acceptance Criteria
1. WHEN focus is lost, the workflow SHALL re-observe and refocus the same target before retrying —
   exactly ONE safe retry, and ONLY for idempotent actions.
2. Non-idempotent actions (click/submit/type-append) SHALL NOT be auto-retried; the workflow stops
   and reports.
3. WHEN an unexpected dialog appears, the workflow SHALL stop and report what is visible.
4. WHEN a page/app fails to load, the workflow SHALL re-observe and explain.

### Requirement 15: Safety tiers and contract preserved

**User Story:** As the owner, I want existing safety guarantees preserved, so the upgrade never
weakens the safety contract.

#### Acceptance Criteria
1. GREEN actions run freely; YELLOW run with verification; RED/BLACK require approval.
2. No normal-mode GUI auto-routing SHALL be introduced; the selected-mode path is preserved.
3. Execution SHALL never originate from raw prompt/OCR/LLM text/raw coordinates.
4. "Stop"/"cancel" SHALL halt immediately (see Requirement 21).

### Requirement 16: Frontend production UX

**User Story:** As a user, I want a clear, responsive UI that shows progress and results.

#### Acceptance Criteria
1. Progress SHALL stream during the turn (observe → plan → per-step execute/verify), not only as
   one batch at the end.
2. The command SHALL NOT block the IPC for the whole turn (non-blocking dispatch).
3. Each turn's prompt and result SHALL render in the chat; sequential turns SHALL not lose
   prompt/response.
4. Output SHALL be layered: layman summary (status badge + headline + facts) on top, developer
   detail collapsible.
5. No hashes/IDs/secrets in the layman layer.
6. A visible **Stop/Cancel** control SHALL abort the active turn mid-flight.

### Requirement 17: Real live testing at every stage (acceptance methodology)

**User Story:** As the owner, I want every implementation stage verified with real live tests, so I
never accept untested claims.

#### Acceptance Criteria
1. Every stage SHALL be validated through `POST /api/testing/desktop-chat-command`,
   `mode_id = gui_cognition`, `execution_mode = execute_live`, asserting on `response.gui_cognition.*`.
2. The capability audit (held-out set) SHALL be re-run after each stage; overall + per-family %
   recorded with a variance band (3 runs).
3. A stage is "done" only when its target families meet their % gate (median of 3 runs) AND prior
   green suites stay green AND no destructive-leak occurred.
4. Tests SHALL use real-life prompts with proper expected outcomes and verify the action actually
   executed with the correct result via re-observe (not just that a plan formed) — see Requirement 23
   for the verification contract.
5. Three-tier testing SHALL be applied: (T1) core Rust unit, (T2) in-process integration with
   deterministic fixtures, (T3) live same-path harness. An additional **E2E UI tier** (Requirement 24)
   covers the rendered frontend.

### Requirement 18: No regression

**User Story:** As the owner, I want all existing suites to stay green, so the upgrade does not
break shipped behavior.

#### Acceptance Criteria
1. Steps 1–12 gui_cognition same-path suites SHALL remain green.
2. The broad `desktop_command` suite SHALL remain green.
3. UI unit + E2E suites SHALL remain green.
4. `git diff --check` SHALL pass.

### Requirement 19: Non-functional budgets (NFRs)

**User Story:** As a user, I want actions to be fast and bounded, so the assistant never hangs.

#### Acceptance Criteria
1. Single-primitive turn SHALL complete (or stop) within a configurable budget (default ≤ 8 s).
2. A combo turn SHALL be bounded by `max_steps` (default ≤ 12) and a turn watchdog (default ≤ 90 s).
3. Per-step target resolution + verification SHALL each have a bounded timeout with a defined
   degraded outcome.
4. Re-observe count per turn SHALL be capped (default ≤ max_steps + 4).
5. Budgets SHALL be configurable and asserted in tests.

### Requirement 20: Test isolation and data-loss safety

**User Story:** As the owner, I want live/destructive tests to never harm my real data.

#### Acceptance Criteria
1. Destructive and approval live tests SHALL run ONLY in a test substrate (nested compositor /
   dedicated seat / scratch user / VM), never against the user's real session.
2. The destructive sandbox SHALL confine actions to scratch apps, scratch files/dirs, and a
   saved-restored clipboard.
3. Auto-approval fixtures SHALL be rejected when not in the test substrate.
4. CI live runs SHALL use the isolated substrate (e.g., Xvfb/headless seat) and SHALL be
   reproducible; a deterministic fixture tier SHALL exist for environments without a display.
5. Non-destructive read/observe live tests MAY run on the real session.

### Requirement 21: Runaway control, abort, and kill-switch

**User Story:** As a user, I want to stop any automation instantly and have the system stop itself
if something goes wrong.

#### Acceptance Criteria
1. A turn SHALL be cancellable mid-flight from the UI and via API; cancellation SHALL halt before
   the next action.
2. The existing GlobalSafetyHalt SHALL be wired into the loop; when engaged, no further action
   executes.
3. Step/loop/time caps (Requirement 19) SHALL trigger a safe abort with a clear reason.
4. Repeated verification failure or screen "flapping" SHALL abort rather than loop.

### Requirement 22: Action audit ledger

**User Story:** As the owner, I want a record of what KRIA did on screen.

#### Acceptance Criteria
1. Every executed action SHALL be recorded in an append-only, sanitized ledger
   (action kind, target label, result, verification status, timestamps) — no secrets/raw payloads.
2. The ledger SHALL be inspectable and SHALL support post-hoc review of a turn.

### Requirement 23: Verification contract (per action type)

**User Story:** As the owner, I want "verified" to mean something precise and reliable.

#### Acceptance Criteria
1. Each action type SHALL define its verification predicate and evidence source:
   open/switch → active window matches (a11y/bridge); focus → focused control matches; type/clear/
   select/paste → focused field text state matches; click/checkbox → expected control state/screen
   delta; dialog → dialog closed/opened; scroll → viewport delta; in-app-search → results region
   present.
2. Verification SHALL use a bounded wait with a confidence threshold; ambiguous/low-confidence
   evidence SHALL yield `inconclusive` (not a false `verified`).
3. `ActionCompleted` (backend success) SHALL be distinct from `verified` (state confirmed).

### Requirement 24: End-to-end UI verification

**User Story:** As a user, I want the rendered UI to actually show my prompt, progress, and result.

#### Acceptance Criteria
1. An E2E test (Playwright against the desktop webview or mocked-bridge) SHALL assert: prompt
   renders, streaming progress appears, layered result renders, sequential turns do not lose
   prompt/response, and the Stop control aborts.
2. The E2E tier SHALL run in CI on the isolated substrate.

### Requirement 25: Preconditions and degraded mode

**User Story:** As a user, I want clear behavior when the environment is not fully capable.

#### Acceptance Criteria
1. Before `execute_live`, the system SHALL health-check preconditions (uinput daemon, AT-SPI,
   focus backend, DISPLAY/session type) and report readiness.
2. WHEN a precondition is missing, the system SHALL degrade gracefully (e.g., observe/plan only)
   with a clear reason — never a silent failure or a misleading "completed".

### Requirement 26: Security, abuse resistance, and locale scope

**User Story:** As the owner, I want GUI automation that cannot be trivially weaponized.

#### Acceptance Criteria
1. In terminal/shell contexts, destructive command keystrokes SHALL be gated by the safety tier
   and a command blacklist; blind typing into terminals stays blocked unless explicitly approved.
2. Prompt-injection via on-screen/OCR text SHALL NOT alter the plan or trigger actions (enforced
   by Requirement 9.2).
3. Control-label matching SHALL be scoped to English for v1 (explicitly documented); locale-aware
   matching is a tracked follow-up.
4. The action ledger (Requirement 22) SHALL provide an abuse-review trail.
