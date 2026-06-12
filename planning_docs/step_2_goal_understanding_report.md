# Step 2 Goal Understanding Report

## Summary

Step 2 hardens GUI Cognition selected-mode goal understanding without adding action execution.

The selected-mode path remains:

```text
GUI Cognition dropdown
 -> send_manual_tool_message
 -> observe/context
 -> deterministic goal extraction
 -> GoalContractCreated
 -> PlanCreated
 -> no Step 2 action execution
```

## Implementation

- Extended the existing `GuiGoalContract` instead of rebuilding it.
- Added canonical app kind, safe query/text summaries and hashes, prompt hash, ambiguity count, and sanitized source evidence.
- Added deterministic phrase/slot extraction for browser search, open/switch/focus/type/click/fill/save/download/copy/paste/recovery/unknown intents.
- Added compound intent handling so `Open Chrome and search for weather` becomes `browser_search`, not `open_app`.
- Added English and Hinglish browser-search handling.
- Added stricter risk taxonomy for submit/send/delete/pay/install/security/system/git actions.
- Added explicit ambiguity for missing query/text/control, multiple app targets, and unsupported goals.
- Preserved privacy rules: raw prompt, OCR, screenshots, clipboard text, hidden prompts, and secret-like values are not emitted.
- Updated frontend types, store sanitization, and panel rendering for the new contract fields.
- Added same-path `goal_contract` eval coverage while preserving existing `goal_intent` tagging.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/goal_contract.rs`
- `crates/kria-core/src/agent/gui_cognition/planner.rs`
- `crates/kria-core/tests/gui_cognition_goal_contract_tests.rs`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/stores/guiCognitionSession.test.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `ui/src/components/GuiCognitionPanel.test.tsx`
- `testing/suites/gui_cognition/scenarios/goal_intent_understanding.json`

## Tests Executed

- `cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cargo check -p kria-desktop --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel app.tool-choice HitlModal`
- `cd ui && npm run test:run`
- `cd ui && npm run build`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`
- `./testing/run.sh gui_cognition --profile ci`
- `./testing/run.sh gui_cognition --tag goal_contract --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag focus_authority --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast`
- `git diff --check`

## Results

- Core goal-contract tests passed.
- Core observation, context, backend route, and LLM planner regression tests passed.
- Desktop GUI Cognition tests and `cargo check` passed.
- UI typecheck passed.
- Targeted and full UI tests passed.
- UI production build passed.
- Harness manifest and inventory validation passed.
- GUI Cognition CI profile passed.
- Same-path `goal_contract` suite passed.
- Same-path broad live `desktop_command` suite passed.
- `git diff --check` passed.

## Remaining Risks

- Deterministic extraction is intentionally broad but not a full natural-language parser.
- Unknown or unsupported layman prompts are fail-closed as `unknown` with explicit ambiguity.
- OCR/context can support evidence only; they cannot create user intent.
- Existing downstream planner/executor behavior is still Step 3+ scope; Step 2 only creates the contract and does not add execution capability.

## Verdict

```text
Step 2: PASS
```
