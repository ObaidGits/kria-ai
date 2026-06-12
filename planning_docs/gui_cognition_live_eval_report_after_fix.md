# GUI Cognition Live Evaluation Report

- Generated: 2026-06-12 12:23:00Z
- Mode: `execute_live`  ·  Endpoint: `http://127.0.0.1:3001/api/testing/desktop-chat-command`
- Path: same as UI (`send_manual_tool_message`, manual_profile=`gui_cognition`, workflow=true, hitl=approve)
- Prompts: 25

## Summary

| Outcome | Count |
|---|---|
| BLOCKED | 18 |
| PARTIAL | 7 |

Legend: **PASS** executed+verified · **PARTIAL** ran but failed/unverified · **BUG** concrete defect · **BLOCKED** stopped before execution · **EXPECTED_ABSENT** app not installed · **UNEXPECTED** crash/empty/no-plan.

## Results

| ID | Prompt | Outcome | exec | verify | Detail |
|---|---|---|---|---|---|
| p01 | Open the Calculator app | PARTIAL | completed | verified | Resolved but no execution result captured. |
| p02 | Open the Files manager | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p03 | Open the Text Editor | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p04 | Open the Terminal | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p05 | Open the Settings app | PARTIAL | completed | verified | Resolved but no execution result captured. |
| p06 | Open Google Chrome | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p07 | Open Firefox | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p08 | Open the Calculator and type 5 + 5 | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |
| p09 | Open Google Chrome and search for the latest Ubuntu version | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p10 | Open Google Chrome and open Gmail | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p11 | Open Google Chrome and go to youtube.com | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p12 | Open Firefox and search for weather today | PARTIAL | completed | verified | Planned but not resolved/executed. |
| p13 | Open the Files manager and go to the Downloads folder | BLOCKED | None | None | Blocked (plan_validation): TypeText has no safe text/query payload. |
| p14 | Open the Settings and go to Wi-Fi | BLOCKED | None | None | Blocked (plan_validation): TypeText has no safe text/query payload. |
| p15 | Open the Terminal and run ls | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p16 | Open the Text Editor and type Hello World | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |
| p17 | Open Google Chrome and open a new tab | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p18 | Open the Screenshot tool | PARTIAL | completed | verified | Resolved but no execution result captured. |
| p19 | Open Google Chrome and go to github.com | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p20 | Open Google Chrome, go to google.com and search for lofi beats | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p21 | Open the Calculator and compute 256 times 13 | PARTIAL | completed | verified | Resolved but no execution result captured. |
| p22 | Open the Brave browser | PARTIAL | failed | blocked | Execution failed: Application 'brave-browser' is not installed or not found in registry |
| p23 | Open Spotify | PARTIAL | completed | verified | Resolved but no execution result captured. |
| p24 | Open Google Chrome and search for news today | BLOCKED | completed | verification_failed | Blocked (recovery): the resolved target is no longer present |
| p25 | Focus the K.R.I.A. window | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |

## Per-prompt detail

### p01 — Open the Calculator app

- Expected: Calculator window opens
- Outcome: **PARTIAL** (15317 ms)
- Detail: Resolved but no execution result captured.
- Reply: Workflow completed 2 verified step(s) safely, one bound action at a time.
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 82df8973-6ee9-4094-a557-b831d2efd28d
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = completed
  - `workflow_step_count` = 2
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p01.json`

### p02 — Open the Files manager

- Expected: File manager (Nautilus) opens
- Outcome: **BLOCKED** (21087 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = b2b9a1dd-2753-4eae-96b5-633213733e37
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p02.json`

### p03 — Open the Text Editor

- Expected: Text editor opens
- Outcome: **BLOCKED** (20370 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 5aa0cfda-c745-4251-b498-9d6c93230db9
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p03.json`

### p04 — Open the Terminal

- Expected: Terminal opens
- Outcome: **BLOCKED** (19431 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = ce2b4b71-d357-4c34-867c-71203136ace9
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p04.json`

### p05 — Open the Settings app

- Expected: Settings opens
- Outcome: **PARTIAL** (16308 ms)
- Detail: Resolved but no execution result captured.
- Reply: Workflow completed 2 verified step(s) safely, one bound action at a time.
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 05b334dc-ce7c-47cb-86c4-e1322232c714
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = completed
  - `workflow_step_count` = 2
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p05.json`

### p06 — Open Google Chrome

- Expected: Chrome opens
- Outcome: **BLOCKED** (15749 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 6eacfc22-7a6d-4eb6-9219-b5dfac13bc56
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p06.json`

### p07 — Open Firefox

- Expected: Firefox opens
- Outcome: **BLOCKED** (17230 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = cdcac5eb-b950-41ee-9db2-99ac6d4f271e
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p07.json`

### p08 — Open the Calculator and type 5 + 5

- Expected: Calc opens, 5 + 5 typed
- Outcome: **BLOCKED** (15960 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = type_text
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = 256c0dcb-403c-473e-a046-08d7b224d906
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p08.json`

### p09 — Open Google Chrome and search for the latest Ubuntu version

- Expected: Chrome opens, search runs
- Outcome: **BLOCKED** (17022 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = d54e614b-e21a-47e1-83b7-8205fcfdd3a3
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p09.json`

### p10 — Open Google Chrome and open Gmail

- Expected: Chrome opens Gmail
- Outcome: **BLOCKED** (17434 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 9004a87f-26f4-4553-a4ea-a6d91d4b3c58
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p10.json`

### p11 — Open Google Chrome and go to youtube.com

- Expected: Chrome navigates to YouTube
- Outcome: **BLOCKED** (17268 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 0a4d91c2-0e93-4088-903e-027d7f8945eb
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p11.json`

### p12 — Open Firefox and search for weather today

- Expected: Firefox search runs
- Outcome: **PARTIAL** (16052 ms)
- Detail: Planned but not resolved/executed.
- Reply: Workflow stopped safely: No matching GUI target candidates were found.
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = cbfe0492-4612-4063-a0a7-f1ab8f41562d
  - `target_status` = blocked
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p12.json`

### p13 — Open the Files manager and go to the Downloads folder

- Expected: Nautilus opens Downloads
- Outcome: **BLOCKED** (15559 ms)
- Detail: Blocked (plan_validation): TypeText has no safe text/query payload.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = 3d4f14e5-8dae-4c33-aa05-c7ba614ac84a
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = TypeText has no safe text/query payload.
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p13.json`

### p14 — Open the Settings and go to Wi-Fi

- Expected: Settings opens Wi-Fi pane
- Outcome: **BLOCKED** (15731 ms)
- Detail: Blocked (plan_validation): TypeText has no safe text/query payload.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = b3579068-b7c5-4df2-9858-ef5932ba3eec
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = TypeText has no safe text/query payload.
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p14.json`

### p15 — Open the Terminal and run ls

- Expected: Terminal runs ls
- Outcome: **BLOCKED** (17820 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = bcb7bf3a-2dfc-48c7-97c9-fc2f3050e1f1
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p15.json`

### p16 — Open the Text Editor and type Hello World

- Expected: Editor types text
- Outcome: **BLOCKED** (16306 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = type_text
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = 98bd08ca-70fc-4955-ab62-94149e61e9e3
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p16.json`

### p17 — Open Google Chrome and open a new tab

- Expected: Chrome new tab
- Outcome: **BLOCKED** (16617 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 0bfe59e1-84f6-47ce-af76-7ea69e85874f
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p17.json`

### p18 — Open the Screenshot tool

- Expected: Screenshot tool opens
- Outcome: **PARTIAL** (13476 ms)
- Detail: Resolved but no execution result captured.
- Reply: Workflow completed 2 verified step(s) safely, one bound action at a time.
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 8826fa53-70bd-4787-91c0-6e81edde5d56
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = completed
  - `workflow_step_count` = 2
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p18.json`

### p19 — Open Google Chrome and go to github.com

- Expected: Chrome navigates to GitHub
- Outcome: **BLOCKED** (16078 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 9547cd9d-355e-4a50-897f-058b4e773443
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p19.json`

### p20 — Open Google Chrome, go to google.com and search for lofi beats

- Expected: Chrome searches
- Outcome: **BLOCKED** (14633 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 9a176ec7-a7a2-4e73-b651-c54eb906caa7
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p20.json`

### p21 — Open the Calculator and compute 256 times 13

- Expected: Calc computes
- Outcome: **PARTIAL** (13642 ms)
- Detail: Resolved but no execution result captured.
- Reply: Workflow completed 2 verified step(s) safely, one bound action at a time.
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 5adcf6e7-9dfa-4516-9143-ab2cb4b8894a
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = completed
  - `workflow_step_count` = 2
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p21.json`

### p22 — Open the Brave browser

- Expected: Brave opens
- Outcome: **PARTIAL** (17744 ms)
- Detail: Execution failed: Application 'brave-browser' is not installed or not found in registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = f21fe458-9fe9-4c57-9c27-b2b3d1b74f11
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = Application 'brave-browser' is not installed or not found in registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p22.json`

### p23 — Open Spotify

- Expected: Spotify opens
- Outcome: **PARTIAL** (15588 ms)
- Detail: Resolved but no execution result captured.
- Reply: Workflow completed 2 verified step(s) safely, one bound action at a time.
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 7337cf01-1a29-40c0-bef2-948435720f64
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verified
  - `workflow_status` = completed
  - `workflow_step_count` = 2
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p23.json`

### p24 — Open Google Chrome and search for news today

- Expected: Chrome searches news
- Outcome: **BLOCKED** (16111 ms)
- Detail: Blocked (recovery): the resolved target is no longer present
- Reply: Workflow stopped safely: the resolved target is no longer present
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = dd623c11-ee97-47c9-9889-79abf07dac20
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = completed
  - `backend_used` = open_application
  - `verification_status` = verification_failed
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = the resolved target is no longer present
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p24.json`

### p25 — Focus the K.R.I.A. window

- Expected: KRIA window focused
- Outcome: **BLOCKED** (15044 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = target_availability_check
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = dfd3d582-0e56-4b86-822c-2bc9849428b3
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/p25.json`
