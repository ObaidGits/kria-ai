# Requirements Document

## Introduction

GUI Cognition V2 (Sight / Brain / Hands) shipped its skeleton, but live testing on the
running desktop proved it behaves like a **blind, mute, single-shot** automaton:

- **Blind** — the default Sight is element-free and the OmniParser sidecar defaults to a
  `DummyOmniParser`, so no real on-screen controls are ever detected (`CONTROLS 0`,
  "Screen off · OCR off · A11y off"). The system can open apps but cannot click, select,
  or find anything.
- **Mute / single-shot** — the "Brain" is a text-only model over an (empty) element list
  plus a tiny hardcoded keyword table. It cannot decompose a layman task, cannot sequence
  `type → Enter`, repeats actions, and a "same app re-open" backstop ends multi-step tasks
  early ("terminal is already open." while "run ls" never ran).
- **Brittle** — a single transport/timeout blip from the model server fails the whole turn
  with no retry; a no-progress stall gives up instead of recovering.

This spec upgrades GUI Cognition into a **production-grade, intelligent, LLM-agnostic**
desktop automation system that a layman can drive entirely through prompts — including
**unordered, mistyped, multi-step natural language** ("open bookmarks tab in chrome",
"write a pascal's triangle program in VS Code, run it and show the output"). It fixes the
six confirmed defects first (perception, premature completion, action sequencing,
real planning, app-name resolution, resilience/recovery) and ends with a pluggable
**vision GUI model upgrade**.

Two hard non-functional constraints from the product owner:

1. **No regressions, ever.** Every task must leave the system MORE capable and provably
   working on the real desktop — never "executed all tasks but landed back where nothing
   works or everything breaks." Each task carries a reliable, proof-based live test using
   the running KRIA app, real screen, logs, and session, plus a flaw/vulnerability review
   loop before it is considered done.
2. **Frictionless defaults.** Do NOT over-tighten safety/security. No repeated approvals,
   no useless manual configuration. Ship best-optimized defaults so the user does nothing
   but type prompts. Human-in-the-loop stays minimal for now (considered, not a focus).

## Glossary

- **Sight**: perception layer; screenshot → `Observation` (elements with id/bbox/kind/
  label/confidence). Backed by the `kria-vision` sidecar.
- **Brain**: cognition layer; `(task, observation, history) → Decision`. Model-agnostic
  behind a trait. MUST NOT be named or coupled to any single model (e.g. "qwen").
- **Hands**: action layer; executes a `Decision` (click/type/key/scroll/open) via uinput.
- **Planner**: turn-level component that decomposes a natural-language task into an ordered
  list of sub-goals and tracks their completion across steps.
- **Sub-goal**: one verifiable unit of intent (e.g. "open Chrome", "open a new tab",
  "navigate to youtube.com").
- **Grounding**: detecting real on-screen elements so the Brain targets a real control.
- **App resolver**: maps a user-supplied app name (possibly mistyped/ambiguous) to an
  installed application, asking to confirm only when genuinely ambiguous.
- **Live proof test**: a test run against the RUNNING KRIA desktop that verifies the real
  outcome (window state, screen text/elements, process, logs) — not just the reply string.
- **LLM-agnostic**: no symbol, file, type, or config keyed to a specific model vendor.

## Requirements

### Requirement 1: LLM-agnostic brain abstraction (de-"qwen")
**User Story:** As an operator who may run any LLM, I want the cognition layer to be
model-neutral in code and config, so I can swap the underlying model without touching
GUI-cognition logic or renaming things.

#### Acceptance Criteria
1. WHEN the codebase is searched THEN there SHALL be no `qwen_brain` file name, no
   `QwenBrain` type, and no `qwen`-keyed public symbol in the GUI-cognition pipeline; the
   default text brain SHALL be named neutrally (e.g. `LlmPlannerBrain` in
   `llm_brain.rs`).
2. WHEN the brain reports its label THEN it SHALL derive from the configured model/runtime
   at runtime (e.g. the served model id), NOT a hardcoded `"qwen"` literal.
3. WHEN the brain is selected via configuration THEN the selector SHALL use neutral values
   (e.g. `text` / `vision`) rather than vendor names, while keeping backward-compatible
   aliases so existing env values do not break.
4. WHEN the rename is performed THEN it SHALL be a pure refactor: all existing tests SHALL
   pass unchanged in behavior and the crate SHALL compile.
5. WHERE documentation or events reference the brain THEN they SHALL use model-neutral
   wording.

### Requirement 2: Sight grounding ON by default with zero manual setup
**User Story:** As a user, I want KRIA to see real on-screen controls out of the box, so
click/find/select tasks work without me configuring anything.

#### Acceptance Criteria
1. WHEN a GUI turn needs to act on an on-screen control THEN the system SHALL obtain a
   grounded `Observation` with real detected elements, using sensible defaults and WITHOUT
   the user setting any environment variable.
2. WHEN the system starts THEN the vision sidecar SHALL be auto-started/auto-selected with
   a real (non-dummy) detection backend by default; the `DummyOmniParser` SHALL NOT be the
   effective default for production use.
3. WHEN grounding is unavailable (sidecar down, model missing) THEN the turn SHALL degrade
   honestly with a clear, user-readable reason and SHALL NOT silently behave as if blind.
4. WHEN a cheap (element-free) observation cannot satisfy a click/find intent THEN the loop
   SHALL escalate to grounded perception automatically (lazy escalation), and SHALL cache
   coherently so it does not act on a stale view.
5. WHEN grounding runs THEN element labels SHALL be treated as untrusted (sanitized) and
   coordinates SHALL map correctly to physical pixels (DPI/multi-monitor).
6. WHEN Sight health is queried THEN the system SHALL expose whether grounding is live,
   which backend is active, and the last degrade reason, for diagnostics and the panel.

### Requirement 3: Multi-step tasks complete fully (no premature "done")
**User Story:** As a user, I want a multi-step task to finish all its parts, so "open
terminal and run ls" actually runs `ls`.

#### Acceptance Criteria
1. WHEN a task has multiple sub-goals THEN the turn SHALL only report completion after ALL
   sub-goals are satisfied (or it honestly stops/asks), never after just the first.
2. WHEN an app is already open THEN the system SHALL proceed to the NEXT unsatisfied
   sub-goal instead of terminating with "X is already open."
3. WHEN re-opening the same app would be a no-op THEN the system SHALL treat it as
   "already focused, continue" rather than "task complete."
4. WHEN the loop ends THEN its terminal status SHALL reflect true sub-goal completion and
   SHALL be recorded for the live proof test to assert against.

### Requirement 4: Intelligent planning of natural, unordered, multi-step prompts
**User Story:** As a layman, I want to type messy, unordered instructions and have KRIA
figure out the right ordered steps, so I don't have to phrase things perfectly.

#### Acceptance Criteria
1. WHEN a task is received THEN a Planner SHALL decompose it into an ordered list of
   sub-goals using the configured LLM (not a fixed keyword table), and the keyword/heuristic
   path SHALL exist ONLY as a fallback.
2. WHEN a prompt is unordered or implicit (e.g. "open bookmarks tab in chrome") THEN the
   Planner SHALL infer the correct order (open Chrome → open bookmarks) regardless of word
   order.
3. WHEN a prompt spans tools/substrates (e.g. "write a pascal's triangle program in VS Code,
   run it and show the output") THEN the Planner SHALL produce a coherent sub-goal sequence
   (open editor → create/write file → run → surface output) and route each sub-goal to the
   correct action/substrate.
4. WHEN sub-goals execute THEN the system SHALL track which are done, feed remaining
   sub-goals to per-step decisions, and re-plan if the screen reveals the plan was wrong.
5. WHEN the Planner cannot form a safe/sensible plan THEN it SHALL ask one targeted
   clarifying question rather than guessing.
6. WHEN planning runs THEN it SHALL be bounded (latency/step caps) and SHALL NOT loop.

### Requirement 5: Correct action sequencing — submit, navigate, composite actions
**User Story:** As a user, I want KRIA to press Enter after typing a URL/command and use
new tabs/address bar correctly, so navigation and search actually happen.

#### Acceptance Criteria
1. WHEN text is typed to be submitted (URL, search, terminal command) THEN the system SHALL
   press Enter/submit as part of the action, not leave the text unsent.
2. WHEN navigating in a browser THEN the system SHALL use the correct sequence (focus
   address bar / new tab as needed → type → Enter) instead of typing into whatever has
   focus.
3. WHEN the same text would be typed again with no screen change THEN the system SHALL NOT
   blindly repeat it (no "github.comgithub.com" concatenation).
4. WHEN a composite action is the right move THEN the action vocabulary SHALL support it
   (e.g. type-and-enter, focus-then-type) so common intents are one reliable step.
5. WHEN a terminal command is requested THEN the system SHALL type the command and execute
   it (Enter), and surface the result where possible.

### Requirement 6: Robust app-name resolution (mistyped / ambiguous / closest match)
**User Story:** As a layman, I want to name an app loosely or wrongly and still have the
right one open, with a quick confirm only when truly ambiguous.

#### Acceptance Criteria
1. WHEN an app name is given THEN the resolver SHALL match it against installed apps using
   fuzzy/alias matching (e.g. "code" → VS Code, "app center" → the software store).
2. WHEN exactly one good match exists THEN the system SHALL open it without asking.
3. WHEN multiple plausible apps match the same name THEN the system SHALL ask the user to
   confirm which one (a single, fast choice), then proceed.
4. WHEN no exact match exists but a close one does THEN the system SHALL choose the closest
   match and proceed, stating which app it chose.
5. WHEN no reasonable match exists THEN the system SHALL say so honestly and suggest the
   nearest installed alternatives rather than failing opaquely.
6. WHEN the resolver runs THEN it SHALL reuse the existing app-registry/launcher rather than
   inventing a parallel one.

### Requirement 7: Resilience — model-serving retries and stability
**User Story:** As a user, I want a transient model hiccup to not kill my task, so the
system retries instead of failing.

#### Acceptance Criteria
1. WHEN a brain call fails with a transport/provider error (connection refused/reset,
   server mid-swap) THEN the system SHALL retry with a bounded backoff (swap/health aware)
   before failing the turn.
2. WHEN a brain call times out THEN the system SHALL retry once (already implemented) and
   the timeout SHALL be configurable with a sensible default.
3. WHEN the planner model is needed THEN model-serving SHALL avoid swap-thrash for the
   duration of a turn (keep the needed model resident / wait for ready before the call).
4. WHEN all bounded retries are exhausted THEN the failure SHALL be reported honestly with a
   user-readable reason, never a silent hang.
5. WHEN retries occur THEN they SHALL be observable in logs/events for the live proof test.

### Requirement 8: No-progress recovery instead of giving up
**User Story:** As a user, I want KRIA to try another approach when stuck, so it completes
more tasks instead of stopping at the first stall.

#### Acceptance Criteria
1. WHEN a state-changing action produces no screen change THEN the system SHALL attempt a
   bounded recovery (e.g. escalate to grounded perception, re-focus the target, try an
   alternate action) before declaring no-progress.
2. WHEN recovery succeeds THEN the loop SHALL continue toward the remaining sub-goals.
3. WHEN recovery is exhausted THEN the loop SHALL stop honestly with a clear reason (still
   never an infinite loop).
4. WHEN recovery runs THEN it SHALL be bounded (attempt cap) and recorded for the proof test.

### Requirement 9: Reliable, proof-based live testing after every step
**User Story:** As the product owner, I want each task validated against the real running
desktop with objective proof, so I never end up back at "nothing works / everything broke."

#### Acceptance Criteria
1. WHEN a task is completed THEN there SHALL be a live test run against the RUNNING KRIA
   desktop (real screen, session, logs, commands) that verifies the ACTUAL outcome (window
   present/focused, expected screen text/element, process effect, file created/run output),
   not merely the reply string.
2. WHEN a live test asserts success THEN it SHALL use an EXTERNAL verifier (e.g. window/
   process query, screenshot/OCR check, filesystem/command output) independent of KRIA's
   own self-report.
3. WHEN a live outcome cannot be externally verified THEN it SHALL be reported INCONCLUSIVE,
   never fabricated as pass.
4. WHEN a task lands THEN a regression suite of previously-passing prompts SHALL be re-run
   and SHALL NOT drop below the prior pass rate (no regressions gate).
5. WHEN a step's tests do not pass/prove THEN the task SHALL iterate (analyze flaws/vulns,
   fix, re-test) before the next task starts.
6. WHEN live tests run THEN they SHALL be runnable via a single documented command and
   SHALL capture logs/artifacts for review.

### Requirement 10: Frictionless defaults and minimal HITL
**User Story:** As a user, I want to just type prompts and have it work, without approval
popups or config chores.

#### Acceptance Criteria
1. WHEN the system runs with defaults THEN GUI automation SHALL work end-to-end with no
   manual environment configuration required by the user.
2. WHEN an action is benign (open/click/type/navigate/scroll) THEN it SHALL execute without
   an approval prompt.
3. WHEN HITL is involved THEN it SHALL be reserved for genuinely destructive/ambiguous
   cases only, and SHALL be a single quick confirm, not repeated.
4. WHEN security/safety is evaluated THEN the existing gate/blacklist SHALL be preserved but
   NOT made stricter; defaults SHALL favor smooth execution.
5. WHERE a setting could be inferred THEN the system SHALL pick the best default rather than
   ask the user to configure it.

### Requirement 11: Frontend upgrades for the new intelligence
**User Story:** As a user, I want the GUI Cognition panel to clearly show the plan, progress,
and any quick confirmations, so I understand and trust what KRIA is doing.

#### Acceptance Criteria
1. WHEN a turn runs THEN the panel SHALL show the decomposed sub-goal plan and mark each
   sub-goal as it completes.
2. WHEN grounding escalates or the system "looks closer" THEN the panel SHALL show a benign
   progress note (not a failure).
3. WHEN the app resolver needs a choice THEN the panel SHALL present a fast confirm
   (which app) inline.
4. WHEN recovery/retry happens THEN the panel SHALL show it as in-progress, not as a hard
   failure.
5. WHEN a turn ends THEN the panel SHALL show the true outcome (completed / asked / stopped
   with reason) consistent with the backend status.
6. WHEN events stream THEN existing event contracts SHALL be preserved or extended
   additively (no breaking renames of existing event names).

### Requirement 12: Pluggable vision GUI model upgrade (last)
**User Story:** As the product owner, I want to later drop in a vision GUI model
(UI-TARS-1.5 / Qwen2.5-VL / GUI-Actor) for SOTA grounding, without rearchitecting.

#### Acceptance Criteria
1. WHEN a vision GUI model is configured THEN it SHALL plug into the existing model-neutral
   Brain trait with no changes to Sight, Hands, planner, or the loop.
2. WHEN the vision brain is active THEN it SHALL ground actions from the screenshot directly
   (coordinate/region) and SHALL be selectable via the neutral `vision` brain option.
3. WHEN the vision model is unavailable THEN the system SHALL fall back to the text brain +
   grounded Sight path honestly.
4. WHEN the vision upgrade lands THEN it SHALL be validated on the same live proof harness
   and SHALL not regress the text-brain path.
5. WHERE serving the vision model competes for VRAM THEN it SHALL reuse the existing
   orchestrator swap/evict machinery (Requirement 7.3) without thrash.

### Requirement 13: Production-grade rollout discipline
**User Story:** As the product owner, I want each change flag-guarded and reversible, so a
bad step can be rolled back without breaking the desktop.

#### Acceptance Criteria
1. WHEN a behavioral change ships THEN it SHALL be guarded by a flag/default that can roll
   back to the prior behavior, and new serialized fields SHALL use defaults.
2. WHEN a task completes THEN it SHALL be independently revertable without breaking earlier
   tasks.
3. WHEN the full upgrade is done THEN there SHALL be a single end-to-end live suite proving
   the layman scenarios (unordered/mistyped/multi-step) pass on the real desktop.
4. WHEN defaults are chosen THEN they SHALL be the optimized production defaults (grounding
   on, retries on, planner on) with overrides available but not required.

---

## Hardening Requirements (loop-proofing addendum)

These requirements exist specifically to break the "implement → same issues → re-plan"
loop. They make PROOF objective, COMPLETION verifiable, and REGRESSIONS impossible to hide.
They refine and take precedence over any looser wording above.

### Requirement 14: Reproducible environment & fail-fast preflight
**User Story:** As the implementer, I want a single command that brings up and health-checks
the entire GUI-cognition stack, so a live proof never stalls on a half-up environment.

#### Acceptance Criteria
1. WHEN a preflight command is run THEN it SHALL start/verify the KRIA desktop, the
   `kria-vision` sidecar, the planner/text model server, the uinput daemon, and the display
   session, and SHALL report each component's health as machine-readable JSON.
2. WHEN any component is missing/unhealthy THEN preflight SHALL fail fast with a specific,
   actionable reason (which component, why, how to fix) and a non-zero exit code.
3. WHEN preflight passes THEN it SHALL emit a `ready: true` JSON record with versions/ports
   used, which every live proof run references as its precondition.
4. WHEN a live proof runs THEN it SHALL refuse to run (and not fabricate results) unless the
   latest preflight is `ready: true`.
5. WHERE a component can be auto-started THEN preflight SHALL auto-start it rather than ask
   the user to do it manually.

### Requirement 15: Per-sub-goal external verification predicates
**User Story:** As the product owner, I want each sub-goal proven by an external signal, so
"completed" never means "the model said so."

#### Acceptance Criteria
1. WHEN a sub-goal completes THEN its success SHALL be decided by a concrete predicate keyed
   to its kind, using a signal INDEPENDENT of KRIA's self-report:
   - OpenApp → target window present AND focused (window query).
   - Navigate → active URL/title matches the target (browser title/OCR).
   - RunCommand → command ran with expected stdout/exit (terminal scrollback/OCR or shell).
   - WriteFile → file exists with expected content (filesystem).
   - Click/Toggle → the expected element/pane/state change is observable (re-observe/OCR).
   - Type → the expected text is present in the focused field (re-observe/OCR).
2. WHEN a predicate cannot be evaluated THEN the sub-goal SHALL be marked UNVERIFIED
   (INCONCLUSIVE), never silently PASS.
3. WHEN the turn completes THEN it SHALL complete only if every sub-goal is VERIFIED-done
   (not merely attempted).
4. WHEN a predicate fails THEN the loop SHALL trigger recovery (Requirement 8) before
   stopping.
5. WHEN verifiers run THEN they SHALL be the SAME predicates the live proof harness uses, so
   in-loop verification and test verification agree.

### Requirement 16: Cross-substrate execution bridge
**User Story:** As a user, I want prompts that mix GUI + file + shell (e.g. "write a pascal's
triangle program in VS Code, run it and show the output") to work end-to-end.

#### Acceptance Criteria
1. WHEN a sub-goal is non-GUI (write-file, run-command, read-output) THEN the loop SHALL
   route it through a defined bridge to the EXISTING tool/shell/file executors, not GUI
   keystroke guessing.
2. WHEN a bridged sub-goal returns THEN its result (file path, stdout, exit code) SHALL be
   captured and made available to subsequent sub-goals and to the final reply.
3. WHEN deciding GUI vs bridge THEN the routing rule SHALL be explicit and testable (by
   sub-goal kind), not heuristic guesswork.
4. WHEN a bridged step needs a visible result (e.g. "show me the output") THEN the system
   SHALL surface it (in the reply and/or by focusing the relevant window).
5. WHEN the bridge executes a command/file op THEN it SHALL pass through the existing safety
   gate unchanged (no new approval surface, no relaxation).

### Requirement 17: Planner quality gate (offline before live)
**User Story:** As the product owner, I want the planner proven on labeled fixtures before it
drives the live loop, so bad decomposition never silently breaks everything.

#### Acceptance Criteria
1. WHEN the planner is built THEN there SHALL be a labeled fixture set (≥40 prompts: ordered,
   unordered, implicit, multi-substrate, ambiguous) with expected sub-goal decompositions.
2. WHEN the planner is evaluated offline THEN it SHALL meet a defined accuracy bar on these
   fixtures BEFORE it is wired into the live loop.
3. WHEN the planner model is selected THEN it SHALL be explicitly chosen (an instruct/large
   enough model) and schema/grammar-constrained; the choice SHALL be recorded.
4. WHEN the planner underperforms the bar THEN the deterministic fallback SHALL remain the
   active path until the bar is met.
5. WHEN the planner runs THEN its input SHALL be the user task only; screen-derived text
   SHALL be treated as untrusted data and SHALL NOT steer the plan (injection hardening).

### Requirement 18: Objective acceptance bars, per-category baseline & flakiness policy
**User Story:** As the product owner, I want numeric, category-wise targets and a flakiness
policy, so progress is measured objectively and flaky tests don't block or falsely pass.

#### Acceptance Criteria
1. WHEN the baseline is captured THEN it SHALL be PER CATEGORY (open-only, multi-step,
   navigation, app-resolution, cross-substrate) with explicit pass counts.
2. WHEN a task defines done THEN it SHALL state a NUMERIC acceptance bar (absolute pass rate
   and/or required scenarios), not "high" or "≥ prior".
3. WHEN the upgrade is complete THEN a FIXED corpus of ≥50 layman prompts SHALL pass at
   ≥90% (external-verified) with ZERO regressions on the regression set.
4. WHEN a live test is classified THEN flakiness SHALL be handled by running N times and
   using a majority/3-of-5 rule; persistently flaky cases go to a quarantine list, not a
   silent pass/fail.
5. WHEN the no-regression gate runs THEN a real regression (deterministic drop) SHALL be
   distinguished from flakiness (variance), and only real regressions SHALL block.
6. WHEN any metric is reported THEN it SHALL be emitted as machine-readable JSON artifacts
   for review.

### Requirement 19: Resource residency, unified turn budget & latency SLO
**User Story:** As a user, I want the agent fast and stable, so it doesn't thrash models,
time out, or take minutes per task.

#### Acceptance Criteria
1. WHEN a GUI turn runs THEN a documented model-residency plan SHALL keep the needed model
   resident for the turn and SHALL enforce text↔vision mutual exclusion to avoid swap-thrash.
2. WHEN resources are sized THEN the spec SHALL state the VRAM budget assumption and what is
   co-resident (planner/text, vision, ComfyUI) and the swap-latency budget.
3. WHEN a turn executes THEN a UNIFIED turn budget SHALL account for steps, re-plans,
   recoveries, and retries together, so combined activity cannot silently exhaust the step
   cap and cause a premature stop; long multi-step tasks SHALL get an adequate budget.
4. WHEN performance is measured THEN per-turn latency SHALL meet a stated SLO (e.g. open-only
   ≤ a few seconds; multi-step bounded per step) captured as a gate metric in the harness.
5. WHEN budgets/SLOs are exceeded THEN the system SHALL stop honestly with a clear reason,
   never hang.

### Requirement 20: Frozen, additive event contract
**User Story:** As a frontend developer, I want the full event schema fixed up front, so
backend and panel never drift.

#### Acceptance Criteria
1. WHEN the upgrade begins THEN the COMPLETE `gui_cognition:event` schema (plan-created,
   sub-goal-updated, app-choice, grounding-status, recovery, retry, terminal) SHALL be
   defined and frozen before backend tasks emit against it.
2. WHEN events evolve THEN changes SHALL be ADDITIVE; no existing event name SHALL be
   renamed or removed.
3. WHEN the backend emits an event THEN it SHALL conform to the frozen schema and SHALL be
   contract-tested.
4. WHEN the frontend consumes events THEN it SHALL render against the frozen schema with
   graceful handling of unknown/added fields.

### Requirement 21: Composite-action focus safety
**User Story:** As a user, I want typed text to land in the right place, so navigation/search
never types into the wrong field.

#### Acceptance Criteria
1. WHEN a type/composite action runs THEN the system SHALL confirm the intended field/window
   is focused (re-observe) before typing; if not, it SHALL focus it first.
2. WHEN focus cannot be confirmed THEN the system SHALL NOT blind-type; it SHALL recover or
   ask.
3. WHEN the same unchanged text would be typed again THEN it SHALL be suppressed (no
   concatenation).
4. WHEN a submit is required THEN it SHALL occur only after a successful focused type.

---

## Concrete Targets, Calibration & Contingency (residual-risk addendum A–H)

These pin the numbers and close the residual risks found in the post-upgrade review. They
are binding; "e.g." values elsewhere are superseded by these.

### Requirement 22: Verifier calibration & confidence (risk A)
**User Story:** As the product owner, I want the verifiers themselves proven accurate, so the
whole proof system is not built on a flaky signal.

#### Acceptance Criteria
1. WHEN a verifier is built THEN it SHALL be calibrated against a labeled ground-truth set of
   known screenshots/window states and SHALL achieve ≥95% agreement with the labels.
2. WHEN a verifier returns a verdict THEN it SHALL attach a confidence score.
3. WHEN confidence is below a defined threshold THEN the verdict SHALL be INCONCLUSIVE, never
   a low-confidence PASS/FAIL.
4. WHEN window/element queries race a still-changing screen THEN the verifier SHALL retry/
   settle (bounded) before deciding, to avoid race-induced false verdicts.
5. WHEN calibration drops below the bar THEN dependent live gates SHALL be treated as
   unreliable until the verifier is fixed.

### Requirement 23: Pinned acceptance metrics & latency SLO (risk B)
**User Story:** As the product owner, I want exact numeric bars, so "done" is never an
argument.

#### Acceptance Criteria
1. WHEN the planner is gated offline THEN it SHALL achieve ≥85% exact-decomposition accuracy
   on the labeled fixture set before driving the live loop.
2. WHEN per-category live bars are evaluated THEN they SHALL be: open-only ≥95%, multi-step
   ≥85%, navigation ≥80%, app-resolution ≥90%, cross-substrate ≥75%.
3. WHEN the final corpus is evaluated THEN overall PASS SHALL be ≥90% (external-verified)
   with ZERO real regressions.
4. WHEN latency is measured THEN the SLO SHALL be: open-only ≤3s end-to-end; multi-step ≤8s
   per step (decide+act+verify); a turn SHALL never exceed `KRIA_GUI_COG_TURN_BUDGET_MS`.
5. WHEN any bar is missed THEN the task SHALL iterate; bars are minimums, not targets to
   round down to.

### Requirement 24: Planner model contingency (risk C)
**User Story:** As an operator whose local model may be too weak, I want a contingency, so
intelligence is not permanently capped by local hardware.

#### Acceptance Criteria
1. WHEN the local model cannot meet the planner accuracy bar (Requirement 23.1) THEN a larger
   instruct model OR a cloud planner SHALL be selectable behind the SAME neutral Brain/planner
   trait, with no pipeline changes.
2. WHEN the contingency planner is used THEN it SHALL be a configuration choice, default to
   local, and fall back honestly when unavailable.
3. WHEN VRAM is assumed THEN the spec SHALL state the budget and co-residency (planner/text,
   vision, ComfyUI); the planner SHALL not be forced to co-reside with vision in a single
   turn (text↔vision mutual exclusion, Requirement 19.1).
4. WHEN the local model meets the bar THEN no cloud/external call SHALL be required (local
   stays the default; privacy preserved).

### Requirement 25: Labeled corpus artifact (risk D)
**User Story:** As the implementer, I want the proof corpus defined as data up front, so the
final gate isn't blocked by missing ground truth.

#### Acceptance Criteria
1. WHEN the harness is built THEN a `corpus.json` SHALL define ≥50 layman prompts, each with
   its category and an explicit, externally-verifiable expected state (window/URL/file/
   stdout/element).
2. WHEN a prompt is added THEN its expected state SHALL be expressed in terms the verifier
   registry can evaluate.
3. WHEN the corpus is used THEN it SHALL be the SAME data for per-task scenarios and the final
   gate (single source of truth).

### Requirement 26: Non-destructive test isolation (risk E)
**User Story:** As the implementer, I want repeatable tests that don't destroy my work, so
isolation is safe and reliable.

#### Acceptance Criteria
1. WHEN the harness normalizes state between prompts THEN it SHALL prefer non-destructive
   resets (fresh/scratch profile or session, save-less apps) over closing apps with unsaved
   work.
2. WHEN a reset could lose user data THEN the harness SHALL skip it and note the residual
   state rather than force-close.
3. WHEN isolation cannot guarantee a clean state THEN affected prompts SHALL be reported with
   the residual-state caveat (not a silent skew).

### Requirement 27: Environment-residual honesty & revert smoke (risks F, G)
**User Story:** As the product owner, I want honest reporting of environment-bound failures
and proof that every change can be turned off, so neither flakiness nor a bad task can trap me.

#### Acceptance Criteria
1. WHEN a failure is environment-bound (e.g. Wayland focus/click instability) and recovery
   cannot resolve it THEN it SHALL be reported INCONCLUSIVE/quarantined with the reason, never
   a bluffed PASS.
2. WHEN a category cannot reach its bar due to documented environment limits THEN the limit
   SHALL be recorded explicitly rather than hidden.
3. WHEN a behavioral task ships THEN a revert-smoke check SHALL prove that flipping its flag
   OFF restores the prior behavior without breaking earlier tasks.

### Requirement 28: Bridge destructive-action safety (risk H)
**User Story:** As a user, I want planner-generated shell/file actions to be just as safe as
any other, so a natural prompt cannot smuggle a destructive command.

#### Acceptance Criteria
1. WHEN the planner decomposes a task into a RunCommand/WriteFile sub-goal THEN that sub-goal
   SHALL pass through the unchanged safety gate before execution.
2. WHEN a bridged command is destructive (Red/Black) THEN it SHALL be blocked/HITL exactly as
   a non-bridged action would be — no new bypass.
3. WHEN this is tested THEN an explicit test SHALL prove a destructive bridged command is
   stopped by the gate.

---

## Generality Requirements (no selective apps/features — the headline product promise)

The single most important product property: GUI Cognition MUST be **app-agnostic and
feature-agnostic**. It must work on UNSEEN apps, UNSEEN prompts, and UNSEEN on-screen
options — not a curated set (Chrome / Calculator / Settings). It must NEVER rely on a
per-app or per-feature hardcoded recipe as its primary path. When something genuinely is
not possible (app not installed, option not on screen, manual/human step required), it must
say so honestly — never silently fail and never fake success. These requirements take
precedence over any narrower wording and are gated by generality-specific tests.

### Requirement 29: App-agnostic operation
**User Story:** As a user, I want any installed application to work, so I am not limited to a
hardcoded set.

#### Acceptance Criteria
1. WHEN any installed app is named THEN the system SHALL open/operate it using the live
   installed-app registry, NOT a fixed allow-list; adding a new app SHALL make it usable with
   no code change.
2. WHEN an app the system has never been tested against is used (e.g. Postman, an IDE, a
   media player) THEN the same Sight→Brain→Hands path SHALL apply with no app-specific branch.
3. WHEN app-specific logic is unavoidable THEN it SHALL be data/registry-driven (aliases,
   launch hints) and discoverable at runtime, NOT compiled per-app branches.
4. WHEN the test suite is assembled THEN it SHALL include apps OUTSIDE the common set to
   prove app-agnosticism (Requirement 33).

### Requirement 30: Feature-agnostic operation (any on-screen option)
**User Story:** As a user, I want any visible control/option to be actionable, so "open
history tab" works as well as "open a new tab".

#### Acceptance Criteria
1. WHEN a prompt references an on-screen control/option THEN the action SHALL be grounded in
   what Sight actually detects, so ANY visible control is targetable — not only those in a
   shortcut table.
2. WHEN the deterministic shortcut/keyword table is used THEN it SHALL be a FALLBACK only;
   the primary path SHALL be planner + grounded element selection (Requirement 4, 2).
3. WHEN a requested option IS visible THEN the system SHALL act on it regardless of whether it
   appears in any predefined list.
4. WHEN a feature works via a keyboard shortcut AND via a visible control THEN the system MAY
   use either, but its capability SHALL NOT be limited to the shortcut set.

### Requirement 31: Honest "not possible" handling
**User Story:** As a user, I want a truthful answer when something can't be done, so I trust
the assistant instead of getting silent failures or fake success.

#### Acceptance Criteria
1. WHEN a named app is not installed THEN the system SHALL clearly say it is not installed and
   suggest the nearest installed alternatives (Requirement 6.5) — never open a wrong app
   silently.
2. WHEN a requested option/control is NOT present on the current screen (after grounding)
   THEN the system SHALL say the option is not available here, rather than clicking a wrong
   target or claiming success (Requirement 3.2, 15).
3. WHEN a task is impossible or out of scope THEN the system SHALL explain why in one clear
   sentence, not loop or fabricate.
4. WHEN the system reports completion THEN it SHALL be backed by an external verifier
   (Requirement 15); an unverifiable outcome is INCONCLUSIVE, never a fake PASS.

### Requirement 32: Manual-step / human-in-the-loop handling
**User Story:** As a user, I want the assistant to pause and ask when a human step is needed
(login, captcha, payment, OS permission), so it doesn't get stuck or do something unsafe.

#### Acceptance Criteria
1. WHEN a step requires manual human action (login, credentials, captcha, 2FA, a system
   permission dialog) THEN the system SHALL detect the blocker, pause, and ask the user to
   complete it, then continue — rather than failing silently or guessing credentials.
2. WHEN waiting for the human step THEN the system SHALL make the request explicit and
   resumable (a single clear prompt), consistent with the minimal-HITL policy (Requirement
   10.3).
3. WHEN the human step cannot be detected automatically THEN a stall on a login/permission
   surface SHALL surface as an honest "looks like this needs your sign-in/permission" rather
   than a generic failure.
4. WHEN HITL is used here THEN it SHALL remain lightweight (one confirm/await), not a chain of
   approvals, and SHALL NOT tighten the safety gate (Requirement 10).

### Requirement 33: Generality is PROVEN, not assumed (negative + unseen corpus)
**User Story:** As the product owner, I want generality demonstrated by tests on unseen and
negative cases, so "works on anything" is evidence, not a claim.

#### Acceptance Criteria
1. WHEN the proof corpus is assembled THEN it SHALL include a dedicated `generality` category
   with: (a) UNSEEN apps outside the common set; (b) UNSEEN/implicit feature prompts (e.g.
   "open chrome and open the history"); (c) NEGATIVE cases — a nonexistent app and a
   nonexistent on-screen option; (d) a manual-step/login case.
2. WHEN a negative case runs THEN PASS means the system gave the correct HONEST response
   (not installed / option absent / needs login) — verified externally — NOT that it forced
   an action.
3. WHEN generality is gated THEN the negative/unseen cases SHALL be part of the final
   acceptance bar (Requirement 18.3) and SHALL NOT be excluded to inflate the pass rate.
4. WHEN a new app/feature is added later THEN re-running the suite SHALL require NO code
   change for the system to attempt it (proving the no-hardcode property).

### Requirement 34: Model selection for generality (switchable, quality-first)
**User Story:** As an operator, I accept switching the GUI-cognition LLM/vision model if it
yields better, more general behavior, so quality is not capped by a default.

#### Acceptance Criteria
1. WHEN a stronger model improves generality THEN the system SHALL allow selecting a different
   text and/or vision model for GUI cognition via configuration, behind the existing
   model-neutral Brain trait, with NO pipeline changes (Requirement 1, 12, 24).
2. WHEN the GUI-cognition model differs from the general chat model THEN the system SHALL
   support a GUI-cognition-specific model selection so chat and GUI can use different models.
3. WHEN a recommended model is chosen THEN it SHALL be a reliable, open-source/free option
   from a genuine source, documented with its license and provenance.
4. WHEN the default local model is insufficient for the generality bar THEN the contingency
   (larger/cloud, Requirement 24) SHALL apply; local remains the default when it meets the bar.
5. WHEN any new tool/tech/repo is introduced to achieve generality THEN it SHALL be
   open-source/free from a trustworthy source, pinned to a specific version, and recorded in
   the design with its provenance.
