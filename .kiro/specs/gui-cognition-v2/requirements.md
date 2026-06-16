# Requirements Document

## Introduction

K.R.I.A.'s current GUI Cognition pipeline is over-built on weak primitives: perception
relies on AT-SPI (weak on Wayland) plus a general VLM that does not reliably ground
elements to coordinates, while a large bespoke planner/validator/ladder stack sits on
top and frequently discards valid plans or falls back to a thin deterministic planner.
The result is unreliable execution of natural, unseen prompts.

This spec rebuilds GUI Cognition around three cleanly separated, independently testable
layers — **Sight** (perception/grounding via OmniParser), **Brain** (a pluggable
cognition layer, Qwen now, UI-TARS-ready later), and **Hands** (action execution via
uinput) — connected by a tight, bounded observe→decide→act→verify loop. Each layer is
built and tested in isolation first, then integrated incrementally. The new pipeline
(V2) runs behind a flag in parallel with the existing pipeline (V1); the over-built V1
logic is removed only after V2 is proven. Production strengths (safety/HITL, audit,
cancel/watchdog, screenshot-diff verification, the real-verify eval harness, uinput,
orchestrator model-swap) are preserved.

All work is flag-gated (default OFF for V2 until proven, then flipped; falsy env rolls
back byte-for-byte). New serialized fields use `#[serde(default)]`. No fabricated
results: anything not externally verified is reported INCONCLUSIVE.

## Glossary
- **Sight**: the perception layer that converts a screenshot into a structured
  `Observation` (elements with bounding boxes, kinds, labels) — OmniParser-backed.
- **Brain**: the cognition layer that, given a task + observation + history, returns one
  next `Decision`. Model-agnostic behind a trait; Qwen-backed initially.
- **Hands**: the action layer that executes a `Decision` (click/type/key/scroll) via the
  input substrate (uinput), with DPI/multi-monitor-correct coordinate mapping.
- **Observation**: a per-turn snapshot of the screen — screenshot + a list of detected
  elements, each with a per-observation `id`, `bbox`, `kind`, `label`, `confidence`.
- **Decision**: one bounded next action chosen by the Brain.
- **Set-of-Mark (SoM)**: overlaying numbered marks on detected elements so the model
  selects an element by id instead of predicting raw coordinates.
- **V1 / V2**: the existing GUI Cognition pipeline / the new Sight-Brain-Hands pipeline.

## Requirements

### Requirement 1: Cleanly separated, contract-driven layers
**User Story:** As a developer, I want Sight, Brain, and Hands isolated behind fixed
contracts, so I can build, test, debug, and scale each independently and swap the Brain
without touching the others.

#### Acceptance Criteria
1. WHEN the system is designed THEN Sight, Brain, and Hands SHALL each be defined as a
   separate Rust trait with no compile-time dependency on the others' implementations.
2. WHEN layers communicate THEN they SHALL exchange only the fixed `Observation`,
   `Decision`, and `ActionResult` data types.
3. WHEN a layer implementation changes THEN no other layer's code SHALL require changes
   to keep compiling.
4. WHEN Brain-specific logic is written THEN it SHALL live behind the Brain trait and
   SHALL NOT leak into the loop, Sight, or Hands.
5. WHERE a layer dependency is constructed THEN it SHALL be injected (constructor/builder)
   so tests can substitute fakes.

### Requirement 2: Sight — OmniParser perception (isolated)
**User Story:** As a user, I want KRIA to actually see the screen's interactable elements,
so it can target the right control instead of guessing.

#### Acceptance Criteria
1. WHEN a screenshot is provided THEN the Sight layer SHALL return an `Observation` whose
   `elements` each carry a per-observation `id`, `bbox`, `kind`, `label`, and `confidence`.
2. WHEN Sight runs THEN it SHALL optionally produce a Set-of-Mark annotated image with the
   element ids overlaid.
3. WHEN the OmniParser sidecar is unavailable THEN Sight SHALL degrade gracefully (return
   an honest empty/limited observation with a reason) and SHALL NOT crash the turn.
4. WHEN element labels are produced THEN they SHALL be treated as untrusted data and
   sanitized, and SHALL NOT be interpreted as instructions.
5. WHEN Sight is tested in isolation THEN feeding static screenshots SHALL yield a
   non-empty element set with a known control detected and correctly labeled, with no
   Brain or Hands involvement.
6. WHEN Sight returns coordinates THEN they SHALL be expressed so Hands can map them to
   physical screen pixels (origin, scale, monitor).

### Requirement 3: Brain — pluggable cognition (isolated)
**User Story:** As a user, I want a reasoning layer that picks the correct next action from
what's on screen, so natural and unseen prompts work without hardcoded recipes.

#### Acceptance Criteria
1. WHEN given a task, an `Observation`, and prior history THEN the Brain SHALL return
   exactly one `Decision` (Click, Type, Key, Scroll, Done, or Ask).
2. WHEN the Brain references a target THEN it SHALL use an element id from the CURRENT
   observation OR a raw screen point, and SHALL NOT invent a target absent from the
   observation.
3. WHEN the Brain is the Qwen implementation THEN it SHALL produce schema/grammar-valid
   output and SHALL NOT return prose.
4. WHEN no safe action can be chosen THEN the Brain SHALL return `Ask{question}` or `Done`,
   never a guessed action.
5. WHEN the Brain is tested in isolation THEN fixed (mock) observations SHALL drive
   decision-quality assertions for seen AND unseen tasks, with no real screen or execution.
6. WHEN a different Brain model is introduced (e.g. UI-TARS) THEN it SHALL implement the
   same trait and integrate without changes to Sight, Hands, or the loop.
7. WHERE the Brain needs visual input THEN it SHALL be able to consume the raw screenshot
   and/or the Set-of-Mark image, selecting what its model requires.

### Requirement 4: Hands — action execution (isolated)
**User Story:** As a user, I want clicks/keystrokes to land on the right place, so actions
actually take effect.

#### Acceptance Criteria
1. WHEN given a `Decision` and an `Observation` THEN Hands SHALL execute the action via the
   input substrate and return an `ActionResult` with success/error and a screen-changed
   signal.
2. WHEN the action is `Click{element_id}` THEN Hands SHALL resolve the id to the element's
   bbox center in physical pixels (DPI- and multi-monitor-correct) before clicking.
3. WHEN the action is `Click{point}` THEN Hands SHALL click the given physical point
   directly (supporting coordinate-emitting Brains like UI-TARS).
4. WHEN the action is `Key{combo}` THEN Hands SHALL map a standard, app-agnostic shortcut
   set (e.g. new tab, zoom, close tab) and SHALL NOT hardcode behavior for a specific
   prompt.
5. WHEN Hands is tested in isolation THEN fixed Decisions SHALL be executed and verified by
   an EXTERNAL observer (xdotool/wmctrl/screenshot-diff), with no Brain involvement.
6. IF the target element id is not present in the supplied observation THEN Hands SHALL
   fail explicitly and SHALL NOT click a fallback location.

### Requirement 5: Bounded observe-act loop
**User Story:** As a user, I want KRIA to take one verified step at a time and re-look,
so multi-step tasks complete reliably and never run away.

#### Acceptance Criteria
1. WHEN a turn runs THEN the loop SHALL repeat observe→decide→act→verify until the Brain
   returns `Done`/`Ask` or a bounded step cap is reached.
2. WHEN a step completes THEN the loop SHALL re-observe to obtain a FRESH observation before
   the next decision.
3. WHEN consecutive observations show no screen change after an action THEN the loop SHALL
   detect no-progress and stop with a clear reason (no infinite loop).
4. WHEN a cancel is requested THEN the loop SHALL stop before the next action.
5. WHEN a watchdog/step cap is exceeded THEN the loop SHALL abort with a sanitized reason.
6. WHEN a step's verification cannot confirm the expected effect THEN the loop SHALL report
   the step as INCONCLUSIVE rather than a false success.

### Requirement 6: Safety, HITL, and untrusted input
**User Story:** As a user, I want risky actions gated, so the agent never does something
destructive without my approval.

#### Acceptance Criteria
1. WHEN a decided action is classified risky (e.g. send, delete, pay, submit, credential
   entry) THEN the system SHALL require human approval (HITL) BEFORE Hands executes it.
2. WHEN approval is denied THEN the action SHALL NOT execute and the turn SHALL stop safely.
3. WHEN any external/observed text (labels, OCR) is consumed THEN it SHALL NOT alter the
   agent's instructions.
4. WHEN an action executes THEN it SHALL be recorded in the audit ledger.
5. WHEN the existing safety policy/HITL/audit/cancel facilities exist THEN V2 SHALL reuse
   them rather than reimplementing.

### Requirement 7: Natural and unseen prompts (no hardcoding)
**User Story:** As a user, I want KRIA to understand prompts it has never seen and act on
the live screen, so it doesn't feel like a fixed set of canned commands.

#### Acceptance Criteria
1. WHEN an unseen natural prompt is given THEN the Brain SHALL plan against the current
   observation rather than a per-prompt recipe.
2. WHEN a known fast-path (e.g. launch app, standard shortcut) applies THEN it MAY be used
   for speed, but SHALL NOT be the only path and SHALL NOT be prompt-specific hardcoding.
3. WHEN the prompt is ambiguous given the screen THEN the system SHALL ask a targeted
   clarification rather than guess.
4. WHEN evaluated THEN a held-out set of unseen prompts SHALL be exercised, not only the
   prompts used during development.

### Requirement 8: Resource and model strategy (6 GB GPU)
**User Story:** As an operator on a 6 GB GPU, I want Sight and Brain to fit together, so I
don't have to choose between seeing and thinking.

#### Acceptance Criteria
1. WHEN Sight and the Brain run THEN their combined resource use SHALL fit the target 6 GB
   GPU (OmniParser kept light; a single primary LLM resident).
2. WHEN the Brain can decide from element labels alone THEN it SHALL operate in a text-first
   mode and SHALL request the Set-of-Mark image only when needed.
3. WHERE a GUI-specialist Brain (e.g. UI-TARS) is selected THEN the system SHALL support
   swapping the resident model for the GUI turn and restoring it afterward, via the
   existing orchestrator model-swap.
4. WHEN model swap occurs THEN it SHALL be bounded and SHALL surface its state honestly.

### Requirement 9: Isolated-then-integrated testing
**User Story:** As a developer, I want each layer testable alone and then together, so I can
pinpoint the source of any failure.

#### Acceptance Criteria
1. WHEN any layer is built THEN it SHALL have isolation tests that exercise it without the
   other layers (fakes/fixtures for inputs).
2. WHEN Sight+Brain are integrated THEN a decision-only (no-execution) test SHALL confirm
   the Brain selects sane targets from real observations, attributing any failure to Sight
   (missing element) vs Brain (wrong pick).
3. WHEN the full loop is integrated THEN external real-verify tests SHALL confirm
   real-world effects (xdotool/wmctrl/filesystem/screenshot-diff), flagging MISMATCH when
   the reply claims success but reality disagrees.
4. WHEN a behavior cannot be verified on the current environment THEN it SHALL be documented
   INCONCLUSIVE, never reported as passing.
5. WHEN the build runs THEN core, desktop, and frontend test suites SHALL pass.

### Requirement 10: Parallel migration and overhead removal
**User Story:** As a maintainer, I want V2 to coexist with V1 and then replace the
over-built parts, so I get a clean single pipeline without a risky big-bang.

#### Acceptance Criteria
1. WHEN V2 is introduced THEN it SHALL run behind a flag in parallel with V1, and V1 SHALL
   remain the default until V2 is proven.
2. WHEN V2 is proven on the eval harness THEN the flag SHALL flip V2 to default with a
   documented env rollback to V1.
3. WHEN V2 is the default THEN the over-built V1 logic (dual plan representation,
   capability ladder, goal-pursuit guard, heavy upfront validators, upfront multi-step
   planning) SHALL be removed from code AND logic.
4. WHEN overhead is removed THEN the result SHALL have a single plan/decision representation
   and a single code path with no dead branches.
5. WHEN removal occurs THEN preserved facilities (safety/HITL, audit, cancel/watchdog,
   verification, eval harness, uinput, model-swap) SHALL remain intact and green.

### Requirement 11: Observability and honesty
**User Story:** As a developer and user, I want truthful per-layer telemetry, so I can see
what each layer did and trust "success".

#### Acceptance Criteria
1. WHEN a turn runs THEN the system SHALL emit per-layer telemetry (Sight observation
   summary, Brain decision + reason, Hands result, per-step verification).
2. WHEN events stream THEN they SHALL be incremental during the turn (not only a single
   end-of-turn batch).
3. WHEN a turn ends THEN the user-facing summary SHALL be layman-friendly (no internal
   hashes/jargon) unless developer mode is on.
4. WHEN any step is unverified THEN telemetry and summary SHALL say so honestly.
