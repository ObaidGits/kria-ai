# Requirements Document

## Introduction

The `gui-cognition-production-upgrade` spec was completed and all its tasks marked done, but
acceptance was satisfied with **deterministic T2 fixtures** (which hand the runtime a perfect
observation + controls). A subsequent **live True-Test** of 112 real prompts through the desktop
endpoint (`POST /api/testing/desktop-chat-command`, `mode_id=gui_cognition`, `execute_live`) on a
Wayland/GNOME session produced **PASS 10 · PARTIAL 36 · FAIL 66 · 0 destructive-leak**.

This spec fixes the **live** behavior. It is driven by the live evidence in
`planning_docs/gui_cognition_user_truetest_results.md` and the root-cause analysis. Each requirement
maps to one observed failure class. Acceptance for every requirement is a **live re-run of the
specific prompts** (not a fixture surrogate), gated, sequential: a phase is not "done" until its
live gate passes, and no later phase starts until the current one is green.

### Non-negotiable invariants (apply to every requirement)
- **No weakening of verification.** A step is `verified` ONLY with real post-action evidence.
  Making a test pass by softening the verdict, faking evidence, or auto-approving is forbidden.
- **No destructive-leak, ever.** Zero unrequested delete/move/rename/submit/install/setting-change
  may execute. Approval-gated/destructive prompts must PAUSE on the real session.
- **Flag-gated + rollback.** Every change sits behind a named feature flag; flag-OFF must preserve
  current behavior byte-for-byte (asserted by test).
- **KRIA runtime authority preserved.** Intent → Capability → Policy → Substrate → Tool →
  Verification. No Prompt→Tool shortcut, no uncontrolled loops, bounded cognition, cancellation
  intact.
- **Truthful capability.** If a backend/handler is not really available, report an honest actionable
  error — never fabricate a success or claim a backend that did not act.

## Glossary

- **Live True-Test:** running a prompt through the real desktop endpoint (`execute_live`) and
  driving the actual Wayland/GNOME session, as opposed to a deterministic in-process fixture (T2).
- **PASS / PARTIAL / FAIL:** PASS = executed AND `verified` (or correct gate/ask); PARTIAL =
  executed but not verified; FAIL = blocked / no progress / wrong behavior.
- **Predicate:** the post-action state a verification contract checks (e.g. `window_visible`,
  `active_window_match`, `focused_control`, `text_present`, `screen_changed`).
- **Window present/visible:** the app's window exists in the desktop open-window set, regardless of
  whether it is the focused/active window.
- **Prerequisite step:** an `OpenApp`/`SwitchWindow` step auto-inserted before a bare primitive so
  the target app/control becomes observable.
- **Destructive-leak:** an unrequested delete/move/rename/submit/install/setting-change that
  executes.

## Requirements

### Requirement 0: Shared multi-backend structured-output adapter (TRUE FIRST BLOCKER)

**User Story:** As a user, I want to choose either a local LLM or a cloud LLM and get the same
working result, so that KRIA is not locked to one backend; as the maintainer, I want every provider
to return schema-valid typed plans.

#### Acceptance Criteria
1. WHEN the planner needs a typed plan THEN a shared structured-output adapter SHALL select the
   strongest method the backend genuinely honors: local → grammar/guided JSON; cloud
   OpenAI-compatible → `response_format` `json_schema` (if honored) → else `json_object` (with the
   word "json" + a compact schema + one few-shot in the prompt) → else function/tool-calling.
2. WHEN a backend's structured capability is unknown THEN the adapter SHALL detect it via a cheap,
   cached per provider+model runtime probe — it SHALL NOT assume a proxy passes `response_format`
   through.
3. WHEN any structured method is available THEN `GuiPlannerCapability` SHALL report
   `capability_validated`; `not_structured_capable` SHALL be reported only when none is available
   AND the bounded re-ask is exhausted.
4. WHEN the model returns content that fails strict schema validation THEN the adapter SHALL re-ask
   1–2 times (bounded by Task 1 caps) feeding the validation error back, and SHALL NEVER
   lenient-scrape prose.
5. WHEN the same prompt is run on a local vs a cloud backend THEN both SHALL return a schema-valid
   typed plan of the same structure (functional parity); byte-identical text is NOT required.
6. WHEN the flag is OFF THEN the prior planner behavior SHALL be byte-for-byte unchanged, and the
   existing `chat`/chat-stream behavior of other features SHALL be unaffected.
7. **Live gate:** with the configured cloud model (`deepseek-v4-flash-free`) AND the local model, a
   GUI turn SHALL report a real structured mode (not `llm_rejected_fallback`), and "Open the
   calculator" SHALL yield a typed plan with `target_app_kind/hint = calculator` (NOT the active
   window) as valid JSON from BOTH backends. (Actual app-open/verify belongs to Requirement 1.)

### Requirement 1: Post-action verification must reflect real Wayland window behavior (Issue #2)

**User Story:** As a user, when KRIA opens an app and the app actually opens, I want KRIA to report
it as verified (not failed), so that successful actions are not falsely downgraded.

#### Acceptance Criteria
1. WHEN an `OpenApp` action completes THEN verification SHALL check that the app's window is
   **present/visible** in the desktop window set (predicate `window_visible`, evidence
   `observation`/desktop-state) — NOT that it is the focused active window.
2. WHEN the window is not yet present THEN verification SHALL perform a **bounded readiness wait**
   (Task 1 caps: `max_reobserve` / per-step verify budget) before concluding, never an unbounded
   poll.
3. WHEN the requested app name differs cosmetically from the window app_name/title (e.g.
   `Chrome`/`chromium`/`google-chrome`) THEN matching SHALL use a tolerant alias match.
4. WHEN evidence is genuinely weak/unreliable THEN the verdict SHALL be honest `inconclusive`, never
   a false `verified`.
5. WHEN the flag is OFF THEN the prior verification verdict SHALL be byte-for-byte unchanged.
6. WHEN the open-app and app-launch-then-act prompts are re-run live THEN previously
   `RAN_NOT_VERIFIED` results SHALL flip to executed+`verified` and the open-app family SHALL be
   ≥ 80% PASS.

### Requirement 2: Bare primitives must establish app/context before acting (Issue #3)

**User Story:** As a user, when I say "Click the Save button" or "Focus the address bar" without
first opening the app, I want KRIA to open/focus the right app first (or ask if it can't tell), so
that the action can actually resolve and execute.

#### Acceptance Criteria
1. WHEN a primitive's target control/app is NOT observable in the current `GuiContext` THEN the
   planner SHALL prepend an `OpenApp` (or `SwitchWindow` if the app is open but unfocused)
   prerequisite step inferred from the intent.
2. WHEN the target app cannot be confidently inferred THEN the runtime SHALL `AskClarification`
   (never guess a wrong target).
3. WHEN a prerequisite step runs THEN the runtime SHALL **re-observe** before resolving the next
   step against fresh context (Task 3 re-observe).
4. WHEN the flag is OFF THEN the prior plan SHALL be byte-for-byte unchanged.
5. WHEN bare focus/type/clear/select/copy/paste/click/checkbox/in-app-search prompts are re-run live
   THEN they SHALL reach executed+`verified` (or correct `AskClarification` where genuinely
   ambiguous), with ≥ 80% PASS across these families.

### Requirement 3: Real Wayland window activation (Issue #1)

**User Story:** As a user, when I say "Switch to the Chrome window", I want KRIA to actually bring
that window forward and confirm it, so that window-switch and cross-app flows work on Wayland.

#### Acceptance Criteria
1. WHEN the session is Wayland/GNOME THEN a compositor-native activate-by-window-identity handler
   (`GnomeBridge`, and `Portal` where available) SHALL be implemented and actually invoked, and
   `window_focus_backend_available` SHALL return true ONLY when the handler is genuinely reachable.
2. WHEN `SwitchWindow` runs THEN it SHALL route through the implemented backend and **verify** by
   re-observing that the active window == the requested window identity.
3. WHEN no real activation path is available THEN KRIA SHALL return a truthful actionable error and
   SHALL NOT fall back to blind Alt+Tab pretending success.
4. WHEN the flag (`gui_cog_wayland_focus`) is OFF THEN prior behavior SHALL be unchanged.
5. WHEN switch-window prompts are re-run live THEN they SHALL execute and verify (active ==
   requested), the switch-window family SHALL be ≥ 80% PASS, and cross-app combos needing a switch
   SHALL be unblocked.

### Requirement 4: Scroll / context-dependent execution (Issue #5)

**User Story:** As a user, when I say "Scroll down", I want KRIA to scroll the active surface, so
that scroll and scroll-combos work once context is established.

#### Acceptance Criteria
1. WHEN an active window/scrollable surface is resolved (post Req 1/3) THEN `Scroll` SHALL execute
   and verify by `screen_changed`.
2. WHEN no active surface exists THEN KRIA SHALL observe/ask, not silently block.
3. WHEN scroll + switch-then-scroll prompts are re-run live THEN they SHALL be ≥ 80% PASS.

### Requirement 5: Context-aware key-press gating (Issue #4)

**User Story:** As a user, when there is a known editable/focused target, I want Enter/Ctrl+S to
work; when there is no context, I want KRIA to stay safe.

#### Acceptance Criteria
1. WHEN a resolved editable/focused context exists THEN `PressKey` (Enter/Ctrl+S/Ctrl+L/Esc) SHALL
   be allowed and executed with `screen_changed` verification.
2. WHEN no target context exists THEN a high-impact key SHALL remain gated/asked (safety preserved —
   this is correct, not a failure).
3. WHEN key-press prompts are re-run live THEN key-press WITH context executes+verifies and bare
   standalone key-press safe-gating is documented as correct behavior.

### Requirement 6: System settings app resolvable (Issue #6)

**User Story:** As a user, when I say "Open system settings", I want KRIA to launch the platform
settings app.

#### Acceptance Criteria
1. WHEN the OS is GNOME THEN the app registry SHALL resolve "system settings"/"settings" to
   `gnome-control-center` (with aliases) and a valid launch command.
2. WHEN settings opens THEN settings-search prompts SHALL reuse the Requirement 2 open+focus-search
   path.
3. WHEN open-settings + settings-search prompts are re-run live THEN they SHALL be ≥ 80% PASS.

### Requirement 7: Ambiguity to ask on multiple candidates (Issue #7)

**User Story:** As a user, when I say "if multiple Search buttons are visible ask me which one", I
want KRIA to ask, not guess.

#### Acceptance Criteria
1. WHEN the resolver finds ≥ 2 high-confidence candidates AND the prompt requests asking on
   ambiguity THEN the runtime SHALL `AskClarification` and SHALL NOT execute.
2. WHEN exactly one clear candidate exists THEN normal execution proceeds.
3. WHEN the multi-candidate "ask me" prompts are re-run live THEN they SHALL produce a correct ask
   (no guess/execute) and prior correctly-asking prompts SHALL stay green.

### Requirement 8: Sequential gated remediation and final acceptance

**User Story:** As the maintainer, I want each fix landed and live-verified in order, so that
regressions are caught and progress is honest.

#### Acceptance Criteria
1. Each requirement (1–7) SHALL be implemented and **live-gated in order**; a phase SHALL NOT start
   until the prior phase's live gate is green.
2. WHEN all phases are complete THEN a **full 112-prompt live re-run** SHALL be recorded with
   before/after, 0 destructive-leak, and no family BROKEN.
3. WHEN a prompt is inherently live-dependent (OCR summarize-visible, network/page-load recovery)
   THEN it SHALL be reported honestly as live-dependent — never faked to pass.
