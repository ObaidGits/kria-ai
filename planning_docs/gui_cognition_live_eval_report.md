# GUI Cognition Live Evaluation Report

- Generated: 2026-06-17 07:17:35Z
- Mode: `execute_live`  ·  Endpoint: `http://127.0.0.1:3001/api/testing/desktop-chat-command`
- Path: same as UI (`send_manual_tool_message`, manual_profile=`gui_cognition`, workflow=true, hitl=approve)
- Prompts: 3

## Summary

| Outcome | Count |
|---|---|
| UNEXPECTED | 3 |

Legend: **PASS** executed+verified · **PARTIAL** ran but failed/unverified · **BUG** concrete defect · **BLOCKED** stopped before execution · **EXPECTED_ABSENT** app not installed · **UNEXPECTED** crash/empty/no-plan.

## Results

| ID | Prompt | Outcome | exec | verify | Detail |
|---|---|---|---|---|---|
| p01 | Open the Calculator app | UNEXPECTED | None | None | Pipeline produced no plan. reply='Task complete.' |
| p08 | Open the Calculator and type 5 + 5 | UNEXPECTED | None | None | Pipeline produced no plan. reply='The screen did not change after the last action; stopping to avoid looping.' |
| p14 | Open the Settings and go to Wi-Fi | UNEXPECTED | None | None | Pipeline produced no plan. reply="I couldn't find element #1 on the current screen. Which control did you mean?" |

## Per-prompt detail

### p01 — Open the Calculator app

- Expected: Calculator window opens
- Outcome: **UNEXPECTED** (5338 ms)
- Detail: Pipeline produced no plan. reply='Task complete.'
- Reply: Task complete.
- Signals:
- Raw: `planning_docs/gui_cognition_live_eval_raw/p01.json`

### p08 — Open the Calculator and type 5 + 5

- Expected: Calc opens, 5 + 5 typed
- Outcome: **UNEXPECTED** (4480 ms)
- Detail: Pipeline produced no plan. reply='The screen did not change after the last action; stopping to avoid looping.'
- Reply: The screen did not change after the last action; stopping to avoid looping.
- Signals:
- Raw: `planning_docs/gui_cognition_live_eval_raw/p08.json`

### p14 — Open the Settings and go to Wi-Fi

- Expected: Settings opens Wi-Fi pane
- Outcome: **UNEXPECTED** (5136 ms)
- Detail: Pipeline produced no plan. reply="I couldn't find element #1 on the current screen. Which control did you mean?"
- Reply: I couldn't find element #1 on the current screen. Which control did you mean?
- Signals:
- Raw: `planning_docs/gui_cognition_live_eval_raw/p14.json`
