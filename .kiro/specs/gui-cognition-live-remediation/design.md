# Design Document

## Overview

This design fixes the **live** GUI Cognition execution path on Wayland/GNOME. It is organized as
seven sequential, live-gated phases, each addressing one root cause from the live True-Test
(`planning_docs/gui_cognition_user_truetest_results.md`). Work is surgical and flag-gated; the
deterministic T2 tiers stay green and flag-OFF preserves current behavior byte-for-byte.

## Architecture

The fixes operate inside the existing `kria-core` GUI Cognition runtime
(`crates/kria-core/src/agent/gui_cognition/`) and its desktop wiring, preserving the authoritative
flow **Intent → Capability → Policy → Substrate → Tool → Verification**. No new top-level component
is introduced; each phase hardens one stage of that pipeline:

- **Perception/observation** (`perception.rs`): adds a window-present probe + app alias map.
- **Planning** (`llm_planner.rs`): adds the auto-prerequisite (open/focus) step and ambiguity
  signalling input.
- **Resolution** (`resolver.rs`): honors the prerequisite, surfaces multi-candidate ambiguity, and
  resolves the active scrollable surface.
- **Execution** (`executor.rs`, `window_focus.rs`): implements real Wayland window activation and
  context-aware scroll/key-press.
- **Verification** (`verifier.rs`): corrects the app-launch predicate/evidence with a bounded
  readiness wait and honest `inconclusive`.
- **Safety** (`safety_polish.rs`): wires ambiguity→ask and context-aware key-press gating.

All phases are gated by the existing feature flags (`gui_cog_*`) constructed in
`crates/kria-desktop/src/commands/gui_cognition.rs`; flag-OFF restores prior behavior.

## Components and Interfaces

- **Verification contract** (`verifier.rs`): `select_verification_strategy(action)`,
  `verification_contract_for(...)`, `evidence_source_for_strategy(...)`, post-action verify +
  `apply_verification_contract`. Phase 1 changes the `OpenApp` predicate to `WindowVisible`.
- **Window-present probe** (`perception.rs`): reads the desktop open-window set; new tolerant app
  alias matcher.
- **Plan builder** (`llm_planner.rs`): post-process that prepends an inferred `OpenApp`/
  `SwitchWindow` prerequisite or converts to `AskClarification`.
- **Target resolver** (`resolver.rs`): `should_defer_until_planned_app`, multi-candidate ambiguity
  signal, active-surface resolution.
- **Window focus** (`window_focus.rs` + `mod.rs::window_focus_backend_available`): real
  `GnomeBridge`/`Portal` activate-by-identity handlers + truthful availability.
- **GNOME shell D-Bus bridge:** extended with activate-by-window-identity (already used to read the
  active window).
- **App registry** (`platform/app_registry.rs`): `gnome-control-center` entry + aliases.
- **Live test harness:** `testing/tools/gui_cognition_capability_audit.py` (`send`, `judge`,
  `detect_leaks`, `gui_of`) driving `POST /api/testing/desktop-chat-command`.

## Data Models

- **`GuiVerificationStrategy`** (enum): `WindowVisible | ActiveWindowMatch | FocusedControl |
  TextPresent | ScreenChanged | ...` — Phase 1 maps `OpenApp → WindowVisible`.
- **`GuiVerificationEvidenceSource`** (enum): `Observation | Accessibility | ActiveWindowProbe |
  BackendReceipt | None`.
- **`GuiVerificationContract`**: `{ action_type, predicate, evidence_source, bounded_wait_ms,
  max_reobserve, min_confidence }`.
- **`GuiTypedPlanStep`**: `{ step_type, target_app_hint, target_window_hint, target_control_hint,
  payload, verification_strategy, idempotent, ... }` — Phase 2 prepends a prerequisite step.
- **`WindowFocusBackend`** (enum): `GnomeBridge | Portal | UinputAltTab | X11Wmctrl` +
  availability predicate.
- **App alias map**: `{ canonical_app → [aliases] }` (chrome/chromium/google-chrome,
  files/nautilus, terminal, gedit/text editor, gnome-calculator, gnome-control-center/settings).

## Correctness Properties

### Property 1: Honest verification
A step is reported `verified` ONLY with real post-action evidence; weak evidence → `inconclusive`,
never a false `verified`.
**Validates: Requirements 1.4, 1.1**

### Property 2: Flag-OFF byte-for-byte
Flag-OFF is byte-for-byte identical to current behavior (asserted per phase).
**Validates: Requirements 1.5, 2.4, 3.4**

### Property 3: Zero destructive-leak
No unrequested destructive action executes, enforced by the audit's leak detector on every live
gate.
**Validates: Requirements 8.2**

### Property 4: Truthful backend availability
A backend is reported available ONLY when genuinely reachable; otherwise a truthful error.
**Validates: Requirements 3.1, 3.3**

### Property 5: Bounded cognition
Every turn stays within Task 1 bounded caps (no unbounded wait/loop).
**Validates: Requirements 1.2**

## Error Handling

- **No activation path:** `SwitchWindow` returns a truthful actionable error (no blind Alt+Tab fake
  success).
- **App not inferable:** bare primitive converts to `AskClarification` instead of guessing.
- **Window never appears within readiness budget:** verdict is `inconclusive`/`failed` with a safe
  explanation, recovery bounded (idempotent-only single retry).
- **App not in registry:** honest "app not found" rather than silent no-progress where possible.

## Testing Strategy

- **CI-safe (T2)** unit/integration per phase: predicate/evidence, alias match, prerequisite
  insertion, backend selection, registry resolve, ambiguity→ask, and flag-OFF byte-for-byte.
- **Live gate per phase:** re-run ONLY that phase's prompt subset through the real desktop endpoint;
  must hit the phase gate (executed+verified or correct ask/gate), 0 leak, no prior regression.
- **Final acceptance:** full 112-prompt live re-run with before/after; inherently live-dependent
  prompts reported honestly.

### Live test harness (shared by every phase gate)

A scoped live runner drives the running desktop exactly as the UI does:
`POST /api/testing/desktop-chat-command` with `manual_profile.mode_id=gui_cognition` and
`gui_cognition_test={execution_mode:"execute_live", workflow:true}`, **no** auto-approve fixture on
the real session. It reuses the production audit's scoring (`testing/tools/gui_cognition_capability_audit.py`:
`send`, `judge`, `detect_leaks`, `gui_of`). Each phase re-runs ONLY its relevant prompt numbers and
must hit its gate before the next phase begins.

Preconditions for any live gate: desktop healthy (`/api/health` 200), `gui-automation-status`
reports `can_execute_actions=true`, uinput daemon + vision sidecar running.

### Verdict semantics (unchanged from the audit)
- **PASS** = action executed AND `verified` (or correct gate/ask for approval/ambiguity/boundary).
- **PARTIAL** = executed but not verified.
- **FAIL** = blocked / no progress / wrong behavior.

---

## Phase 0 — Shared multi-backend structured-output adapter (Requirement 0, TRUE FIRST BLOCKER)

**Why:** Live debug showed the cloud planner route (`opencode`/`deepseek-v4-flash-free`) returns
prose and is rejected every turn because `LlmBackend::chat_with_grammar` falls back to an
unconstrained `chat` for cloud backends and `supports_grammar` is binary-`false` for cloud. The
local llama.cpp path already posts grammar. The fix gives BOTH local and cloud a schema-valid typed
plan via the strongest method each backend honors.

**Files:** `crates/kria-core/src/llm/mod.rs` (`LlmBackend` trait: add `StructuredOutputMode` +
`structured_output_mode()`, keep `supports_grammar`), `crates/kria-core/src/llm/cloud.rs`
(OpenAI-compatible `response_format`/tool-calling + capability probe), `crates/kria-core/src/llm/local.rs`
(map grammar), `crates/kria-core/src/agent/gui_cognition/llm_planner.rs` (capability mapping +
adapter use + bounded re-ask), desktop flag wiring `crates/kria-desktop/src/commands/gui_cognition.rs`.

**Design:**
- `StructuredOutputMode { Grammar, JsonSchema, JsonObject, ToolCalling, None }`; `structured_output_mode()`
  default `None`; `supports_grammar() == (mode == Grammar)` for back-compat.
- Cloud adapter posts the strongest honored constraint; result normalized to a JSON object
  (tool-calling extracts `tool_calls[0].arguments`). Non-streaming for the structured request.
- Cheap cached capability probe per provider+model (proxies like opencode/zen may strip
  `response_format`; never assume — detect, and allow a per-provider `structured_output` config
  override).
- Universal safety net: strict serde validate + bounded re-ask (1→2) with the validation error fed
  back; bounded by Task 1 caps; never lenient-scrape.
- GUI planner adopts the adapter first, flag-gated (`gui_cog_structured_planner`); flag-OFF =
  byte-for-byte prior behavior; other features' `chat` path untouched.

**Known flaws addressed (plan re-check):**
- *Overpromise of "same response":* parity is schema/typed-plan + functional, NOT byte-identical —
  encoded in Requirement 0.5; the live gate never asserts identical text.
- *Proxy capability unknown (opencode/zen):* runtime probe + config override + re-ask safety net, so
  an endpoint that honors no structured mode still degrades to validate-and-re-ask honestly.
- *Tool-calling shape differs:* adapter normalizes `tool_calls` → JSON object.
- *Streaming conflict:* structured request is non-streaming.
- *Re-ask latency/budget:* re-ask capped at 1–2 and bounded by Task 1 timeouts.
- *Context budget:* compact schema + a single few-shot to fit 4096 ctx.
- *Regression risk:* additive trait method + flag-gated; existing `chat` unchanged.
- *Scope:* Task 0 success = correct typed plan + valid JSON on both backends; actually launching the
  app + verifying it is Phase 1's job (no coupling).

**Tests:** CI (mock OpenAI server) per 0.7; live gate per 0.8.

## Phase 1 — Verification predicate for app-launch (Requirement 1)

**Files:** `verifier.rs` (predicate/evidence + downgrade), `perception.rs` (window-present probe),
`mod.rs` (readiness wait wiring).

**Design:**
- `select_verification_strategy(OpenApp)` → `WindowVisible` (already an enum variant) instead of
  `ActiveWindowMatch`. `evidence_source_for_strategy(WindowVisible)` → `Observation`/desktop-state.
- New presence check: app window appears in the desktop open-window set (title/app_name), using a
  tolerant alias map (`chrome`↔`chromium`↔`google-chrome`, `files`↔`nautilus`, `gnome-terminal`↔
  `terminal`, `gedit`/`text editor`, `gnome-calculator`/`calculator`, `gnome-control-center`/
  `settings`).
- Bounded readiness wait before the verdict: re-observe up to `max_reobserve` (Task 1 caps) until
  the window appears or the budget is exhausted.
- `SwitchWindow` verification stays `ActiveWindowMatch` (correct for switch) — it only passes after
  Phase 3 lands the real activation; do not touch it here.
- Honest `inconclusive` retained for genuinely weak evidence.
- Gated behind `gui_cog_safety_polish` (extend) or a focused sub-flag `gui_cog_verify_live`; OFF =
  prior verdict byte-for-byte.

**Tests:** unit (predicate/evidence/alias/flag-OFF) + live re-run #1–6, 41, 42, 46, 47, 70, 80,
62–65, 73, 74, 78, 79, 97, 98.

**Gate-1:** open-app family 6/6 (excl. #7 settings → Phase 6); previously PARTIAL app-launch prompts
≥ 80% now executed+verified; 0 leak; flag-OFF unchanged.

---

## Phase 2 — Auto-prerequisite (open/focus the target app) (Requirement 2)

**Files:** `llm_planner.rs` (plan post-process / deterministic builder), `resolver.rs`
(`should_defer_until_planned_app`), `platform/app_registry.rs` + intent extractor (app inference).

**Design:**
- Plan post-process: if the first executable step is a control/type/click whose target app is NOT
  observable in the current `GuiContext`, infer the app-kind from the intent
  (address-bar/search-box→browser; Save/text/note→active or default text editor; file-manager
  search→file manager) and **prepend** an `OpenApp` (or `SwitchWindow` if open-but-unfocused)
  prerequisite.
- If the app cannot be confidently inferred → convert to `AskClarification` (no wrong guess).
- After the prerequisite, re-observe (Task 3 `gui_cog_reobserve`) and resolve the next step against
  fresh context.
- Gated behind `gui_cog_smart_planner` / `gui_cog_step_completeness`; OFF = prior plan unchanged.

**Tests:** unit (prerequisite inserted; ambiguous→ask; flag-OFF) + live re-run #12–25, 32–40, 53,
59, 60, 43, 44, 52, 61, 67, 68, 69, 110.

**Gate-2:** these primitive families ≥ 80% PASS (or correct ask); Phase-1 results intact; 0 leak.

---

## Phase 3 — Real Wayland window activation (Requirement 3)

**Files:** `window_focus.rs` (handlers), `mod.rs` (`window_focus_backend_available`), GNOME shell
D-Bus bridge (already used for active-window read) extended with activate-by-identity.

**Design:**
- Implement `GnomeBridge` activate: resolve window identity (app_name+title) → window id via the
  GNOME shell bridge; call activate. Implement `Portal` activate where available.
- `window_focus_backend_available(GnomeBridge)` = true **only** when the bridge is probed reachable;
  otherwise false (truthful).
- Route `SwitchWindow` through the implemented backend; verify by re-observe active == requested.
- No real path → truthful actionable error (no blind Alt+Tab fake success).
- Gated behind `gui_cog_wayland_focus`; OFF unchanged.

**Tests:** unit (backend selection prefers reachable GnomeBridge; switch verify) + live re-run
#8–11, 48–50, 71, 82.

**Gate-3:** switch-window family ≥ 80% PASS (executed+verified); honest error when no bridge; Phases
1–2 intact; 0 leak.

---

## Phase 4 — Scroll / context-dependent execution (Requirement 4)

**Files:** `executor.rs` (scroll), `resolver.rs` (active surface), `mod.rs`.

**Design:** resolve the active window/scrollable surface (from Phase 1 window-present + Phase 3
active-window); `Scroll` executes + verifies `screen_changed`; no surface → observe/ask.

**Tests:** unit + live re-run #30, 31, 49, 77.

**Gate-4:** scroll family ≥ 80% PASS; prior phases intact; 0 leak.

---

## Phase 5 — Context-aware key-press (Requirement 5)

**Files:** `safety_polish.rs` / safety policy (PressKey gate), `mod.rs`.

**Design:** with a resolved editable/focused context, `PressKey` is allowed + verified
(`screen_changed`); without context, high-impact keys stay gated/asked (correct).

**Tests:** unit (with/without context) + live re-run #26–29 (standalone vs after-focus).

**Gate-5:** context key-press executes+verifies; standalone safe-gating documented correct; prior
intact.

---

## Phase 6 — System settings registry (Requirement 6)

**Files:** `platform/app_registry.rs`.

**Design:** add `gnome-control-center` entry + aliases ("system settings","settings","control
center"); OS-detected launch command. Settings-search reuses Phase 2 path.

**Tests:** unit (registry resolve) + live re-run #7, 56–58, 75, 76, 112.

**Gate-6:** settings family ≥ 80% PASS; prior intact.

---

## Phase 7 — Ambiguity → ask (Requirement 7)

**Files:** `resolver.rs` (multi-candidate signal), `safety_polish.rs` (`ambiguity_no_guess_event`),
`mod.rs`.

**Design:** when ≥ 2 high-confidence candidates AND the prompt requests asking → `AskClarification`,
no execution; single clear candidate → normal flow.

**Tests:** unit (2-candidate→ask) + live re-run #91, 92, 93 (and #107 regression).

**Gate-7:** ambiguity prompts correct ask (no guess); #107 stays PASS; prior intact.

---

## Final acceptance (Requirement 8)

Full 112-prompt live re-run; record before/after in
`planning_docs/gui_cognition_user_truetest_results.md`; 0 destructive-leak; no family BROKEN.
Inherently live-dependent prompts (OCR summarize, network page-load recovery) reported honestly.

## Risk / rollback

Each phase behind its flag; any regression → flip that flag OFF (restores prior behavior). Live GUI
+ LLM + OCR has irreducible flakiness; the target is a large, honest live improvement — not a
fabricated 100%.
