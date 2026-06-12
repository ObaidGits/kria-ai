# CONTINUE GUI UPGRADE

Date: 2026-06-11

This document is the implementation handoff for KRIA GUI Cognition automation from the current state after Step 7 through Step 11. It is intended to be detailed enough that a new chat or a different AI model can continue the work without relying on prior conversation context.

## Goal

KRIA GUI Cognition selected mode should turn layman prompts into safe verified GUI work:

```text
Layman prompt
 -> KRIA observes the GUI
 -> understands the task
 -> creates a typed plan
 -> validates the plan
 -> resolves exact targets
 -> asks approval only when needed
 -> executes deterministic GUI actions
 -> verifies each result
 -> recovers or pauses safely
 -> checkpoints and resumes without duplicate risky actions
```

The product goal is not "LLM controls the desktop". The product goal is:

```text
LLM may advise planning.
Deterministic contracts decide intent, targets, safety, execution, verification, recovery, and resume.
No raw prompt, OCR text, hidden instruction, stale target, or unapproved risky action can execute.
```

## Current State

Current implementation point:

```text
Step 1: Real Perception                 PASS with known visual/OCR follow-ups
Step 2: Goal Understanding              PASS
Step 3: GUI Planner Orchestrator        PASS
Step 4: Plan Validator                  PASS
Step 5: Target Resolver                 PASS
Step 6: Safety Gate + HITL              PASS
Step 7: Deterministic Executor          PASS
Step 8: Post-Action Verification        PASS
Step 9: Recovery Loop                   PASS
Step 10: Multi-Step Workflow Runtime    PASS
Step 11: Checkpoint / Resume            PASS (durable restart persistence PARTIAL)
Step 12: Real Task Eval Suite           PASS (9/9 live; execute_live physical-input proof manual)
```

Step 7 was implemented and verified. The report is:

```text
planning_docs/step_7_deterministic_executor_report.md
```

Step 7 verdict:

```text
Step 7: PASS
Ready for Step 8 Verification/Recovery
```

Important current limitation:

```text
Step 7 v1 executes one immutable action proposal per execution cycle.
Multi-step autonomous workflows are intentionally deferred until Step 8-10.
```

## Must-Preserve Scope

Scope is GUI Cognition selected mode only:

```text
GUI Cognition dropdown
 -> send_manual_tool_message
 -> GUI Cognition pipeline
```

Do not implement or change:

```text
normal-mode GUI auto-routing
global planner behavior
native LLM tool-loop execution
execution from raw prompt
execution from OCR/context instructions
untrusted coordinate execution
direct send/delete/pay/install APIs
broad AI OS architecture
```

## Current Selected-Mode Pipeline

The selected-mode event path after Step 7 is:

```text
TurnStarted
 -> ObservationCompleted
 -> ContextBuilt
 -> GoalContractCreated
 -> PlanCreated
 -> PlanValidationCompleted
 -> TargetResolutionStarted
 -> TargetResolutionCompleted
 -> SafetyGateStarted
 -> SafetyGateCompleted
 -> HitlRequired when approval is required
 -> HitlDecisionRecorded / HitlDecisionInvalidated when a decision exists
 -> ActionStarted when execution is authorized and mode allows execution
 -> ActionCompleted | ActionFailed | ExecutionBlocked
 -> ExecutionVerificationCompleted
 -> TurnCompleted
```

No action may start unless all these are true:

```text
valid GuiActionProposal exists
proposal_hash matches
target_hash matches for control actions
stable target identity still matches
authorization is fresh
high/critical risk has HITL approval
backend is healthy and execution_mode allows execution
```

Core production rule:

```text
No proposal, no execution.
No fresh authorization, no execution.
No fresh target match, no execution.
No HITL approval for risky action, no execution.
```

## Execution Modes

Step 7 added explicit execution modes:

```text
safety_only
execute_fixture
execute_live
```

Behavior:

```text
safety_only:
  stop after Step 6
  no ActionStarted
  default for Step 1-6 tests

execute_fixture:
  deterministic fake executor
  real contracts and events
  CI and same-path Step 7 tests

execute_live:
  real deterministic desktop backend
  manual/live proofs only unless explicitly added to guarded live tests
```

Do not remove this boundary. It protects old no-execution suites from Step 7 behavior.

## Key Reports To Read First

Read in this order before changing code:

```text
planning_docs/gui_cognition_perception_completion_report.md
planning_docs/step_2_goal_understanding_report.md
planning_docs/step_3_llm_gui_planner_report.md
planning_docs/step_4_plan_validator_report.md
planning_docs/step_5_target_resolver_report.md
planning_docs/step_6_safety_hitl_report.md
planning_docs/step_7_deterministic_executor_report.md
```

Step 1 caveat:

```text
Core perception is usable and tested.
Visual controls and OCR performance still have partial follow-ups:
- visual model-backed sidecar quality depends on environment/model enablement
- full changed-region OCR and RapidOCR/PaddleOCR benchmarking are optional follow-ups
```

These are not blockers for Step 8, but do not treat weak visual/OCR evidence as executable authority.

## Key Code Map

Core GUI Cognition:

```text
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/src/agent/gui_cognition/perception.rs
crates/kria-core/src/agent/gui_cognition/context.rs
crates/kria-core/src/agent/gui_cognition/goal_contract.rs
crates/kria-core/src/agent/gui_cognition/planner.rs
crates/kria-core/src/agent/gui_cognition/llm_planner.rs
crates/kria-core/src/agent/gui_cognition/validator.rs
crates/kria-core/src/agent/gui_cognition/resolver.rs
crates/kria-core/src/agent/gui_cognition/safety_hitl.rs
crates/kria-core/src/agent/gui_cognition/executor.rs
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/recovery.rs
```

Desktop bridge and selected-mode command path:

```text
crates/kria-desktop/src/commands/gui_cognition.rs
crates/kria-desktop/src/commands/local_api.rs
crates/kria-desktop/src/commands/app_commands.rs
crates/kria-desktop/src/commands/app_state.rs
crates/kria-desktop/src/commands/runtime.rs
crates/kria-desktop/src/commands/sessions.rs
crates/kria-desktop/src/commands/gui_automation_control.rs
```

UI:

```text
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/stores/guiCognitionSession.test.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
ui/src/components/HitlModal.tsx
ui/src/components/HitlModal.test.tsx
ui/src/stores/app.ts
ui/src/stores/app.tool-choice.test.ts
```

Harness and same-path suites:

```text
testing/harness/models.py
testing/harness/drivers/desktop_chat_command.py
testing/harness/tests/test_manifest_validation.py
testing/harness/tests/test_inventory.py
testing/suites/gui_cognition/manifest.json
testing/suites/gui_cognition/scenarios/goal_intent_understanding.json
testing/suites/gui_cognition/scenarios/llm_gui_planner.json
testing/suites/gui_cognition/scenarios/plan_validator.json
testing/suites/gui_cognition/scenarios/target_resolver.json
testing/suites/gui_cognition/scenarios/safety_gate_hitl.json
testing/suites/gui_cognition/scenarios/executor.json
testing/suites/gui_cognition/scenarios/live_execution_hardening.json
testing/suites/gui_cognition/scenarios/natural_work_prompts.json
```

Core test files:

```text
crates/kria-core/tests/gui_cognition_observation_perception_tests.rs
crates/kria-core/tests/gui_cognition_context_builder_tests.rs
crates/kria-core/tests/gui_cognition_goal_contract_tests.rs
crates/kria-core/tests/gui_cognition_llm_planner_tests.rs
crates/kria-core/tests/gui_cognition_backend_route_tests.rs
crates/kria-core/tests/gui_cognition_target_resolver_tests.rs
crates/kria-core/tests/gui_cognition_safety_hitl_tests.rs
crates/kria-core/tests/gui_cognition_executor_tests.rs
```

## Safety And Privacy Invariants

These invariants must remain true in every step:

```text
raw prompt never executes directly
raw OCR never creates intent
raw coordinates from user/LLM/OCR are rejected
hidden prompt text is ignored
clipboard contents are not emitted
screenshot path/content is not emitted
terminal output/code contents are not emitted
secrets are redacted or hashed
HITL approval binds proposal_id/proposal_hash/target_hash
stale approvals do not authorize execution
denied approvals do not authorize execution
ActionCompleted means backend success only
ActionFailed means backend failure
ExecutionBlocked means no backend action started
```

For text typing and paste:

```text
events/UI show only text_payload_summary/text_payload_hash
raw payload lives only in backend-only payload vault
secret-like payload requires approval
secret payload verification must use state_changed style evidence, not raw text display
```

## Step 1: Real Perception

Status:

```text
PASS for core selected-mode perception.
Some visual/OCR production-quality improvements remain partial follow-ups.
```

Implemented:

```text
screenshot capture and screen hash
OCR status and bounded OCR output
accessibility tree summary
active window authority
focused control authority
fused controls
visible buttons/inputs summary
monitor/DPI metadata
accessibility health scoring
visual controls as supporting evidence
OCR injection handling
```

Important rule:

```text
Accessibility/fused controls can support executable authority.
Visual-only/OCR-only evidence is supporting evidence only.
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet
cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet
cargo test -p kria-desktop gui_cognition --quiet
./testing/run.sh gui_cognition --tag perception_completion --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag focus_authority --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag control_fusion --include-live --include-slow --fail-fast
```

Manual prompt:

```text
What is on my screen?
```

Expected panel evidence:

```text
active app/window
visible buttons/inputs
OCR status
accessibility status
screenshot hash
focus source
control fusion confidence
```

## Step 2: Goal Understanding

Status:

```text
PASS
```

Implemented:

```text
Natural prompt -> GuiGoalContract
intent/action_type
target app kind/hint
target control hints
query/text summaries and hashes
expected final state
risk level
approval requirement
ambiguity count
prompt hash
sanitized source evidence
```

Examples:

```text
Open Chrome and search for weather -> browser_search, Chrome, low risk
Chrome me weather search karo -> browser_search, Chrome, low risk
Click delete and confirm -> high/critical risk, approval required
Click the button -> ambiguous target
Do something magical -> unknown/unsupported
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
```

Manual prompt:

```text
Open Chrome and search for weather
```

Expected:

```text
intent/action_type = browser_search
target_app = Chrome/browser
final_state = search results visible
risk = low
requires_approval = false
```

## Step 3: GUI Planner Orchestrator

Status:

```text
PASS
```

Implemented:

```text
GuiGoalContract + GuiContext
 -> deterministic baseline plan
 -> optional schema-bound LLM advisory plan
 -> strict plan validation
 -> PlanCreated + PlanValidationCompleted
 -> UI typed plan display
```

Canonical planner type:

```text
GuiLlmPlan with typed_steps
```

Do not create a parallel planner.

Allowed typed step examples:

```text
Observe
OpenApp
SwitchWindow
FocusField
TypeText
ClickControl
PressKey
BrowserNavigate
Scroll
Copy
Paste
Save
Download
WaitForState
VerifyState
AskClarification
RequireApproval
SummarizeVisibleContent
```

Invariant:

```text
Step 3 never executes.
Every typed step has allowed_to_execute=false.
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
```

Manual prompt:

```text
Open browser, search KRIA, and summarize page
```

Expected typed plan:

```text
OpenApp(browser)
FocusField(address/search field)
TypeText(KRIA)
PressKey(Enter) or ClickControl(Search)
WaitForState(result_visible)
SummarizeVisibleContent
```

## Step 4: Plan Validator

Status:

```text
PASS
```

Implemented:

```text
validate_plan_for_resolution(goal_contract, context, plan)
```

Readiness statuses:

```text
valid_for_resolution
needs_clarification
approval_required
blocked
rejected
```

Invariant:

```text
Step 4 validates readiness for Step 5 only.
Step 4 can_execute=false.
Step 4 never executes and never resolves controls.
```

Reject/block cases:

```text
unsupported step_type
allowed_to_execute=true
raw coordinates
missing verification strategy
high/critical without approval
goal contradiction
shell/native/browser automation commands
raw prompt/OCR/screenshot/clipboard/terminal/code/secrets
stale context/goal IDs
control action with no target hint
OCR-only executable target
impossible order
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
./testing/run.sh gui_cognition --tag step4_validator --include-live --include-slow --fail-fast
```

Manual prompt:

```text
Click delete and confirm
```

Expected:

```text
risk = high/critical
readiness_status = approval_required or blocked
requires_user_approval = true
can_execute = false
no ActionStarted
no ActionCompleted
```

## Step 5: Target Resolver

Status:

```text
PASS
```

Implemented:

```text
GuiTargetResolutionSummary
GuiTargetResolutionResult
GuiResolvedTarget
GuiTargetCandidate
TargetResolutionCompleted
```

Resolver source authority:

```text
GuiContext.fused_controls is candidate authority.
executable_controls is eligibility evidence, not the only candidate source.
visual/OCR can support candidates but cannot resolve executable action targets alone.
```

Resolved target fields:

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

Invariant:

```text
Step 5 resolves target metadata only.
Step 5 never executes.
Step 5 never opens HITL by itself.
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_target_resolver_tests --quiet
./testing/run.sh gui_cognition --tag step5_target_resolver --include-live --include-slow --fail-fast
```

Manual prompt:

```text
Click the Search button
```

Expected:

```text
target_resolution.status = resolved
resolved_target.label = Search
resolved_target.role = button
confidence >= 0.85
control_id exists
bounds exists
target_hash exists
can_execute = false
```

Ambiguity pass condition:

```text
If multiple Search buttons exist, KRIA must not guess.
It must return ambiguous/needs_clarification with safe candidate labels.
```

## Step 6: Safety Gate + HITL

Status:

```text
PASS
```

Implemented:

```text
GuiActionProposal
GuiSafetyGateResult
GuiHitlDecision
GuiHitlDecisionFixture
GuiHitlProposalStore
```

The proposal binds:

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

Risk/HITL behavior:

```text
low safe action -> SafetyGateCompleted safe_no_approval_required
high/critical action -> SafetyGateCompleted + HitlRequired
deny -> HitlDecisionRecorded can_authorize_step7=false
fresh approve -> HitlDecisionRecorded can_authorize_step7=true
stale/hash mismatch -> HitlDecisionInvalidated
```

Invariant:

```text
Step 6 approval authorizes Step 7 only.
Step 6 never executes.
Every Step 6 event has can_execute=false.
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_safety_hitl_tests --quiet
./testing/run.sh gui_cognition --tag step6_safety_hitl --include-live --include-slow --fail-fast
```

Manual prompt:

```text
Send this email
```

Expected:

```text
HITL modal opens
exact action proposal shown
approve/deny buttons shown
deny records denied and no execution is authorized
approve records fresh same-proposal authorization only
```

## Step 7: Deterministic Executor

Status:

```text
PASS
```

Implemented:

```text
GuiExecutionMode
GuiExecutionAuthorizationSource
GuiExecutionRequest
GuiExecutionResult
GuiExecutionPreconditionReport
GuiPayloadVault
ActionStarted
ActionCompleted
ActionFailed
ExecutionBlocked
ExecutionVerificationCompleted
```

Supported v1 actions:

```text
OpenApp
SwitchWindow
FocusField
TypeText
ClickControl
PressKey
Hotkey
Scroll
Copy
Paste
```

Current backend mapping:

```text
OpenApp -> open_application
SwitchWindow -> focus_window/window helper
FocusField -> AT-SPI focus/click path where available
TypeText -> focused text typing path
ClickControl -> click_ui_element/trusted target path
PressKey/Hotkey -> press_shortcut
Copy/Paste -> guarded Ctrl+C/Ctrl+V path
Scroll -> blocked if no supported backend helper is available
```

Invariant:

```text
ActionStarted only after precondition gate passes.
ActionCompleted only means backend success.
ActionFailed means backend failure.
ExecutionBlocked means no backend action started.
```

Verify:

```bash
cargo test -p kria-core --test gui_cognition_executor_tests --quiet
./testing/run.sh gui_cognition --tag step7_executor --include-live --include-slow --fail-fast
```

Manual live proof already completed:

```text
Prompt: Open Chrome
Result: Step 7 executed OpenApp through deterministic backend open_application and verified the result.
ActionStarted: true
ActionCompleted: true
ExecutionVerificationCompleted: true
execution.status: completed
backend_used: open_application
action_type: OpenApp
```

Remaining Step 7 limitations:

```text
one immutable proposal per cycle
multi-step workflows deferred
live Scroll fails closed unless supported helper exists
fixture execution is CI authority; live execution is manually verified
```

## Step 8: Post-Action Verification

Status:

```text
NEXT IMPLEMENTATION TARGET
```

Goal:

```text
After each action, KRIA must re-observe the GUI and compare actual state to expected postcondition.
Execution is not complete until verification passes or fails explicitly.
```

Build:

```text
after ActionCompleted or backend action attempt
 -> re-observe current GUI
 -> rebuild GuiContext
 -> compare expected_postcondition and verification_strategy
 -> emit rich ExecutionVerificationCompleted
 -> mark completed only if verification passes
 -> otherwise mark verification_failed with recovery hint
```

Current Step 7 already emits `ExecutionVerificationCompleted`, but Step 8 should harden it into a production-grade verifier/re-observer contract.

Primary files:

```text
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/executor.rs
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/src/agent/gui_cognition/context.rs
crates/kria-desktop/src/commands/gui_cognition.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/components/GuiCognitionPanel.tsx
testing/suites/gui_cognition/scenarios/executor.json
```

Suggested new test file:

```text
crates/kria-core/tests/gui_cognition_verification_tests.rs
```

Suggested new same-path scenario file:

```text
testing/suites/gui_cognition/scenarios/post_action_verification.json
```

Suggested tag:

```text
step8_verification
```

Verifier contract to add or harden:

```text
GuiPostActionVerificationRequest
  verification_id
  execution_id
  proposal_id
  proposal_hash
  action_type
  target_hash
  stable_target_identity_hash
  expected_postcondition
  verification_strategy
  pre_action_context_id
  post_action_observation_id
  post_action_context_id
  started_at_ms

GuiPostActionVerificationResult
  verification_id
  execution_id
  proposal_id
  status:
    verified
    verification_failed
    inconclusive
    blocked
  verification_strategy
  evidence[]
  pre_state_summary
  post_state_summary
  matched_expected_state
  target_still_present
  target_identity_matches
  confidence
  safe_error_summary
  recovery_hint
  can_retry
  prompt_hash
```

Verification strategies:

```text
window_visible
active_window_match
focused_control
text_present
state_changed
screen_changed
result_visible
dialog_visible
file_saved
download_started_or_completed
clipboard_changed
target_resolved
visible_content_summarized
```

Step 8 rules:

```text
OpenApp verifies window_visible or active_window_match.
SwitchWindow verifies active_window_match.
FocusField verifies focused_control and stable target identity when available.
TypeText verifies text_present for non-secret payload, state_changed for secret payload.
ClickControl verifies screen_changed/result_visible/dialog_visible or expected postcondition.
PressKey/Hotkey verifies expected postcondition or screen_changed.
Scroll verifies screen_changed.
Copy verifies clipboard_changed without emitting clipboard value.
Paste verifies text_present/state_changed without emitting raw payload.
```

Failure handling:

```text
backend success + verification pass -> ActionCompleted remains completed
backend success + verification fail -> status verification_failed
backend failure -> ActionFailed
precondition failure -> ExecutionBlocked
inconclusive observation -> verification_failed or inconclusive, no blind success
```

Step 8 must not implement broad recovery yet. It can emit `recovery_hint`, but Step 9 owns recovery actions.

Step 8 failure tests first:

```text
OpenApp verification passes when window visible
OpenApp verification fails when window missing
FocusField verification passes when focused control matches
FocusField verification fails when focus moves elsewhere
TypeText verification passes when expected text appears
secret TypeText uses state_changed and never emits raw text
ClickControl verification passes on screen_changed/result_visible
ClickControl verification fails on unchanged screen
Copy verification emits clipboard_changed without clipboard content
ActionCompleted is not treated as final success when verification fails
ExecutionVerificationCompleted includes safe evidence only
```

Step 8 same-path scenarios:

```text
step8.verify.open_app_window_visible
step8.verify.focus_field_focused_control
step8.verify.type_text_present
step8.verify.secret_type_state_changed_only
step8.verify.click_screen_changed
step8.verify.verification_failed_no_blind_success
step8.verify.copy_no_clipboard_leak
step8.verify.no_raw_secret_leakage
```

Manual prompt:

```text
Type hello in text editor
```

Expected:

```text
ActionStarted
ActionCompleted
post-action ObservationCompleted or verification observation evidence
ExecutionVerificationCompleted status=verified
text "hello" present or safe state_changed evidence
```

Pass condition:

```text
Action is not only executed; it is verified.
```

## Step 9: Recovery Loop

Status:

```text
NOT IMPLEMENTED
```

Goal:

```text
If execution or verification fails, KRIA should safely re-observe, classify the failure, and either recover once or pause with a clear explanation.
```

Build:

```text
verification_failed / target_missing / focus_lost / modal_appeared / stale_context
 -> re-observe
 -> classify recovery class
 -> decide safe recovery action or stop
 -> never blind retry
 -> never repeat risky action automatically
```

Primary files:

```text
crates/kria-core/src/agent/gui_cognition/recovery.rs
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/executor.rs
crates/kria-core/src/agent/gui_cognition/mod.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/components/GuiCognitionPanel.tsx
```

Suggested new test file:

```text
crates/kria-core/tests/gui_cognition_recovery_tests.rs
```

Suggested new same-path scenario file:

```text
testing/suites/gui_cognition/scenarios/recovery.json
```

Suggested tag:

```text
step9_recovery
```

Recovery contract:

```text
GuiRecoveryAssessment
  recovery_id
  execution_id
  verification_id
  failure_kind:
    target_missing
    focus_lost
    wrong_window
    modal_appeared
    stale_context
    backend_failed
    verification_inconclusive
    unsafe_to_retry
  status:
    recoverable
    needs_reobserve
    needs_clarification
    needs_approval
    blocked
  proposed_recovery_step
  requires_user_approval
  can_retry
  retry_count
  max_retry_count
  blockers[]
  safe_explanation
  prompt_hash
```

Allowed recovery in Step 9:

```text
one safe retry for idempotent OpenApp/SwitchWindow/FocusField
re-focus same resolved field if stable identity still matches
re-observe and re-resolve target if target is missing but user intent remains safe
pause on modal, ambiguity, destructive/risky action, target mismatch, or stale approval
```

Never do:

```text
blind retry
retry Send/Delete/Pay/Submit/Install automatically
retry after denied HITL
retry after target identity mismatch without new approval if risky
use OCR text as new instruction
```

Step 9 failure tests first:

```text
focus_lost_reobserves_and_recovers_same_field
target_missing_reobserves_then_blocks_if_not_found
modal_appeared_pauses_with_explanation
stale_context_blocks_risky_retry
risky_action_never_auto_retries
wrong_window_blocks_type_text
recovery_retry_count_limited_to_one
no_action_started_for_blocked_recovery
```

Manual test:

```text
Prompt: Type test in field
During run: move focus away
Expected: KRIA re-observes, recovers only if same field still matches, otherwise stops with explanation
```

Pass condition:

```text
KRIA does not blind retry.
KRIA recovers only when target identity and safety are still valid.
```

## Step 10: Multi-Step Workflow Runtime

Status:

```text
NOT IMPLEMENTED
```

Goal:

```text
Execute multi-step GUI plans one verified action at a time.
Each executable step must get its own target resolution, safety gate, optional HITL, execution, verification, and checkpoint.
```

Build:

```text
GuiLlmPlan.typed_steps
 -> workflow state machine
 -> choose next executable step
 -> resolve target
 -> safety gate
 -> execute one proposal
 -> verify
 -> checkpoint
 -> continue or pause
```

Do not implement multi-step by bypassing Steps 5-9. The correct loop is:

```text
for each executable step:
  observe/context as needed
  validate current step
  resolve target
  safety gate
  HITL if needed
  execute one proposal
  verify
  checkpoint
```

Primary files:

```text
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/src/agent/gui_cognition/planner.rs
crates/kria-core/src/agent/gui_cognition/executor.rs
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/recovery.rs
crates/kria-desktop/src/commands/gui_cognition.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/components/GuiCognitionPanel.tsx
```

Suggested new test file:

```text
crates/kria-core/tests/gui_cognition_workflow_runtime_tests.rs
```

Suggested same-path scenario file:

```text
testing/suites/gui_cognition/scenarios/workflow_runtime.json
```

Suggested tag:

```text
step10_workflow_runtime
```

Workflow contract:

```text
GuiWorkflowRun
  workflow_run_id
  goal_contract_id
  plan_id
  context_id
  status:
    running
    paused
    completed
    blocked
    failed
  current_step_index
  step_states[]
  checkpoint_id
  risk_level
  prompt_hash

GuiWorkflowStepState
  step_id
  step_type
  status:
    pending
    resolving
    awaiting_approval
    executing
    verifying
    completed
    blocked
    skipped
  proposal_id
  execution_id
  verification_id
  blockers[]
  retry_count
```

Runtime rules:

```text
execute only one proposal at a time
verify each action before the next action
re-observe between state-changing actions
never carry stale target hashes across screen changes without revalidation
ask HITL again for each risky executable action unless policy explicitly says previous approval covers the same immutable proposal
pause on ambiguity, stale approval, verification failure, or recovery blocker
```

Step 10 failure tests first:

```text
workflow_executes_open_focus_type_enter_fixture
workflow_verifies_each_step_before_next
workflow_pauses_on_ambiguous_step
workflow_pauses_on_risky_step_until_hitl
workflow_does_not_duplicate_approved_risky_action
workflow_reobserves_after_state_change
workflow_blocks_on_step_verification_failure
workflow_progress_renders_step_status_list
```

Manual prompt:

```text
Open browser, search for KRIA, copy first result title into notes
```

Expected:

```text
step status list:
1 OpenApp done
2 FocusField done
3 TypeText done
4 PressKey/Search done
5 WaitForState verified
6 Copy title done
7 Open/Switch notes done
8 Paste done
```

Pass condition:

```text
Every step is verified before the next step starts.
No stale target or unapproved risky action executes.
```

## Step 11: Checkpoint / Resume

Status:

```text
NOT IMPLEMENTED
```

Goal:

```text
Long GUI workflows can pause/resume safely without duplicating risky actions or using stale approvals.
```

Build:

```text
persist workflow state
persist safe proposal/decision metadata
persist completed step receipts
on resume:
  re-observe
  revalidate workflow state
  reject stale approvals
  avoid duplicate risky action
  continue only from safe next step
```

Primary files:

```text
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/src/agent/gui_cognition/safety_hitl.rs
crates/kria-core/src/agent/gui_cognition/executor.rs
crates/kria-core/src/agent/gui_cognition/verifier.rs
crates/kria-core/src/agent/gui_cognition/recovery.rs
crates/kria-desktop/src/commands/app_state.rs
crates/kria-desktop/src/commands/sessions.rs
crates/kria-desktop/src/commands/gui_cognition.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/components/GuiCognitionPanel.tsx
```

Suggested new test file:

```text
crates/kria-core/tests/gui_cognition_checkpoint_resume_tests.rs
```

Suggested same-path scenario file:

```text
testing/suites/gui_cognition/scenarios/checkpoint_resume.json
```

Suggested tag:

```text
step11_checkpoint_resume
```

Checkpoint contract:

```text
GuiWorkflowCheckpoint
  checkpoint_id
  workflow_run_id
  session_id
  goal_contract_id
  plan_id
  current_step_index
  completed_step_receipts[]
  pending_proposal_id
  pending_proposal_hash
  pending_target_hash
  approved_decision_id
  approved_decision_hash
  last_observation_id
  last_context_id
  created_at_ms
  expires_at_ms
  prompt_hash

GuiResumeResult
  resume_id
  checkpoint_id
  status:
    resumed
    stale_rejected
    needs_reobserve
    needs_approval
    blocked
  next_step_id
  invalidated_approvals[]
  duplicate_action_guard[]
  blockers[]
  safe_explanation
```

Resume rules:

```text
always re-observe before resuming an executable step
approval from before restart is invalid unless same proposal and freshness still pass
completed risky actions are never repeated automatically
pending risky actions require fresh HITL if screen/context changed
checkpoint stores hashes and receipts, not raw prompt/OCR/clipboard/secrets
```

Step 11 failure tests first:

```text
resume_safe_workflow_after_pause
resume_reobserves_before_next_action
stale_approval_invalidated_after_restart
target_hash_changed_blocks_resume
completed_risky_action_not_repeated
pending_risky_action_requires_fresh_approval
checkpoint_serializes_without_raw_prompt_or_secrets
```

Manual prompt:

```text
Prompt risky submit
Approve later / restart app
```

Expected:

```text
KRIA re-observes
stale approval invalidated if screen changed
submit/send/delete is not duplicated
```

Pass condition:

```text
No duplicate risky action after pause/resume.
No stale approval executes.
```

## Optional Step 12: Real Task Eval Suite

Step 12 is outside the requested Step 1-11 implementation handoff, but it should become the final production proof after Step 11.

Build real prompt tests:

```text
Open Chrome
Search Google
Open terminal
Open file manager
Create text file
Rename file
Fill dummy form
Click safe button
Draft email but do not send
Send email only after approval
```

Pass:

```text
90%+ real tasks pass on the same supported desktop environment.
No unsafe auto-action.
Verification evidence exists for every action.
```

## Recommended Implementation Order From Here

Continue in this order:

```text
1. Step 8 verifier contract and tests
2. Step 8 post-action re-observe integration
3. Step 8 UI verification rendering
4. Step 8 same-path suite
5. Step 9 recovery classifier and no-blind-retry tests
6. Step 9 safe one-retry recovery for focus/open/switch only
7. Step 10 workflow state machine
8. Step 10 per-step proposal/verification loop
9. Step 11 checkpoint serialization
10. Step 11 resume/stale approval invalidation
11. Step 12 real task eval suite
```

Do not jump directly to multi-step execution before Step 8 verification and Step 9 recovery are strong.

## Baseline Verification Before Starting New Work

Run this before implementing Step 8 if the workspace may have drifted:

```bash
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
```

UI baseline:

```bash
cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel HitlModal app.tool-choice
cd ui && npm run test:run
cd ui && npm run build
```

Harness baseline:

```bash
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py

./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step4_validator --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step5_target_resolver --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step6_safety_hitl --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step7_executor --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast

git diff --check
```

Known test-output note:

```text
UI tests/build may print existing warning noise.
Do not treat it as a regression unless an assertion/build actually fails.
```

## How To Add A New Step Suite

For each new step:

1. Add Rust tests first:

```text
crates/kria-core/tests/gui_cognition_<step_name>_tests.rs
```

2. Add same-path scenario file:

```text
testing/suites/gui_cognition/scenarios/<step_name>.json
```

3. Add it to:

```text
testing/suites/gui_cognition/manifest.json
testing/harness/models.py SUPPORTED_TAGS
```

4. Regenerate inventory:

```bash
python3 testing/tools/collect_test_inventory.py --write
```

5. Validate inventory:

```bash
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py
```

6. Add UI store/panel tests when event payloads change.

7. Add a final report:

```text
planning_docs/step_8_post_action_verification_report.md
planning_docs/step_9_recovery_loop_report.md
planning_docs/step_10_multistep_workflow_runtime_report.md
planning_docs/step_11_checkpoint_resume_report.md
```

Each report must include:

```text
Verdict: PASS or PARTIAL
Implementation summary
Files changed
Contract/schema changes
Runtime behavior
UI behavior
Tests executed
Failures found
Fixes applied
Remaining limitations
Next step readiness
```

## Required Same-Path Assertion Pattern

Every Step 8-11 same-path scenario should assert:

```text
desktop_command.path = send_manual_tool_message
RouteConfirmed.llm_tool_loop = false
ObservationCompleted exists
ContextBuilt exists
GoalContractCreated exists
PlanCreated exists
PlanValidationCompleted exists
TargetResolutionCompleted exists when target/action step exists
SafetyGateCompleted exists when proposal exists
ActionStarted exists only when authorized and execution_mode != safety_only
ActionCompleted only on backend success
ActionFailed on backend failure
ExecutionBlocked before ActionStarted for blockers
ExecutionVerificationCompleted after execution attempt or block
no raw prompt/OCR/screenshot/clipboard/secrets
```

For no-execution safety tests:

```text
forbidden_gui_event_types:
  ActionStarted
  ActionCompleted
```

For Step 8+ execution tests:

```text
expected_gui_event_types:
  ActionStarted
  ActionCompleted or ActionFailed
  ExecutionVerificationCompleted
```

## Manual Live Verification Pattern

Use live mode only intentionally:

```text
execution_mode=execute_live
```

Safe manual prompts:

```text
Open Chrome
Focus the search box
Type KRIA
Click the Search button
Scroll page
Copy selected text
Paste into focused field
```

Risky manual prompts:

```text
Send this email
Delete this file
Submit this form
Pay for this order
Install this extension
```

Expected risky behavior:

```text
HITL required first
Deny blocks execution
Approve executes only same fresh bound target
Step 8 verifies result
Step 9 recovers or pauses safely if verification fails
```

## Definition Of Done For Each Capability

A capability is complete only when:

```text
5-10 natural prompt variations pass
fixture same-path passes
core Rust tests pass
UI state/panel tests pass when UI changes
broad desktop_command suite passes
manual live proof passes where applicable
git diff --check passes
final report says PASS
```

For GUI Cognition automation, "works" means:

```text
observed
understood
planned
validated
resolved
approved if needed
executed deterministically
verified after action
recoverable or safely stopped
rendered in UI
tested through same-path harness
```

## Immediate Next Prompt For A New Agent

Use this prompt to continue:

```text
Please implement Step 8: Post-Action Verification for KRIA GUI Cognition selected mode.

Read CONTINUE_GUI_UPGRADE.md and the Step 7 report first.
Preserve execution_mode boundaries.
Do not implement broad recovery or multi-step runtime yet.
Harden post-action re-observe and verification:
  ActionCompleted -> observe -> compare expected_postcondition -> ExecutionVerificationCompleted.
Add core tests, UI tests, same-path step8_verification suite, and report:
  planning_docs/step_8_post_action_verification_report.md.
Acceptance:
  action success is not final until post-action verification passes;
  verification failures are explicit and safe;
  no raw prompt/OCR/screenshot/clipboard/secrets leak;
  Step 1-7 suites remain green.
```

