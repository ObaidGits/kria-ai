# Design Document

## Overview

This upgrade turns GUI Cognition V2 from a blind, single-shot automaton into a
production-grade, **LLM-agnostic, planner-driven, vision-grounded** desktop agent that a
layman can drive entirely through prompts. It builds strictly on the existing
Sight / Brain / Hands loop (`crates/kria-core/src/agent/gui_cognition_v2/`) and its desktop
glue (`crates/kria-desktop/src/commands/gui_cognition.rs`) — no parallel stack.

The work is delivered as a sequence of small, independently-revertable, flag-guarded
changes. Each change is proven on the **running desktop** with an external verifier before
the next begins. Six confirmed defects are fixed first (perception, premature completion,
action sequencing, real planning, app resolution, resilience/recovery); a pluggable
**vision GUI model** upgrade is last.

### Design principles
- **Reuse over rebuild.** Extend existing modules (loop_engine, brain, sight, hands, app
  registry/launcher, orchestrator, event stream, eval harness). New files only where a
  genuinely new responsibility appears (planner, app resolver, live-proof harness).
- **Model-neutral.** No vendor-named symbols/files/config. The brain is a trait; the
  default text implementation is `LlmPlannerBrain` (in `llm_brain.rs`).
- **Frictionless defaults.** Grounding on, retries on, planner on — by default. No env
  setup, no extra approvals. Safety gate preserved, not tightened.
- **Proof or it didn't happen.** Every step has a live test that checks the real screen/
  window/process/file, independent of KRIA's self-report; INCONCLUSIVE when unverifiable.
- **No regressions.** A regression gate re-runs prior passing prompts each step.

### Flags & defaults (all optimized defaults; overrides optional, never required)
| Flag | Default | Controls |
|------|---------|----------|
| `KRIA_GUI_COG_SIGHT` | `auto` | `auto` (lazy grounding) / `grounded` / `light` |
| `KRIA_VISION_MODEL` | real detector (auto) | sidecar backend; dummy only if nothing installed |
| `KRIA_GUI_COG_PLANNER` | `on` | turn-start sub-goal decomposition; `off` = legacy per-step |
| `KRIA_GUI_COG_BRAIN` | `text` | `text` / `vision` (neutral; `qwen`/`ui_tars` accepted as aliases) |
| `KRIA_GUI_COG_BRAIN_TIMEOUT_SECS` | `60` | per-attempt decision budget |
| `KRIA_GUI_COG_MAX_STEPS` | `12` | hard loop cap |
| `KRIA_GUI_COG_TURN_BUDGET_MS` | `120000` | unified turn budget (steps+re-plan+recovery+retry) |

### Lazy-grounding guarantee (anti-regression)
`auto` is the production default and is **lazy**: the cheap light Sight serves
open/launch/key/type-into-focused intents with no OmniParser cost; grounded perception is
escalated ONLY when the Brain needs an on-screen control it cannot see. Grounding MUST NOT
add latency to or break pure-open prompts (Requirement 2, 19.4). The "open-only latency
unchanged" check is an explicit gate in the grounding task.

### Model residency & VRAM budget (anti swap-thrash)
A GUI turn keeps ONE brain model resident for its duration; text↔vision are mutually
exclusive within a turn. Documented assumption: planner/text (default) is resident during
text-brain turns; the vision model is loaded only for vision-brain turns and competes with
ComfyUI via the existing orchestrator evict/swap. The first brain call of a turn waits for
the model to be ready before issuing the request (Requirement 7.3, 19.1, 19.2). This
directly removes the observed `grammar chat transport error` swap-thrash root cause.

## Architecture

```
        GUI turn (manual_profile = gui_cognition)  ── natural-language prompt
                                   │
                    ┌──────────────▼───────────────┐
                    │  PLANNER (LLM)                │  R4
                    │  task → ordered sub-goals[]   │
                    │  + app resolution hints       │
                    └──────────────┬───────────────┘
                                   │ sub-goal cursor
   ┌──────────────────── BOUNDED OBSERVE→DECIDE→ACT→VERIFY LOOP ──────────────────────┐
   │ step k (for current sub-goal):                                                   │
   │  1. Sight.observe()  ── light by default                                         │
   │       └─ needs control & none seen? → observe_grounded() (lazy escalation)  R2   │
   │  2. Brain.decide(task, sub_goal, obs, history) → Decision (one action)      R1   │
   │  3. App resolver (for OpenApp): fuzzy/ambiguous → confirm or closest        R6   │
   │  4. Safety gate (existing, unchanged) → allow/deny                          R10  │
   │  5. Hands.execute() — incl. composite type→Enter / focus→type              R5   │
   │  6. Verify: re-observe + sub-goal satisfied?  → advance cursor             R3   │
   │  7. Stall? bounded recovery (re-ground / re-focus / alt action)            R8   │
   │  8. Brain/transport error? bounded retry (swap/health aware)               R7   │
   │  guards: step cap · cancel · all-sub-goals-done = complete                 R3   │
   └──────────────────────────────────────────────────────────────────────────────────┘
                                   │ per-step + plan events (stream)
                                   ▼
                    frontend GuiCognitionPanel (plan, sub-goals, confirms, recovery)  R11
```

## Components and Interfaces

### 1. LLM-agnostic brain (Requirement 1)
- Rename `gui_cognition_v2/qwen_brain.rs` → `llm_brain.rs`; `QwenBrain` → `LlmPlannerBrain`
  (semantic rename across the workspace so all references update).
- `label()` returns the served model id from the backend (e.g. `backend.model_label()`),
  not a `"qwen"` literal.
- `BrainChoice::Qwen` → `BrainChoice::Text`; `BrainChoice::UiTars` → `BrainChoice::Vision`.
  Keep `brain_choice_lookup` accepting legacy aliases (`qwen`→Text, `ui_tars`→Vision) so
  existing env values still work (R1.3, R13.1).
- Pure refactor — no behavior change; all existing brain tests pass (R1.4).

### 2. Sight grounding by default (Requirement 2)
- Keep the `HybridSight` (light + grounded) and lazy escalation already added.
- **Default backend**: desktop selects a REAL vision backend automatically:
  - Ensure the `kria-vision` sidecar is auto-started by the desktop runtime (reuse the
    existing sidecar supervisor) and `KRIA_VISION_MODEL` defaults to the lightweight real
    detector when weights are present; `vl7b` when a VL model is configured. `DummyOmniParser`
    is used ONLY as an explicit last-resort and is reported as degraded (R2.2, R2.3).
  - Endpoint/health surfaced via a `sight_status` accessor (R2.6) for the panel + tests.
- Lazy escalation trigger already present (`decision_needs_grounding`); extend to also fire
  when the Brain emits a click/find sub-goal whose target is not in the cheap observation.
- Coherent cache: a grounded observation is invalidated after any executed action (R2.4).
- Labels sanitized (existing `sanitize_label`); coordinates mapped to physical px
  (existing dims/monitor math) (R2.5).

### 3. Multi-step completion via sub-goal tracking (Requirement 3)
- Replace the loop's `last_opened_app` "X is already open → Completed" short-circuit with a
  **sub-goal cursor**: opening an already-open app marks the OPEN sub-goal satisfied and
  advances to the next sub-goal; the turn completes only when the cursor passes the last
  sub-goal (R3.1–R3.3).
- `TurnOutcomeV2` carries the sub-goal list + which were satisfied, for proof tests (R3.4).
- When the planner is off (fallback), retain a safe legacy completion that still does not
  end a known multi-part task on a duplicate open.

### 4. Planner — decompose natural/unordered prompts (Requirement 4)
- New module `gui_cognition_v2/planner.rs`: `Plan { sub_goals: Vec<SubGoal> }` where each
  `SubGoal { intent, kind, target_hint, done }`.
- `decompose(task, &backend) -> Plan` uses the configured LLM with a grammar/JSON-schema to
  return an ordered sub-goal list; order is inferred semantically (handles "open bookmarks
  tab in chrome" → [open chrome, open bookmarks]) (R4.1, R4.2).
- Cross-substrate routing: a sub-goal can be a GUI action OR a delegation hint (e.g.
  write-file / run-command) so prompts like the VS Code pascal-triangle case map to
  open editor → write file (file substrate) → run (shell) → surface output (R4.3). The
  loop executes GUI sub-goals; delegation hints route to the existing tool/substrate path.
- Per-step decide receives `(task, current_sub_goal, observation, history)`; planner can
  **re-plan** if verification shows the screen contradicts the plan (R4.4).
- The existing keyword helpers (`task_followup_action`, `task_open_app_target`) become a
  deterministic FALLBACK used only when the LLM planner is unavailable (R4.1).
- Bounded: one decomposition call per turn (+ at most one re-plan), schema-constrained,
  timeout-guarded; never loops (R4.6). Ask one question when no safe plan (R4.5).

### 5. Action sequencing & composites (Requirement 5)
- Extend the action vocabulary with composite, intent-level actions (additive to the
  existing enum, serde-default):
  - `TypeAndSubmit { text }` → type then Enter (URL/search/command).
  - `Navigate { url }` → focus address bar (ctrl+l) → type → Enter, new tab if asked.
  - `FocusThenType { target, text }` for fields.
- Brain prompt/system guidance updated (model-neutral) to: after typing a URL/command/search
  query, SUBMIT; never retype unchanged text; use address bar/new tab for navigation
  (R5.1–R5.3, R5.5).
- Hands maps composites to the existing uinput primitives (no new backend) (R5.4).

### 6. App-name resolver (Requirement 6)
- New module `gui_cognition_v2/app_resolver.rs` wrapping the EXISTING app registry/launcher
  (`tools/app_lifecycle.rs` / `open_application`):
  - `resolve(name) -> Resolution::{ Unique(app), Ambiguous(candidates), Closest(app, score),
    None(suggestions) }` using fuzzy/alias matching over the installed app list (R6.1).
  - Loop behavior: `Unique`/`Closest` → open (Closest states the choice); `Ambiguous` →
    emit an app-choice `Ask` (single confirm) surfaced in the panel; `None` → honest message
    + nearest suggestions (R6.2–R6.5).
- Reuses the registry; no parallel launcher (R6.6).

### 7. Resilience & serving stability (Requirement 7)
- Brain decide: retry on **transport/provider errors** too (not only timeout) with bounded
  backoff, calling the backend's swap/health wait before retry (mirror `local.rs`
  `wait_for_backend_ready`) (R7.1, R7.2).
- Before a turn's first brain call, request the planner/text model and `wait_for_swap`/ready
  so the model is resident for the turn; avoid vision↔text thrash within a turn (R7.3).
- Exhausted retries → honest, user-readable error; never a silent hang (R7.4). Retries
  logged + evented (R7.5).

### 8. No-progress recovery (Requirement 8)
- In the loop, before declaring `StoppedNoProgress`, run a bounded recovery ladder:
  1. escalate to grounded perception (if not already), re-decide;
  2. re-focus the intended target / try the composite variant;
  3. one alternate action.
- Recovery attempts capped; success continues the loop; exhaustion stops honestly
  (R8.1–R8.4). Recorded for the proof test.

### 9. Live proof-test harness (Requirement 9)
- Extend the existing live probe (`testing/tools/gui_cognition_live_eval.py`) into a
  **proof harness** that, per prompt, verifies the REAL outcome via external signals:
  - window present/focused (GNOME ext / wmctrl / `xdotool getactivewindow`),
  - screen content (screenshot + OCR / the sidecar `/parse` element check),
  - process/file effects (`pgrep`, file exists, command stdout),
  and classifies PASS / FAIL / INCONCLUSIVE (never fabricate) (R9.1–R9.3).
- A `regression_set` of prompts that passed in earlier steps is re-run each step; the gate
  fails if pass rate drops (R9.4).
- Single command entry point; captures logs + per-prompt JSON + screenshots as artifacts
  (R9.6). A per-task "flaw/vuln review" checklist is part of task done-criteria (R9.5).
- Backed by the existing Rust eval harness (`kria-eval`) for deterministic unit/integration
  coverage where a live screen is not needed.

### 10. Frictionless defaults & HITL (Requirement 10)
- Safety gate unchanged (`V2DesktopSafetyGate`, `assess_action_risk`): Green/Yellow auto,
  Red→single confirm, Black blocked. No new approval surfaces; no stricter rules (R10.2–R10.4).
- All new features default ON with optimized values; no required env (R10.1, R10.5).

### 11. Frontend (Requirement 11)
- `ui/` GUI Cognition panel (e.g. `components`/`views` for gui cognition) extended to:
  - render the sub-goal plan + per-sub-goal status from new plan events,
  - show grounding "looking closer" and recovery/retry as in-progress (not failure),
  - render the app-choice confirm inline,
  - reflect the true terminal status.
- New events are ADDITIVE; existing `gui_cognition:event` names preserved (R11.6). Map new
  core `LoopEvent`s (plan created, sub-goal done, app choice, recovery) to wire envelopes in
  `gui_cognition.rs` `v2_loop_event_to_wire`.

### 12. Pluggable vision GUI model (Requirement 12)
- Reuse the existing vision brain seam (`ui_tars_brain.rs`, to be renamed
  `vision_brain.rs`): implement the model-neutral Brain trait, ground from the screenshot,
  selectable via `KRIA_GUI_COG_BRAIN=vision`.
- Serve a SOTA grounding model (UI-TARS-1.5-7B / Qwen2.5-VL-7B / GUI-Actor-7B) on the vision
  route; fall back to text brain + grounded Sight when unavailable (R12.1–R12.3).
- Validated on the same live proof harness; reuses orchestrator swap/evict (R12.4, R12.5).

**IMPLEMENTED (Task 15) + PROVENANCE (Req 34.3/34.5):** `VisionBrain` (`vision_brain.rs`)
implements the neutral `GuiBrain` trait, attaches the live screenshot, and emits coordinate
actions (`click_point{x,y}`/`type`/`key`/`scroll`) validated to on-screen bounds (off-screen →
`Ask`). Selected via `KRIA_GUI_COG_BRAIN=vision`. The served grounding model is
**Qwen2.5-VL-7B-Instruct (Q4_K_M GGUF)** — **Apache-2.0**, at `~/.kria/models/llm/` — one of the
task's listed SOTA options. Because Qwen2.5-VL is MULTIMODAL, the SAME resident model serves
both the text brain (planner) and the vision brain, so there is NO text↔vision swap-thrash on
this setup (residency trivially satisfied). Graceful fallback: when no vision route is available
the desktop falls back to the text brain + grounded Sight (Req 12.3). When the vision brain is
active, the Task-9 planner/plan-mode is OFF — the vision brain grounds + decides directly from
pixels (no element list needed). UI-TARS-1.5-7B / GUI-Actor-7B can be swapped onto the same
vision route later with no pipeline change (the brain is model-neutral).

## Verification & Proof Architecture (loop-proofing)

This is the core of escaping the implement→break→replan loop: COMPLETION and PROOF are
defined as external-signal predicates, shared by the in-loop verifier AND the live harness.

### Per-sub-goal verifier registry (Requirement 15)
A `SubGoalVerifier` maps each `SubGoalKind` to an external-signal predicate, returning
`Verified | Failed | Unverified`:
- **OpenApp** → target window present + focused (GNOME ext `ListWindows`/`GetFocusedWindow`,
  or `wmctrl`/`xdotool`).
- **Navigate** → active window title/URL matches target (browser title via window query/OCR).
- **RunCommand** → command produced expected stdout/exit (bridge shell result, or terminal
  scrollback OCR).
- **WriteFile** → file exists with expected content (filesystem read).
- **Click/Toggle** → expected element/pane/state observable on re-observe (grounded Sight/OCR).
- **Type** → expected text present in the focused field (re-observe/OCR).
The loop marks a sub-goal done ONLY on `Verified`; `Failed`→recovery; `Unverified`→honest
INCONCLUSIVE (never silent pass). The SAME verifier code backs the live proof harness so
in-loop and test verdicts agree.

### Cross-substrate bridge (Requirement 16)
`SubstrateBridge` routes non-GUI sub-goals to existing executors:
- `RunCommand` → existing shell tool; `WriteFile` → existing file tool; `ReadOutput` →
  capture stdout/file. Routing is by `SubGoalKind` (explicit table, not heuristic). Results
  flow into a per-turn `WorkingContext` available to later sub-goals and the final reply.
  All bridged ops pass through the unchanged safety gate.

### Environment preflight (Requirement 14)
`scripts/gui_cog_preflight.*` (or a `kria-eval` subcommand): starts/health-checks desktop,
`kria-vision` sidecar, model server, uinput daemon, display; emits JSON
`{ ready, components:[{name,ok,detail,port,version}], reason }`. Live proofs refuse to run
unless the latest preflight is `ready:true`.

### Live proof harness & gates (Requirements 9, 18)
- Per prompt: run on the live desktop, evaluate the matching sub-goal verifiers, classify
  PASS / FAIL / INCONCLUSIVE; capture JSON + logs + screenshots to an artifacts dir.
- **Flakiness policy**: each prompt runs N times (default 3); majority rules; persistently
  variable prompts go to a `quarantine` list (reported, not gating).
- **Per-category baseline**: open-only, multi-step, navigation, app-resolution,
  cross-substrate — each with explicit counts; stored once.
- **Numeric gates**: each task states absolute pass bars (not "≥ prior"); final corpus ≥50
  layman prompts at ≥90% with zero real regressions.
- **Regression vs flakiness**: a real regression (deterministic drop across N runs) blocks;
  variance within the flakiness band does not.

### Test isolation (Requirement 9, 18.4)
Before each prompt the harness normalizes state (close target apps / fresh session / known
window set) so results are order-independent and "already open" carry-over cannot pollute
proofs.

### Planner quality gate (Requirement 17)
Offline: ≥40 labeled prompts (ordered/unordered/implicit/multi-substrate/ambiguous) with
expected decompositions; the planner must meet the accuracy bar BEFORE it drives the live
loop; until then the deterministic fallback stays active. Planner input is the user task
only; screen text is untrusted (injection hardening).

### Frozen event contract (Requirement 20)
The full `gui_cognition:event` schema is defined up front (additive). IMPLEMENTED (Task 2):
the complete `event.type` vocabulary is pinned in code as
`gui_cognition.rs::GUI_COGNITION_EVENT_TYPES` with a per-type canonical example
(`gui_cognition_event_example`) as the contract oracle, plus contract tests
(`frozen_event_vocabulary_snapshot_is_unchanged`, `every_emitted_event_type_is_in_the_frozen_vocabulary`,
`every_frozen_type_has_a_well_formed_example`, `additive_event_examples_carry_their_required_fields`).

Frozen `type` values + shapes:

| `type` | When | Required fields | Status |
|--------|------|-----------------|--------|
| `TurnStarted` | turn begins | `mode_id` | emitted |
| `ObservationStarted` | observe begins (also grounding "looking closer") | — | emitted |
| `ObservationCompleted` | observe done | `active_window`, `visible_control_count`, `degraded` | emitted |
| `PlanCreated` | brain decided (per-step rationale) | `summary`, `steps` | emitted |
| `SafetyGateCompleted` | gate allowed | `status`, `safety_status` | emitted |
| `ExecutionBlocked` | gate denied | `status`, `reason` | emitted |
| `ActionStarted` | hands begins | `action_kind`, `target` | emitted |
| `ActionCompleted` | hands ok | `status`, `backend_used` | emitted |
| `ActionFailed` | hands failed | `status`, `backend_used`, `error` | emitted |
| `VerificationCompleted` | screen changed (positive only) | `status` | emitted |
| `TurnCompleted` | done / needs_clarification | `status` | emitted |
| `TurnFailed` | any stopped_* terminal | `status`, `reason` | emitted |
| `SubGoalUpdated` | planner advances a sub-goal | `index`, `total`, `goal`, `status` | reserved (Task 9) |
| `AppChoiceRequested` | ambiguous app | `query`, `candidates[]` | reserved (Task 7/13) |
| `GroundingStatus` | sight backend live/degraded | `backend`, `live`, `degraded_reason` | reserved (Task 6/12) |
| `RecoveryAttempted` | no-progress recovery rung | `rung`, `ok` | reserved (Task 11) |
| `RetryAttempted` | brain transport/timeout retry | `kind`, `attempt` | reserved (Task 4) |

Evolution is ADDITIVE ONLY: append to the vocabulary; never rename/remove (Req 20.1, 20.2).
Backend emits against it (contract-tested); no existing names change.

### Pinned acceptance metrics (Requirement 23, risk B)
| Metric | Bar |
|--------|-----|
| Planner offline decomposition accuracy (fixtures) | ≥ 85% |
| Open-only category (live) | ≥ 95% |
| Multi-step category (live) | ≥ 85% |
| Navigation category (live) | ≥ 80% |
| App-resolution category (live) | ≥ 90% |
| Cross-substrate category (live) | ≥ 75% |
| Final ≥50-prompt corpus (overall, external-verified) | ≥ 90%, 0 real regressions |
| Latency SLO — open-only | ≤ 3s end-to-end |
| Latency SLO — multi-step | ≤ 8s per step (decide+act+verify) |
| Verifier calibration vs ground-truth labels | ≥ 95% agreement |

Latency SLOs are measured in steady state (model warm/resident). A cold first-turn model
load is excluded from the SLO and reported separately (mitigated by residency, Requirement
19.1).

### Verifier calibration & confidence (Requirement 22, risk A)
Each verifier carries a confidence score and is calibrated against a labeled ground-truth set
of screenshots/window states (≥95% agreement). Below the confidence threshold → INCONCLUSIVE.
Window/element queries settle (bounded retry) against a still-changing screen to avoid races.
The proof system is only trusted while calibration holds.

### Planner model & contingency (Requirement 24, risk C)
Default planner = local text model. If it cannot meet the ≥85% offline bar, a larger instruct
model OR a cloud planner is selectable behind the SAME neutral planner/Brain trait (config
choice, default local, honest fallback). VRAM assumption is stated; planner/text and vision
are mutually exclusive within a turn (no co-residence thrash). Local meeting the bar requires
no external call (privacy preserved).

### Non-destructive test isolation (Requirement 26, risk E)
Between prompts the harness prefers non-destructive resets (fresh/scratch profile or session,
save-less apps). If a reset could lose user data it is skipped and the residual state noted —
never a force-close of unsaved work. Unclean state → reported with caveat, not a silent skew.

### Environment-residual honesty (Requirement 27, risk F)
Environment-bound failures (Wayland focus/click instability) that recovery cannot resolve are
reported INCONCLUSIVE/quarantined with the reason — never a bluffed PASS. Documented category
limits are recorded explicitly.

## Generality Strategy (app- & feature-agnostic — the headline promise)

The system must work on UNSEEN apps/prompts/options and answer honestly when something is
not possible. Generality is achieved by REMOVING per-app/per-feature hardcoding and replacing
it with runtime perception + reasoning + registry lookup, all verified by generic predicates.

### The four generality pillars (and where each lives)
1. **Perceive anything (Sight).** Detect real on-screen controls for ANY app via a layered,
   app-agnostic perception stack (no per-app code):
   - **AT-SPI2 accessibility tree** (Linux a11y) — fast, GPU-free, structured roles/labels for
     toolkit apps; already partially wired (`atspi_engine.rs`). License: LGPL (system lib).
   - **OmniParser v2** (icon/region detection) in the `kria-vision` sidecar — visual fallback
     for apps the a11y tree misses (e.g. canvas/Electron). PROVENANCE (Task 6, downloaded):
     `microsoft/OmniParser-v2.0` →
     `icon_detect/model.pt` (39 MB, YOLOv8 icon detector, **AGPL-3.0** — copyleft, see flag below)
     and `icon_caption/model.safetensors` (1.1 GB Florence-2 fine-tune, **MIT**). Stored at
     `models/omniparser/`. Default labels come from fast OCR; caption is opt-in (CPU cost).
   - **OCR (tesseract, Apache-2.0)** — text reading for "describe the screen" and text checks.
   - **Vision GUI model** (Task 15) — for direct coordinate grounding when needed.
   These are merged into one `Observation`; the Brain consumes elements regardless of source.
2. **Reason generally (Brain + Planner).** The planner (Task 9) decomposes ANY natural prompt
   into sub-goals and the Brain picks actions from the GROUNDED elements — never a per-feature
   recipe. The deterministic keyword/shortcut table is a FALLBACK only (Requirement 30.2).
3. **Resolve apps from the live registry (Task 7).** App names map to installed apps via the
   live registry + fuzzy/alias matching — adding an app needs no code (Requirement 29.1).
4. **Verify generically (Task 1).** Verifiers are keyed by sub-goal KIND, not app, so any
   app/feature is provable (Requirement 15). Honest verdicts (Verified/Failed/Unverified)
   feed the honest-reply behavior.

### Honest "not possible" paths (Requirement 31, 32)
- App not installed → resolver `None` → "not installed; nearest: …".
- Option not on screen (after grounding) → no-invented-target → "that option isn't here".
- Manual/human step (login/captcha/2FA/permission) → detect the blocker surface (a11y/OCR
  signals like "Sign in", "Password", permission dialog) → pause + single HITL ask → resume.
- Anything unverifiable → INCONCLUSIVE, never fake PASS.

### Open-source tools/techs introduced (all free, trustworthy, version-pinned in impl)
| Tool | Use | License / source |
|------|-----|------------------|
| AT-SPI2 (atspi) | a11y element tree (app-agnostic, GPU-free) | LGPL — freedesktop.org |
| OmniParser v2 icon_detect | visual element/icon detection fallback | **AGPL-3.0** — Microsoft/Ultralytics (YOLOv8) |
| OmniParser v2 icon_caption | optional region captioning (opt-in) | MIT — Microsoft (Florence-2 ft) |
| tesseract | OCR (describe-screen, text verification) | Apache-2.0 — Google/community |
| Qwen2.5-VL / UI-TARS-1.5 / GUI-Actor (one, Task 15) | vision GUI grounding model | Apache-2.0 / MIT |
Each is pinned to a specific version and recorded with provenance when added (Requirement 34.5).

> **Licensing flag (Task 6, AGPL):** the bundled OmniParser `icon_detect` weights are
> **AGPL-3.0** (YOLOv8 lineage). AGPL is fine for local/personal use, but if KRIA is
> distributed OR offered as a network service, AGPL §13 source-availability obligations
> attach to that component. Mitigations if that becomes a concern: (a) ship without the
> detector weights and let the user opt in to download; (b) swap to a permissively-licensed
> detector; or (c) move grounding to the Task-15 vision brain (Apache-2.0/MIT). The
> `icon_caption` weights are MIT and unaffected. Flagged here for an eyes-open decision.

### Model selection for GUI cognition (Requirement 34)
GUI cognition may use a DIFFERENT text/vision model than general chat, selected via config
behind the model-neutral Brain trait. Default local; if it misses the generality bar, the
contingency (larger/cloud) applies (Requirement 24). Switching the model requires no pipeline
change.

## Data Models

All additions are additive and serde-default.
```rust
// planner.rs
pub struct SubGoal { pub intent: String, pub kind: SubGoalKind, pub target_hint: Option<String>, pub done: bool }
pub enum SubGoalKind { OpenApp, Click, Type, Navigate, RunCommand, WriteFile, Verify, Other }
pub struct Plan { pub sub_goals: Vec<SubGoal> }

// types.rs (additive Action variants)
Action::TypeAndSubmit { text }
Action::Navigate { url }
Action::FocusThenType { target, text }

// loop_engine.rs (additive outcome fields)
TurnOutcomeV2 { /* … */ plan: Option<Plan>, satisfied_sub_goals: usize }

// app_resolver.rs
pub enum Resolution { Unique(AppRef), Ambiguous(Vec<AppRef>), Closest(AppRef, f32), None(Vec<AppRef>) }

// verifier.rs (Requirement 15) — shared by loop AND live harness
pub enum VerifyVerdict { Verified, Failed(String), Unverified(String) }
pub trait SubGoalVerifier { async fn verify(&self, sub_goal: &SubGoal, ctx: &WorkingContext) -> VerifyVerdict; }

// bridge.rs (Requirement 16)
pub struct WorkingContext { pub outputs: Vec<SubGoalOutput> } // file paths, stdout, exit codes
pub enum BridgeRoute { Gui, Shell, File, ReadOutput }

// budget.rs (Requirement 19) — unified turn budget accounting
pub struct TurnBudget { pub max_steps: u32, pub max_replans: u32, pub max_recoveries: u32, pub max_retries: u32, pub deadline_ms: u64 }

// preflight JSON (Requirement 14)
// { "ready": bool, "components": [{ "name", "ok", "detail", "port", "version" }], "reason": "" }
```

## Error Handling
- Sight degrade → honest reason; loop continues with light view or asks (R2.3).
- Planner failure → fallback heuristic plan, else single clarifying Ask (R4.5).
- Brain transport/timeout → bounded retry, then honest stop (R7).
- App resolve ambiguity → single confirm; none → suggestions (R6).
- Every terminal state maps to a user-readable reply + a status the proof test asserts.

## Correctness Properties

These invariants must hold for every GUI turn and are asserted by unit/integration tests
and the live proof harness:

### Property 1: No invented targets
A click/type targets only an element present in the CURRENT observation or a resolved
point; an absent target downgrades to Ask (never a guess).
**Validates: Requirements 2.1, 6.1**

### Property 2: Full completion
A turn reports `Completed` only when ALL planned sub-goals are satisfied; a duplicate open
never ends a multi-part task (Requirement 3).
**Validates: Requirements 3.1, 3.2, 3.4**

### Property 3: Bounded everything
Planner decomposition, re-plan, brain retries, recovery ladder, and the step loop are each
hard-capped — no path can loop infinitely.
**Validates: Requirements 4.6, 7.1, 8.4**

### Property 4: Honest perception
When grounding is unavailable the turn degrades with a stated reason; it never silently
behaves as if blind, and never fabricates elements.
**Validates: Requirements 2.2, 2.3**

### Property 5: Submit-or-don't-repeat
Text meant to be submitted is followed by Enter; unchanged text is never blindly re-typed
(no concatenation).
**Validates: Requirements 5.1, 5.3**

### Property 6: Proof over self-report
A live outcome is PASS only if an external signal confirms it; otherwise INCONCLUSIVE,
never a fabricated pass.
**Validates: Requirements 9.2, 9.3**

### Property 7: No regressions
Each task keeps the regression pass rate at or above the prior task.
**Validates: Requirements 9.4, 13.2**

### Property 8: Reversibility
Every behavioral change is flag/default-guarded and rolls back without breaking earlier
tasks; new serialized fields default safely.
**Validates: Requirements 13.1, 13.2**

### Property 9: Untrusted screen text
OCR/element labels and on-screen text are data, never instructions, at every layer (Sight,
planner, brain).
**Validates: Requirements 2.5, 4.5**

### Property 10: Safety preserved, not tightened
The existing gate/blacklist classification is unchanged; benign actions execute without
approval.
**Validates: Requirements 10.2, 10.4**

### Property 11: Verified completion only
A sub-goal is marked done only on an external-signal `Verified` verdict; the turn completes
only when every sub-goal is verified-done; unverifiable outcomes are INCONCLUSIVE, never a
silent pass.
**Validates: Requirements 15.1, 15.2, 15.3**

### Property 12: Shared verifier identity
The in-loop verifier and the live-proof harness use the SAME predicate code, so what the
loop believes and what the test proves cannot diverge.
**Validates: Requirements 15.5, 9.2**

### Property 13: Preflight-gated proof
A live proof run produces results only when the latest environment preflight is
`ready:true`; otherwise it refuses and reports, never fabricates.
**Validates: Requirements 14.4, 9.3**

### Property 14: Objective, category-wise gates
Every task passes a NUMERIC acceptance bar; the final corpus (≥50 prompts) meets ≥90% with
zero real regressions, measured per category with a flakiness policy that separates variance
from regression.
**Validates: Requirements 18.2, 18.3, 18.5**

### Property 15: No swap-thrash within a turn
One brain model stays resident per turn (text↔vision mutually exclusive); the first call
waits for ready — eliminating mid-turn transport drops.
**Validates: Requirements 19.1, 7.3**

### Property 16: Bounded unified budget
Steps, re-plans, recoveries, and retries draw from one accounted budget; combined activity
cannot silently exhaust the step cap, and nothing hangs past the turn deadline.
**Validates: Requirements 19.3, 19.5**

### Property 17: Focused typing
Text is typed only after the intended field/window is confirmed focused; unchanged text is
never re-typed.
**Validates: Requirements 21.1, 21.3**

### Property 18: Frozen additive events
The event schema is fixed up front; backend conforms (contract-tested) and no existing event
name is renamed or removed.
**Validates: Requirements 20.1, 20.2, 20.3**

### Property 19: Calibrated, confidence-gated verifiers
Verifiers agree ≥95% with ground-truth labels and attach confidence; low-confidence verdicts
are INCONCLUSIVE, and queries settle to avoid race-induced false verdicts.
**Validates: Requirements 22.1, 22.2, 22.3, 22.4**

### Property 20: Pinned bars met
Every numeric bar (planner ≥85%; categories 95/85/80/90/75; corpus ≥90%/0-regress; latency
SLO) is an enforced minimum gate, not a soft target.
**Validates: Requirements 23.1, 23.2, 23.3, 23.4**

### Property 21: Capability contingency without lock-in
If the local model cannot meet the planner bar, a larger/cloud planner plugs in behind the
same trait; local stays default and no external call is required when local suffices.
**Validates: Requirements 24.1, 24.2, 24.4**

### Property 22: Safe isolation & honest residuals
Test isolation is non-destructive (no unsaved-work loss); environment-bound failures are
INCONCLUSIVE/quarantined, never bluffed PASS; every behavioral flag has a proven OFF path.
**Validates: Requirements 26.1, 27.1, 27.3**

### Property 23: Bridge actions are gate-equal
Planner-generated shell/file sub-goals pass the unchanged safety gate; a destructive bridged
command is blocked/HITL exactly like any other action.
**Validates: Requirements 28.1, 28.2, 28.3**

### Property 24: App-agnostic (no allow-list)
Any installed app is operable via the live registry + the same Sight→Brain→Hands path; adding
an app requires no code change, and unseen apps use no app-specific branch.
**Validates: Requirements 29.1, 29.2, 29.3**

### Property 25: Feature-agnostic (any visible control)
Any control Sight detects is targetable; the shortcut/keyword table is fallback-only and never
caps capability.
**Validates: Requirements 30.1, 30.2**

### Property 26: Honest impossibility
A missing app, a missing on-screen option, or an out-of-scope task yields a clear honest
response (not installed / option absent / why-not) — never a wrong action or fake success.
**Validates: Requirements 31.1, 31.2, 31.4**

### Property 27: Human-step handling
A manual step (login/captcha/2FA/permission) is detected and pauses for a single HITL ask,
then resumes — never a silent fail or guessed credential.
**Validates: Requirements 32.1, 32.2, 32.3**

### Property 28: Generality is proven
The acceptance corpus includes unseen apps, unseen/implicit features, negative cases, and a
manual-step case; these are part of the final bar and cannot be excluded to inflate results.
**Validates: Requirements 33.1, 33.2, 33.3**

## Testing Strategy
- **Unit/integration (kria-core, deterministic):** planner decomposition (ordered/unordered
  fixtures), sub-goal completion, composite action mapping, app resolver matching, retry on
  transport vs timeout, recovery ladder, brain rename parity.
- **Live proof (running desktop):** per-task scenario prompts verified by external signals;
  regression set re-run each task; artifacts captured. Layman scenarios in the final suite:
  "open bookmarks tab in chrome", "open terminal and run ls", "open calculator and compute
  256 times 13", "write a pascal's triangle program in VS Code, run it and show output",
  mistyped/ambiguous app names.
- **No-regression gate:** pass rate must not drop between tasks.
