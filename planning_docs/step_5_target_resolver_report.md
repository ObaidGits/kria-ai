# Step 5 Target Resolver Report

Date: 2026-06-07

## Verdict

```text
Step 5: PASS
Ready for Step 6 Safety Gate + HITL
```

## Implementation Summary

Step 5 is implemented as a target-resolution-only gate for GUI Cognition selected mode. The selected-mode path now reaches:

```text
ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> TargetResolutionStarted
 -> TargetResolutionCompleted
 -> TurnCompleted
```

No Step 5 path calls the executor, safety gate execution, click/type/focus/hotkey execution, or `execute_and_verify`. Every target resolution result emits `can_execute=false`.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/resolver.rs`
- `crates/kria-core/src/agent/gui_cognition/mod.rs`
- `crates/kria-core/tests/gui_cognition_target_resolver_tests.rs`
- `crates/kria-core/tests/gui_cognition_backend_route_tests.rs`
- `crates/kria-desktop/src/commands/gui_cognition.rs`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `ui/src/stores/guiCognitionSession.test.ts`
- `ui/src/components/GuiCognitionPanel.test.tsx`
- `testing/suites/gui_cognition/scenarios/target_resolver.json`
- `testing/suites/gui_cognition/scenarios/backend_route_modularization.json`
- `testing/suites/gui_cognition/scenarios/goal_intent_understanding.json`
- `testing/suites/gui_cognition/scenarios/live_execution_hardening.json`
- `testing/suites/gui_cognition/scenarios/natural_work_prompts.json`
- `testing/suites/gui_cognition/scenarios/plan_validator.json`
- `testing/harness/models.py`
- `testing/suites/gui_cognition/manifest.json`
- `testing/inventory/current_inventory.json`
- `testing/inventory/migration_map.md`

## Resolver Schema

Added canonical Step 5 output:

```text
GuiTargetResolutionSummary
GuiTargetResolutionResult
GuiResolvedTarget
GuiTargetCandidate
TargetResolutionCompleted
```

The resolver emits safe target metadata only:

```text
label
role
control_id
target_hash
bounds
confidence
candidates
ambiguity/blocker reasons
can_proceed_to_safety_gate
can_execute=false
```

## Scoring And Ambiguity

Candidate collection now starts from `GuiContext.fused_controls`; executable controls are eligibility evidence, not the only candidate source.

Implemented:

- role groups: editable, button-like, selectable, link, tab, menu, app/window
- deterministic normalized label matching
- confidence threshold: resolved at `>= 0.85`
- ambiguity when duplicate same-label/same-role controls exist
- ambiguity when top candidates are within `0.10`
- confidence caps for missing bounds, disabled/hidden, partial, visual-only, and OCR-only candidates
- OCR-only and visual-only targets remain supporting candidates only, not resolved action targets
- app/window future steps can defer target resolution after planned `OpenApp`

## Runtime Behavior

For `valid_for_resolution`, KRIA emits `TargetResolutionStarted` and `TargetResolutionCompleted`.

For approval/clarification/blocked validation states, KRIA emits safe skipped/metadata target-resolution output where appropriate and still does not execute.

Legacy execution-era selected-mode expectations were updated to the Step 5 contract:

```text
TargetResolutionCompleted exists
ActionStarted absent
ActionCompleted absent
SafetyGateCompleted absent for Step 5-only safe action scenarios
ExecutionBlocked absent for Step 5-only safe action scenarios
```

## Tests Executed

Passed:

```text
cargo test -p kria-core --test gui_cognition_target_resolver_tests --quiet
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
./testing/run.sh gui_cognition --tag step5_target_resolver --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
```

## Failures Found And Fixed

- Browser-search plans were blocked because future in-browser controls cannot resolve before `OpenApp`. Fixed by adding deferred target-resolution results after planned app/window prerequisites.
- Approval-required replies were overwritten by skipped target-resolution messaging. Fixed by preserving `needs_approval` replies.
- Legacy same-path scenarios expected execution-era events after target resolution. Updated them to assert Step 5 metadata-only behavior.
- Hidden/disabled blocker wording was too narrow for the scenario assertion. Standardized blocker reason to `candidate is hidden or disabled`.

## Remaining Limitations

- Step 5 does not execute, open apps, click, type, focus, or trigger HITL approval.
- Future control targets behind an `OpenApp` prerequisite are deferred until a later execution/re-observation loop exists.
- OCR-only and visual-only controls are intentionally non-executable in this phase.

## Final Result

```text
Step 5: PASS
Ready for Step 6 Safety Gate + HITL
```
