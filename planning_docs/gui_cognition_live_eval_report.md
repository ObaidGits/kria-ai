# GUI Cognition Live Evaluation Report

- Generated: 2026-06-12 10:30:29Z
- Mode: `execute_live`  ·  Endpoint: `http://127.0.0.1:3001/api/testing/desktop-chat-command`
- Path: same as UI (`send_manual_tool_message`, manual_profile=`gui_cognition`, workflow=true, hitl=approve)
- Prompts: 25

## Summary

| Outcome | Count |
|---|---|
| BLOCKED | 5 |
| BUG | 18 |
| PARTIAL | 2 |

Legend: **PASS** executed+verified · **PARTIAL** ran but failed/unverified · **BUG** concrete defect · **BLOCKED** stopped before execution · **EXPECTED_ABSENT** app not installed · **UNEXPECTED** crash/empty/no-plan.

## Results

| ID | Prompt | Outcome | exec | verify | Detail |
|---|---|---|---|---|---|
| p01 | Open the Calculator app | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p02 | Open the Files manager | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p03 | Open the Text Editor | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p04 | Open the Terminal | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p05 | Open the Settings app | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p06 | Open Google Chrome | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p07 | Open Firefox | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p08 | Open the Calculator and type 5 + 5 | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |
| p09 | Open Google Chrome and search for the latest Ubuntu version | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p10 | Open Google Chrome and open Gmail | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p11 | Open Google Chrome and go to youtube.com | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p12 | Open Firefox and search for weather today | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p13 | Open the Files manager and go to the Downloads folder | BLOCKED | None | None | Blocked (plan_validation): TypeText has no safe text/query payload. |
| p14 | Open the Settings and go to Wi-Fi | BLOCKED | None | None | Blocked (plan_validation): TypeText has no safe text/query payload. |
| p15 | Open the Terminal and run ls | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p16 | Open the Text Editor and type Hello World | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |
| p17 | Open Google Chrome and open a new tab | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p18 | Open the Screenshot tool | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p19 | Open Google Chrome and go to github.com | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p20 | Open Google Chrome, go to google.com and search for lofi beats | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p21 | Open the Calculator and compute 256 times 13 | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p22 | Open the Brave browser | PARTIAL | failed | blocked | Execution failed: application 'OpenApp' is not found in the installed app registry |
| p23 | Open Spotify | PARTIAL | failed | blocked | Execution failed: application 'OpenApp' is not found in the installed app registry |
| p24 | Open Google Chrome and search for news today | BUG | failed | blocked | Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry |
| p25 | Focus the K.R.I.A. window | BLOCKED | None | None | Blocked (plan_validation): Plan validation blocked target resolution. |

## Per-prompt detail

### p01 — Open the Calculator app

- Expected: Calculator window opens
- Outcome: **BUG** (21209 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = f0fbbd03-e903-4eeb-b20a-738ef6753b20
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p01.json`

### p02 — Open the Files manager

- Expected: File manager (Nautilus) opens
- Outcome: **BUG** (21456 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 6db79996-39d3-4fdf-899d-2b51a56b3834
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p02.json`

### p03 — Open the Text Editor

- Expected: Text editor opens
- Outcome: **BUG** (15831 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 40fff0cb-2791-42dc-b786-0f1b00b7c8fb
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p03.json`

### p04 — Open the Terminal

- Expected: Terminal opens
- Outcome: **BUG** (20709 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = a30a3c09-ad86-4810-a240-25d785a04513
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p04.json`

### p05 — Open the Settings app

- Expected: Settings opens
- Outcome: **BUG** (13183 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 481859ad-a288-4e0a-a02c-4cefadf9c05d
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p05.json`

### p06 — Open Google Chrome

- Expected: Chrome opens
- Outcome: **BUG** (20736 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 0f4be66a-d3c7-4454-87fa-b974950bdac6
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p06.json`

### p07 — Open Firefox

- Expected: Firefox opens
- Outcome: **BUG** (16026 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = c14db21a-50cf-42b6-9ab0-a6774037592b
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p07.json`

### p08 — Open the Calculator and type 5 + 5

- Expected: Calc opens, 5 + 5 typed
- Outcome: **BLOCKED** (19059 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = type_text
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = f5d6d584-c058-4e0e-a4e4-7ffb04426c8f
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw/p08.json`

### p09 — Open Google Chrome and search for the latest Ubuntu version

- Expected: Chrome opens, search runs
- Outcome: **BUG** (14083 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 6560dbd7-a5b1-400b-b7ab-f92aa2dacc54
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p09.json`

### p10 — Open Google Chrome and open Gmail

- Expected: Chrome opens Gmail
- Outcome: **BUG** (21911 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 73cc07b3-883c-4697-b22c-163dda05256e
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p10.json`

### p11 — Open Google Chrome and go to youtube.com

- Expected: Chrome navigates to YouTube
- Outcome: **BUG** (20802 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = b1100b94-51f0-4404-a54c-ed4dc93d9d88
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p11.json`

### p12 — Open Firefox and search for weather today

- Expected: Firefox search runs
- Outcome: **BUG** (19720 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 8c10cfd3-0f64-4e48-a378-fd87e0478aba
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p12.json`

### p13 — Open the Files manager and go to the Downloads folder

- Expected: Nautilus opens Downloads
- Outcome: **BLOCKED** (19150 ms)
- Detail: Blocked (plan_validation): TypeText has no safe text/query payload.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = 30ba88b1-ccbe-459d-9583-2659e3d7d344
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = TypeText has no safe text/query payload.
- Raw: `planning_docs/gui_cognition_live_eval_raw/p13.json`

### p14 — Open the Settings and go to Wi-Fi

- Expected: Settings opens Wi-Fi pane
- Outcome: **BLOCKED** (18478 ms)
- Detail: Blocked (plan_validation): TypeText has no safe text/query payload.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = f7474bf9-1a68-4f9e-810a-057adc01489f
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = TypeText has no safe text/query payload.
- Raw: `planning_docs/gui_cognition_live_eval_raw/p14.json`

### p15 — Open the Terminal and run ls

- Expected: Terminal runs ls
- Outcome: **BUG** (14599 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = bdd805f9-fefd-4421-a6ab-a956fb844bd9
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p15.json`

### p16 — Open the Text Editor and type Hello World

- Expected: Editor types text
- Outcome: **BLOCKED** (19634 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = type_text
  - `risk_level` = medium
  - `requires_approval` = False
  - `plan_id` = 5d67ba58-d9b6-49de-a909-f8b89b078a16
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw/p16.json`

### p17 — Open Google Chrome and open a new tab

- Expected: Chrome new tab
- Outcome: **BUG** (14617 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 688b6cc1-f10f-4a05-aedf-b0abf237f5c3
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p17.json`

### p18 — Open the Screenshot tool

- Expected: Screenshot tool opens
- Outcome: **BUG** (14435 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = bba3627e-d4eb-46d8-9ce4-c4aba1f7b02c
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p18.json`

### p19 — Open Google Chrome and go to github.com

- Expected: Chrome navigates to GitHub
- Outcome: **BUG** (14396 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 27a817ba-2262-4b7e-8b74-22a2d4958202
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p19.json`

### p20 — Open Google Chrome, go to google.com and search for lofi beats

- Expected: Chrome searches
- Outcome: **BUG** (20671 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = a8443fec-820f-4ec6-8312-6369386dfa7f
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p20.json`

### p21 — Open the Calculator and compute 256 times 13

- Expected: Calc computes
- Outcome: **BUG** (20040 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 58960a24-261b-477a-8dca-c0110d4c26e4
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p21.json`

### p22 — Open the Brave browser

- Expected: Brave opens
- Outcome: **PARTIAL** (21366 ms)
- Detail: Execution failed: application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = f7ce8677-89bc-4b16-b46c-a6852c96c9f7
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p22.json`

### p23 — Open Spotify

- Expected: Spotify opens
- Outcome: **PARTIAL** (18109 ms)
- Detail: Execution failed: application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = analyze_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 21648b92-3b28-4773-a057-b14ca80bf12d
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 2
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p23.json`

### p24 — Open Google Chrome and search for news today

- Expected: Chrome searches news
- Outcome: **BUG** (18264 ms)
- Detail: Executor passed action kind 'OpenApp' as the app name → application 'OpenApp' is not found in the installed app registry
- Reply: Workflow stopped safely: deterministic action backend failed
- Signals:
  - `intent` = browser_search_plan
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 28695423-79fa-448e-9188-90ca4352c627
  - `target_status` = resolved
  - `action_type` = OpenApp
  - `exec_status` = failed
  - `exec_error_code` = backend_failed
  - `exec_error` = application 'OpenApp' is not found in the installed app registry
  - `backend_used` = open_application
  - `verification_status` = blocked
  - `workflow_status` = blocked
  - `workflow_step_count` = 6
  - `blocker_kind` = recovery
  - `blocker_reason` = deterministic action backend failed
- Raw: `planning_docs/gui_cognition_live_eval_raw/p24.json`

### p25 — Focus the K.R.I.A. window

- Expected: KRIA window focused
- Outcome: **BLOCKED** (10691 ms)
- Detail: Blocked (plan_validation): Plan validation blocked target resolution.
- Reply: Target resolution was skipped after plan validation. I did not execute any GUI action.
- Signals:
  - `intent` = target_availability_check
  - `risk_level` = low
  - `requires_approval` = False
  - `plan_id` = 968ff299-9bd6-4b90-945b-41022534753e
  - `target_status` = skipped
  - `blocker_kind` = plan_validation
  - `blocker_reason` = Plan validation blocked target resolution.
- Raw: `planning_docs/gui_cognition_live_eval_raw/p25.json`
