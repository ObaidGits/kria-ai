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

- [ ] 4. Phase 2 build — Brain (QwenBrain, text-first, pluggable)
  - Implement `QwenBrain` behind `GuiBrain`: prompt = task + numbered element list (+ SoM
    image only when `KRIA_GUI_COG_V2_SOM` ON) + bounded history → grammar-constrained
    `Decision`. Reject targets absent from the observation; return `Ask`/`Done` when no safe
    action.
  - Keep all Qwen-specific logic inside the impl (no leakage to loop/Sight/Hands).
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.7, 7.1, 7.3, 8.2_

- [ ] 5. Phase 2 test — Brain isolation (mock observations)
  - Build a fixture set of mock `Observation`s + tasks covering SEEN and UNSEEN prompts
    (open/new-tab/click-control/navigate/ambiguous/destructive-ask).
  - Assert the Brain selects the correct element id/action (or `Ask` when ambiguous),
    purely from fixtures — no real screen, no execution.
  - _Requirements: 3.5, 7.2, 7.4, 9.1_

- [ ] 6. Phase 3 — Sight + Brain decision-only integration (no execution)
  - Pipe real `OmniParserSight` output into `QwenBrain`; log the `Decision` and the matched
    element; do NOT execute.
  - Add a diagnostic that attributes a wrong outcome to Sight (target element missing) vs
    Brain (wrong pick among present elements).
  - Integration test on real screens: decisions are sane; mismatch source is identifiable.
  - _Requirements: 9.2, 11.1_

- [ ] 7. Phase 4 build — Hands (UinputHands)
  - Implement `UinputHands` behind `GuiHands`: `Click{element_id}` → resolve in supplied obs
    → bbox center → physical px (reuse monitor_layout/DPI math) → uinput click;
    `ClickPoint{x,y}` → direct click; `Key{combo}` → standard shortcut table (new_tab/zoom/
    close/save, app-agnostic, no per-prompt hardcode) or literal → uinput; `Type`/`Scroll`.
  - Missing element id → explicit failure, no fallback click.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.6_

- [ ] 8. Phase 4 test — Hands isolation (external verify)
  - Feed FIXED Decisions (known bbox click, ClickPoint, Key ctrl+t, Type) → execute →
    verify via EXTERNAL observer (xdotool/wmctrl/screenshot-diff). No Brain.
  - Assert coordinate landing (where environment allows) and key effect; missing-id case
    fails explicitly; document INCONCLUSIVE where Wayland bounds prevent verification.
  - _Requirements: 4.5, 9.3, 9.4_

- [ ] 9. Phase 5 build — full observe-act loop
  - Wire `run_turn_v2`: observe → decide → safety/HITL gate (reuse existing) → execute →
    verify (screenshot-diff/re-observe) → re-observe; bound with step cap, no-progress
    (screen-hash) stop, cancel token, watchdog (reuse `GuiTurnBudgetTracker`).
  - Stream per-step events on the existing `gui_cognition:event` channel (incremental, not
    end-of-turn batch); record executed actions in the audit ledger.
  - Route GUI turns to V2 when `KRIA_GUI_COG_V2` is ON (V1 stays default until Task 12).
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

- [ ] 12. Phase 6 — flip V2 to default
  - After V2 meets the eval bar, flip `KRIA_GUI_COG_V2` to default ON (falsy = documented
    V1 rollback). Verify safety/HITL, audit, cancel/watchdog, verification, eval harness,
    uinput, model-swap all remain intact and green.
  - _Requirements: 10.2, 10.5_

- [ ] 13. Phase 6 — remove V1 over-built logic
  - Delete the over-built V1 paths (code AND logic): dual plan representation (typed_steps +
    legacy steps/action_kind), capability ladder, goal-pursuit guard, heavy upfront
    validators, upfront multi-step planner, large contract extraction. Collapse to V2's
    single representation and single code path; no dead branches.
  - Re-run core/desktop/UI suites + GUI real-verify; `cargo build` clean.
  - _Requirements: 10.3, 10.4, 10.5_

## Notes
- Read backend env flags live per turn (matching `KRIA_GUI_COG_*`); flip without rebuild.
- Each phase: build before test; isolation before integration; external verify, never
  fake-pass; INCONCLUSIVE where the environment blocks verification.
- Brain pluggability is the hinge: keep ALL model-specific logic inside the `GuiBrain`
  impl so UI-TARS (or any future model) drops in with no other changes.
- Do not touch `~/.kria/kria.db`, `~/.kria/secrets/`, `~/.kria/config.toml`.
- Commit when green; keep V2 behind the flag until Task 12.
