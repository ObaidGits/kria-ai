# Step 10 Multi-Step Workflow Runtime Report

Date: 2026-06-12

## Verdict

```text
Step 10: PASS
Ready for Step 11 Checkpoint / Resume
```

KRIA can now take a natural multi-step prompt and execute it one bound proposal
at a time, verifying each step before continuing, re-observing between
state-changing actions, applying safety/HITL per step, and pausing/blocking
safely on ambiguity, missing targets, risk, or verification failure. The runtime
reuses every Step 1-9 contract; it does not bypass target resolution, the safety
gate, HITL, the executor, the verifier, or recovery. State is in-memory only;
durable checkpoint/resume remains Step 11.

## Selected-Mode Pipeline

```text
GUI Cognition dropdown
 -> send_manual_tool_message
 -> ObservationCompleted -> ContextBuilt -> GoalContractCreated
 -> PlanCreated -> PlanValidationCompleted
 -> WorkflowRunStarted
 -> (per executable step)
      WorkflowStepStarted
       -> TargetResolutionStarted/Completed
       -> SafetyGateStarted/Completed
       -> HitlRequired/HitlDecisionRecorded (risky)
       -> ActionStarted -> ActionCompleted | ActionFailed | ExecutionBlocked
       -> ExecutionVerificationCompleted
       -> RecoveryAssessmentCompleted / RecoveryActionStarted/Completed | RecoveryBlocked (if needed)
      WorkflowStepCompleted | WorkflowStepBlocked
 -> next WorkflowStepStarted ...
 -> WorkflowRunCompleted | WorkflowRunBlocked | WorkflowRunPaused
 -> TurnCompleted -> UI panel
```

## Implementation Summary

- New module `crates/kria-core/src/agent/gui_cognition/workflow_runtime.rs` holds
  the contracts, step classification, and event builders.
- `mod.rs` gained `run_workflow`, which iterates the plan's typed steps and, for
  each executable step, reuses the existing per-step primitives.
- The runtime is gated behind `GuiTurnRequest.workflow_enabled` (default
  `false`), so the single-proposal path (Steps 1-9 same-path scenarios) is
  byte-for-byte unchanged. The desktop test harness opts in via
  `gui_cognition_test.workflow = true`.
- Per-step reuse strategy (no parallel system):
  - A per-step "sub-plan" (`single_step_plan`) carries exactly one typed step.
  - `resolve_step_target_for_workflow` resolves the step (via the real
    `resolve_plan_targets`) for control steps, or synthesizes a no-control
    "resolved" summary for app/window/key steps.
  - `safety_hitl::build_action_proposal_for_step` (refactored out of
    `build_action_proposal`) builds the step's own immutable proposal.
  - `handle_safety_gate` runs safety -> HITL -> executor -> verifier -> recovery
    for that single proposal, exactly as in Steps 6-9.
- Re-observation happens before resolving every step after the first, so a
  later step never resolves against stale state.

## Workflow Contracts

`GuiWorkflowRun`: workflow_run_id, session_id, workflow_id, turn_id,
goal_contract_id, plan_id, initial_context_id, current_context_id, status
(running|completed|paused|blocked|failed), current_step_index, step_count,
step_states[], completed_step_receipts[], blocked_reason, recovery_summary,
risk_level, requires_user_approval, execution_mode, prompt_hash.

`GuiWorkflowStepState`: step_id, step_index, step_type, summary, status
(pending|started|resolving_target|awaiting_approval|executing|verifying|
recovering|completed|skipped|blocked|failed), target_resolution_id, proposal_id,
proposal_hash, hitl_decision_id, execution_id, verification_id, recovery_id,
started_at_ms, completed_at_ms, blockers[], warnings[], can_continue,
prompt_hash.

`GuiWorkflowStepReceipt`: receipt_id, workflow_run_id, step_id, step_index,
step_type, status, target_hash, proposal_hash, execution_id, verification_id,
verification_status, recovery_id, recovery_status, started_at_ms,
completed_at_ms, safe_summary, prompt_hash.

Events: `WorkflowRunStarted`, `WorkflowStepStarted`, `WorkflowStepCompleted`,
`WorkflowStepBlocked`, `WorkflowRunCompleted`, `WorkflowRunBlocked`,
`WorkflowRunPaused` — safe IDs/statuses only, no raw prompt/payload.

## Runtime State Machine

```text
WorkflowRun(running)
  for each typed step:
    Observe/Summarize/WaitForState/VerifyState -> re-observe only -> completed (or blocked if no signal)
    AskClarification -> paused
    RequireApproval  -> paused (awaiting approval; risky prompts front-load this)
    Executable:
      re-observe (if not first) -> resolve target -> safety gate -> HITL (per proposal)
      -> execute -> verify -> recover (if needed)
      verified or safely recovered -> WorkflowStepCompleted + receipt -> continue
      needs approval / ambiguous / missing / verification_failed / recovery_blocked
        -> WorkflowStepBlocked -> WorkflowRunPaused or WorkflowRunBlocked -> stop
  all steps done -> WorkflowRunCompleted
```

## Per-Step Safety / Verification Rules

- Each executable step builds its own immutable proposal and its own HITL
  decision; approval for step N never authorizes step N+1 (different
  proposal_id/hash).
- Risky/high/critical steps pause for approval before execution; in practice the
  planner front-loads a `RequireApproval` step for submit/send/delete prompts,
  so the workflow pauses before any risky execution.
- A step is completed only when verification passes (Step 8) or recovery
  restores a verified-safe state (Step 9). No verified step, no next step.
- Ambiguous targets pause (`WorkflowRunPaused`); missing targets block; modals
  and backend failures block via the Step 9 assessment. Never guess.
- `safety_only` creates the run and step state and runs the gate, but never
  emits `ActionStarted`; the run pauses.

## UI Behavior

`ui/src/types/guiCognition.ts`, `ui/src/stores/guiCognitionSession.ts`, and
`ui/src/components/GuiCognitionPanel.tsx` handle the seven workflow events and a
`GuiCognitionWorkflowState`. The panel shows the run status, current step,
step-count progress, completed count, a per-step status list with blockers, and
the blocked reason. No raw prompt/OCR/screenshot/clipboard/secret/payload is
rendered.

## Files Changed

```text
crates/kria-core/src/agent/gui_cognition/workflow_runtime.rs   (new)
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/src/agent/gui_cognition/safety_hitl.rs        (build_action_proposal_for_step)
crates/kria-core/tests/gui_cognition_workflow_runtime_tests.rs (new)
crates/kria-core/tests/gui_cognition_backend_route_tests.rs    (workflow_enabled field)
crates/kria-desktop/src/commands/gui_cognition.rs              (cursor seq + workflow_enabled option)
crates/kria-desktop/src/commands/local_api.rs                  (workflow option parse)
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/stores/guiCognitionSession.test.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
testing/suites/gui_cognition/scenarios/workflow_runtime.json   (new)
testing/suites/gui_cognition/manifest.json
testing/harness/models.py
testing/inventory/current_inventory.json
testing/inventory/migration_map.md
```

## Tests Executed

Core (passed):

```text
gui_cognition_workflow_runtime_tests    7 passed
gui_cognition_recovery_tests           17 passed
gui_cognition_verification_tests       12 passed
gui_cognition_executor_tests            4 passed
gui_cognition_safety_hitl_tests         6 passed
gui_cognition_target_resolver_tests    10 passed
gui_cognition_llm_planner_tests        16 passed
gui_cognition_goal_contract_tests      10 passed
gui_cognition_context_builder_tests     5 passed
gui_cognition_backend_route_tests      19 passed
gui_cognition_observation_perception_tests 16 passed
kria-desktop gui_cognition              2 passed
cargo check -p kria-desktop            ok
```

UI (passed): `npm run check`, `npm run test:run` 108 passed, `npm run build` ok.

Harness: manifest/inventory 16 passed; `gui_cognition --profile ci` 5 passed;
`git diff --check` clean.

## Live Proof

The KRIA Desktop app was rebuilt and relaunched (same local API the Kria UI
uses, `http://127.0.0.1:3001`). All tiers ran live:

```text
Tier 1 — Step 10 workflow:   step10_workflow_runtime   7 passed, 0 failed
Tier 2a — Step 8 verify:     step8_verification        4 passed, 0 failed
Tier 2b — Step 9 recovery:   step9_recovery            5 passed, 0 failed
Tier 3 — broad desktop:      desktop_command         127 passed, 0 failed
```

Live Step 10 scenarios:

```text
step10.workflow.open_focus_completes           OpenApp -> FocusField, both verified, WorkflowRunCompleted
step10.workflow.verify_each_step_before_next   ExecutionVerificationCompleted precedes each WorkflowStepCompleted
step10.workflow.safety_only_no_action_started  run + step state created, no ActionStarted, WorkflowRunPaused
step10.workflow.ambiguous_target_pauses        duplicate targets -> WorkflowStepBlocked -> WorkflowRunPaused
step10.workflow.risky_submit_requires_approval submit -> RequireApproval pause, no ActionStarted
step10.workflow.denied_hitl_no_execution       deny -> no ActionStarted, no WorkflowRunCompleted
step10.workflow.no_raw_secret_leakage          forbidden raw prompt/OCR/screenshot/clipboard paths absent
```

Confirmed end-to-end live: a two-step OpenApp + FocusField workflow ran one bound
action at a time, verified each step, and reported
`Workflow completed 2 verified step(s) safely, one bound action at a time`
with `workflow_run.status = completed` and two completed step receipts.

## Failures Found and Fixed

- A per-step sub-plan containing only an app/window step made
  `resolve_plan_targets` return `skipped`, which made the safety gate early-exit
  and skip execution. Fixed by synthesizing a no-control "resolved" summary for
  non-target steps so the gate proceeds with an app/window proposal.
- The first integration test assumed an OpenApp window mismatch would hard-fail;
  in reality OpenApp with no concrete app hint goes inconclusive ->
  Step 9 ReObserve -> recovered. Re-targeted the block test to a missing-target
  case, which deterministically blocks before any `ActionStarted`.
- The "no leak" test initially asserted an arbitrary typed payload was hidden;
  arbitrary typed text is legitimately shown as the sanitized payload summary, so
  the test now asserts the real guarantee: the raw prompt is never echoed.

## Remaining Limitations

- The workflow runtime is gated behind `workflow_enabled` (default off) so
  Steps 1-9 behavior is preserved exactly; flipping the default to on for all
  multi-step plans is a separate, low-risk follow-up once single-step scenarios
  are migrated.
- State is in-memory only; pause/resume across restarts and duplicate-risky-action
  guards across sessions are Step 11 (Checkpoint/Resume).
- `execute_live` (real input backend) multi-step proofs remain a manual step on a
  real desktop with an action backend.
- Live verified multi-step coverage uses the OpenApp+FocusField sequence
  (reachable with deterministic fixtures). Longer heterogeneous sequences
  (TypeText/PressText/WaitForState verifying together) are proven by the
  in-process workflow tests and Steps 8/9 strategy tests.

## Acceptance Check

```text
[x] Multi-step plan executes one bound proposal at a time.
[x] Each executable step is verified before the next starts.
[x] Re-observe happens after state-changing actions.
[x] Safety/HITL applies per step (per immutable proposal).
[x] Denied/stale/risky failures stop the workflow safely.
[x] Ambiguous/missing targets pause or block, not guess.
[x] Recovery integrates safely but does not blind-retry.
[x] safety_only mode emits no ActionStarted.
[x] UI shows the workflow step status list.
[x] No raw prompt/OCR/screenshot/clipboard/secrets leak.
[x] Step 1-9 suites remain green.
[x] step10_workflow_runtime suite passes live (7/7).
[x] broad desktop_command suite passes live (127/0).
[x] git diff --check passes.
[x] Report says Step 10: PASS.
```
