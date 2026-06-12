# Step 7 Deterministic Executor Report

Date: 2026-06-08

## Verdict

```text
Step 7: PASS
Ready for Step 8 Verification/Recovery
```

## Implementation Summary

Step 7 is implemented as the first GUI Cognition selected-mode phase that can execute a GUI action. Execution is opt-in through `execution_mode` and can start only from a fresh, authorized `GuiActionProposal` produced by Step 6.

The selected-mode path now reaches:

```text
ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> TargetResolutionCompleted
 -> SafetyGateCompleted
 -> HitlDecisionRecorded when approval is required and approved
 -> ActionStarted
 -> ActionCompleted | ActionFailed | ExecutionBlocked
 -> ExecutionVerificationCompleted
 -> TurnCompleted
```

Step 7 v1 executes one immutable proposal per execution cycle. Multi-step autonomous execution is still deferred until continuation/recovery work can bind every executable step to its own proposal and authorization.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/executor.rs`
- `crates/kria-core/src/agent/gui_cognition/mod.rs`
- `crates/kria-core/src/agent/gui_cognition/resolver.rs`
- `crates/kria-core/tests/gui_cognition_executor_tests.rs`
- `crates/kria-core/tests/gui_cognition_backend_route_tests.rs`
- `crates/kria-desktop/src/commands/gui_cognition.rs`
- `crates/kria-desktop/src/commands/local_api.rs`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `ui/src/stores/guiCognitionSession.test.ts`
- `testing/suites/gui_cognition/scenarios/executor.json`
- `testing/suites/gui_cognition/manifest.json`
- `testing/harness/models.py`
- `testing/inventory/current_inventory.json`
- `testing/inventory/migration_map.md`

## Executor Contract

Added canonical Step 7 contracts:

```text
GuiExecutionMode
GuiExecutionAuthorizationSource
GuiExecutionRequest
GuiExecutionResult
GuiExecutionPreconditionReport
GuiPayloadVault
```

Execution modes:

```text
safety_only     -> stop after Step 6
execute_fixture -> deterministic fixture backend for CI and same-path tests
execute_live    -> live deterministic desktop backend
```

The default is `safety_only`, so Step 1-6 suites keep their no-execution invariant.

## Safety And Binding Rules

The executor blocks before `ActionStarted` unless all required bindings pass:

```text
proposal_id exists
proposal_hash matches
proposal is not expired
authorization source is valid
high/critical risk has matching HITL approved decision
target_hash matches for control actions
stable target identity matches when present
payload handle exists for TypeText/Paste
backend reports action execution is allowed
```

`ActionCompleted` is emitted only on backend success. Backend failures emit `ActionFailed`; precondition failures emit `ExecutionBlocked` and do not emit `ActionStarted`.

## Payload Vault

Step 7 now supports backend-only payload handles for text entry and paste. Raw payloads are not serialized into events, UI state, logs, or reports. Secret-like payloads remain represented by redacted summaries and hashes; verification uses state-change style evidence instead of raw text display.

## Backend Behavior

Fixture execution records deterministic evidence:

```text
action kind
target hash
payload hash only
backend_used=fixture_executor
```

Live desktop execution maps supported actions onto existing deterministic helpers:

```text
OpenApp -> open_application
SwitchWindow -> focus_window / window helper
FocusField -> AT-SPI focus/click path where available
TypeText -> focused text typing path
ClickControl -> click_ui_element / trusted target path
PressKey/Hotkey -> press_shortcut
Copy/Paste -> guarded Ctrl+C / Ctrl+V path
Scroll -> blocked if no supported backend helper is available
```

Destructive domain actions are not direct APIs. They may only execute later as an approved bound `ClickControl` or `PressKey`.

## UI Behavior

The GUI Cognition panel now renders:

```text
execution status
action type
backend used
proposal hash prefix
target hash prefix
control id when present
precondition result
verification result
safe error summary
recovery hint
```

The UI accepts old minimal action events and rich Step 7 action events. It does not render raw prompt, OCR text, screenshot content/path, clipboard contents, terminal/code content, raw text payload, or secrets.

## Tests Executed

Passed:

```text
cargo test -p kria-core --test gui_cognition_executor_tests --quiet
cargo test -p kria-core --test gui_cognition_safety_hitl_tests --quiet
cargo test -p kria-core --test gui_cognition_target_resolver_tests --quiet
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet
cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet
cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet
cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet
cargo test -p kria-desktop gui_cognition --quiet
cargo check -p kria-desktop --quiet
cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel HitlModal app.tool-choice
cd ui && npm run test:run
cd ui && npm run build
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py
./testing/run.sh gui_cognition --tag step7_executor --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step6_safety_hitl --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
git diff --check
```

Previously verified during this Step 7 pass:

```text
./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step4_validator --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step5_target_resolver --include-live --include-slow --fail-fast
```

Same-path reports:

```text
step7_executor:
  testing/eval_reports/kria_testing_gui_cognition_20260608_221555_3dbbe100.md

step6_safety_hitl:
  testing/eval_reports/kria_testing_gui_cognition_20260608_221608_3029714e.md

desktop_command:
  testing/eval_reports/kria_testing_gui_cognition_20260608_221715_39530156.md
```

## Live Proof

Manual selected-mode live proof was run through the local API with `execution_mode=execute_live`:

```text
Prompt: Open Chrome
Reply: Step 7 executed OpenApp through deterministic backend open_application and verified the result.
ActionStarted: true
ActionCompleted: true
ExecutionVerificationCompleted: true
execution.status: completed
backend_used: open_application
action_type: OpenApp
```

## Failures Found And Fixed

- Approval-required plans were not reaching target metadata resolution, so an approved risky Step 7 fixture could not execute its bound target. Fixed both the runtime gate and the resolver's internal readiness gate to allow approval-required metadata resolution without treating it as execution.
- The local live proof request initially failed with `401 Unauthorized`. Retried through the same token discovery path used by the harness.
- Event extraction for local proof initially looked at the wrong envelope level. Confirmed the canonical event path is `payload.event.type`.

## Remaining Limitations

- Step 7 v1 executes one bound proposal per cycle.
- Browser-search chains and other multi-step autonomous workflows are deferred until continuation/recovery can bind every executable step independently.
- Live `Scroll` execution fails closed unless a supported desktop scroll helper is available.
- Fixture execution is the CI authority for deterministic action evidence; live execution is manually verified.
- UI test/build output still includes existing warning noise, but all assertions pass.

## Final Result

```text
Step 7: PASS
Ready for Step 8 Verification/Recovery
```
