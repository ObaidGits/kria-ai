# Step 11 Checkpoint / Resume Report

Date: 2026-06-12

## Verdict

```text
Step 11: PASS (durable app-restart persistence: PARTIAL — in-memory store)
Ready for Step 12 Real Task Eval Suite
```

KRIA persists safe workflow progress as a checkpoint, can pause and resume a
multi-step workflow, re-observes before continuing, validates checkpoint
integrity and target/approval bindings, invalidates stale approvals, blocks
duplicate risky actions, and continues only from the correct next safe step. A
checkpoint can restore state but not trust: resume always re-observes and
revalidates, and fails closed. Cross-process app-restart persistence uses an
in-memory store keyed by session (process-local), so full app-restart durability
is PARTIAL by design; everything else is PASS.

## Selected-Mode Pipeline

```text
WorkflowRunStarted -> per-step (resolve/safety/HITL/execute/verify/recover)
 -> WorkflowStepCompleted -> WorkflowCheckpointSaved
 -> pause / restart / approval delay
 -> WorkflowResumeRequested -> WorkflowCheckpointLoaded -> re-observe
 -> validate integrity / freshness / target / approval / duplicate guard
 -> WorkflowResumeValidated | WorkflowResumeRejected
    | WorkflowApprovalInvalidated | WorkflowDuplicateActionBlocked
 -> continue next safe step OR block
 -> WorkflowRunCompleted | WorkflowRunBlocked | WorkflowRunPaused
```

## Implementation Summary

- New module `crates/kria-core/src/agent/gui_cognition/checkpoint.rs`: pure
  checkpoint contracts, `checkpoint_hash`, `build_checkpoint`, and the
  deterministic `validate_resume` (all safety rules live here).
- `workflow_runtime.rs`: the step receipt was extended into a completed-step
  receipt (`proposal_id`, `action_type`, `risk_level`, `side_effect_kind`,
  `receipt_hash`) plus `side_effect_kind_for` / `side_effect_is_risky` /
  `compute_receipt_hash`.
- `mod.rs` `run_workflow`: saves a checkpoint after each completed step and on
  pause/block (`WorkflowCheckpointSaved`), and handles resume — re-observe
  (already done before the loop), `WorkflowResumeRequested`,
  `WorkflowCheckpointLoaded`, `validate_resume`, then
  `WorkflowResumeValidated`/`Rejected`, seeding completed receipts so completed
  steps are never replayed and continuing per-step HITL/verify/recover for the
  pending step. Reject fails closed before any `ActionStarted`.
- `GuiTurnRequest` gained `resume_checkpoint` + `resume_reason`. Approval
  freshness on resume is enforced by re-running Step 6 on the pending step;
  `validate_resume` owns integrity, identity, expiry, screen/target freshness,
  and the duplicate-risky-action guard.
- Storage: a process-local in-memory checkpoint store keyed by `session_id` in
  the desktop layer (`GUI_WORKFLOW_CHECKPOINTS`). The desktop saves the latest
  checkpoint after each turn and loads it on a `workflow_resume` request.

## Checkpoint Contracts

`GuiWorkflowCheckpoint` (schema v1): checkpoint_id, workflow/session/turn IDs,
goal_contract_id, plan_id, prompt_hash, current_step_index, step_count,
step_states[], completed_step_receipts[], pending_step/proposal/target/identity
hashes, pending_hitl_request_id, approved_decision_id/hash, last_observation_id,
last_context_id, last_screen_hash_prefix, last_active_window_hash,
created_at_ms, expires_at_ms, risk_level, requires_user_approval,
checkpoint_hash, source_evidence[], can_resume, can_execute=false.

`GuiCompletedStepReceipt` (the extended `GuiWorkflowStepReceipt`): adds
proposal_id, action_type, risk_level, side_effect_kind
(none|local_ui|external_submit|destructive|payment|install_system), and
receipt_hash.

`GuiWorkflowResumeRequest` / `GuiWorkflowResumeResult`: resume_id, checkpoint_id,
workflow_run_id, status (resumed|needs_reobserve|needs_approval|stale_rejected|
target_mismatch_rejected|approval_invalidated|duplicate_action_blocked|blocked),
next_step_id/index, invalidated_approvals[], duplicate_action_guards[],
blockers[], warnings[], safe_explanation, can_continue_workflow, can_execute=false.

Events: WorkflowCheckpointSaved, WorkflowResumeRequested,
WorkflowCheckpointLoaded, WorkflowResumeValidated, WorkflowResumeRejected,
WorkflowApprovalInvalidated, WorkflowDuplicateActionBlocked — safe IDs, hash
prefixes, and statuses only.

## Resume Validation Rules (fail closed)

```text
checkpoint_hash mismatch                 -> blocked (integrity)
workflow_run_id/session_id mismatch      -> blocked
now > expires_at_ms                      -> stale_rejected
pending step already a risky receipt     -> duplicate_action_blocked
pending target not present / id changed  -> target_mismatch_rejected
risky pending + denied decision          -> blocked
risky pending + approved fresh matching  -> resumed (proposal_hash+target_hash+TTL+no screen change)
risky pending + mismatched approval      -> approval_invalidated
risky pending + screen changed, no decision -> approval_invalidated
risky pending + no decision, same screen -> needs_approval
safe pending + screen changed            -> needs_reobserve
safe pending + same screen               -> resumed
```

The pending target identity is failed closed: a changed screen means the bound
target can no longer be trusted, so the runtime marks target-present/identity as
false on resume.

## Duplicate Risky Action Guard

Completed receipts carry `side_effect_kind`. On resume, if the pending step id
matches a completed receipt with a risky side effect
(external_submit/destructive/payment/install_system), the resume is
`duplicate_action_blocked` and never re-executes. Completed steps (by receipt
index) are also skipped during the resume loop so no completed action is
replayed.

## Approval Invalidation Rules

A pending approval is invalidated on: proposal_hash mismatch, target_hash
mismatch, screen change before approval, expired checkpoint, denied decision, or
a decision that no longer authorizes (can_authorize_step7=false). Fresh approval
only resumes when proposal_hash + target_hash match, the checkpoint is unexpired,
and the screen is unchanged.

## Storage Model

In-memory, process-local store keyed by `session_id`. It survives within a
running app across pause/HITL/resume turns (proven live). It does NOT survive a
full OS-process restart of the desktop app — that durable persistence is the
PARTIAL item; the contracts serialize safely to JSON, so wiring a durable
session/AppState store is a contained follow-up with no checkpoint format change.

## UI Behavior

`ui/src/types/guiCognition.ts`, `ui/src/stores/guiCognitionSession.ts`, and
`ui/src/components/GuiCognitionPanel.tsx` handle the seven new events and a
`GuiCognitionCheckpointState`. The panel shows checkpoint id/hash prefix,
completed step count, pending step, resume status, invalidated-approval reason,
and the duplicate-action-blocked guard. No raw prompt/OCR/screenshot/clipboard/
secret/payload is rendered.

## Files Changed

```text
crates/kria-core/src/agent/gui_cognition/checkpoint.rs           (new)
crates/kria-core/src/agent/gui_cognition/workflow_runtime.rs     (completed-step receipt + side-effect helpers)
crates/kria-core/src/agent/gui_cognition/mod.rs                  (save + resume orchestration)
crates/kria-core/tests/gui_cognition_checkpoint_resume_tests.rs  (new)
crates/kria-core/tests/gui_cognition_workflow_runtime_tests.rs   (resume integration tests)
crates/kria-core/tests/gui_cognition_backend_route_tests.rs      (request fields)
crates/kria-desktop/src/commands/gui_cognition.rs                (in-memory checkpoint store + resume option)
crates/kria-desktop/src/commands/local_api.rs                    (workflow_resume/resume_reason parse)
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/stores/guiCognitionSession.test.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
testing/suites/gui_cognition/scenarios/checkpoint_resume.json    (new)
testing/suites/gui_cognition/manifest.json
testing/harness/models.py
testing/inventory/current_inventory.json
testing/inventory/migration_map.md
```

## Tests Executed

Core (passed):

```text
gui_cognition_checkpoint_resume_tests   18 passed
gui_cognition_workflow_runtime_tests    10 passed (incl. 3 resume integration tests)
gui_cognition_recovery_tests            17 passed
gui_cognition_verification_tests        12 passed
gui_cognition_executor_tests             4 passed
gui_cognition_safety_hitl_tests          6 passed
gui_cognition_target_resolver_tests     10 passed
gui_cognition_llm_planner_tests         16 passed
gui_cognition_goal_contract_tests       10 passed
gui_cognition_context_builder_tests      5 passed
gui_cognition_backend_route_tests       19 passed
gui_cognition_observation_perception_tests 16 passed
kria-desktop gui_cognition               2 passed
cargo check -p kria-desktop             ok
```

UI (passed): `npm run check`, `npm run test:run` 112 passed, `npm run build` ok.

Harness: manifest/inventory 16 passed; `gui_cognition --profile ci` 5 passed;
`git diff --check` clean.

## Live Proof

The KRIA Desktop app was rebuilt and relaunched (same local API the Kria UI
uses, `http://127.0.0.1:3001`). All tiers ran live:

```text
Tier 1 — Step 11 checkpoint/resume:  step11_checkpoint_resume   5 passed, 0 failed
Tier 2a — Step 8 verify:             step8_verification         4 passed, 0 failed
Tier 2b — Step 9 recovery:           step9_recovery             5 passed, 0 failed
Tier 2c — Step 10 workflow:          step10_workflow_runtime    7 passed, 0 failed
Tier 3 — broad desktop:              desktop_command          132 passed, 0 failed
```

Live Step 11 scenarios:

```text
step11.checkpoint.saved_after_each_step          WorkflowCheckpointSaved + completed run
step11.checkpoint.pending_hitl_saved             risky pause saves a checkpoint, no ActionStarted
step11.resume.validated_continues_without_replay save -> resume -> ResumeRequested/CheckpointLoaded/ResumeValidated, no replay (no ActionStarted)
step11.resume.safety_only_no_action_started      resume in safety_only never starts an action
step11.resume.no_raw_secret_leakage              forbidden raw prompt/OCR/screenshot/clipboard/checkpoint paths absent
```

Confirmed live cross-turn resume in one session:
`WorkflowResumeRequested -> WorkflowCheckpointLoaded -> WorkflowResumeValidated`
with no `ActionStarted` (completed steps were not replayed).

## Failures Found and Fixed

- Resume tests initially returned `blocked` because the test request used a
  hardcoded `workflow_run_id` that did not match the generated hash; fixed by
  deriving the resume request from the checkpoint.
- A fully-completed run's checkpoint points `current_step_index` at the last
  completed step, so a naive `index < resume_start_index` skip would re-execute
  that step on resume. Fixed by also skipping any step index that has a seeded
  completed receipt, guaranteeing no completed action is replayed.

## Remaining Limitations

- Durable app-restart persistence is PARTIAL: the checkpoint store is in-memory
  and process-local. The checkpoint serializes safely to JSON, so a durable
  session/AppState store is a contained follow-up with no format change.
- Same-path reject paths (denied/stale/target-mismatch/duplicate-risky) are hard
  to reach live with static fixtures within one process (risky steps front-load
  a RequireApproval pause and never complete; screen hashes reset per turn).
  These are fully proven by the 18 deterministic core resume tests and the
  in-process resume integration tests (validated continue + screen-change reject
  before action).
- `execute_live` (real input backend) resume proofs remain a manual step on a
  real desktop with an action backend.

## Acceptance Check

```text
[x] Checkpoint saved after completed/pending workflow steps.
[x] Resume always re-observes before executable action.
[x] Fresh matching approval can resume the same pending proposal.
[x] Denied/stale/mismatch approval never executes.
[x] Completed risky action is not duplicated.
[x] Target/context/screen mismatch blocks before ActionStarted.
[x] Checkpoint contains no raw prompt/OCR/screenshot/clipboard/secrets.
[x] UI shows checkpoint/resume/invalidated/duplicate-guard state.
[x] Step 1-10 suites remain green.
[x] step11_checkpoint_resume suite passes live (5/5).
[x] broad desktop_command suite passes live (132/0).
[x] git diff --check passes.
[~] Durable app-restart persistence: PARTIAL (in-memory store).
```
