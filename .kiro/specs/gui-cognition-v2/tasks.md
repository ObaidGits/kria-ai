# Implementation Plan

## Overview

Build GUI Cognition V2 (Sight / Brain / Hands) phase by phase: each layer is built and
tested IN ISOLATION first, then integrated incrementally (Sight+Brain decision-only →
Hands alone → full loop), then the over-built V1 logic is removed. V2 runs behind
`KRIA_GUI_COG_V2` (default OFF until proven). Brain is pluggable (`GuiBrain` trait) so
UI-TARS drops in later. Preserve safety/HITL, audit, cancel/watchdog, verification, eval
harness, uinput, model-swap.

Order is strict: Phase 0 → 1 → 2 → 3 → 4 → 5 → 6. Within a phase, build before test.

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 1, "tasks": [1], "description": "Phase 0 — contracts, traits, loop skeleton, dummy impls" },
    { "wave": 2, "tasks": [2, 3], "description": "Phase 1 — Sight (OmniParser) build + isolation test" },
    { "wave": 3, "tasks": [4, 5], "description": "Phase 2 — Brain (Qwen) build + isolation test" },
    { "wave": 4, "tasks": [6], "description": "Phase 3 — Sight+Brain decision-only integration" },
    { "wave": 5, "tasks": [7, 8], "description": "Phase 4 — Hands build + isolation external-verify test" },
    { "wave": 6, "tasks": [9, 10], "description": "Phase 5 — full loop wiring + real-verify eval" },
    { "wave": 7, "tasks": [11], "description": "Phase 5b — optional UI-TARS Brain (pluggable, swap)" },
    { "wave": 8, "tasks": [12, 13], "description": "Phase 6 — flip default + remove V1 overhead" }
  ]
}
```

- Phase 0 (Task 1) underpins everything.
- Sight (2,3) and Brain (4,5) are independent of each other; both feed Phase 3 (6).
- Hands (7,8) is independent of Brain quality; feeds the full loop (9).
- Full loop (9,10) depends on Sight+Brain+Hands. UI-TARS (11) depends on the loop+trait.
- Cleanup (12,13) is last, after V2 is proven.

## Tasks

- [x] 1. Phase 0 — Contracts, traits, loop skeleton
  - Create `kria-core/src/agent/gui_cognition_v2/` module. Define `Observation`,
    `UiElement`, `Action`, `Decision`, `ActionResult`, `TurnStep` (serde, `#[serde(default)]`
    on additive fields).
  - Define `Sight`, `GuiBrain`, `GuiHands` traits (async, injected). Add dummy impls
    (`FakeSight`, `FakeBrain`, `FakeHands`) returning fixed data.
  - Add a minimal loop function `run_turn_v2` wiring the three traits with a step cap, and
    a `KRIA_GUI_COG_V2` flag helper (default OFF; falsy rollback).
  - Unit tests: types round-trip serde; loop skeleton runs N dummy steps and stops on
    `Done`/cap.
  - _Requirements: 1.1, 1.2, 1.3, 1.5, 5.1, 10.1_

- [x] 2. Phase 1 build — Sight (OmniParser sidecar + client)
  - Add OmniParser to the `kria-vision` sidecar: `POST /parse { screenshot, want_som }` →
    JSON Observation (elements id/bbox/kind/label/confidence) + optional Set-of-Mark PNG.
    Keep detection light (CPU or <1 GB GPU).
  - Implement `OmniParserSight` in kria-core calling the sidecar (reuse sidecar HTTP
    pattern); map JSON → `Observation`; sanitize labels (untrusted, no instructions);
    carry monitor/scale so coords map to physical pixels.
  - Graceful degrade: sidecar down → `source = "degraded:<reason>"`, empty elements, no
    crash.
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.6, 8.1_

- [x] 3. Phase 1 test — Sight isolation
  - Isolation tests with static screenshots (Chrome, Settings, file manager): assert
    elements > 0, a known control detected + correctly labeled, Set-of-Mark overlay
    produced; measure latency.
  - Test the degraded path (sidecar unavailable) returns an honest empty observation.
  - No Brain/Hands involved.
  - _Requirements: 2.5, 9.1, 9.4_

- [x] 4. Phase 2 build — Brain (QwenBrain, text-first, pluggable)
  - Implement `QwenBrain` behind `GuiBrain`: prompt = task + numbered element list (+ SoM
    image only when `KRIA_GUI_COG_V2_SOM` ON) + bounded history → grammar-constrained
    `Decision`. Reject targets absent from the observation; return `Ask`/`Done` when no safe
    action.
  - Keep all Qwen-specific logic inside the impl (no leakage to loop/Sight/Hands).
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.7, 7.1, 7.3, 8.2_

- [x] 5. Phase 2 test — Brain isolation (mock observations)
  - Build a fixture set of mock `Observation`s + tasks covering SEEN and UNSEEN prompts
    (open/new-tab/click-control/navigate/ambiguous/destructive-ask).
  - Assert the Brain selects the correct element id/action (or `Ask` when ambiguous),
    purely from fixtures — no real screen, no execution.
  - _Requirements: 3.5, 7.2, 7.4, 9.1_

- [x] 6. Phase 3 — Sight + Brain decision-only integration (no execution)
  - Implemented `decide_only(sight, brain, task, want_som, expected_label)` in
    `gui_cognition_v2/decide_only.rs`: observes with the real `Sight`, decides with the real
    `GuiBrain`, returns the `Decision` + matched element label WITHOUT executing.
  - `OutcomeAttribution` + pure `attribute(...)` diagnose a wrong outcome to the responsible
    layer: `TargetMissingFromSight`/`DegradedSight` (Sight) vs `BrainPickedAbsentElement`/
    `BrainPickedWrongElement` (Brain), with `blame_layer()` + layman `human()`.
  - Tests (7) cover correct pick, target-missing→sight, wrong-pick→brain, absent-id→brain,
    degraded→sight, direct/terminal→none, and decide_only end-to-end with fakes.
  - _Requirements: 9.2, 11.1_

- [x] 7. Phase 4 build — Hands (UinputHands)
  - Implement `UinputHands` behind `GuiHands`: `Click{element_id}` → resolve in supplied obs
    → bbox center → physical px (reuse monitor_layout/DPI math) → uinput click;
    `ClickPoint{x,y}` → direct click; `Key{combo}` → standard shortcut table (new_tab/zoom/
    close/save, app-agnostic, no per-prompt hardcode) or literal → uinput; `Type`/`Scroll`.
  - Missing element id → explicit failure, no fallback click.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.6_

- [x] 8. Phase 4 test — Hands isolation (external verify)
  - Feed FIXED Decisions (known bbox click, ClickPoint, Key ctrl+t, Type) → execute →
    verify via EXTERNAL observer (xdotool/wmctrl/screenshot-diff). No Brain.
  - Assert coordinate landing (where environment allows) and key effect; missing-id case
    fails explicitly; document INCONCLUSIVE where Wayland bounds prevent verification.
  - _Requirements: 4.5, 9.3, 9.4_

- [x] 9. Phase 5 build — full observe-act loop + desktop wiring (Part B)
  - Wired `run_turn_v2`: observe → decide → safety gate → execute → re-observe; bounded by
    step cap, no-progress (screen-signature) stop, and a cancel flag bridged from the
    existing GUI cancel registry. (Done in core loop_engine; tested 33/33.)
  - Desktop glue (`crates/kria-desktop/src/commands/gui_cognition.rs`):
    `V2DesktopScreenCapturer` (GNOME-extension capture → base64 + PNG-dim probe),
    `V2DesktopInputSink` (uinput `YdotoolBackend`; Wayland absolute-coordinate
    normalization), `V2DesktopSafetyGate` (honest halt + master-switch gate), and
    `run_gui_cognition_v2` which streams per-step `gui_cognition:event` envelopes on the
    existing channel and returns the same `DesktopChatCommandCapture` shape as V1.
  - Routes GUI turns to V2 when `KRIA_GUI_COG_V2` is ON (V1 stays default until Task 12).
  - NOTE: full HITL pause/approve round-trip is deferred (gate denies risky/halted rather
    than pausing); audit-ledger recording of V2 steps is a follow-up (Task 10).
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 6.1, 6.2, 6.3, 6.4, 6.5, 11.1, 11.2, 11.3, 11.4_

- [ ] 10. Phase 5 test — full-loop real-verify eval
  - Extend the real-verify harness with V2 cases: multi-step + held-out UNSEEN natural
    prompts; external truth (wmctrl/pgrep/filesystem/screenshot-diff); flag MISMATCH when
    the reply claims success but reality disagrees.
  - Record PASS/FAIL/INCONCLUSIVE honestly; produce a baseline report for the V2 default flip.
  - _Requirements: 7.4, 9.3, 9.4, 9.5_

- [ ] 11. Phase 5b — optional UI-TARS Brain (pluggable)
  - Implement `UiTarsBrain` behind the SAME `GuiBrain` trait (consumes raw screenshot;
    emits `ClickPoint`/`Type`/`Key`). Select via `KRIA_GUI_COG_V2_BRAIN=ui_tars`; trigger
    the orchestrator to swap the resident model for the GUI turn and restore after.
  - A/B against Qwen+OmniParser on the eval harness; no changes to Sight/Hands/loop.
  - _Requirements: 3.6, 8.3, 8.4_

- [x] 12. Phase 6 — flip V2 to default
  - DONE: `v2_enabled()` is now default-ON (falsy `KRIA_GUI_COG_V2` = documented V1
    rollback, covered by `flag_tests`). Verified live in default mode (no flag): `engine=v2`,
    held-out eval 6 PASS / 0 FAIL / 0 MISMATCH / 0 BLOCKED. Safety/HITL (A3), verification
    re-observe (A4), cancel/no-progress, audit, uinput, model-swap all intact.
  - _Requirements: 10.2, 10.5_

- [ ] 13. Phase 6 — remove V1 over-built logic
  - Delete the over-built V1 paths (code AND logic): dual plan representation (typed_steps +
    legacy steps/action_kind), capability ladder, goal-pursuit guard, heavy upfront
    validators, upfront multi-step planner, large contract extraction. Collapse to V2's
    single representation and single code path; no dead branches.
  - Re-run core/desktop/UI suites + GUI real-verify; `cargo build` clean.
  - _Requirements: 10.3, 10.4, 10.5_

## Parity-to-default queue (A3–A6) — bring V2 to parity, then flip + remove V1

Status so far: Tasks 1–9 done; plus parity increments already landed + live-proven —
app-launch (`OpenApp`), fast perception-light Sight, dedup-open, and the deterministic
multi-action follow-up assist. Remaining queue (do strictly in order, build→test→commit;
each behind `KRIA_GUI_COG_V2`, never fake-pass, external verify):

- [x] 14. A3 — Safety/HITL parity (must-do before default)
  - `gui_cognition_v2/safety.rs`: pure `assess_action_risk(decision, observation) -> RiskLevel`
    (reuses `kria_core::safety::{RiskLevel, BlacklistChecker}`): typed text / clicked label /
    key combo → `Black` (hardcoded blacklist e.g. `rm -rf /`), `Red` (destructive verbs:
    delete/remove/send/pay/format/shutdown/… whole-word), else `Green`. 5 unit tests.
  - `V2DesktopSafetyGate` now: halt + master-switch deny; `Black` always denied; `Red`
    auto-approved ONLY in `test_substrate` else denied with "needs your approval" (safe floor —
    risky never executes unapproved); `Green`/`Yellow` allow. `auto_approve` derived from
    `GuiExecutionEnvironment::from_env()`.
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5_

- [x] 15. A4 — verification reuse (re-observe / screenshot-diff)
  - The loop now records an honest per-step `screen_changed` by diffing the
    re-observed screen signature against what the previous action acted on (the
    same signal drives the no-progress guard). A "did nothing" action is flagged
    `screen_changed = Some(false)`. Test: `records_screen_changed_on_each_executed_step`.
  - _Requirements: 5.5, 9.3, 9.4_

- [x] 16. A5 — V2 live eval (held-out) — CLEAN PASS
  - `scripts/gui_cog_e2e_v2.py` (V2-aware: engine/status/steps + `action_detail`) ran a
    held-out set in test_substrate with `KRIA_GUI_COG_V2=1`, external compositor/pgrep truth.
    **Result: 6 PASS, 0 FAIL, 0 MISMATCH, 0 BLOCKED, 0 INCONCLUSIVE.**
  - Root-caused + fixed the last BLOCKED case ("open the files manager"): the Brain emits
    `OpenApp{app:"files_manager"}` (snake_case), but `app_registry::normalize_alias` never
    treated `_` as a space, so resolution failed. Fix: normalize `_`→space in
    `normalize_alias` (shared infra — benefits V1, V2, and every app-launch tool). Added 2
    regression tests. Externally verified live: nautilus 0→1.
  - Hardened the startup self-test (`gui_wiring::validate_gui_tool_registry`): its
    `open_application` probe name `__kria_selftest_nonexistent__` would, after the new `_`
    normalization, first-token-match "kria" → `com.kria.desktop` and self-launch the app at
    boot (single-instance race → panic). Changed the probe to an inert nonsense token.
  - Added `Action::detail()` + `action_detail` to V2 step telemetry so the exact OpenApp app
    string (and combo/text/point) is visible for diagnostics — this is what surfaced the bug.
  - Loop fix landed earlier: a FAILING action now arms no-progress.
  - _Requirements: 7.4, 9.3, 9.4, 9.5, 10.2_

- [~] 17. A6 — flip default (Task 12 DONE) + remove V1 over-built pipeline (Task 13 PENDING)
  - **Task 12 — DONE + verified.** `v2_enabled()` flipped to DEFAULT-ON (falsy
    `KRIA_GUI_COG_V2` = byte-for-byte V1 rollback; logic covered by `flag_tests`).
    Verified LIVE in default mode (no env flag set): `engine=v2`, held-out eval
    6 PASS / 0 FAIL / 0 MISMATCH / 0 BLOCKED. Desktop routing reads core `v2_enabled()`,
    so the flip propagates; stale "default OFF" doc comments updated.
  - **Known V2 quality gap (not a flip regression):** multi-action follow-up
    (e.g. "open chrome AND new tab/reload/close tab") is INCONSISTENT — the 7B Brain
    sometimes returns `needs_clarification` after the open instead of chaining the
    follow-up `Key`. App-launch + single-action are solid. This should be hardened
    (stronger `apply_followup_assist` / Brain prompt) before V1 is deleted.
  - **Task 13 — PENDING (gated on confirmation + soak).** Deleting the V1 over-built
    pipeline (dual representation, capability ladder, goal-pursuit guard, heavy
    validators, upfront planner, large contract) is a large destructive change. With V2
    only just flipped to default and multi-action chaining still variable, removing the
    V1 rollback net now is high-risk. Recommendation: let V2 soak as default + harden
    multi-action first, then delete V1 in a dedicated change. KEEP shared infra (uinput,
    capture, app-registry, safety/HITL, audit, cancel, verification, orchestration).
  - _Requirements: 10.2, 10.3, 10.4, 10.5_

## Notes
- Read backend env flags live per turn (matching `KRIA_GUI_COG_*`); flip without rebuild.
- Each phase: build before test; isolation before integration; external verify, never
  fake-pass; INCONCLUSIVE where the environment blocks verification.
- Brain pluggability is the hinge: keep ALL model-specific logic inside the `GuiBrain`
  impl so UI-TARS (or any future model) drops in with no other changes.
- Do not touch `~/.kria/kria.db`, `~/.kria/secrets/`, `~/.kria/config.toml`.
- Commit when green; keep V2 behind the flag until Task 12.

## Honest status (do not fake-complete)

Done: Phase 0–4 (Tasks 1–5, 7, 8), Phase 3 decision-only (Task 6), and Phase 5 loop +
desktop wiring (Task 9). All behind `KRIA_GUI_COG_V2` (default OFF). Core V2 unit tests +
desktop glue tests green.

Real-verify harness (Task 10 infra) is BUILT — `scripts/gui_cog_eval.py` (plan-level) and
`scripts/gui_cog_e2e_live.py` (live, auto-approve + ExecuteLive, external compositor truth
via the GNOME extension + pgrep). It has been run live against the DEFAULT (V1) path and
found + fixed real execution bugs (OpenApp app-hint backfill; ungroundable-shortcut repair).

Tasks 10 (V2 baseline), 11, 12, 13 are intentionally LEFT OPEN — they are gated and would
REGRESS the product if forced now:

- **V2 capability gap**: the V2 `Action` set has no app-launch action (no `OpenApp`), and
  Sight depends on the OmniParser sidecar (CPU-slow, ~180s/full-desktop, not live-viable as
  configured). So V2 cannot yet open apps or run a fast live turn — it is NOT at parity with
  the now-working V1 path.
- **Task 10 (V2 baseline)**: blocked until V2 can launch apps + has a viable fast Sight.
- **Task 11 (UI-TARS Brain)**: optional/future; needs the model + orchestrator swap.
- **Task 12 (flip default to V2)**: MUST NOT flip until V2 passes the live eval bar.
  Flipping now regresses real behavior (the fixed V1 path is what works live today).
- **Task 13 (remove V1 overhead)**: MUST NOT delete V1 until Task 12 is done and proven —
  V1 is the live, externally-verified working path.

Path to truly finishing V2: give V2 an app-launch action + a fast Sight (e.g. reuse the
existing fast perception, or a Qwen-VL multimodal Brain), pass `gui_cog_e2e_live.py` on a
held-out prompt set, THEN flip default (12) and remove V1 (13).
