# Step 8 Post-Action Verification Report

Date: 2026-06-11

## Verdict

```text
Step 8: PASS
Ready for Step 9 Recovery Loop
```

Post-action verification is now a production-grade, deterministic gate. A
backend-successful action is no longer treated as final: KRIA re-observes the
GUI, compares the actual state to the expected postcondition using an
action-specific verification strategy, and emits a rich
`ExecutionVerificationCompleted` result with explicit `verified`,
`verification_failed`, `inconclusive`, or `blocked` status. No raw
prompt/OCR/screenshot/clipboard/secret value is exposed.

## Selected-Mode Path (unchanged scope)

```text
GUI Cognition dropdown
 -> send_manual_tool_message
 -> ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> TargetResolutionCompleted
 -> SafetyGateCompleted
 -> HITL approval if required
 -> ActionStarted (only when authorized and execution_mode != safety_only)
 -> ActionCompleted | ActionFailed | ExecutionBlocked
 -> post-action ObservationCompleted (re-observe)
 -> ExecutionVerificationCompleted
 -> TurnCompleted
 -> UI panel
```

No normal-mode auto-routing, no execution from raw prompt/OCR/LLM text/raw
coordinates, no Step 6 bypass, and no multi-step autonomous runtime were added.
The `safety_only` / `execute_fixture` / `execute_live` boundary is preserved.

## Implementation Summary

Step 8 hardens the existing thin verifier stub into a contract-driven
re-observe-and-compare gate.

- The executor path now captures the pre-action observation
  (`context.observation`), executes one immutable proposal, re-observes the GUI,
  and runs `verify_post_action_detailed`.
- `ActionCompleted` is emitted on backend success only. The turn outcome is then
  driven by the verification verdict:
  - `verified` -> turn status `completed`
  - `verification_failed` -> turn status `verification_failed`
  - `inconclusive` -> turn status `inconclusive`
  - backend failure -> `ActionFailed`, turn status `blocked`
- The rich verification result is exposed at `gui_cognition.verification` and as
  the `ExecutionVerificationCompleted` event.

### Verification strategies

Selected per action kind, secret-aware:

```text
OpenApp        -> active_window_match
SwitchWindow   -> active_window_match
FocusField     -> focused_control
TypeText/Fill  -> text_present (non-secret) | state_changed (secret)
Paste          -> text_present (non-secret) | state_changed (secret)
ClickControl   -> result_visible (screen_changed | dialog_visible | postcondition)
PressKey/Hotkey-> screen_changed
Scroll         -> screen_changed
Copy           -> clipboard_changed (backend receipt only; never echoes clipboard)
```

### Safety / no-blind-success rules

- Backend failure -> `blocked`; nothing is verified, no success claimed.
- A strategy that cannot be evaluated from available evidence (e.g. screen hash
  unavailable) returns `inconclusive`, never a blind pass.
- For control actions, a missing or identity-mismatched bound target downgrades
  a would-be `verified` result to `verification_failed`.
- Secret payloads use `state_changed`; the raw text is never searched for or
  written into evidence/results.
- All evidence and state summaries are sanitized; active window labels and
  screen hashes are truncated/redacted.

## Contract / Schema Changes

New in `verifier.rs`:

```text
GuiVerificationStrategy (enum + as_str/from_str)
select_verification_strategy(action_kind, is_secret_payload)
GuiPostActionVerificationRequest
GuiPostActionVerificationResult
verify_post_action_detailed(request, pre_obs, post_obs, backend_success, expected_text, now)
```

`GuiPostActionVerificationResult` fields:

```text
verification_id, execution_id, proposal_id
status (verified | verification_failed | inconclusive | blocked)
verification_strategy
evidence[]
pre_state_summary, post_state_summary
matched_expected_state, target_still_present, target_identity_matches
confidence, safe_error_summary, recovery_hint, can_retry, prompt_hash
```

The legacy `verify_post_action` / `GuiVerificationReport` remain for backward
compatibility with the pre-Step-7 path and existing tests.

UI:

- `ExecutionVerificationCompleted` event type extended with the Step 8 fields.
- `GuiCognitionVerificationState` extended (strategy, evidence, pre/post state
  summaries, matched/target flags, safe error summary, recovery hint, can_retry).
- Store maps verification status to lifecycle (`verified -> completed`,
  `verification_failed`/`blocked -> failed`) and panel renders strategy, state
  summary, failure reason, and recovery hint.

## Files Changed

```text
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/tests/gui_cognition_verification_tests.rs   (new)
crates/kria-core/tests/gui_cognition_backend_route_tests.rs
crates/kria-desktop/src/commands/gui_cognition.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/stores/guiCognitionSession.test.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
testing/suites/gui_cognition/scenarios/post_action_verification.json  (new)
testing/suites/gui_cognition/scenarios/executor.json
testing/suites/gui_cognition/manifest.json
testing/harness/models.py
testing/inventory/current_inventory.json
testing/inventory/migration_map.md
```

## Runtime Behavior

- `execute_fixture` flows now re-observe after the action; the fixture
  perception provider tracks an observation sequence so Step 8 fixtures can model
  a real post-action screen change.
- `gui_cognition.execution.status` continues to report backend success/failure;
  `gui_cognition.verification.status` reports the verified verdict.

## UI Behavior

- Verification line shows status + strategy + confidence.
- Post-action state summary is shown.
- On failure, the safe error summary and recovery hint are shown; the lifecycle
  badge reflects failure, so a backend-successful-but-unverified action is not
  presented as success.

## Tests Executed

Core (passed):

```text
cargo test -p kria-core --test gui_cognition_verification_tests          (12 passed)
cargo test -p kria-core --test gui_cognition_backend_route_tests         (18 passed, incl. 2 new pipeline tests)
cargo test -p kria-core --test gui_cognition_executor_tests              (4 passed)
cargo test -p kria-core --test gui_cognition_target_resolver_tests       (10 passed)
cargo test -p kria-core --test gui_cognition_safety_hitl_tests           (6 passed)
cargo test -p kria-core --test gui_cognition_llm_planner_tests           (16 passed)
cargo test -p kria-core --test gui_cognition_goal_contract_tests         (10 passed)
cargo test -p kria-core --test gui_cognition_context_builder_tests       (5 passed)
cargo test -p kria-core --test gui_cognition_observation_perception_tests (16 passed)
cargo test -p kria-desktop gui_cognition                                 (2 passed)
cargo check -p kria-desktop                                              (ok)
```

UI (passed):

```text
cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel       (29 passed)
cd ui && npm run test:run                                                (100 passed)
cd ui && npm run build                                                   (ok)
```

Harness / same-path:

```text
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py   (16 passed)
./testing/run.sh gui_cognition --profile ci                              (5 passed)
./testing/run.sh gui_cognition --tag step8_verification --include-live --include-slow
./testing/run.sh gui_cognition --tag step7_executor --include-live --include-slow
git diff --check                                                         (clean)
```

## Same-Path Step 8 Scenarios (live-verified)

```text
step8.verify.focus_field_focused_control          FocusField   -> focused_control      verified
step8.verify.open_app_active_window_match          OpenApp      -> active_window_match   verified
step8.verify.click_screen_changed                  ClickControl -> result_visible        verified
step8.verify.verification_failed_no_blind_success  ClickControl -> result_visible        verification_failed
```

Each asserts the selected-mode path (`send_manual_tool_message`,
`llm_tool_loop=false`), the execute/verify event sequence, the executed
`action_type`, the verification status/strategy, and forbids raw
prompt/OCR/screenshot leakage.

Step 7 v1 executes one immutable proposal per cycle, and typing/paste sit behind
an `OpenApp`/`FocusField` step in the plan, so `TypeText`/secret `state_changed`
are not reachable as a first executable action yet (they become reachable with
the Step 10 multi-step runtime). Those strategies are fully proven by the core
verifier tests (`type_text_passes_when_expected_text_present`,
`type_text_fails_when_expected_text_absent`,
`secret_type_uses_state_changed_and_never_emits_text`,
`copy_reports_clipboard_changed_without_clipboard_content`).

## Three-Tier Live Verification

The KRIA Desktop application was launched (the same local API on
`http://127.0.0.1:3001` that the Kria UI uses) and all three tiers were run live
against it:

```text
Tier 1 — Step 8 same-path:   ./testing/run.sh gui_cognition --tag step8_verification --include-live --include-slow
   4 passed, 0 failed

Tier 2 — Step 7 executor:    ./testing/run.sh gui_cognition --tag step7_executor --include-live --include-slow
   6 passed, 0 failed

Tier 3 — broad desktop:      ./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow
   115 passed, 0 failed
```

Live findings fixed during this pass:

- The original focus scenario asserted `focused_control`, but "Focus the search
  box" resolves to an `OpenApp` first step. Split into two real scenarios: a
  focus phrasing that produces `FocusField` (`focused_control`) and an
  `OpenApp` scenario (`active_window_match`).
- The live `TypeText`/secret scenarios were unreachable as a first executable
  action in Step 7 v1; replaced with reachable scenarios and documented the v1
  single-proposal limitation. Strategy coverage for them remains in core tests.

## Failures Found and Fixes Applied

- Initial verifier construction left a non-existent field in the
  backend-failure branch; fixed to construct the result directly.
- Verifier recomputed the stable target identity with a `target_label`
  fallback that did not match the hint used at execution time, which falsely
  flagged identity mismatches. Fixed by recomputing identity with the exact
  resolved target hints and using `target_label` only as a window-match
  fallback.
- The existing static-screen click scenarios would (correctly) report
  `verification_failed` under Step 8. Switched those two Step 7 executor
  scenarios to a screen-changing fixture so a real click is genuinely verified,
  and added a dedicated no-change scenario for the failure path.
- A pipeline integration test over-asserted the exact strategy for a focus
  prompt whose first executable step is a window action; relaxed to assert the
  verified verdict (strategy specifics are covered by unit tests).

## Environment Note

The live/`slow` same-path execution scenarios require the desktop local API
(`http://127.0.0.1:3001`, the same API the Kria UI uses). The KRIA Desktop app
was launched and all three live tiers passed against it (4 + 6 + 115 = 0
failures). End-to-end execute-and-verify wiring is additionally proven in-process
by two pipeline integration tests
(`gui_cognition_pipeline_executes_and_verifies_focus_field_fixture` and
`gui_cognition_pipeline_reports_verification_failed_without_blind_success`) and
by the 12 core verifier tests. `execute_live` (real input backend) proofs remain
a manual step on a real desktop session with an action backend.

## Acceptance Check

```text
[x] Action success is not final until post-action verification passes.
[x] Verification failure is explicit and safe (verification_failed / inconclusive / blocked).
[x] ActionCompleted means backend success only.
[x] ExecutionVerificationCompleted reports whether the expected state was verified.
[x] No raw prompt/OCR/screenshot/clipboard/secret leaks into events or UI.
[x] Step 1-7 core suites remain green.
[x] step8_verification same-path suite passes live (4/4).
[x] step7_executor same-path suite passes live (6/6).
[x] broad desktop_command suite passes live (115/0).
[x] git diff --check passes.
```

## Remaining Limitations

- Step 8 emits a `recovery_hint` only; it does not perform recovery. Re-observe,
  failure classification, and safe one-retry are Step 9 scope.
- `clipboard_changed` relies on the backend receipt because clipboard content is
  intentionally never read into the observation pipeline.
- Live production-quality verification still depends on real OS/app source
  exposure; weak visual/OCR-only evidence remains supporting evidence and cannot
  create executable authority.
```
