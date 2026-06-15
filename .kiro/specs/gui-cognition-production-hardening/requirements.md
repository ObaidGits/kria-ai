# Requirements Document

Feature: GUI Cognition Production Hardening

## Introduction

The live remediation spec (`gui-cognition-live-remediation`) fixed the 7 targeted capability
issues and reached **58.4% live coverage with 0 destructive-leak**. A subsequent live + code review
surfaced **13 faults/deficiencies** that keep GUI Cognition from production-grade. This spec fixes all
13, one at a time, each behind a feature flag (flag-OFF = byte-for-byte unchanged) and each gated by:
(a) CI-safe tests green, (b) a focused LIVE re-test on the running desktop (same path as the UI:
`POST /api/testing/desktop-chat-command`, `mode_id=gui_cognition`, `execute_live`+workflow), (c) 0
destructive-leak, (d) no regression in prior fixes. **A fix is not "done" until its live gate is green;
the next fix does not start until the current one passes.** Verification is never weakened to pass; no
fabricated numbers.

Environment of record: GNOME Shell 46 Wayland (Ubuntu), self-owned `kria-active-window@kria.ai`
extension active, `kria-uinput-daemon` (uinput) input path, cloud LLM `deepseek-v4-flash-free`
(grammar-incapable), no local llama-server by default. All findings below were observed directly in
logs/code/tests unless marked (design inference).

## Glossary

- **Live gate**: a focused re-run of the relevant prompt family through the UI backend path, scored by
  `testing/tools/gui_cognition_capability_audit.py` (`send`/`judge`/`detect_leaks`).
- **Surface**: an on-screen, focused, observable target (field/control/scrollable view) an action acts on.
- **Flag-OFF parity**: with the fix's feature flag disabled, behavior is byte-for-byte the prior behavior
  (asserted by a test).

## Requirements

### Requirement 1: Real visual perception (replace dummy vision)

**User Story:** As a user, I want KRIA to actually SEE on-screen controls (buttons, checkboxes, fields,
text) so that click / checkbox / read-screen / form-field actions work, instead of relying on a stub
that returns fixed elements.

#### Acceptance Criteria
1. WHERE the vision sidecar is configured with a real model, the system SHALL detect real on-screen
   elements (bounding boxes + labels + types) for a known test screen, not a fixed/stub list.
2. WHEN no real vision model is available, the system SHALL report `vision_degraded` honestly and fall
   back to accessibility/OCR — it SHALL NOT emit fabricated element detections.
3. IF the `gui_cog_real_vision` flag is OFF, THEN perception output SHALL be byte-for-byte the prior
   behavior.
4. WHEN a real model is present, a click/checkbox prompt against a visible labeled control SHALL resolve
   a unique target from the vision detections (live).

### Requirement 2: Reliable planner (local grammar model rung)

**User Story:** As a user, I want KRIA to produce reliable, schema-valid plans so that complex/novel
prompts don't degrade to a generic deterministic fallback every turn.

#### Acceptance Criteria
1. WHEN a grammar-capable local model is served, the Capability Ladder SHALL produce schema-valid plans
   via the local grammar rung (Rung B) for prompts the cloud model rejects (live `ladder_rung=local_grammar`).
2. WHILE no local model is served, the system SHALL keep the honest deterministic fallback + capability
   notice (no regression).
3. IF the `gui_cog_local_planner` flag is OFF, THEN planner selection SHALL be byte-for-byte the prior
   behavior.
4. WHEN the local grammar rung is used, the planner SHALL NOT call the cloud model redundantly for that
   turn.

### Requirement 3: Open-then-act focus guarantee

**User Story:** As a user, I want "open X and do Y in X" to work so that after opening/launching an app
the next in-app step acts on the correct, focused window — not whatever was focused before.

#### Acceptance Criteria
1. WHEN a plan contains an `OpenApp`/`SwitchWindow` step followed by an in-app step (Focus/Type/Click/
   PressKey), the runtime SHALL ensure the target app window is the ACTIVE/focused window (via the
   extension `ActivateWindow`) before resolving the next step's target.
2. IF the target app does not become active within the bounded readiness wait, THEN the runtime SHALL
   stop with a clear reason — it SHALL NOT resolve the in-app target against the wrong window (no flap).
3. WHEN the target app is already running but unfocused, the runtime SHALL activate it (not rely on
   `gio launch` raising it).
4. IF the `gui_cog_open_then_act_focus` flag is OFF, THEN behavior SHALL be byte-for-byte the prior path.
5. Live: "Open Chrome and search for <q>" SHALL execute the address-bar focus + type in Chrome (not flap).

### Requirement 4: Wayland absolute pointer (coordinate click)

**User Story:** As a user, I want KRIA to click a target at given coordinates on Wayland so that
vision-resolved controls can actually be clicked (today absolute click falls back to X11-only xdotool
which cannot position over native Wayland windows).

#### Acceptance Criteria
1. WHEN the uinput backend is active on Wayland, the system SHALL support absolute pointer positioning +
   button click (via `EV_ABS` virtual pointer or the GNOME extension), landing on native Wayland windows.
2. WHEN a click target has trusted physical bounds, a `ClickControl` SHALL move-and-click at the target
   center and verify the post-click change.
3. IF no absolute-pointer path is available, THEN `ClickControl` SHALL block honestly (never a silent
   no-op / wrong-location click).
4. IF the `gui_cog_abs_pointer` flag is OFF, THEN the click path SHALL be byte-for-byte the prior path.

### Requirement 5: Deterministic approval/boundary gate (SAFETY)

**User Story:** As a user, I want every risky/approval-required action to ALWAYS pause for approval so
that a destructive/external action can never bypass the gate on any code path.

#### Acceptance Criteria
1. WHEN a prompt is approval-required (risk high/critical OR destructive verb OR explicit "after
   approval"), the runtime SHALL pause for HITL on EVERY path — single-step AND workflow AND
   deterministic-fallback — never executing before approval.
2. The approval decision SHALL be deterministic for the same prompt + screen state (no run-to-run
   `CORRECTLY_GATED` vs `EXECUTED_WITHOUT_APPROVAL` divergence).
3. WHEN no concrete target is resolved for an approval-required action, the runtime SHALL still gate/ask
   — it SHALL NOT fall through to execution.
4. Live: #36-class prompt ("Click Submit only after approval") SHALL be CORRECTLY_GATED across ≥3
   consecutive runs, 0 destructive-leak.

### Requirement 6: Latency reduction

**User Story:** As a user, I want GUI Cognition turns to complete in a usable time so that simple actions
don't take 30–150s.

#### Acceptance Criteria
1. WHEN an observation is collected, expensive probes (OCR, vision) SHALL run only when needed for the
   current intent (skip/defer for actions that don't read screen text), bounded + cached correctly.
2. A simple single-step action (open/scroll/key/switch) SHALL complete in a target budget (e.g. p50 ≤ 15s
   on the reference machine), measured live.
3. IF the `gui_cog_fast_observe` flag is OFF, THEN probe scheduling SHALL be byte-for-byte the prior path.
4. Latency changes SHALL NOT weaken verification or skip a probe a verdict depends on.

### Requirement 7: OCR quality + scope

**User Story:** As a user, I want read/summarize-visible to actually read the screen so that
content-reading prompts work.

#### Acceptance Criteria
1. WHEN a read/summarize intent runs, OCR SHALL operate on the relevant region at adequate resolution
   (not a blind/over-downscaled full-screen), sourced from the extension capture (sees Wayland windows).
2. OCR output SHALL be labeled trusted/untrusted with injection scanning preserved.
3. IF the `gui_cog_ocr_quality` flag is OFF, THEN OCR behavior SHALL be byte-for-byte the prior path.
4. Live: a read-visible prompt against a content window SHALL return a non-empty, on-screen-grounded
   summary.

### Requirement 8: AT-SPI reliability on Wayland

**User Story:** As a user, I want accessibility-based targeting to be reliable so that control resolution
doesn't silently degrade.

#### Acceptance Criteria
1. WHEN AT-SPI is degraded (timeouts, anonymous bus, app a11y off), the system SHALL report the degraded
   health honestly and prefer the extension/vision path rather than emitting low-quality candidates.
2. The AT-SPI snapshot SHALL be bounded (no unbounded scan) and SHALL NOT block the turn beyond its cap.
3. IF the `gui_cog_atspi_health` flag is OFF, THEN AT-SPI behavior SHALL be byte-for-byte the prior path.

### Requirement 9: Caching coherence

**User Story:** As a developer, I want the observation/OCR/screenshot caches to never serve a stale frame
across an action boundary so that verification compares true pre/post state.

#### Acceptance Criteria
1. WHEN a post-action re-observe is performed, it SHALL NOT be served a pre-action cached observation/
   screenshot (the pre/post pair SHALL be fresh).
2. The cache layers SHALL have a single documented coherence rule (which cache, what TTL, when
   invalidated) and SHALL be covered by a regression test.
3. IF the relevant flag is OFF, THEN caching SHALL be byte-for-byte the prior path.

### Requirement 10: Verification decoupled from fragile capture

**User Story:** As a user, I want post-action verification to be trustworthy even when one evidence source
is weak so that a real success isn't reported as failure (and vice-versa).

#### Acceptance Criteria
1. WHEN the primary evidence source (screenshot) is unavailable/unreliable, the verifier SHALL use a
   secondary source (accessibility/active-window/process/backend receipt) and report `inconclusive`
   rather than a false `verification_failed` or false `verified`.
2. The chosen evidence source per action type SHALL be explicit and never OCR-only/coordinates-only for a
   state-change verdict.
3. IF the `gui_cog_verify_evidence` flag is OFF, THEN the verdict SHALL be byte-for-byte the prior path.

### Requirement 11: Reduce single-point GNOME-extension dependency

**User Story:** As a user on a non-GNOME or extension-less session, I want KRIA to degrade gracefully so
that it reports what it can/can't do instead of silently failing.

#### Acceptance Criteria
1. WHEN the extension is unavailable, the system SHALL detect this and report a clear capability notice
   (window activation/capture unavailable) and use the best available fallback.
2. The window-focus / capture / activate abstraction SHALL expose a backend-availability status the UI can
   surface.
3. IF the relevant flag is OFF, THEN behavior SHALL be byte-for-byte the prior path.
4. (Stretch) A documented path for at least one non-GNOME mechanism (portal/wlr) SHALL be scoped (design
   only, implementation optional).

### Requirement 12: Clear failure reporting (replace opaque flapping)

**User Story:** As a user, I want a clear reason when KRIA can't do something so that "flapping" is
replaced by an actionable message ("couldn't find X / app not focused / no such control").

#### Acceptance Criteria
1. WHEN a turn stops without progress, the user-facing reason SHALL name the ROOT cause (target not found
   / app not focused / vision unavailable / needs clarification), not just "screen repeated N times".
2. The flapping guard SHALL still bound the loop, but SHALL classify the stop reason from the upstream
   blocker.
3. IF the relevant flag is OFF, THEN messaging SHALL be byte-for-byte the prior path.

### Requirement 13: Smarter bounded recovery

**User Story:** As a user, I want KRIA to retry sensibly (re-observe/wait/alternative) within bounds so
that a transient failure (window still loading, focus race) doesn't immediately dead-end.

#### Acceptance Criteria
1. WHEN a step fails for a transient/idempotent reason (load not ready, focus lost), the runtime SHALL
   perform a bounded retry/alternative (re-activate, wait-then-reobserve) — still capped by the Task-1
   runaway caps, never for non-idempotent/destructive actions.
2. The recovery decision SHALL be recorded (telemetry) and SHALL never auto-retry a risky action.
3. IF the `gui_cog_smart_recovery` flag is OFF, THEN recovery SHALL be byte-for-byte the prior path.
4. Live: a focus-race / load-not-ready prompt SHALL recover-and-complete OR stop with a clear reason
   (no flap), 0 destructive-leak.
