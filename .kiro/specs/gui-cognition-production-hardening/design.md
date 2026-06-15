# Design: GUI Cognition Production Hardening

## Overview

Thirteen sequential, flag-gated, live-gated fixes. Each fix follows the SAME contract proven in
`gui-cognition-live-remediation`:

1. **Implement behind a feature flag** (default decided per fix; flag-OFF = byte-for-byte unchanged,
   asserted by a test).
2. **CI-safe tests** (T2/unit, mock where needed) green.
3. **Focused LIVE gate** via the running desktop, same path as the UI
   (`POST /api/testing/desktop-chat-command`, `mode_id=gui_cognition`, `execute_live`+workflow), scored by
   `testing/tools/gui_cognition_capability_audit.py`.
4. **0 destructive-leak** + **no regression** in prior fixes.
5. Honest reporting — `inconclusive` over false verdicts, never fabricate numbers.

**Strict ordering**: a fix's live gate MUST be green before the next fix starts. Order is chosen by
safety-first, then dependency, then impact.

## Approved local-model setup (hardware-confirmed)

Reference machine: **RTX 4050 Laptop (6 GB VRAM)**, 24-core CPU, 15 GB RAM (~4.6 GB free — tight).
Display runs on the Intel iGPU (Optimus), so the NVIDIA 6 GB is mostly free for compute. Models are
ALREADY present locally — **no downloads needed**:
- `models/llm/Qwen2.5-VL-7B-Instruct-Q4_K_M.gguf` (4.4 GB) + `models/llm/mmproj-F16.gguf` (1.3 GB)
- `models/llm/Qwen2.5-3B-Instruct-Q4_K_M.gguf` (2.0 GB)
- `llama-server` + `libmtmd`/`libllava` present → multimodal serving supported.

**Decision (approved): ONE resident model — `Qwen2.5-VL-7B-Instruct` — serves BOTH vision AND the local
grammar planner rung, sequentially, via a single `llama-server` (with `--mmproj`).** Rationale: 6 GB VRAM
cannot hold two resident models; load/unload swapping is worse than one resident model; vision (element/
read intents) and the local planner rung (cloud-reject only) are occasional and never run simultaneously
within a turn, so sequential single-model has no real latency penalty. The 3B is NOT kept resident.

**Flag-gated light fallback** (when VL-7B is too tight / OOM / slow): OCR (CPU, PaddleOCR/RapidOCR/Tesseract)
for vision + `Qwen2.5-3B-Instruct` for the planner. An env switch toggles `vl7b-single` ↔ `light-combo`
so there is no lock-in.

VL-7B hardware-fit knobs: screenshot downscale (~1024–1280 px longest side), small ctx (4096), partial/
max GPU offload, on-demand calls only. If a vision call OOMs, degrade to the OCR path honestly.

## Architecture


Touch-points by area:

| Area | Files (primary) |
|---|---|
| Perception / vision | `kria-core/src/tools/vision_automation.rs`, `sidecars/kria-vision/`, `kria-desktop/.../gui_cognition.rs` (capture/probe) |
| Planner / ladder | `kria-core/src/agent/gui_cognition/{llm_planner.rs,mod.rs}`, `kria-core/src/llm/*` |
| Workflow / focus / recovery | `kria-core/src/agent/gui_cognition/{mod.rs,workflow_runtime.rs,recovery.rs,window_focus.rs}` |
| Input (uinput) | `kria-uinput-daemon/src/{uinput.rs,main.rs}` |
| Safety / approval | `kria-core/src/agent/gui_cognition/{safety_hitl.rs,safety_polish.rs}` |
| Verify / evidence | `kria-core/src/agent/gui_cognition/verifier.rs` |
| GNOME extension | `kria-desktop/gnome-shell/extensions/kria-active-window@kria.ai/extension.js` (+ installed copy; re-login to reload) |
| Live harness | `testing/tools/gui_cognition_capability_audit.py`, `_user_list_gate.py`, `_focused_gate.py` |

## Execution order (waves)

```json
{
  "waves": [
    { "wave": 0, "issue": 5,  "title": "Approval/boundary gate determinism (SAFETY first)" },
    { "wave": 1, "issue": 3,  "title": "Open-then-act focus guarantee (highest prompt impact)" },
    { "wave": 2, "issue": 9,  "title": "Caching coherence (prevents stale-frame verify bugs)" },
    { "wave": 3, "issue": 10, "title": "Verification evidence decoupling" },
    { "wave": 4, "issue": 12, "title": "Clear failure reporting (replace flapping)" },
    { "wave": 5, "issue": 13, "title": "Smarter bounded recovery" },
    { "wave": 6, "issue": 4,  "title": "Wayland absolute pointer (enables real clicks)" },
    { "wave": 7, "issue": 1,  "title": "Real visual perception (replace dummy vision)" },
    { "wave": 8, "issue": 7,  "title": "OCR quality + scope (depends on capture+vision)" },
    { "wave": 9, "issue": 8,  "title": "AT-SPI reliability" },
    { "wave": 10, "issue": 2, "title": "Local grammar planner rung" },
    { "wave": 11, "issue": 6, "title": "Latency reduction (after probes/vision settle)" },
    { "wave": 12, "issue": 11,"title": "Reduce single-point extension dependency + graceful degrade" }
  ]
}
```

Rationale: SAFETY (#5) first. #3 unblocks the most common real prompts. #9/#10/#12/#13 harden the
runtime/verify/UX (cheap, no external deps). #4 (abs pointer) must land BEFORE #1 (vision) — a
vision-resolved control is useless if the click can't be delivered. #1 then unlocks click/checkbox/read.
#7/#8 depend on capture+vision. #2 (local model) needs a served model (user resource). #6 (latency)
last so it tunes the final probe set. #11 portability last (stretch).

## Per-issue design

### #5 Approval/boundary gate determinism (SAFETY)
- Audit ALL execution entry points (single-step `handle_*`, workflow Executable branch, deterministic
  fallback) to confirm the safety gate + `requires_user_approval` is evaluated on every path BEFORE any
  `executor.execute`.
- Root-cause the #36 flakiness: likely the proposal `action_type` / risk classification differs when no
  target resolves vs resolves (the "Submit" path). Make approval-required a property of the GOAL contract
  (risk/explicit-after-approval), not dependent on target resolution, so a no-target approval prompt still
  gates.
- Flag `gui_cog_gate_determinism` (default ON after gate). Test: same prompt × N → identical gated verdict;
  a property test over (resolved/unresolved × risky verb) asserting "never executes before approval".

### #3 Open-then-act focus guarantee
- In the workflow runtime, after an `OpenApp`/`SwitchWindow` step that changed state, before resolving the
  next in-app step's target, ACTIVATE the target app via the extension (`ext_activate_target`) and confirm
  `focused_after == target` within the bounded readiness wait. If not focused → stop with clear reason
  (Issue #12 message), never resolve against the wrong window.
- Reuse Issue #1 mechanism (`mod kria_ext` ActivateWindow) already in `kria-desktop`; thread an
  "activate target app" hook into the readiness wait keyed on the plan's target app hint.
- Flag `gui_cog_open_then_act_focus` (default ON after gate). Live: "Open Chrome and search …", "Open
  editor and type …" land in the right window.

### #9 Caching coherence
- Document + enforce: per-observation screenshot cache cleared by `begin_observation()` (already added);
  observation cache (750ms) MUST NOT serve a post-action re-observe (mark re-observe calls "force fresh").
- Add a single `ObservationFreshness` policy + regression test that a pre/post pair around an action are
  distinct captures. Flag `gui_cog_cache_coherence`.

### #10 Verification evidence decoupling
- Extend the verification contract: each action type has an ordered evidence list
  (e.g. screen_changed → process/active-window → accessibility). If primary unavailable/low-confidence,
  try next; if none reliable → `inconclusive` (never false verified/failed).
- Flag `gui_cog_verify_evidence`. Tests: weak-screenshot → inconclusive; strong secondary → verified.

### #12 Clear failure reporting
- Map the flapping/no-progress stop to the UPSTREAM blocker (target-not-found / app-not-focused /
  vision-unavailable / needs-clarification) and surface that as the user reason. Keep the bounded guard.
- Flag `gui_cog_clear_failure`. Test: a no-candidate stop reports "target not found", not "screen repeated".

### #13 Smarter bounded recovery
- In `recovery.rs` + workflow loop: for transient/idempotent failures (load_not_ready, focus_lost), do a
  bounded re-activate / wait-then-reobserve (capped by Task-1 caps); never retry non-idempotent/destructive.
- Flag `gui_cog_smart_recovery`. Live: focus-race / load-not-ready recovers or stops cleanly.

### #4 Wayland absolute pointer
- Option A (preferred): register `EV_ABS` (ABS_X/ABS_Y with a screen-sized range) on the uinput virtual
  device → absolute move + `BTN_LEFT` click. Option B: add `ClickAt(x,y,button)` to the GNOME extension
  (in-shell pointer warp + click) as the Wayland path, uinput-abs as fallback.
- Wire `ClickControl` → physical bounds (already computed by `physical_bounds_for_target`) → abs click →
  verify. Flag `gui_cog_abs_pointer`. Test: abs move+click event shape; live click on a known control.

### #1 Real visual perception
- Replace `dummy-omniparser-v0.1` in `sidecars/kria-vision/` with the **approved real model:
  `Qwen2.5-VL-7B-Instruct` (+ `mmproj-F16`)** served via `llama-server --mmproj`, called ON-DEMAND with a
  DOWNSCALED screenshot from the GNOME extension capture (sees native Wayland windows). VL-7B does GUI
  grounding: returns bbox + label + type for the requested control(s). Honest `vision_degraded` when the
  model/server is unavailable — never fabricated detections.
- **Flag-gated light fallback** (`gui_cog_real_vision=light`): OCR (CPU) + heuristic element detection for
  text-labeled controls + read; used when VL-7B OOMs/too slow.
- Perception consumes real detections; capture is the extension's full composited stage.
- Flag `gui_cog_real_vision` (`vl7b` | `light` | `off`). Live: click/checkbox against a visible labeled
  control resolves uniquely from VL-7B detections; read-visible returns grounded text.

### #7 OCR quality + scope
- Region-of-interest OCR (active window bounds from extension), adequate resolution, only on read intents;
  reuse extension capture. Flag `gui_cog_ocr_quality`. Live: read-visible returns grounded summary.

### #8 AT-SPI reliability
- Bound the snapshot strictly; when degraded, downgrade candidates' trust + prefer extension/vision;
  surface honest health. Flag `gui_cog_atspi_health`.

### #2 Local grammar planner rung
- The **SAME resident `Qwen2.5-VL-7B-Instruct`** `llama-server` serves the local grammar planner rung: a
  TEXT + GBNF-grammar request (no image) → schema-valid typed plan. Wired as Capability-Ladder Rung B
  (CI-verified). Invoked ONLY when the cloud planner is strictly rejected (occasional), so 7B planning
  latency is acceptable; grammar enforces schema validity regardless of model size.
- No separate resident 3B (6 GB cannot hold both). The `light` fallback uses `Qwen2.5-3B-Instruct` for the
  planner when VL-7B is not serving vision.
- Availability detection: if the local server is down, keep the honest deterministic fallback + capability
  notice (no regression). No redundant cloud call once the local rung is used.
- Flag `gui_cog_local_planner`. Live: a cloud-rejected prompt yields `ladder_rung=local_grammar`
  schema-valid plan + executes. Same `llama-server` instance as #1 — sequential, one model.

### #6 Latency reduction
- Intent-aware probe scheduling (skip OCR/vision for non-reading actions), ROI OCR, async/parallel probes,
  cache reuse within a turn. Flag `gui_cog_fast_observe`. Measure p50 live before/after.

### #11 Reduce single-point dependency
- Backend-availability status for window-focus/capture/activate; clear capability notice when extension
  absent; scope (design) a portal/wlr fallback. Flag `gui_cog_backend_status`.

## Implementation lessons from Tasks 1–2 (apply to remaining tasks)

These were learned by shipping #5 and #3 live and materially affect #3–#13:

1. **`execution_mode` downgrade gotcha.** The runtime downgrades `ExecuteLive`→`SafetyOnly` when
   `preconditions.ready == false` (`mod.rs` runtime-guard). After a desktop restart the probes need a
   moment to warm; the FIRST live action can otherwise come back `"execution_mode is safety_only"`. Always
   send one `observe` warm-up and confirm `preconditions.ready == true` before the gate prompt.
2. **Stale originating-window hint causes false flapping.** `typed_step` defaults `target_window_hint`
   from the goal contract = the active window at PLAN time (e.g. the editor that issued the prompt). For a
   step that operates on a DIFFERENT app (post-`OpenApp`), `await_step_readiness` then waits for a window
   that never reappears → re-observes a static screen → trips the flapping guard. Fix pattern: clear the
   window hint (`with_window_hint(None)`) on post-switch steps so readiness keys on the `app_hint`. This is
   directly relevant to #3/#5/#6/#12/#13 (any multi-app flow).
3. **A focus-only action is not observable; verify the OBSERVABLE outcome.** A Ctrl+L address-bar focus
   produces ~no pixel change and Chrome a11y is off, so `text_present` and a too-early `screen_changed`
   both fail. The robust pattern is to make the action ATOMIC (focus+type+submit) and verify the
   downstream observable change (navigation), with a BOUNDED navigation/render wait before capturing the
   post-action frame. Generalize this into the ordered-evidence model (#10) and the freshness rule (#9).
4. **Verification CONTRACT can override the plan's strategy.** `verification_contract_for` maps by action
   kind (TypeText→`text_present`) and `apply_verification_contract` can downgrade. When a step needs a
   different evidence source, override BOTH the selected strategy and (if needed) the contract — see the
   Task-2 sentinel handling; #10 turns this into a first-class ordered list.
5. **Synthetic KEYBOARD input works on Wayland incl. XWayland (Chrome).** Confirmed via the uinput daemon
   (`Type command succeeded`) + the active window navigating to the typed URL. So #7 is ONLY about the
   absolute POINTER (`EV_ABS`); do not spend effort on keyboard routing.
6. **`press_shortcut` needs split tokens.** `parse_key_string` rejects a combined `"ctrl+l"`; pass
   `["ctrl","l"]` (the desktop arm now splits on `+`).
7. **Run live turns sequentially.** Overlapping live turns can exhaust the ~4.6 GB free RAM and crash the
   desktop (observed). One gate prompt at a time; rebuild→restart→warm-up→run.
8. **The `browser_search` contract validator scans plan text** for destructive verbs
   (`delete/remove/send/submit/pay/…`). Avoid those words in benign step summaries (Task 2 used "run the
   search", not "submit").


- Flag-OFF = byte-for-byte (asserted).
- KRIA stays the authoritative orchestrator; no raw-prompt/OCR/coordinate-originated action.
- No secret/credential leakage in logs/events.
- 0 destructive-leak; approval/boundary/ambiguity gates never weakened.
- No fabricated live numbers.

## Components and Interfaces

- **Perception provider** (`GuiPerceptionProvider`): observation, capture (`begin_observation` freshness),
  AT-SPI snapshot, vision detections, OCR. Touched by #1/#7/#8/#9/#10.
- **Capability Ladder / planner** (`llm_planner.rs`, `mod.rs`): cloud → local-grammar (#2) → deterministic;
  gate determinism (#5).
- **Workflow runtime** (`mod.rs`, `workflow_runtime.rs`): step sequencing, open-then-act focus hook (#3),
  recovery (#6/#13), failure reporting (#5/#12).
- **Input substrate** (`kria-uinput-daemon`, GNOME extension): absolute pointer (#4), activate/capture (#11).
- **Verifier** (`verifier.rs`): ordered evidence contract (#10).
- **Safety/HITL** (`safety_hitl.rs`, `safety_polish.rs`): approval/boundary determinism (#5).
Each new behavior is gated by its own `gui_cog_*` feature flag and exposes additive telemetry only.

## Data Models

- `GuiObservationSnapshot` — extended freshness/evidence metadata (no breaking change); vision detections
  carry bbox+label+type+confidence+source.
- `GuiVerificationContract` — ordered `evidence_sources: Vec<EvidenceSource>` per action type (#10).
- `GuiBackendStatus` — availability of focus/capture/activate backends + capability notice (#11).
- `RecoveryDecision` — kind/idempotency/bounded-retry telemetry (#13).
- All new fields are additive + serde-default so flag-OFF deserialization is unchanged.

## Correctness Properties

### Property 1: Approval safety
For any prompt classified approval-required, no `executor.execute` occurs before a HITL approval — on
every code path (single-step/workflow/fallback), regardless of target resolution.
**Validates: Requirements 5.1, 5.3**

### Property 2: No-guess
An ambiguous/unresolved target never auto-executes; it asks/blocks. (preserved)
**Validates: Requirements 5.3, 12.1**

### Property 3: Observation freshness
A post-action observation used for verification is a distinct capture from the pre-action one.
**Validates: Requirements 9.1, 9.2**

### Property 4: Honest verdict
A verdict is `verified` only with reliable evidence, `inconclusive` otherwise — never a false
`verified`/`failed`.
**Validates: Requirements 10.1, 10.2**

### Property 5: Flag parity
For every fix, flag-OFF output equals the prior behavior byte-for-byte.
**Validates: Requirements 1.3, 2.3, 3.4, 4.4, 5.3, 6.3, 7.3, 8.3, 9.3, 10.3, 11.3, 12.3, 13.3**

### Property 6: Bounded loops
All re-observe/retry/readiness loops remain bounded by the Task-1 runaway caps.
**Validates: Requirements 6.1, 13.1**

## Error Handling

- Missing model/backend (vision, local planner, extension) → honest degraded status + capability notice,
  never fabricated output or silent failure (#1/#2/#11).
- Unresolvable target → clear root-cause reason (target-not-found / app-not-focused / vision-unavailable /
  needs-clarification), not opaque flapping (#12).
- Degraded evidence → `inconclusive` (#10). Transient/idempotent failure → bounded recovery; risky → never
  auto-retry (#13).

## Testing Strategy

- CI-safe T2/unit + property tests per fix (mock vision/LLM/extension where needed); a flag-OFF parity
  test for every fix.
- Focused LIVE gate per fix on the running desktop (UI path), scored by the capability audit; ≥3-run
  stability for the safety gate (#5).
- Known pre-existing unrelated failures excluded (see tasks.md Notes).
- No fabricated numbers; honest `inconclusive`/degraded reporting.
