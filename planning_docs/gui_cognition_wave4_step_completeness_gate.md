# GUI Cognition — Wave 4 / Task 5 Plan-Step Completeness Gate (affected families ≥ 80%)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 5.4 — Live gate: file-manager search /
summarize-visible / copy no longer blocked; affected families ≥ 80%.
**Flag:** `gui_cog_step_completeness` (env `KRIA_GUI_COG_STEP_COMPLETENESS`).
**Requirements:** 4 (plan-step completeness — payload + verification); methodology per 17 (live
testing), 18 (no regression), 23 (verification contract — every step carries a type-correct
`verification_strategy`).
**Status of this record:** **CI-safe verification COMPLETE + gate flip applied.** The live
numeric **affected-families ≥ 80%** (file-manager search / summarize-visible / copy) held-out
audit is **PENDING** a reachable live desktop API (`http://127.0.0.1:3001` health fails in this
environment). No live percentages are fabricated.

---

## 1. Why this document exists

Task 5 ("Plan-step completeness") closes the validator-blocking root cause: well-formed plan steps
were being rejected at validation when a step lacked a `verification_strategy` or a payload, so
families like file-manager search, summarize-visible, and copy stalled on "Plan validation
blocked" even though the intent was sound. Task 5 post-processes every step to ensure a type-correct
`verification_strategy` is set per step type (5.1), sources a sanitized payload for payload steps
and converts a *genuinely* missing payload to `AskClarification` rather than emitting an invalid
step (5.2), and proves the validator no longer blocks well-formed steps for missing
payload/verification (5.3). The authoritative acceptance gate for Task 5 is the live held-out
capability audit landing the **affected families ≥ 80%** with file-manager search /
summarize-visible / copy **no longer blocked**.

This environment has **no reachable live KRIA desktop session** (the audit's own health probe to
`http://127.0.0.1:3001` returns `000` and the tool fails safe with exit 2, writing no report). The
live numeric audit therefore cannot run here and is **not** fabricated. This document records the
**CI-safe surrogate** — the deterministic T1/T2 evidence that proves the step-completeness gate's
INTENT is met — exactly as the Task 0.5/0.6 baseline, the Task 2.9 planner gate, the Task 3.6
re-observe gate, and the Task 4.5 Wayland-focus gate established for every "live gate" task in this
spec.

> The deterministic T1/T2 tiers are the CI-verifiable proof that every step carries a type-correct
> `verification_strategy`, payload steps carry a sanitized payload (or convert to
> `AskClarification` when truly missing), and the validator no longer blocks well-formed steps for
> missing payload/verification; the ≥ 80% numeric figures (file-manager search /
> summarize-visible / copy) are the live confirmation of that same property and are gated on a
> running desktop session (reproduction in §5).

---

## 2. CI-safe verification performed (the deterministic surrogate for affected families ≥ 80%)

### 2.1 Held-out prompt set integrity (frozen + digest-locked)

| Check | Command | Result |
|---|---|---|
| Frozen set intact | `python3 testing/tools/heldout_prompt_set.py --verify` | **PASS** — "Held-out set OK: frozen + valid (21 families, 105 prompts, >= 5/family)." |
| Audit plan dry-run (real session) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session` | **PASS (exit 0)** — 21 families / 105 prompts enumerated; per-kind assertions correct; destructive-leak detector active |
| Audit plan dry-run (test substrate) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate` | **PASS (exit 0)** — same plan; substrate auto-approval path armed |

The frozen set is digest-locked (`heldout_prompt_set.v1.lock`, SHA-256); it cannot be silently
edited to make a build pass. The audit tool's scoring lives in pure functions and runs end-to-end
in `--dry-run` with no network. The families affected by Task 5 — `C12_in_app_search`
(file-manager / in-app search), `C16_read_visible` (summarize-visible), and `C6_clipboard` (copy) —
are each present at n=5 and scored on the strict execute+verify contract.

### 2.2 Task 5 deterministic test evidence (proves the step-completeness gate INTENT)

`cargo build -p kria-core` → **Finished (exit 0)**; `cargo build -p kria-desktop` → **Finished
(exit 0)** (call-site flip compiles). Then:

| # | Suite | Command | Result |
|---|---|---|---|
| 1 | Step-completeness (T1, lib) | `cargo test -p kria-core --lib gui_cognition::llm_planner` | **PASS — 36 passed** (incl. `task_5_1_step_completeness_tests`, `task_5_2_step_payload_tests`, `task_5_3_validator_tests`) |
| 2 | LLM planner integration (T1/T2) | `cargo test -p kria-core --test gui_cognition_llm_planner_tests` | **PASS — 49 passed** |
| 3 | Workflow runtime (T1/T2) | `cargo test -p kria-core --test gui_cognition_workflow_runtime_tests` | **PASS — 16 passed** |
| 4 | Target resolver (T1) | `cargo test -p kria-core --test gui_cognition_target_resolver_tests` | **PASS — 10 passed** |
| 5 | T2 fixture tier (no display) | `cargo test -p kria-core --test gui_cognition_t2_fixture_tier` | **PASS — 16 passed** |
| 6 | Verification (T1/T2) | `cargo test -p kria-core --test gui_cognition_verification_tests` | **PASS — 12 passed** |

These collectively prove the gate's intent without a live run:

- **Every step carries a type-correct `verification_strategy` (Req 4.1 / 5.1, 23):**
  `post_process_fills_correct_strategy_when_empty`,
  `post_process_replaces_incompatible_strategy_with_type_correct_default`,
  `default_strategy_is_valid_for_every_supported_step_type`,
  `default_strategy_matches_the_design_per_type_mapping`,
  `post_process_never_assigns_an_invalid_strategy`, and
  `post_process_preserves_already_valid_strategy` show the post-process fills/repairs the
  per-step strategy without ever assigning an invalid one and without clobbering a valid one.
- **Payload steps carry a sanitized payload; genuinely-missing → AskClarification (Req 4.2 / 5.2):**
  `type_text_payload_sourced_from_contract_text_payload`,
  `in_app_search_payload_sourced_from_query_summary`,
  `browser_navigate_payload_sourced_from_query_summary`,
  `existing_payload_is_not_overwritten`, `non_payload_steps_are_untouched`,
  `type_text_missing_payload_converts_to_ask_clarification`, and
  `missing_payload_step_is_never_left_invalid` show payloads are sourced from the goal contract,
  never overwritten, and a truly-missing payload becomes `AskClarification` instead of an invalid
  step.
- **Privacy preserved (Req 4.2 / 15):** `sourced_payload_does_not_echo_secrets` shows sourced
  payloads never echo secret values.
- **Validator no longer blocks well-formed steps (Req 5.3):**
  `genuinely_missing_payload_yields_needs_clarification_not_blocked`,
  `resolution_blocks_missing_verification_before_post_process`,
  `resolution_not_blocked_after_post_process_fills_strategy_and_payload`, and
  `llm_plan_validator_blocks_then_clears_after_post_process` show the validator blocks a missing
  strategy *before* the pass and clears *after* it — directly proving file-manager search /
  summarize-visible / copy steps are no longer "Plan validation blocked".
- **Unsupported steps still blocked (no over-reach):**
  `unsupported_step_type_is_still_blocked_after_post_process` and
  `executable_at_plan_stage_is_still_blocked_after_post_process` show the pass does not paper over
  genuinely invalid step types.
- **Flag OFF preserves the plan byte-for-byte (Req 18):** `flag_off_leaves_plan_unchanged` and
  `step_completeness_flag_defaults_off` confirm the post-process only runs when the flag is ON;
  with it OFF, the Steps 1–12 plan is preserved unchanged.

This is the deterministic, CI-verifiable surrogate for the affected-families ≥ 80% held-out target:
the file-manager search (C12), summarize-visible (C16), and copy (C6) families that stalled on
"Plan validation blocked" in the ~28% baseline now post-process into complete, valid, verifiable
steps.

### 2.3 "Steps 1–12 green" — full gui_cognition suite surface

`cargo test -p kria-core` across the Step 1–12 same-path integration suites:

| Suite | Passed |
|---|---|
| `gui_cognition_backend_route_tests` | 27 |
| `gui_cognition_checkpoint_resume_tests` | 18 |
| `gui_cognition_context_builder_tests` | 5 |
| `gui_cognition_executor_tests` | 4 |
| `gui_cognition_goal_contract_tests` | 13 |
| `gui_cognition_llm_planner_tests` | 49 |
| `gui_cognition_observation_perception_tests` | 20 |
| `gui_cognition_preconditions_tests` | 7 |
| `gui_cognition_recovery_tests` | 17 |
| `gui_cognition_runtime_guards_tests` | 26 |
| `gui_cognition_safety_hitl_tests` | 6 |
| `gui_cognition_target_resolver_tests` | 10 |
| `gui_cognition_t2_fixture_tier` | 16 |
| `gui_cognition_verification_tests` | 12 |
| `gui_cognition_window_focus_tests` | 7 |
| `gui_cognition_workflow_runtime_tests` | 16 |
| **Integration total** | **253 passed; 0 failed** |

Plus the `gui_cognition::llm_planner` lib unit tests: **36 passed** (step-completeness post-process,
payload sourcing, validator-clears-after-post-process, default-off + default-on/rollback flag
tests).

**Grand total: 289 passed; 0 failed** across the Task 5 + Steps 1–12 surface.

Known pre-existing UNRELATED failures (explicitly excluded from this gate per the task brief) live
in the crate's inline lib unit tests, not in this integration surface:
`agent::atspi_engine::tests::atspi_snapshot_request_defaults_are_bounded_for_gui_cognition`,
`agent::loop_engine::tests::deterministic_dispatch_create_project_folder`,
`agent::continuation_reentry::tests::duplicate_continuation_is_rejected`. The first appeared in the
broad `cargo test -p kria-core gui_cognition` lib run (and is excluded by the task brief); it is not
in the Task 5 / Steps 1–12 integration surface above.

---

## 3. Gate flip: `gui_cog_step_completeness` → default ON (with env rollback)

All CI-safe Task 5 evidence is green, so the flag is flipped to default-ON following the **same
pattern Task 1 (`gui_cog_runtime_guards`), Task 2 (`gui_cog_smart_planner`), Task 3
(`gui_cog_reobserve`), and Task 4 (`gui_cog_wayland_focus`) used** (`*::from_env_default_on()`).

**Code changes:**

- `crates/kria-desktop/src/commands/gui_cognition.rs`
  - The live-turn construction now calls
    `GuiStepCompletenessConfig::from_env_default_on()` (was `from_env()`). Server-side only — a
    client cannot toggle it. The surrounding comment block was updated to record the Wave 4 flip
    and the rollback switch.
- `crates/kria-core/src/agent/gui_cognition/llm_planner.rs` (no change required this task)
  - `GuiStepCompletenessConfig::from_env_default_on()` + the testable core
    `from_env_lookup_default_on()` were already added in Task 5.1 (mirroring the runtime-guards /
    smart-planner / re-observe / Wayland-focus rollback semantics): the post-process is ON unless
    `KRIA_GUI_COG_STEP_COMPLETENESS` is an explicit opt-out (`0`/`false`/`no`/`off`/empty); an
    absent value keeps it ON. The OFF-by-default `from_env()` and the truthy parser are unchanged.
  - `step_completeness_default_on_enables_when_env_unset_or_truthy` and
    `step_completeness_default_on_rolls_back_when_env_explicitly_falsy` assert the default-on +
    rollback semantics. Both green within the 36-test lib run.

**Rollback (no code change):** set `KRIA_GUI_COG_STEP_COMPLETENESS=0` (or `false`/`no`/`off`) in the
desktop environment to restore the prior plan-preserving behavior (the post-process does not run;
the Steps 1–12 plan is preserved byte-for-byte). The deterministic T2 fixture tier is unaffected —
those runtimes set their step-completeness config explicitly and never read the env.

**Post-flip re-verification (all green):** `cargo build -p kria-desktop` (exit 0);
`cargo test -p kria-core --lib gui_cognition::llm_planner` (36 passed);
`cargo test -p kria-core --test gui_cognition_llm_planner_tests` (49 passed),
`--test gui_cognition_workflow_runtime_tests` (16 passed),
`--test gui_cognition_verification_tests` (12 passed),
`--test gui_cognition_t2_fixture_tier` (16 passed),
`--test gui_cognition_target_resolver_tests` (10 passed).

---

## 4. KRIA runtime-authority self-check

The flip preserves every authority invariant:

- **KRIA authoritative; substrates execution-only.** The flag only changes whether the planner runs
  the deterministic step-completeness post-process before validation. KRIA still owns the
  Intent → Capability → Policy → Substrate → Tool → Verification flow; no substrate gains authority.
- **No Prompt→Tool shortcut.** The post-process operates on already-parsed, schema-validated plan
  steps and the goal contract; it sources payloads from the contract (not from raw prompt/OCR/LLM
  prose) and converts a genuinely-missing payload to `AskClarification`. It never synthesizes an
  executable action from raw text or coordinates.
- **No recursive / uncontrolled loops.** The post-process is a single bounded pass over the plan's
  steps; it adds no loop and remains within the Task 1 runaway caps. Cancel / GlobalSafetyHalt
  still halt before the next action.
- **Verification truth preserved.** Every step receives a type-correct `verification_strategy`, so
  each executed step is still verified by the existing verification contract; the pass never marks a
  step verified without evidence and never assigns an invalid strategy.
- **Safety / bounded-cognition / deterministic-orchestration / cancellation** unchanged —
  safety_hitl, verification, runtime_guards, preconditions, recovery suites all green; secrets are
  never echoed by sourced payloads (`sourced_payload_does_not_echo_secrets`).

---

## 5. Environment limitation & live numeric reproduction (PENDING)

This gate was driven to **green at the CI-safe level**. The live numeric **affected families ≥ 80%**
held-out audit — file-manager search / summarize-visible / copy **no longer blocked**, executed +
verified — is **PENDING a reachable live desktop API**: `http://127.0.0.1:3001/api/health` is
unreachable here, and the audit fails safe (exit 2, no report) rather than fabricating numbers.

To close the live numeric gate on a machine with a desktop session (held-out, 3-run median):

```bash
# 0. Stand up the desktop so /api/health returns 200
cargo run -p kria-desktop        # (or: cd crates/kria-desktop && cargo tauri dev)

# 1. Preflight (uinput / AT-SPI / focus / DISPLAY)
bash scripts/gui_cognition_desktop_preflight.sh

# 2. Frozen set intact
python3 testing/tools/heldout_prompt_set.py --verify

# 3. Step-completeness ON (default after this flip; explicit for clarity),
#    non-destructive audit on the REAL session (3 runs, gate on median)
KRIA_GUI_COG_STEP_COMPLETENESS=1 python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --out planning_docs/gui_cognition_capability_audit_wave4.md

# 4. Destructive / approval families in the TEST SUBSTRATE only (Xvfb/headless seat),
#    scratch files + clipboard save/restore; auto-approval rejected on real session
KRIA_GUI_COG_STEP_COMPLETENESS=1 scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
    python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --environment test_substrate \
    --out planning_docs/gui_cognition_capability_audit_wave4_substrate.md
```

**Close condition:** the affected families — **file-manager / in-app search (C12_in_app_search),
summarize-visible (C16_read_visible), and copy (C6_clipboard) each ≥ 80%** (3-run median) — with
those families **no longer "Plan validation blocked"**, each step executed + verified against the
verification contract, **zero destructive-leak** (audit exit 0), and Steps 1–12 suites still green.
The overall median should hold its prior gate (≥ 70% from Wave 3) and trend toward the Task 6 ≥ 80%
target. Commit the generated matrices alongside this doc.

---

## 6. Acceptance for Task 5.4

- [x] Frozen held-out set integrity verified (digest lock + invariants) and audit dry-run green
      (real session + substrate).
- [x] Deterministic T1/T2 evidence green: every step carries a type-correct
      `verification_strategy`, payload steps carry a sanitized payload (or convert to
      `AskClarification` when truly missing), and the validator no longer blocks well-formed steps
      for missing payload/verification — directly covering file-manager search, summarize-visible,
      and copy.
- [x] Steps 1–12 same-path suites green (253 integration passed) + `llm_planner` lib tests green
      (36 passed) → 289 passed, 0 failed; known unrelated lib unit failures excluded.
- [x] `gui_cog_step_completeness` flipped to default ON via `from_env_default_on()`, mirroring
      Task 1's `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
      `gui_cog_reobserve`, and Task 4's `gui_cog_wayland_focus`; env rollback
      (`KRIA_GUI_COG_STEP_COMPLETENESS=0`) preserved + tested.
- [x] KRIA runtime-authority invariants self-checked (post-process operates on validated plan +
      contract, no Prompt→Tool shortcut, single bounded pass, verification truth preserved).
- [ ] **Live numeric affected families ≥ 80%** (file-manager search / summarize-visible / copy, no
      longer blocked) — PENDING a reachable live desktop API (reproduction in §5). No live
      percentages fabricated.

_Last recorded by Task 5.4. The authoritative live affected-families ≥ 80% median is gated on a
running desktop session and is the only outstanding item; all CI-verifiable evidence is green and
the flag is ON._
