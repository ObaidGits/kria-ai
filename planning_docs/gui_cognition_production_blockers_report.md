# GUI Cognition — Live Daily-Task Evaluation: Production Blockers Report

**Date:** 2026-06-12
**Method:** 25 real daily GUI prompts driven through the **same backend path the UI uses**
(`POST /api/testing/desktop-chat-command`, `manual_profile = gui_cognition`,
`execution_mode = execute_live`, `workflow = true`, `hitl_decision_fixture = approve`),
against the running desktop app at `http://127.0.0.1:3001`.
**Runner:** `testing/tools/gui_cognition_live_eval.py`
**Raw per-prompt JSON:** `planning_docs/gui_cognition_live_eval_raw/*.json`
**Data report (auto):** `planning_docs/gui_cognition_live_eval_report.md`

---

## Headline result

| Outcome | Count | Meaning |
|---|---|---|
| **PASS** (executed + verified) | **0 / 25** | No daily task completed and verified end-to-end |
| BUG (concrete defect) | 18 | OpenApp executed with the wrong app name |
| BLOCKED (stopped pre-execution) | 5 | Planner/validator refused to proceed |
| PARTIAL (app genuinely absent) | 2 | Brave/Spotify not installed (still hit Bug #2) |

**Bottom line:** the perception → safety → executor *plumbing* is healthy
(uinput ready, `can_execute_actions = true`, focus/type/click supported), but
**GUI Cognition cannot complete a single real daily task.** Two critical defects
upstream of execution break everything, and the existing fixture-based tests
were masking the worst one.

---

## Blocker #1 (CRITICAL) — LLM planner is rejected on *every* prompt; everything runs on the heuristic fallback

Every single prompt reported:

```
planner.mode               = llm_rejected_fallback
planner.llm_attempted      = true
planner.llm_status         = rejected
planner.llm_failure_reason = "LLM planner returned prose or non-object content"
planner.model              = null
```

- The real LLM planner never produces a usable structured plan, so the pipeline
  silently falls back to a **generic template planner** (`confidence = 0.62`).
- The fallback emits canned, domain-blind steps (e.g. always "Open or switch to
  the requested browser → focus address field → type query"), and **cannot bind
  concrete app names, text payloads, or non-browser intents**. This is the root
  cause feeding Blockers #2–#6.
- `model = null` indicates the planner is not bound to a working model / the
  grammar-constrained (JSON) decoding is not enforced, so the model returns prose
  and is discarded.

**Impact:** GUI Cognition is effectively running with no real reasoning layer.
**Fix direction:** enforce grammar/JSON-constrained decoding for the planner
(llguidance), confirm the planner model is actually loaded (`model` must be
non-null), and treat `llm_rejected_fallback` as a *degraded* state in telemetry
(not a silent success).

---

## Blocker #2 (CRITICAL) — OpenApp executes with the **action kind** as the app name

18/25 prompts failed in execution with:

```
execution.action_type       = OpenApp
execution.status            = failed
execution.backend_used      = open_application
execution.error_code        = backend_failed
execution.safe_error_summary= "application 'OpenApp' is not found in the installed app registry"
target_resolution.status    = resolved        (but resolved_target = null)
```

**Root cause** — `crates/kria-core/src/agent/gui_cognition/mod.rs` (~line 854):

```rust
let target_name = proposal
    .target_label
    .clone()
    .or_else(|| proposal.target_control_id.clone())
    .unwrap_or_else(|| proposal.action_type.clone());   // ← "OpenApp" leaks here
```

For `OpenApp`, target resolution returns `status = resolved` with
`resolved_target = null` (an app launch has no on-screen control to resolve), so
`proposal.target_label` and `proposal.target_control_id` are both `None`. The
fallback then uses `proposal.action_type` ("OpenApp") as the target name, and the
executor passes that literal string to the `open_application` tool
(`gui_cognition.rs:3236` → `"name": request.target_name`).

**Impact:** every "open <app>" action is impossible — the single most common
daily operation.
**Fix direction:** thread the concrete app name from the goal contract's
target-app hint / plan step into `proposal.target_label` for `OpenApp` /
`SwitchWindow`, and **never** fall back to `action_type` as a target name
(fail explicitly instead).

---

## Blocker #3 (HIGH) — Test coverage gap: `execute_fixture` masks Blocker #2

Cross-check in `execute_fixture` mode:

- p06 "Open Google Chrome": `execution.status = completed` (the fixture executor
  returns OK **regardless of the target name**), yet `verification.status =
  inconclusive`.

So the fixture executor reports success for an action that fails 100% of the time
live. **This is why the existing same-path suites stayed green while real daily
tasks are completely broken.** Fixture "success" was never cross-checked against a
real `open_application` registry lookup, and verification never actually turned
`verified`.

**Fix direction:** add a live (or registry-validated) executor assertion to the
suite so a target name that the app registry cannot resolve fails the test;
require `verification.status = verified` (not `inconclusive`) for a PASS.

---

## Blocker #4 (HIGH) — Multi-step "open then act" is not sequenced; planner asks for clarification

- p08 "Open the Calculator and type 5 + 5" → plan step = *"Which visible field
  should receive the text?"* → `plan_validation.status = needs_clarification` →
  BLOCKED.

At plan time the app is not open yet, so no target field exists. Instead of
sequencing (open → re-observe → bind field → type), the planner demands
clarification and the workflow stops.

**Fix direction:** the workflow runtime must support deferred target binding —
open/launch first, re-observe, then resolve the field for the type step
(consistent with the per-step re-observe model already in Step 10).

---

## Blocker #5 (HIGH) — Intent mis-routing: non-browser tasks planned as browser navigation

- p13 "Open the Files manager and go to the Downloads folder" was planned as
  *"Open or switch to the requested browser → Type the requested URL"* and then
  blocked with `TypeText has no safe text/query payload.`

A file-manager navigation was mapped onto the browser-URL template. The fallback
planner has no notion of the Files/Settings/Terminal domains.

**Fix direction:** domain-aware planning (file manager, settings, terminal,
calculator are distinct from browser); do not default every "go to X" to a
browser URL.

---

## Blocker #6 (MEDIUM) — Simple intents collapse into clarification

- p25 "Focus the K.R.I.A. window" → plan step = *"What exact GUI task should I
  plan?"* → `needs_clarification` → BLOCKED.

A trivial `SwitchWindow`/focus intent was not recognized by the fallback planner.

**Fix direction:** the planner (or a deterministic pre-router) should handle
obvious single-action intents (focus window, open app, switch app) without an LLM
round-trip.

---

## Blocker #7 (MEDIUM) — TypeText payload extraction missing

- p13 / p14 blocked with `TypeText has no safe text/query payload.` The fallback
  planner produced a TypeText step but never extracted the query/text/URL from the
  prompt, so the validator (correctly) refused it.

**Fix direction:** payload extraction must populate the text/query for TypeText
steps; if absent, the step should be dropped or the user asked — but only after a
real plan, not as a blanket block.

---

## Per-prompt outcomes

| ID | Prompt | Outcome | exec | verify |
|---|---|---|---|---|
| p01 | Open the Calculator app | BUG | failed | blocked |
| p02 | Open the Files manager | BUG | failed | blocked |
| p03 | Open the Text Editor | BUG | failed | blocked |
| p04 | Open the Terminal | BUG | failed | blocked |
| p05 | Open the Settings app | BUG | failed | blocked |
| p06 | Open Google Chrome | BUG | failed | blocked |
| p07 | Open Firefox | BUG | failed | blocked |
| p08 | Open the Calculator and type 5 + 5 | BLOCKED | – | – |
| p09 | Open Chrome and search latest Ubuntu version | BUG | failed | blocked |
| p10 | Open Chrome and open Gmail | BUG | failed | blocked |
| p11 | Open Chrome and go to youtube.com | BUG | failed | blocked |
| p12 | Open Firefox and search weather today | BUG | failed | blocked |
| p13 | Open Files manager → Downloads | BLOCKED | – | – |
| p14 | Open Settings → Wi-Fi | BLOCKED | – | – |
| p15 | Open Terminal and run ls | BUG | failed | blocked |
| p16 | Open Text Editor and type Hello World | BLOCKED | – | – |
| p17 | Open Chrome and open a new tab | BUG | failed | blocked |
| p18 | Open the Screenshot tool | BUG | failed | blocked |
| p19 | Open Chrome and go to github.com | BUG | failed | blocked |
| p20 | Open Chrome → google.com → search lofi beats | BUG | failed | blocked |
| p21 | Open Calculator and compute 256 × 13 | BUG | failed | blocked |
| p22 | Open the Brave browser | PARTIAL (absent) | failed | blocked |
| p23 | Open Spotify | PARTIAL (absent) | failed | blocked |
| p24 | Open Chrome and search news today | BUG | failed | blocked |
| p25 | Focus the K.R.I.A. window | BLOCKED | – | – |

---

## Recommended fix order (highest value, lowest risk first)

1. **Blocker #2** — stop the `action_type` fallback for `OpenApp`; thread the real
   app name into `proposal.target_label`. Unblocks ~18 prompts immediately.
2. **Blocker #1** — make the LLM planner actually return constrained JSON (bind the
   model, enforce grammar). Removes the silent `llm_rejected_fallback` regime.
3. **Blocker #3** — close the test gap: require registry-valid target + real
   `verification = verified` for a PASS; add live execution to the suite.
4. **Blockers #4–#7** — sequencing for open-then-act, domain-aware intent routing,
   simple-intent handling, and TypeText payload extraction.

## What is actually healthy (so we don't regress it)

- Action backend: `uinput_accessibility` selected, daemon + socket healthy,
  `can_execute_actions = true`, focus/type/click/verification all supported.
- Safety gate + HITL approve path engaged correctly (no unsafe auto-execution).
- Perception, observation, and the workflow runtime state machine ran without
  crashes or raw prompt/OCR/secret leakage in any of the 25 runs.


---

# UPDATE — Fix #1 applied and live-verified (2026-06-12)

## What was fixed (generic, intelligence-preserving — no per-app hardcoding)

**Root cause of the 18 "OpenApp" failures:** the app name was extracted correctly
by the goal contract (`target_app_hint = "Chrome"`), but it was never threaded
into the action proposal for app-launch flows, so the executor received the
literal action kind `"OpenApp"`.

Three data-driven edits:

1. `crates/kria-core/src/agent/gui_cognition/llm_planner.rs` — `browser_search_steps`
   and `browser_navigation_steps` now attach `.with_app_hint(contract.target_app_hint)`
   to their `OpenApp` step (the direct `OpenApp` branch already did). The concrete
   app name now rides on the plan step for *every* path.
2. `crates/kria-core/src/agent/gui_cognition/safety_hitl.rs` — `build_action_proposal_for_step`
   now derives `target_label` for `OpenApp`/`SwitchWindow` from the step's
   app/window hint, falling back to the goal contract's `target_app_hint` /
   `target_window_hint`, when the target resolver returns no on-screen control.
3. `crates/kria-core/src/agent/gui_cognition/mod.rs` — removed the dangerous
   `.unwrap_or_else(|| proposal.action_type.clone())` fallback so the action kind
   can **never** leak into the executor as an app name.

Regression test added: `open_app_step_threads_concrete_app_hint_not_action_kind`
in `gui_cognition_llm_planner_tests.rs` (asserts the OpenApp step carries a
concrete app hint that is never the action kind). All gui_cognition core suites
remain green (planner 17, safety_hitl 6, backend_route, workflow_runtime 10).

## Live re-run (same 25 prompts, `execute_live`, after restart)

| Outcome | Before | After |
|---|---|---|
| **PASS** (app opened + verified) | **0** | **6** |
| BUG (OpenApp name leak) | **18** | **0** |
| PARTIAL_PROGRESS (first action ran, workflow then blocked) | – | 13 |
| BLOCKED (plan validation) | 5 | 5 |
| EXPECTED_ABSENT (app not installed) | 2 | 1 |

PASS now: Calculator, Settings, Firefox (search), Screenshot tool, Calculator
(compute), Spotify — apps **actually launch and verify** end-to-end.
Raw: `planning_docs/gui_cognition_live_eval_raw_after_fix/`,
report: `planning_docs/gui_cognition_live_eval_report_after_fix.md`.

## New #1 blocker (next slice) — re-observe between workflow steps

The 13 PARTIAL_PROGRESS are multi-step flows (e.g. "open Chrome and search…").
The first `OpenApp` step now **succeeds**, but the next step (`FocusField` on the
address bar) resolves its target against the observation captured **before** the
app opened, so the runtime reports `the resolved target is no longer present` and
safely blocks (recovery). The screen genuinely changed — the safety check is
correct — but the runtime should **re-observe and re-resolve** the next step's
target against the fresh screen instead of blocking.

The intent already exists in code (`workflow_step_is_state_changing` documents
"require a re-observation before the next step resolves its target"), but the
runtime does not yet capture a fresh observation between a state-changing step
and the next target resolution. Fixing this (a re-observe hook the desktop layer
provides to the workflow loop) is expected to convert most of the 13
PARTIAL_PROGRESS into PASS, and is the highest-value next step toward production.

## Remaining (unchanged priority)

- Re-observe between steps (new #1 above) — unblocks multi-step daily tasks.
- LLM planner JSON (`llm_rejected_fallback` on every prompt) — still running on the
  heuristic planner; making the model return constrained JSON raises plan quality.
- Plan-validation blocks: TypeText payload extraction (p13/p14), simple
  focus/switch intents collapsing to clarification (p25), open-then-type
  sequencing (p08/p16).


---

# UPDATE 2 — UI-path fixes (2026-06-12): prompt now acted on + non-blocking command

## Issue A — GUI Cognition only observed, never acted (prompt "not checked")

**Root cause:** the interactive UI path (`send_message_with_profile` →
`desktop_gui_cognition_command_capture`) passed `None` options, so it defaulted to
`execution_mode = safety_only` and `workflow_enabled = false`. Result: every UI
prompt ran observe → plan → safety-gate only and stopped at "Step 7 review only;
I did not execute any GUI action." The prompt *was* understood; it simply was
never executed.

**Fix (`crates/kria-desktop/src/commands/chat.rs`):** the interactive GUI
Cognition override now opts into `execution_mode = ExecuteLive` +
`workflow_enabled = true`. The Step 6 safety gate + HITL still gate every risky
action, so no safety contract is bypassed. Programmatic/test callers (local API
with explicit `gui_cognition_test`) keep the conservative `safety_only` default,
so the harness suites are unaffected.

**Verified (capture level):** "Open Chrome and Search for google docs" now
executes `OpenApp` (`exec.status = completed`, backend `open_application`) — Chrome
actually opens — then proceeds into the multi-step workflow (it currently blocks at
step 2 on the known re-observe gap, Blocker #4, which is the deferred next slice).

## Issue B — second/third prompt + response not visible in the same chat

**Investigation:** the backend emits the correct event batch every turn
(`agent:thinking` → … → `agent:token` (reply) → `agent:done`), verified across two
sequential turns. A new frontend unit test
(`ui/src/stores/app.tool-choice.test.ts` → "renders sequential GUI Cognition
prompts and replies in the active chat") drives two full gui_cognition turns
through the real store listeners and **passes**: both prompts and both replies
render and `thinking` clears. So the message-rendering logic is correct.

The observed "prompt/response vanish" happens when the next prompt is submitted
**while the previous turn is still running** — `sendMessage` has a guard
(`if (isScopedThinking(...)) return;`) that correctly drops a prompt mid-turn (a
second existing test confirms this). With Issue A now enabling `execute_live`,
turns take longer and emit all events only at completion, so the user sees just
"Running GUI cognition workflow" for the whole turn and tends to resend.

**Fix (`crates/kria-desktop/src/commands/chat.rs`):** the gui_cognition turn now
runs in a spawned task (matching the normal agent path) and the command returns
immediately, instead of blocking the IPC for the entire (now multi-second) turn.
On error it still emits `{prefix}:token` + `{prefix}:done` so `thinking` always
clears. This removes the long IPC hang and keeps the UI responsive.

**Remaining UX follow-up (documented, not yet done):** gui_cognition events are
still emitted as one batch at turn completion, so there is no incremental progress
during a long `execute_live` turn. Streaming events *during* the turn (so the user
sees observe → plan → execute → verify progress live) is the next UX improvement;
it requires the runtime to emit incrementally rather than returning all events at
once.

**Verification:** `kria-desktop` builds; full UI suite 119/119 (incl. the new
two-turn test); core planner suite 17/17 (incl. the OpenApp regression test);
`git diff --check` clean. The app was rebuilt and restarted on
`http://127.0.0.1:3001`. The Tauri IPC path (execute_live default + spawn) is
exercised only by the real desktop UI, so a UI click-test is the final
confirmation step for the interactive path.
