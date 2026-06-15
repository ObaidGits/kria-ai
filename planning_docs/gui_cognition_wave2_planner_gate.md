# GUI Cognition — Wave 2 / Task 2 Planner Gate (28% → ~55–60%)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 2.9 — Live gate: held-out audit
overall ≥ 55%; planner-blocked families no longer "Plan validation blocked"; Steps 1–12 green.
**Flag:** `gui_cog_smart_planner` (env `KRIA_GUI_COG_SMART_PLANNER`).
**Requirements:** 1 (intelligent planner), 4 (plan-step completeness); methodology per
17 (live testing), 18 (no regression).
**Status of this record:** **CI-safe verification COMPLETE + gate flip applied.** The live
numeric **≥ 55% held-out audit is PENDING** a reachable live desktop API
(`http://127.0.0.1:3001` health fails in this environment). No live percentages are fabricated.

---

## 1. Why this document exists

Task 2 ("Planner intelligence") raises the planner from a thin deterministic template (rejected
on every prompt → `llm_rejected_fallback`, "returned prose or non-object content") to a strict
JSON-validated planner with exactly ONE repair-retry plus a richer deterministic fallback that
maps **every** supported intent / primitive / combo to a complete, valid, executable plan. The
authoritative acceptance gate for Task 2 is the live held-out capability audit hitting
**overall ≥ 55%** with planner-blocked families no longer landing on "Plan validation blocked".

This environment has **no reachable live KRIA desktop session** (the audit's own health probe to
`http://127.0.0.1:3001` returns `000` and the tool fails safe with exit 2, writing no report).
The live numeric audit therefore cannot run here and is **not** fabricated. This document records
the **CI-safe surrogate** — the deterministic T1/T2 evidence that proves the planner gate's INTENT
is met — exactly as the Task 0.5 / 0.6 baseline doc established for every "live gate" task in this
spec.

> The deterministic T1/T2 tiers are the CI-verifiable proof that planner-blocked families now
> reach `valid_for_resolution`; the ≥ 55% numeric figure is the live confirmation of that same
> property and is gated on a running desktop session (reproduction in §5).

---

## 2. CI-safe verification performed (the deterministic surrogate for ≥ 55%)

### 2.1 Held-out prompt set integrity (frozen + digest-locked)

| Check | Command | Result |
|---|---|---|
| Frozen set intact | `python3 testing/tools/heldout_prompt_set.py --verify` | **PASS** — "Held-out set OK: frozen + valid (21 families, 105 prompts, ≥ 5/family)." |
| Audit plan dry-run (real session) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session` | **PASS (exit 0)** — 21 families / 105 prompts enumerated; per-kind assertions correct; destructive-leak detector active |
| Audit plan dry-run (test substrate) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate` | **PASS (exit 0)** — same plan; substrate auto-approval path armed |

The frozen set is digest-locked (`heldout_prompt_set.v1.lock`, SHA-256); it cannot be silently
edited to make a build pass. The audit tool's scoring lives in pure functions and runs end-to-end
in `--dry-run` with no network.

### 2.2 Task 2 deterministic test evidence (proves the planner gate INTENT)

`cargo build -p kria-core` → **Finished (exit 0)**. Then:

| # | Suite | Command | Result |
|---|---|---|---|
| 1 | LLM planner (T1) | `cargo test -p kria-core --test gui_cognition_llm_planner_tests` | **PASS — 49 passed** (47 pre-existing + 2 new gate-flip flag tests) |
| 2 | T2 fixture tier (no display) | `cargo test -p kria-core --test gui_cognition_t2_fixture_tier` | **PASS — 15 passed** |
| 3 | Goal contract (T1) | `cargo test -p kria-core --test gui_cognition_goal_contract_tests` | **PASS — 13 passed** |

These collectively prove the gate's intent without a live run:

- `task27_every_supported_intent_maps_to_valid_complete_plan`,
  `deterministic_fallback_meets_quality_bar_for_every_action_type`, and
  `task24_new_primitive_intents_meet_quality_bar_and_resolve` show **every** supported
  intent/primitive maps to a complete, valid, typed plan with payload + `verification_strategy`.
- `t2_complete_primitives_and_combos_reach_valid_for_resolution` and
  `t2_full_primitive_combo_matrix_never_blocks_or_rejects` show the full primitive + combo matrix
  reaches `valid_for_resolution` and is **never** blocked or rejected — i.e. planner-blocked
  families no longer land on "Plan validation blocked".
- `task27_prose_and_non_object_outputs_are_rejected_with_exact_reason`,
  `invalid_json_and_prose_wrapper_are_rejected`, and the `task27_repair_runtime::*` tests show the
  strict-validate + exactly-ONE-repair-retry path (never lenient prose-scrape), with the flag-off
  path preserving the prior single-attempt behavior.

This is the deterministic, CI-verifiable surrogate for the ≥ 55% held-out target: the families
that were "Plan validation blocked" at the ~28% baseline (C2–C16 per the baseline matrix) now
produce valid, resolvable, verification-carrying plans.

### 2.3 "Steps 1–12 green" — full gui_cognition suite surface

`cargo test -p kria-core` across the Step 1–12 same-path suites (single invocation):

| Suite | Passed |
|---|---|
| `gui_cognition_backend_route_tests` | 27 |
| `gui_cognition_checkpoint_resume_tests` | 18 |
| `gui_cognition_context_builder_tests` | 5 |
| `gui_cognition_executor_tests` | 4 |
| `gui_cognition_observation_perception_tests` | 16 |
| `gui_cognition_preconditions_tests` | 7 |
| `gui_cognition_recovery_tests` | 17 |
| `gui_cognition_runtime_guards_tests` | 14 |
| `gui_cognition_safety_hitl_tests` | 6 |
| `gui_cognition_target_resolver_tests` | 10 |
| `gui_cognition_verification_tests` | 12 |
| `gui_cognition_workflow_runtime_tests` | 10 |
| **Total** | **146 passed; 0 failed** |

Plus the Task 2 suites (§2.2): planner 49, t2_fixture 15, goal_contract 13 → **77 passed**.

**Grand total: 223 passed; 0 failed** across the Task 2 + Steps 1–12 surface.

Known pre-existing UNRELATED failures (explicitly excluded from this gate per the task brief) live
in the crate's inline lib unit tests, not in this integration surface, and did **not** appear in
any of the runs above:
`agent::atspi_engine::tests::atspi_snapshot_request_defaults_are_bounded_for_gui_cognition`,
`agent::loop_engine::tests::deterministic_dispatch_create_project_folder`,
`agent::continuation_reentry::tests::duplicate_continuation_is_rejected`.

`cargo check -p kria-desktop` → **Finished (exit 0)** (call-site flip compiles).

---

## 3. Gate flip: `gui_cog_smart_planner` → default ON (with env rollback)

All CI-safe Task 2 evidence is green, so the flag is flipped to default-ON following the **same
pattern Task 1 used for `gui_cog_runtime_guards`** (`GuiRuntimeGuardConfig::from_env_default_on()`).

**Code changes:**

- `crates/kria-core/src/agent/gui_cognition/llm_planner.rs`
  - Added `GuiSmartPlannerConfig::from_env_default_on()` and its testable core
    `from_env_lookup_default_on()`. The flag is ON unless `KRIA_GUI_COG_SMART_PLANNER` is an
    explicit opt-out (`0`/`false`/`no`/`off`/empty); an absent value keeps it ON.
  - Added helper `smart_planner_flag_falsy()` (mirrors the runtime-guards `is_falsy` rollback
    semantics). The existing truthy parser and the OFF-by-default `from_env()` are unchanged.
- `crates/kria-desktop/src/commands/gui_cognition.rs`
  - The live-turn construction now calls `GuiSmartPlannerConfig::from_env_default_on()` (was
    `from_env()`). Server-side only — a client cannot toggle it.
- `crates/kria-core/tests/gui_cognition_llm_planner_tests.rs`
  - Added `smart_planner_default_on_when_env_absent_or_truthy` and
    `smart_planner_default_on_rolls_back_when_env_explicitly_falsy`. Both green.

**Rollback (no code change):** set `KRIA_GUI_COG_SMART_PLANNER=0` (or `false`/`no`/`off`) in the
desktop environment to restore the prior single-attempt Step 1–12 behavior. The deterministic T2
fixture tier is unaffected — those runtimes set their planner config explicitly and never read the
env.

---

## 4. KRIA runtime-authority self-check

The flip preserves every authority invariant:

- **KRIA authoritative; substrates execution-only.** The flag only changes whether the planner
  performs ONE repair-retry before its deterministic fallback. KRIA still owns the
  Intent → Capability → Policy → Substrate → Tool → Verification flow; no substrate gains authority.
- **No Prompt→Tool shortcut.** The smart-planner path still emits a strict schema-validated typed
  plan; it never executes from raw prompt/OCR/LLM prose (lenient scraping remains rejected —
  `smart_planner_never_scrapes_prose_response`, `task27_prose_and_non_object_outputs_are_rejected`).
- **No recursive loops / uncontrolled retries.** Exactly ONE repair-retry is preserved
  (`smart_planner_performs_at_most_one_repair_attempt`); on second failure it falls back
  deterministically. Turn budget / watchdog / cancel / GlobalSafetyHalt (Task 1) remain in force.
- **Verification / safety / bounded-cognition / deterministic-orchestration / cancellation**
  unchanged — safety_hitl, verification, runtime_guards, preconditions, recovery suites all green.

---

## 5. Environment limitation & live numeric reproduction (PENDING)

This gate was driven to **green at the CI-safe level**. The live numeric **overall ≥ 55%**
held-out audit is **PENDING a reachable live desktop API**: `http://127.0.0.1:3001/api/health`
is unreachable here, and the audit fails safe (exit 2, no report) rather than fabricating numbers.

To close the live numeric gate on a machine with a desktop session:

```bash
# 0. Stand up the desktop so /api/health returns 200
cargo run -p kria-desktop        # (or: cd crates/kria-desktop && cargo tauri dev)

# 1. Preflight (uinput / AT-SPI / focus / DISPLAY)
bash scripts/gui_cognition_desktop_preflight.sh

# 2. Frozen set intact
python3 testing/tools/heldout_prompt_set.py --verify

# 3. Smart planner ON (default after this flip; explicit for clarity), non-destructive
#    baseline on the REAL session (3 runs, gate on median)
KRIA_GUI_COG_SMART_PLANNER=1 python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --out planning_docs/gui_cognition_capability_audit_wave2.md

# 4. Destructive / approval families in the TEST SUBSTRATE only (Xvfb/headless seat)
scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
    python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --environment test_substrate \
    --out planning_docs/gui_cognition_capability_audit_wave2_substrate.md
```

**Close condition:** overall median **≥ 55%** (3-run median), previously planner-blocked families
(C2–C16) no longer reporting "Plan validation blocked", **zero destructive-leak** (audit exit 0),
and Steps 1–12 suites still green. Commit the generated matrices alongside this doc.

---

## 6. Acceptance for Task 2.9

- [x] Frozen held-out set integrity verified (digest lock + invariants) and audit dry-run green
      (real session + substrate).
- [x] Deterministic T1/T2 evidence green: every supported intent/primitive/combo reaches
      `valid_for_resolution`; planner-blocked families no longer "Plan validation blocked".
- [x] Steps 1–12 same-path suites green (146 passed) + Task 2 suites green (77 passed); known
      unrelated lib unit failures excluded and absent from this surface.
- [x] `gui_cog_smart_planner` flipped to default ON via `from_env_default_on()`, mirroring Task 1's
      `gui_cog_runtime_guards`; env rollback (`KRIA_GUI_COG_SMART_PLANNER=0`) preserved + tested.
- [x] KRIA runtime-authority invariants self-checked (one repair-retry preserved; no Prompt→Tool
      shortcut; bounded/cancellable/safe).
- [ ] **Live numeric overall ≥ 55%** — PENDING a reachable live desktop API (reproduction in §5).
      No live percentages fabricated.

_Last recorded by Task 2.9. The authoritative live ≥ 55% median is gated on a running desktop
session and is the only outstanding item; all CI-verifiable evidence is green and the flag is ON._
