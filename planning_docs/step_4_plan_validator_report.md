# Step 4 Plan Validator Report

## Implementation Summary

Step 4 is implemented as a deterministic execution-readiness gate for GUI Cognition selected mode. It runs after `PlanCreated` and before any target-resolution or execution path:

```text
ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> UI panel
```

The validator keeps Step 3 planner/schema validation intact and adds runtime-readiness validation through `validate_plan_for_resolution`. Step 4 validates `GuiGoalContract`, `GuiContext`, and `GuiLlmPlan.typed_steps`, emits a richer `PlanValidationCompleted` payload, and always reports `can_execute=false`.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/llm_planner.rs`
- `crates/kria-core/src/agent/gui_cognition/mod.rs`
- `crates/kria-core/src/agent/gui_cognition/goal_contract.rs`
- `crates/kria-core/src/agent/gui_cognition/planner.rs`
- `crates/kria-core/tests/gui_cognition_llm_planner_tests.rs`
- `crates/kria-core/tests/gui_cognition_goal_contract_tests.rs`
- `crates/kria-core/tests/gui_cognition_backend_route_tests.rs`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/stores/guiCognitionSession.test.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `ui/src/components/GuiCognitionPanel.test.tsx`
- `testing/harness/models.py`
- `testing/suites/gui_cognition/manifest.json`
- `testing/suites/gui_cognition/scenarios/plan_validator.json`
- `testing/suites/gui_cognition/scenarios/goal_intent_understanding.json`
- `testing/suites/gui_cognition/scenarios/natural_work_prompts.json`
- `testing/inventory/current_inventory.json`
- `testing/inventory/migration_map.md`

## Validator Rules

Step 4 now rejects or blocks plans for unsupported step types, `allowed_to_execute=true`, raw coordinates, missing verification, high/critical actions without approval, goal contradictions, shell/native/browser automation commands, raw prompt/OCR/screenshot/clipboard/code/terminal leakage, secret-like unredacted values, stale context/goal IDs, control actions with no target hint, OCR-only executable targets, and invalid ordering.

Readiness statuses:

```text
valid_for_resolution
needs_clarification
approval_required
blocked
rejected
```

Every result includes:

```text
can_execute=false
can_proceed_to_target_resolution=<true only for valid_for_resolution>
step_results[]
blockers/warnings
sanitized validation_errors
```

## Failures Found

- Form-fill prompts with no provided values previously produced a `TypeText` step and were then blocked by Step 4. This was corrected to ask clarification instead of pretending values exist.
- Legacy tests and scenarios treated negated risk phrases like `do not submit` as approval-required. Risk extraction now respects negation, and fixtures were updated so safe browser-search planning is not blocked.
- A broad scenario expectation was initially patched too broadly; it was corrected so only the no-values form-fill case expects clarification.

## Fixes Applied

- Added `validate_plan_for_resolution` and rich `GuiPlanValidationReport` fields.
- Added per-step validation results.
- Added Step 4 runtime gate after `PlanCreated`.
- Added UI store/panel rendering for readiness, target-resolution readiness, execution-disabled state, blockers, warnings, and per-step validation.
- Added `step4_validator` scenarios.
- Added regression tests for raw coordinates, missing verification, approval-required plans, missing targets, redacted secret labels, and form-fill-without-values clarification.
- Added negation-aware risk phrase handling.

## Tests Executed

Passed:

```bash
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet
cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet
cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet
cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet
cargo test -p kria-desktop gui_cognition --quiet
cargo check -p kria-desktop --quiet

cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel app.tool-choice HitlModal
cd ui && npm run test:run
cd ui && npm run build

python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py
./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step4_validator --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
```

UI tests still emit existing Solid disposal/null mock warnings in app-store tests, but all assertions pass.

## Remaining Limitations

- Step 4 validates readiness for Step 5 Target Resolver only; it does not resolve controls.
- Step 4 does not execute actions and does not implement real HITL approval execution.
- Form-fill prompts without concrete field values now correctly ask for clarification.
- Existing compatibility display-only approval events may still be emitted for risky plans, but no action is executed.

## Final Verdict

```text
Step 4: PASS
Ready for Step 5 Target Resolver.
```
