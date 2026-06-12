# Step 6 Safety Gate + HITL Report

Date: 2026-06-07

## Verdict

```text
Step 6: PASS
Ready for Step 7 Deterministic Executor
```

## Implementation Summary

Step 6 is implemented as a Safety Gate + HITL authorization layer for GUI Cognition selected mode. The selected-mode path now reaches:

```text
ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> TargetResolutionCompleted
 -> SafetyGateStarted
 -> SafetyGateCompleted
 -> HitlRequired when approval is required
 -> HitlDecisionRecorded / HitlDecisionInvalidated when a fixture or command decision is recorded
 -> TurnCompleted
```

Step 6 does not execute GUI actions. Approval only creates `can_authorize_step7=true` for the same fresh bound proposal. Every Step 6 event and response summary keeps `can_execute=false`.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/safety_hitl.rs`
- `crates/kria-core/src/agent/gui_cognition/mod.rs`
- `crates/kria-core/tests/gui_cognition_safety_hitl_tests.rs`
- `crates/kria-core/tests/gui_cognition_backend_route_tests.rs`
- `crates/kria-desktop/src/commands/app_commands.rs`
- `crates/kria-desktop/src/commands/app_state.rs`
- `crates/kria-desktop/src/commands/gui_cognition.rs`
- `crates/kria-desktop/src/commands/local_api.rs`
- `crates/kria-desktop/src/commands/runtime.rs`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `ui/src/components/HitlModal.tsx`
- `ui/src/stores/guiCognitionSession.test.ts`
- `ui/src/components/GuiCognitionPanel.test.tsx`
- `ui/src/components/HitlModal.test.tsx`
- `testing/suites/gui_cognition/scenarios/safety_gate_hitl.json`
- `testing/suites/gui_cognition/scenarios/natural_work_prompts.json`
- `testing/harness/models.py`
- `testing/suites/gui_cognition/manifest.json`
- `testing/inventory/current_inventory.json`
- `testing/inventory/migration_map.md`

## Safety + HITL Schema

Added canonical Step 6 contracts:

```text
GuiActionProposal
GuiSafetyGateResult
GuiHitlDecision
GuiHitlDecisionFixture
GuiHitlProposalStore
```

The action proposal binds:

```text
request_id
proposal_id
proposal_hash
target_hash
goal_contract_id
plan_id
validation_id
resolution_id
context_id
observation_id
step_id
action_type
text_payload_hash
risk_level
expected_postcondition
prompt_hash
```

The proposal hash is stable and deterministic over safe identity fields. It excludes raw prompt, raw OCR, clipboard text, screenshots, terminal/code content, and unredacted secrets.

## Runtime Behavior

Implemented:

- low-risk safe proposals emit `SafetyGateCompleted` with `safe_no_approval_required`
- high/critical and sensitive proposals emit `SafetyGateCompleted` plus `HitlRequired`
- deny records `HitlDecisionRecorded` with `can_authorize_step7=false`
- fresh approve records `HitlDecisionRecorded` with `can_authorize_step7=true`
- stale/expired/hash-mismatch approvals record `HitlDecisionInvalidated`
- no-action plans skip Step 6 proposal generation cleanly
- approval-required metadata-only plans still produce SafetyGate/HITL even when no control target resolves yet

Desktop `approve_action` / `deny_action` now check the GUI proposal registry first. GUI approvals are validated and recorded without invoking the generic executor path. Non-GUI HITL requests continue using the existing generic gateway.

## UI Behavior

The existing HITL modal is reused. For GUI Cognition proposals it renders only sanitized `args.gui_cognition` metadata:

```text
action type
target label/role
risk level and reason
proposal hash prefix
target hash prefix
expected result
expiry metadata
```

The modal copy now states that approval authorizes the bound proposal for the executor after freshness checks, and that Step 6 will not execute it.

## Tests Executed

Passed:

```text
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
./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step4_validator --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step5_target_resolver --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step6_safety_hitl --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
```

Notes:

- Same-path suites were run against a live KRIA local API started with `cargo tauri dev --config crates/kria-desktop/tauri.conf.json`.
- UI tests still print the pre-existing Solid/app-store test-environment warnings, but all assertions pass.

## Failures Found And Fixed

- Inventory validation failed after adding the new Rust safety test. Fixed by regenerating `testing/inventory/current_inventory.json` and `testing/inventory/migration_map.md`.
- Step 3 valid observe/planning-only scenarios were marked `blocked` because Step 6 tried to build an action proposal from skipped target resolution. Fixed by skipping Step 6 proposal generation for non-risky no-action plans.
- Risky approval-only LLM plans were skipped too aggressively after that fix. Tightened the skip condition so approval-required plans still emit SafetyGate/HITL.
- Older goal-contract assertions expected the phrase `approval required`; Step 6 now includes that explicit phrase in HITL replies.
- Natural work prompt scenarios still forbade `SafetyGateCompleted` for safe action proposals. Updated those scenarios because Step 6 intentionally emits auditable safety results while still forbidding execution.

## Remaining Limitations

- Step 6 does not execute, click, type, focus, open apps, submit, send, delete, pay, or install.
- Approval freshness is checked at Step 6 decision time; Step 7 must re-observe and revalidate the same proposal/target binding before execution.
- Only one active GUI Cognition proposal per session is supported by the registry.
- Live HITL approve/deny through the modal is wired via existing commands, but same-path automation uses deterministic decision fixtures to avoid hanging tests.

## Final Result

```text
Step 6: PASS
Ready for Step 7 Deterministic Executor
```
