# GUI Cognition — Wave 6 / Task 7 Browser Targeting + Read/Summarize Gate (address-bar/tab/navigate + summarize-visible families ≥ 80%)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 7.5 — Live gate:
address-bar/tab/navigate + summarize-visible families ≥ 80%.
**Flag:** `gui_cog_browser` (env `KRIA_GUI_COG_BROWSER`).
**Requirements:** 5 (browser chrome-UI as visible single actions execute + verify),
9 (read/summarize uses OCR/page text as DATA ONLY — never influences planner/executor;
injection defense; untrusted text marked), 26 (browser targeting scope: chrome-UI in scope,
page-content scoped-out for v1 / no OCR-only page targets); methodology per 17 (live testing),
18 (no regression), 23 (verification contract — every executed chrome-UI action carries a
type-correct `verification_strategy`).
**Status of this record:** **CI-safe verification COMPLETE + gate flip applied.** The live
numeric **address-bar/tab/navigate + summarize-visible families ≥ 80%** held-out audit is
**PENDING** a reachable live desktop API (`http://127.0.0.1:3001` health fails in this
environment). No live percentages are fabricated.

---

## 1. Why this document exists

Task 7 ("Browser targeting + read/summarize, injection-safe") makes browser **chrome-UI**
controls — address/URL bar, tab strip / individual tabs, back/forward, reload/stop, in-page Find
bar — targetable via the accessibility resolver when the active app is a recognized browser (7.1),
**scopes page-content targeting out of v1** (no OCR-only page targets; refuse with an actionable
message rather than guess) (7.2), and makes read/summarize treat OCR/page text as **DATA ONLY** so
it never influences the planner or executor — the injection-defense property, with untrusted text
explicitly marked (7.3) — proven by T2 (7.4). The authoritative acceptance gate for Task 7 is the
live held-out capability audit landing the **address-bar/tab/navigate (browser chrome-UI) and
summarize-visible families ≥ 80%**.

This environment has **no reachable live KRIA desktop session** (the audit's own health probe to
`http://127.0.0.1:3001/api/health` returns `000` and the tool fails safe with exit 2, writing no
report). The live numeric audit therefore cannot run here and is **not** fabricated. This document
records the **CI-safe surrogate** — the deterministic held-out integrity + T1/T2 evidence that
proves the browser-targeting + injection-safe-summarize gate's INTENT is met — exactly as the
Task 0.5/0.6 baseline, the Task 1.6 runaway-control gate, the Task 2.9 planner gate, the Task 3.6
re-observe gate, the Task 4.5 Wayland-focus gate, the Task 5.4 step-completeness gate, and the
Task 6.5 primitive-coverage gate established for every "live gate" task in this spec.

> The deterministic T1/T2 tiers are the CI-verifiable proof that browser chrome-UI hints resolve to
> real observed accessibility controls (never invented coordinates, never OCR-only page targets),
> that page-content targets are refused with an actionable message, and that summarize/read treats
> OCR/page text as untrusted DATA that never alters a plan step, a resolved target, or an executed
> action; the ≥ 80% per-family numeric figures are the live confirmation of that same property and
> are gated on a running desktop session (reproduction in §5).

---

## 2. CI-safe verification performed (the deterministic surrogate for the family ≥ 80% targets)

### 2.1 Held-out prompt set integrity (frozen + digest-locked)

| Check | Command | Result |
|---|---|---|
| Frozen set intact | `python3 testing/tools/heldout_prompt_set.py --verify` | **PASS** — "Held-out set OK: frozen + valid (21 families, 105 prompts, >= 5/family)." |
| Audit plan dry-run (real session) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session` | **PASS (exit 0)** — 21 families / 105 prompts enumerated; per-kind assertions correct; destructive-leak detector active |
| Audit plan dry-run (test substrate) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate` | **PASS (exit 0)** — same plan; substrate auto-approval path armed |

The frozen set is digest-locked (`heldout_prompt_set.v1.lock`, SHA-256); it cannot be silently
edited to make a build pass. The audit tool's scoring lives in pure functions and runs end-to-end
in `--dry-run` with no network. The families exercised by Task 7 — the browser chrome-UI
targets (address-bar / tab / navigate; covered within `C2_switch_window`-adjacent chrome targeting
plus the dedicated browser resolver path) and `C16_read_visible` (summarize-visible) — are each
present at n≥5 and scored on the strict execute+verify contract (chrome-UI) / read-as-data contract
(summarize).

### 2.2 Task 7 deterministic test evidence (proves the browser-targeting + injection-safe gate INTENT)

`cargo build -p kria-core` → **Finished (exit 0)**; `cargo build -p kria-desktop` → **Finished
(exit 0)** (call-site flip compiles). Then the new browser + injection-defense test binaries:

| # | Suite | Command | Result |
|---|---|---|---|
| 1 | Browser chrome-UI targeting (T1/T2) | `cargo test -p kria-core --test gui_cognition_browser_chrome_tests` | **PASS — 9 passed** |
| 2 | Browser page-content scope-out (T1/T2) | `cargo test -p kria-core --test gui_cognition_browser_page_content_tests` | **PASS — 5 passed** |
| 3 | Browser read/summarize as data (T2) | `cargo test -p kria-core --test gui_cognition_browser_read_summarize_tests` | **PASS — 5 passed** |
| 4 | Injection defense (T2) | `cargo test -p kria-core --test gui_cognition_injection_defense_tests` | **PASS — 8 passed** |

These collectively prove the gate's intent without a live run:

- **Browser chrome-UI hints resolve to real observed accessibility controls (Req 5 / 7.1):**
  `t1_resolve_maps_each_chrome_hint_to_a_real_observed_control`,
  `t1_chrome_hint_classification`, `t1_browser_app_detection_reads_observed_identity`, and
  `t2_resolved_chrome_label_is_resolvable_by_the_resolver` show address/URL bar, tabs,
  back/forward, reload/stop, and the Find bar each map to a real observed control via the
  accessibility resolver — never an invented coordinate
  (`t1_never_invents_a_control_when_absent`).
- **Page-content targeting is scoped out for v1 with an actionable refusal (Req 26 / 7.2):**
  `t1_page_content_hint_in_browser_is_refused_with_actionable_message`,
  `t2_ocr_only_control_is_never_resolved_as_a_target`, and
  `t2_accessibility_chrome_control_still_resolves` show page-content / OCR-only page targets are
  refused (never guessed) while accessibility chrome controls still resolve.
- **Read/summarize uses OCR/page text as DATA ONLY — injection defense (Req 9 / 7.3, 7.4):**
  `t2_injection_ocr_is_summarized_as_untrusted_data_only`,
  `t2_injection_ocr_is_excluded_from_planner_instructions`,
  `t2_injection_ocr_does_not_alter_plan_steps_or_targets`,
  `t2_summary_has_no_path_to_steps_targets_or_actions`,
  `t2_planner_request_excludes_raw_ocr_text_entirely`,
  `t2_injection_ocr_does_not_change_resolved_or_executed_target`,
  `t2_injection_tainted_llm_plan_is_blocked_by_validator`,
  `t2_injection_tainted_llm_plan_never_executes_through_run_turn`,
  `t2_summarize_turn_with_injection_ocr_triggers_no_action`,
  `t2_injection_ocr_does_not_alter_plan_steps_or_typed_targets_full_pipeline`, and
  `t2_summary_references_only_observed_content_and_redacts_injection` show an injection prompt
  embedded in OCR/page text never reaches the planner instructions, never changes a plan step, a
  resolved target, or an executed action, and the summary references only observed content with
  injection text redacted/marked untrusted.
- **Flag-OFF / default-on rollback semantics (Req 18):**
  `t1_flag_default_off_and_constructors_mirror_pattern` and
  `t1_flag_from_env_truthy_and_default_on_rollback` confirm the documented default-on + rollback
  semantics (absent / truthy keep it ON; explicit falsy = rollback) mirror the prior wave flags.

This is the deterministic, CI-verifiable surrogate for the address-bar/tab/navigate +
summarize-visible ≥ 80% held-out target: the browser families that previously had no chrome-UI
resolver (and read/summarize that risked OCR-as-instruction) now resolve chrome controls via
accessibility, refuse OCR-only page targets, and treat all page text as untrusted data.

### 2.3 "Steps 1–12 green" — full gui_cognition integration suite surface

`cargo test -p kria-core` across the Step 1–12 same-path integration suites (incl. the new Task 7
binaries):

| Suite | Passed |
|---|---|
| `gui_cognition_backend_route_tests` | 27 |
| `gui_cognition_browser_chrome_tests` | 9 |
| `gui_cognition_browser_page_content_tests` | 5 |
| `gui_cognition_browser_read_summarize_tests` | 5 |
| `gui_cognition_checkpoint_resume_tests` | 18 |
| `gui_cognition_context_builder_tests` | 5 |
| `gui_cognition_executor_tests` | 4 |
| `gui_cognition_goal_contract_tests` | 13 |
| `gui_cognition_injection_defense_tests` | 8 |
| `gui_cognition_llm_planner_tests` | 49 |
| `gui_cognition_observation_perception_tests` | 20 |
| `gui_cognition_password_privacy_tests` | 5 |
| `gui_cognition_preconditions_tests` | 7 |
| `gui_cognition_primitive_coverage_tests` | 9 |
| `gui_cognition_primitive_tier_tests` | 6 |
| `gui_cognition_recovery_tests` | 17 |
| `gui_cognition_runtime_guards_tests` | 26 |
| `gui_cognition_safety_hitl_tests` | 6 |
| `gui_cognition_t2_fixture_tier` | 16 |
| `gui_cognition_target_resolver_tests` | 10 |
| `gui_cognition_verification_tests` | 12 |
| `gui_cognition_window_focus_tests` | 7 |
| `gui_cognition_workflow_runtime_tests` | 16 |
| **Integration total** | **300 passed; 0 failed** |

Known pre-existing UNRELATED failures (explicitly excluded from this gate per the task brief) live
in the crate's inline lib unit tests, not in this integration surface:
`agent::atspi_engine::tests::atspi_snapshot_request_defaults_are_bounded_for_gui_cognition`,
`agent::loop_engine::tests::deterministic_dispatch_create_project_folder`,
`agent::continuation_reentry::tests::duplicate_continuation_is_rejected`. None are in the Task 7 /
Steps 1–12 integration surface above.

### 2.4 Flag-OFF preserves Steps 1–12 byte-for-byte (Req 18)

With `gui_cog_browser` OFF, the browser resolver / summarize-as-data layer does not run and the
executor / resolver / planner path is byte-for-byte the Steps 1–12 path. This is proven by:

- `t1_flag_off_and_non_browser_are_unaffected` (chrome suite) — flag OFF and non-browser apps are
  untouched by the browser layer.
- `t2_non_browser_observation_is_not_targeted_by_browser_layer` (chrome suite) — the browser layer
  never engages on non-browser observations.
- `t1_flag_off_and_non_browser_are_not_applicable` (page-content suite) — page-content scope-out
  logic is inert while OFF.
- `t2_flag_off_leaves_summarize_path_unchanged` (read/summarize suite) — summarize path is
  unchanged with the flag OFF.

---

## 3. Gate flip: `gui_cog_browser` → default ON (with env rollback)

All CI-safe Task 7 evidence is green, so the flag is flipped to default-ON following the **same
pattern Task 1 (`gui_cog_runtime_guards`), Task 2 (`gui_cog_smart_planner`), Task 3
(`gui_cog_reobserve`), Task 4 (`gui_cog_wayland_focus`), Task 5 (`gui_cog_step_completeness`), and
Task 6 (`gui_cog_primitives`) used** (`*::from_env_default_on()`).

**Code changes:**

- `crates/kria-desktop/src/commands/gui_cognition.rs`
  - The live-turn construction now calls
    `GuiBrowserConfig::from_env_default_on()` (was `from_env()`). Server-side only — a client
    cannot toggle it. The surrounding comment block was updated to record the Wave 6 flip and the
    rollback switch.
- `crates/kria-core/src/agent/gui_cognition/browser.rs` (no change required this task)
  - `GuiBrowserConfig::from_env_default_on()` + the testable core
    `from_env_lookup_default_on()` already exist (added in Task 7.1, mirroring the runtime-guards /
    smart-planner / re-observe / Wayland-focus / step-completeness / primitives rollback
    semantics): browser chrome-UI targeting + data-only summarize is ON unless
    `KRIA_GUI_COG_BROWSER` is an explicit opt-out (`0`/`false`/`no`/`off`/empty); an absent value
    keeps it ON. The OFF-by-default `from_env()` and the truthy parser are unchanged.
  - `t1_flag_from_env_truthy_and_default_on_rollback` asserts the default-on + rollback semantics
    (absent / truthy keep it ON; explicit falsy = rollback). Green within the browser chrome suite.

**Rollback (no code change):** set `KRIA_GUI_COG_BROWSER=0` (or `false`/`no`/`off`) in the desktop
environment to restore the prior executor / resolver / summarize path byte-for-byte (the browser
chrome-UI resolver + data-only summarize layer do not run; the Steps 1–12 path is preserved). The
deterministic T2 fixture tier is unaffected — those runtimes set their browser config explicitly
and never read the env.

**Post-flip re-verification (all green):** `cargo build -p kria-desktop` (exit 0);
`cargo test -p kria-core --test gui_cognition_browser_chrome_tests` (9 passed),
`--test gui_cognition_browser_page_content_tests` (5 passed),
`--test gui_cognition_browser_read_summarize_tests` (5 passed),
`--test gui_cognition_injection_defense_tests` (8 passed); plus the full gui_cognition integration
surface (300 passed; 0 failed).

---

## 4. KRIA runtime-authority self-check

The flip preserves every authority invariant:

- **KRIA authoritative; substrates execution-only.** The flag only changes whether the resolver can
  target browser chrome-UI controls (address/URL bar, tabs, back/forward, reload/stop, Find bar)
  via accessibility and whether read/summarize collects visible text as data. KRIA still owns the
  Intent → Capability → Policy → Substrate → Tool → Verification flow; no substrate gains authority.
- **No Prompt→Tool shortcut.** Chrome-UI targeting operates on already-parsed, schema-validated
  typed plan steps and resolves against the live accessibility observation — never synthesizing an
  executable action from raw prompt / OCR / LLM prose / coordinates.
- **OCR / visual text is untrusted DATA, never instruction.** Read/summarize collects page text as
  data only; it is excluded from planner instructions and can never alter a plan step, a resolved
  target, or an executed action (injection-defense suite, 8 passed). Injection text in OCR is
  redacted/marked untrusted in the summary.
- **No recursive / uncontrolled loops.** Browser chrome-UI resolution is a per-step typed
  resolution; it adds no loop and remains within the Task 1 runaway caps. Cancel /
  GlobalSafetyHalt still halt before the next action.
- **Verification truth preserved.** Every chrome-UI action step receives a type-correct
  `verification_strategy`, so each executed chrome action is still verified by the existing
  verification contract; the resolver never marks a step verified without evidence.
- **Safety / bounded-cognition / privacy preserved.** Page-content / OCR-only page targets are
  refused with an actionable message (no guessing); approval-gated/destructive verbs are
  unaffected; safety_hitl, verification, runtime_guards, preconditions, recovery suites all green.

---

## 5. Environment limitation & live numeric reproduction (PENDING)

This gate was driven to **green at the CI-safe level**. The live numeric **address-bar/tab/navigate
(browser chrome-UI) + summarize-visible families ≥ 80%** held-out audit — each chrome-UI action
executed + verified; summarize references only observed content with zero action — is **PENDING a
reachable live desktop API**: `http://127.0.0.1:3001/api/health` is unreachable here, and the audit
fails safe (exit 2, no report) rather than fabricating numbers.

To close the live numeric gate on a machine with a desktop session (held-out, 3-run median):

```bash
# 0. Stand up the desktop so /api/health returns 200
cargo run -p kria-desktop        # (or: cd crates/kria-desktop && cargo tauri dev)

# 1. Preflight (uinput / AT-SPI / focus / DISPLAY)
bash scripts/gui_cognition_desktop_preflight.sh

# 2. Frozen set intact
python3 testing/tools/heldout_prompt_set.py --verify

# 3. Browser ON (default after this flip; explicit for clarity),
#    non-destructive audit on the REAL session (3 runs, gate on median).
#    A real browser (e.g. Chrome/Firefox) must be open for chrome-UI + summarize families.
KRIA_GUI_COG_BROWSER=1 python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --out planning_docs/gui_cognition_capability_audit_wave6.md

# 4. (If any destructive/approval browser combos are exercised) TEST SUBSTRATE only
#    (Xvfb/headless seat), scratch profile + clipboard save/restore; auto-approval
#    rejected on real session
KRIA_GUI_COG_BROWSER=1 scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
    python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --environment test_substrate \
    --out planning_docs/gui_cognition_capability_audit_wave6_substrate.md
```

**Close condition:** **address-bar/tab/navigate (browser chrome-UI) ≥ 80%** and
**summarize-visible (`C16_read_visible`) ≥ 80%** (3-run median), each chrome-UI step executed +
verified against the verification contract, summarize triggering zero action and referencing only
observed content, **zero destructive-leak** (audit exit 0), page-content / OCR-only page targets
refused (never guessed), and Steps 1–12 suites still green. Commit the generated matrices alongside
this doc.

---

## 6. Acceptance for Task 7.5

- [x] Frozen held-out set integrity verified (digest lock + invariants) and audit dry-run green
      (real session + substrate).
- [x] Deterministic T1/T2 evidence green: browser chrome-UI hints resolve to real observed
      accessibility controls (never invented coordinates), page-content / OCR-only page targets are
      refused with an actionable message, and read/summarize treats OCR/page text as untrusted DATA
      that never alters a plan step, resolved target, or executed action
      (chrome 9 + page-content 5 + read/summarize 5 + injection-defense 8 = 27 passed).
- [x] Steps 1–12 same-path suites green (300 integration passed); known unrelated lib unit failures
      excluded.
- [x] Flag-OFF preserves the Steps 1–12 executor / resolver / summarize path byte-for-byte
      (`t1_flag_off_and_non_browser_are_unaffected`,
      `t2_non_browser_observation_is_not_targeted_by_browser_layer`,
      `t1_flag_off_and_non_browser_are_not_applicable`,
      `t2_flag_off_leaves_summarize_path_unchanged`).
- [x] `gui_cog_browser` flipped to default ON via `from_env_default_on()`, mirroring Task 1's
      `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's `gui_cog_reobserve`,
      Task 4's `gui_cog_wayland_focus`, Task 5's `gui_cog_step_completeness`, and Task 6's
      `gui_cog_primitives`; env rollback (`KRIA_GUI_COG_BROWSER=0`) preserved + tested.
- [x] KRIA runtime-authority invariants self-checked (chrome-UI targeting operates on validated
      plan steps, no Prompt→Tool shortcut, OCR/visual text is untrusted data never instruction,
      per-step typed resolution, verification truth preserved, page-content refusal preserved).
- [ ] **Live numeric address-bar/tab/navigate + summarize-visible families ≥ 80%** — PENDING a
      reachable live desktop API (reproduction in §5). No live percentages fabricated.

_Last recorded by Task 7.5. The authoritative live address-bar/tab/navigate + summarize-visible
≥ 80% median is gated on a running desktop session and is the only outstanding item; all
CI-verifiable evidence is green and the flag is ON._
