# Step 9 Recovery Loop Report

Date: 2026-06-12

## Verdict

```text
Step 9: PASS
Ready for Step 10 Multi-Step Workflow Runtime
```

After execution + post-action verification, KRIA now runs a deterministic,
bounded recovery loop. It classifies the failure, decides whether a single safe,
idempotent recovery action is allowed, and either performs one bounded recovery
or stops with a safe explanation. It never blind-retries, never auto-retries
risky actions, never recovers after denied/stale approval, and never continues
the next user task (that is Step 10).

## Selected-Mode Pipeline (scope unchanged)

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
 -> ActionStarted
 -> ActionCompleted | ActionFailed | ExecutionBlocked
 -> ExecutionVerificationCompleted
 -> RecoveryAssessmentCompleted          (only when verification != verified)
 -> RecoveryActionStarted                (only for safe, idempotent recovery)
 -> RecoveryActionCompleted | RecoveryBlocked
 -> TurnCompleted
 -> UI panel
```

## Implementation Summary

- Hardened `crates/kria-core/src/agent/gui_cognition/recovery.rs` with the Step 9
  contracts and a pure, deterministic `assess_recovery` classifier.
- Wired a `run_recovery_loop` into the executor path in `mod.rs`. It runs only
  when `should_attempt_recovery(verification.status)` is true (i.e. verification
  did not confirm the expected state). Verified actions never trigger recovery.
- Recovery decisions use only bounded structural signals derived from the
  post-action observation/verification (no raw prompt/OCR/clipboard text).
- Safe recovery executes at most one bounded action, re-observes, re-verifies,
  and reports `recovered` or `blocked`. `can_continue_workflow` is always
  `false` in Step 9.

## Recovery Contract

`GuiRecoveryAssessment`:

```text
recovery_id, execution_id, verification_id, proposal_id, proposal_hash,
target_hash, action_type,
failure_kind, status, proposed_recovery_step, recovery_action_kind,
requires_user_approval, can_recover, can_execute_recovery,
retry_count, max_retry_count, blockers[], warnings[],
safe_explanation, recovery_hint, prompt_hash
```

`GuiRecoveryResult`:

```text
recovery_id, execution_id, status, recovery_action_kind,
started_at_ms, completed_at_ms, backend_used,
post_recovery_observation_id, post_recovery_context_id,
verification_result, safe_error_summary, next_recommended_state,
can_retry_original_action, can_continue_workflow(=false), prompt_hash
```

failure_kind:

```text
focus_lost, wrong_window, target_missing, target_moved, target_ambiguous,
modal_appeared, stale_context, backend_failed, verification_failed,
verification_inconclusive, unsafe_to_retry
```

recovery_action_kind:

```text
ReObserve, RefocusSameTarget, SwitchBackToWindow, ReResolveTarget,
RetryIdempotentAction, AskClarification, Stop
```

status: `recoverable | needs_reobserve | needs_clarification | needs_approval | blocked`

result status: `recovered | blocked | failed | skipped`

next_recommended_state: `retry_original_action | replan | ask_clarification | await_approval | stop`

## Failure Classification

```text
risky / high|critical / requires_approval / denied HITL / stale HITL -> unsafe_to_retry (Stop)
retry_count >= 1                                                      -> blocked (Stop)
backend did not complete (verification "blocked")                    -> backend_failed (Stop)
dialog/modal visible after action                                    -> modal_appeared (Stop)
context stale                                                        -> stale_context (ReObserve)
verification inconclusive                                            -> verification_inconclusive (ReObserve)
target absent + >1 re-resolve candidates                             -> target_ambiguous (AskClarification)
target absent + 0/1 candidate                                        -> target_missing (Stop)
target present + identity changed                                    -> target_moved (AskClarification)
target present + duplicates                                          -> target_ambiguous (AskClarification)
focus_lost (FocusField, focused_control)                             -> RefocusSameTarget
wrong_window (OpenApp/SwitchWindow, active_window_match, win known)  -> SwitchBackToWindow
other idempotent (OpenApp/SwitchWindow/FocusField)                   -> RetryIdempotentAction
non-idempotent (Click/Type/Paste/Press/Hotkey/Scroll/Copy)          -> verification_failed (Stop)
```

## Allowed Recovery Rules

```text
ReObserve after inconclusive / stale context (no input backend).
RefocusSameTarget when stable target identity still matches.
SwitchBackToWindow when intended window is known and action is low-risk/idempotent.
RetryIdempotentAction for OpenApp/SwitchWindow/FocusField only, once.
AskClarification on ambiguity / moved target.
At most one recovery attempt per action (max_retry_count = 1).
```

## Blocked Recovery Rules

```text
Never auto-recover Submit/Send/Delete/Pay/Install/System/Git or any high/critical risk action.
Never recover after a denied HITL decision.
Never recover after a stale/invalidated approval.
Never blind-retry after backend failure.
Never guess on target_moved / target_ambiguous / target_missing.
Never auto-retry non-idempotent actions.
Pause on a newly visible modal/dialog.
Recovery never continues the next user workflow step (Step 10 owns that).
```

## UI Behavior

`ui/src/types/guiCognition.ts`, `ui/src/stores/guiCognitionSession.ts`, and
`ui/src/components/GuiCognitionPanel.tsx` now handle the four new events and a
`GuiCognitionRecoveryState`. The panel shows recovery status, failure kind,
proposed recovery action, retry count, blockers, safe explanation, next
recommended state, and whether the original action can be retried. Raw
prompt/OCR/screenshot/clipboard/terminal/code/secrets are never rendered.

## Files Changed

```text
crates/kria-core/src/agent/gui_cognition/recovery.rs
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/tests/gui_cognition_recovery_tests.rs        (new)
crates/kria-core/tests/gui_cognition_backend_route_tests.rs   (recovery pipeline tests)
crates/kria-desktop/src/commands/gui_cognition.rs             (step9_focus_recovers fixture + cursor seq)
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/stores/guiCognitionSession.test.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
testing/suites/gui_cognition/scenarios/recovery.json          (new)
testing/suites/gui_cognition/manifest.json
testing/harness/models.py
testing/inventory/current_inventory.json
testing/inventory/migration_map.md
```

## Tests Executed

Core (passed):

```text
gui_cognition_recovery_tests           17 passed
gui_cognition_backend_route_tests      19 passed (incl. recovery pipeline tests)
gui_cognition_verification_tests       12 passed
gui_cognition_executor_tests            4 passed
gui_cognition_safety_hitl_tests         6 passed
gui_cognition_target_resolver_tests    10 passed
gui_cognition_llm_planner_tests        16 passed
gui_cognition_goal_contract_tests      10 passed
gui_cognition_context_builder_tests     5 passed
gui_cognition_observation_perception_tests 16 passed
cargo check -p kria-desktop            ok
```

UI (passed):

```text
npm run check                           ok
npm run test:run                        104 passed
npm run build                           ok
```

Harness / same-path:

```text
python3 -m pytest test_manifest_validation.py test_inventory.py   16 passed
./testing/run.sh gui_cognition --profile ci                       5 passed
git diff --check                                                  clean
```

## Three-Tier Live Verification

The KRIA Desktop app was rebuilt and relaunched (same local API the Kria UI
uses, `http://127.0.0.1:3001`). All tiers ran live against it:

```text
Tier 1 — Step 9 recovery:    step9_recovery        5 passed, 0 failed
Tier 2a — Step 8 verify:     step8_verification    4 passed, 0 failed
Tier 2b — Step 7 executor:   step7_executor        6 passed, 0 failed
Tier 3 — broad desktop:      desktop_command     120 passed, 0 failed
```

Live Step 9 scenarios:

```text
step9.recovery.focus_lost_refocuses_same_target_fixture   focus_lost -> RefocusSameTarget -> recovered
step9.recovery.non_idempotent_click_blocks                verification_failed -> Stop -> RecoveryBlocked (no RecoveryActionStarted)
step9.recovery.high_risk_submit_no_auto_retry             unsafe_to_retry -> RecoveryBlocked (approved risky never auto-retries)
step9.recovery.denied_approval_no_recovery                denied HITL -> no execution, no recovery events
step9.recovery.no_raw_secret_leakage                      forbidden raw prompt/OCR/screenshot/clipboard paths absent
```

The recovered focus path was confirmed end-to-end live:
`KRIA recovered safely via RefocusSameTarget and restored the expected state`,
with `recovery.status = recovered`, `recovery.can_continue_workflow = false`,
`recovery.next_recommended_state = retry_original_action`.

## Failures Found and Fixed

- Recovery initially overrode the turn status to a generic `blocked` for
  non-actionable cases, which broke the Step 8 `verification_failed` scenario.
  Fixed so RecoveryBlocked preserves the verification verdict unless recovery
  needs the user (needs_clarification / needs_approval).
- A live pre-check showed the desktop fixture could not reliably drive
  focus-return because `capture_screenshot` and `get_cursor_focus_state` race
  within one observation. Fixed by adding an independent `cursor_focus_seq`
  counter advanced only by `get_cursor_focus_state`, so the Step 9 fixture
  models focus returning to the target on the post-recovery observation without
  a cross-probe ordering hazard.

## Remaining Limitations

- Step 9 performs at most one bounded recovery and never continues the user's
  next task step; multi-step continuation is Step 10.
- Live same-path coverage demonstrates the reachable Step 7 v1 single-proposal
  outcomes (focus recovery, non-idempotent block, risky block, denied block).
  Branches that need a re-observe state transition unavailable to static
  fixtures (wrong_window recovered, target_missing/ambiguous/modal, inconclusive
  ReObserve) are fully covered by the 17 deterministic core recovery tests and
  the in-process recovery pipeline tests.
- `execute_live` (real input backend) recovery proofs remain a manual step on a
  real desktop session with an action backend.

## Acceptance Check

```text
[x] KRIA never blind retries.
[x] Safe focus/window recovery works when same target/window is still valid.
[x] Risky actions never auto-retry.
[x] Denied/stale approval never recovers into execution.
[x] Ambiguous/missing target asks clarification or blocks.
[x] Recovery events are visible in UI.
[x] No raw prompt/OCR/screenshot/clipboard/secrets leak.
[x] Step 1-8 suites remain green.
[x] step9_recovery suite passes live (5/5).
[x] broad desktop_command suite passes live (120/0).
[x] Report says Step 9: PASS.
```
