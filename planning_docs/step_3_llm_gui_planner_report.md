# Step 3 LLM GUI Planner Report

## Verdict

Step 3: PASS

KRIA GUI Cognition selected mode now emits a typed, execution-disabled planner contract from the existing planner layer. The implementation evolved the existing `GuiLlmPlan` path instead of creating a second planner system.

## Implementation Summary

Implemented Step 3 as a Planner Orchestrator:

```text
GuiGoalContract + GuiContext
 -> deterministic baseline plan
 -> optional schema-bound LLM advisory plan
 -> strict plan validation
 -> PlanCreated + PlanValidationCompleted
 -> UI plan display
 -> no ActionStarted / no ActionCompleted
```

Key behavior:

- `GuiLlmPlan` is now the canonical plan shape with backward-compatible readable `steps` plus new `typed_steps`.
- Every typed step carries `allowed_to_execute=false`.
- Browser search plans are expanded into concrete typed steps instead of emitting a duplicated `BrowserSearch` macro step.
- Invalid/prose/unsafe/contradictory LLM output is rejected and falls back to a deterministic typed plan.
- Plan validation rejects raw coordinates, unsafe tool/action text, unsupported step types, missing verification strategies, secret leaks, and goal contradictions.
- UI accepts old minimal plan events and renders the new typed plan details.

## Planner Schema

Plan-level fields added or hardened:

```text
plan_id
goal_contract_id
context_id
prompt_hash
goal_action_type
plan_status
ambiguity_count
validation_errors
source_evidence
typed_steps
```

Typed step fields:

```text
step_id
step_type
summary
target_app_hint
target_window_hint
target_control_hint
text_payload_summary
text_payload_hash
expected_precondition
expected_postcondition
verification_strategy
risk_level
requires_approval
allowed_to_execute
confidence
reason
```

Allowed typed steps:

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

## Validator Rules

The Step 3 validator now enforces:

- JSON schema only for LLM advisory plans.
- No prose or markdown wrapper.
- Allowed typed step enum only.
- Every step must be `allowed_to_execute=false`.
- Action-like steps must include a verification strategy.
- Risky steps must require approval.
- Raw coordinates and coordinate-like strings are rejected.
- LLM plans that contradict the goal contract are blocked.
- OCR/context evidence cannot invent user intent.
- Prompt/OCR/screenshot/clipboard/terminal/code/secrets are not exposed.

Plan status mapping:

```text
valid schema + valid plan -> valid
parse/schema/prose failure -> rejected
unsafe or contradictory plan -> blocked
clarification step -> needs_clarification
```

## Files Changed In This Step

Primary Step 3 files:

```text
crates/kria-core/src/agent/gui_cognition/llm_planner.rs
crates/kria-core/src/agent/gui_cognition/mod.rs
crates/kria-core/tests/gui_cognition_llm_planner_tests.rs
ui/src/types/guiCognition.ts
ui/src/stores/guiCognitionSession.ts
ui/src/components/GuiCognitionPanel.tsx
ui/src/components/GuiCognitionPanel.test.tsx
testing/harness/models.py
testing/suites/gui_cognition/scenarios/llm_gui_planner.json
testing/inventory/current_inventory.json
testing/inventory/migration_map.md
crates/kria-core/src/lib.rs
```

## Tests Executed

Core and desktop:

```text
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet
cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet
cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet
cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet
cargo test -p kria-desktop gui_cognition --quiet
cargo check -p kria-desktop --quiet
```

Results:

```text
gui_cognition_llm_planner_tests: 9 passed
gui_cognition_goal_contract_tests: 9 passed
gui_cognition_context_builder_tests: 5 passed
gui_cognition_backend_route_tests: 16 passed
gui_cognition_observation_perception_tests: 16 passed
kria-desktop gui_cognition tests: 2 passed
cargo check -p kria-desktop: passed
```

UI:

```text
cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel app.tool-choice HitlModal
cd ui && npm run test:run
cd ui && npm run build
```

Results:

```text
ui check: passed
targeted UI tests: 54 passed
full UI tests: 93 passed
ui build: passed
```

Harness and same-path:

```text
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py
./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag step3_planner --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
```

Results:

```text
harness manifest/inventory tests: 16 passed
gui_cognition --profile ci: passed
goal_contract suite: passed
step3_planner suite: passed
desktop_command suite: passed
```

Generated eval reports:

```text
testing/eval_reports/kria_testing_gui_cognition_20260607_184707_f3baf6a6.md
testing/eval_reports/kria_testing_gui_cognition_ci_20260607_184722_0a99ba94.md
testing/eval_reports/kria_testing_gui_cognition_20260607_184743_5a494cbb.md
testing/eval_reports/kria_testing_gui_cognition_20260607_184832_9362d079.md
```

## Failures Found

During implementation:

- The expanded JSON schema exceeded the Rust macro recursion limit.
- Some legacy planner mode strings did not match the new Step 3 naming.
- Existing plan events needed to preserve old `steps: string[]` while adding typed step details.
- Broad same-path tests required the new `step3_planner` tag to be added to the harness tag registry and inventory.

## Fixes Applied

- Added a higher crate recursion limit for the expanded planner schema.
- Mapped planner modes to `deterministic`, `llm_schema`, and `llm_rejected_fallback`.
- Kept readable legacy `steps` while emitting new `typed_steps`.
- Added old action-kind to typed-step compatibility mapping.
- Added `step3_planner` harness tag and regenerated inventory.
- Added unit and same-path tests for typed browser-search planning, ambiguous target clarification, unknown goal clarification, invalid LLM fallback, hallucinated target rejection, raw coordinate rejection, and goal contradiction blocking.

## Remaining Limitations

- Step 3 is planning-only. It does not execute OpenApp/click/type/hotkey steps.
- Step 3 does not open HITL approval. It marks approval requirements in the plan; actual HITL handling remains a later safety/execution step.
- Live LLM quality is not the production dependency. CI and same-path tests use fixture LLM outputs; deterministic fallback remains the safe baseline.
- The broader worktree contains many existing GUI Cognition changes from prior phases; this report only claims the Step 3 planner hardening pass.

## Final Gate

Step 3 is ready for the next roadmap step because:

```text
typed_steps are emitted
old plan summaries remain compatible
browser-search-with-summary exact plan passes
LLM invalid/prose/hallucinated/unsafe output is rejected
deterministic fallback works
risky prompts require approval in the plan
ambiguous prompts ask clarification
every typed step is execution-disabled
action-like steps include verification strategy
same-path Step 3 planner suite passes
broad desktop-command suite passes
UI renders typed plan status and steps
```
