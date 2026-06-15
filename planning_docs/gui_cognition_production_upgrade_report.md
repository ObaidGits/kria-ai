# GUI Cognition Production Upgrade — Final Acceptance Report (Wave 9 / Task 11)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 11 — Production acceptance
(overall ≥ 90%, every family ≥ 80%, 0 BROKEN, 0 destructive-leak).
**Flag state:** ALL ten wave flags default-ON (`from_env_default_on()`); each has a falsy-env
rollback switch.
**Requirements covered:** 1–26 (every requirement in the spec).
**Status of this record:** **CI-safe acceptance COMPLETE across all waves.** The single
authoritative **live numeric held-out audit** (3-run median: overall ≥ 90%, every family ≥ 80%,
0 BROKEN, 0 destructive-leak) is **PENDING a reachable live desktop API**
(`http://127.0.0.1:3001/api/health` returns `000` in this environment; the audit fails safe with
exit 2 and writes no report). **No live percentages are fabricated** — see §8 Reproduction.

---

## 1. Executive summary

The GUI Cognition production upgrade took the selected-mode GUI execution path from a
**~28% overall** pre-upgrade baseline (essentially no multi-step task completed end-to-end; most
primitive families "Plan validation blocked") to a fully-built, fully-flagged-ON production system
across ten waves (Tasks 1–10), each gated behind a named feature flag (default OFF during
development, flipped default-ON only after its wave's CI-safe evidence was green).

This Wave 9 report is the closing acceptance:

- **11.1** — Held-out set integrity verified; audit dry-runs green (real session + substrate);
  destructive-leak detector armed. Live numeric audit PENDING (no live API) with full reproduction.
- **11.2** — `cargo build -p kria-core` + `cargo build -p kria-desktop` clean; all 27 gui_cognition
  integration binaries green (**360 passed**); broad `kria-desktop` suite green (**97 passed**);
  three known-unrelated pre-existing failures explicitly excluded.
- **11.3** — `npm run build` clean; vitest **151 passed (13 files)**; Playwright `e2e-tauri-mock`
  **23 passed**.
- **11.4** — `git diff --check` clean; privacy/no-leak + injection surfaces green.
- **11.5** — All ten wave flags verified default-ON via `from_env_default_on()`; each rollback
  switch documented (§6). No flags needed flipping — all were already ON from their wave gates.
- **11.6** — This report.

The authoritative **live numeric** acceptance audit remains the one outstanding item, gated on a
running desktop session, exactly as every prior wave gate doc recorded. It is not fabricated here.

---

## 2. Before / after capability matrix (21 held-out families)

Baseline column = pre-upgrade qualitative state from
`planning_docs/gui_cognition_capability_baseline.md` (~28% overall). Target column = the wave gate
that owns each family. **CI-safe evidence** = the deterministic T1/T2 surrogate proven green for
that family's wave (the live numeric % is PENDING a live desktop, §8).

| Cap | Family | Baseline (~28%) | Target | Owning wave (flag) | CI-safe evidence (green) |
|---|---|---|---|---|---|
| C1 | Open app | PARTIAL (name-leak fixed → ~6/25 verified opens) | ≥ 80% | W2/W5 (`smart_planner`,`primitives`) | planner 49 + primitive_coverage 9 |
| C2 | Switch window | BROKEN ("Plan validation blocked"/no Wayland focus) | ≥ 80% | W3 (`wayland_focus`) | window_focus 7 + backend_route 37 |
| C3 | Focus control | BROKEN (stale observation; no field binding) | ≥ 80% | W3/W5 (`reobserve`,`primitives`) | primitive_coverage 9 + target_resolver 10 |
| C4 | Type text | BROKEN ("no safe text/query payload") | ≥ 80% | W4 (`step_completeness`) | llm_planner 49 (task_5 payload/validator) |
| C5 | Clear / select text | BROKEN (no plan path) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 + primitive_tier 6 |
| C6 | Copy / paste | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 |
| C7 | Key press / shortcut | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 |
| C8 | Scroll | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 |
| C9 | Click button | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 + executor 4 |
| C10 | Checkbox / toggle | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 + primitive_tier 6 |
| C11 | Dialog handling | BROKEN (not reached) | ≥ 80% | W5 (`primitives`) | primitive_coverage 9 |
| C12 | In-app search | BROKEN (browser mis-route + payload block) | ≥ 80% | W4/W5 (`step_completeness`,`primitives`) | llm_planner 49 + primitive_coverage 9 |
| C13 | Multi-step combo | BROKEN (first step runs, re-observe gap) | ≥ 80% | W3 (`reobserve`) | workflow_runtime 24 |
| C14 | Cross-app clipboard | BROKEN (no clipboard helper) | ≥ 80% | W6 (`crossapp`) | crossapp_integration 14 + clipboard lib 13 |
| C15 | File-manager select | BROKEN (planned as URL nav) | ≥ 80% | W6 (`crossapp`) | crossapp_integration 14 |
| C16 | Read / summarize visible | BROKEN (no read/summarize path) | ≥ 80% | W6 (`browser`) | browser_read_summarize 5 + injection_defense 8 |
| C17 | Approval-gated action | healthy gate; no auto-execute | ≥ 80% (substrate) | W7 (`safety_polish`) | safety_hitl 6 + audit_ledger 5 |
| C18 | Ambiguity → ask | PARTIAL (collapsed to generic clarify) | ≥ 80% | W7 (`safety_polish`) | verification 12 + workflow_runtime 24 |
| C19 | Boundaries (no change) | held (no destructive-leak) | ≥ 80% | W7 (`safety_polish`) | recovery 26 + safety_hitl 6 |
| C20 | Verify-and-stop | BROKEN (verification rarely `verified`) | ≥ 80% | W7 (`safety_polish`) | verification_contract 4 + verification 12 |
| C21 | Recovery / re-focus | BROKEN (no re-observe/recovery loop) | ≥ 80% | W7 (`safety_polish`) | recovery 26 |

**Overall:** baseline **~28%** → target **≥ 90%** (3-run median), every family **≥ 80%**, **0
BROKEN**, **0 destructive-leak**. The CI-safe deterministic surrogate for every family is green;
the live numeric confirmation is PENDING a live desktop session (§8).

---

## 3. Per-family ledger / evidence summary (by wave)

Each wave drove its families to green at the CI-safe level and flipped its flag default-ON. The
authoritative live numeric % for each is PENDING (§8). Cited gate docs hold the full evidence.

| Wave | Task | Families raised | CI-safe evidence | Gate doc |
|---|---|---|---|---|
| 1 | Runaway/abort/NFR/preconditions | (foundation for all) | runtime_guards 26 + preconditions 7 | (flag default-ON; see §6) |
| 2 | Planner intelligence (28→~55–60%) | C1–C16 reach `valid_for_resolution` | llm_planner 49 + goal_contract 19 + t2_fixture 16 | `gui_cognition_wave2_planner_gate.md` |
| 3 | Per-step re-observe (→ ≥70%; combo ≥80%) | C13 multistep | workflow_runtime 24 | `gui_cognition_wave3_reobserve_gate.md` |
| 3 | Wayland-safe focus (switch ≥80%) | C2 switch-window | window_focus 7 + backend_route 37 | `gui_cognition_wave3_wayland_focus_gate.md` |
| 4 | Plan-step completeness (≥80%) | C4 type, C12 search (payload/verify) | llm_planner task_5 payload + validator | `gui_cognition_wave4_step_completeness_gate.md` |
| 5 | Primitive coverage (every primitive ≥80%; overall ≥80%) | C3,C5–C12 | primitive_coverage 9 + primitive_tier 6 + password_privacy 5 | `gui_cognition_wave5_primitive_coverage_gate.md` |
| 6 | Browser targeting + read/summarize (≥80%) | C16 read-visible, browser chrome | browser_chrome 9 + browser_page_content 5 + browser_read_summarize 5 + injection_defense 8 | `gui_cognition_wave6_task7_browser_gate.md` |
| 6 | Cross-app clipboard + fm-select (≥80%) | C14 cross-app, C15 fm-select | crossapp_integration 14 + clipboard lib 13 | `gui_cognition_wave6_task8_crossapp_gate.md` |
| 7 | Approval/ambiguity/boundary/verify/recovery + contract + ledger (all ≥80%; 0 leak) | C17–C21 | verification_contract 4 + verification 12 + audit_ledger 5 + safety_hitl 6 + recovery 26 | `gui_cognition_wave7_task9_safety_polish_gate.md` |
| 8 | Frontend prod UX + E2E | (UI delivery for all families) | vitest 151 + Playwright 23 | `gui_cognition_wave8_task10_stream_ux_gate.md` |

---

## 4. Aggregated CI-safe verification results (this Wave 9 run)

All commands run in this environment; results captured fresh for this acceptance.

### 4.1 Held-out integrity + audit dry-run (11.1)

| Check | Command | Result |
|---|---|---|
| Frozen set intact | `python3 testing/tools/heldout_prompt_set.py --verify` | **PASS** — "Held-out set OK: frozen + valid (21 families, 105 prompts, >= 5/family)." |
| Audit dry-run (real session) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session` | **PASS (exit 0)** — 21 families / 105 prompts; per-kind asserts correct; destructive-leak detector active |
| Audit dry-run (test substrate) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate` | **PASS (exit 0)** — same plan; substrate auto-approval path armed |
| Live health probe | `curl http://127.0.0.1:3001/api/health` | **000 (unreachable)** → live numeric audit PENDING (§8) |

### 4.2 Rust builds + suites (11.2)

| Check | Command | Result |
|---|---|---|
| Core build | `cargo build -p kria-core` | **Finished (exit 0)** |
| Desktop build | `cargo build -p kria-desktop` | **Finished (exit 0)** |
| gui_cognition lib (name-matched) | `cargo test -p kria-core gui_cognition` (lib) | **201 passed** (excl. 1 known-unrelated, §5) |
| gui_cognition integration binaries (27) | per-`--test` (see below) | **360 passed; 0 failed** |
| Broad desktop suite | `cargo test -p kria-desktop` | **97 passed; 0 failed** |

gui_cognition integration binary breakdown (360 total):
audit_ledger 5 · backend_route 37 · browser_chrome 9 · browser_page_content 5 ·
browser_read_summarize 5 · checkpoint_resume 18 · context_builder 5 · crossapp_integration 14 ·
event_streaming 4 · executor 4 · goal_contract 19 · injection_defense 8 · llm_planner 49 ·
observation_perception 20 · password_privacy 5 · preconditions 7 · primitive_coverage 9 ·
primitive_tier 6 · recovery 26 · runtime_guards 26 · safety_hitl 6 · t2_fixture_tier 16 ·
target_resolver 10 · verification_contract 4 · verification 12 · window_focus 7 ·
workflow_runtime 24.

### 4.3 UI build + unit + E2E (11.3)

| Check | Command | Result |
|---|---|---|
| UI build | `cd ui && npm run build` | **exit 0** (vite, 89 modules; benign dynamic-import advisory only) |
| UI unit (vitest single-run) | `cd ui && npm run test:run` | **13 files / 151 tests passed** |
| Playwright E2E | `KRIA_UI_URL=http://127.0.0.1:1420 npx playwright test --project=e2e-tauri-mock --reporter=list` | **23 passed (7.0s)** |

### 4.4 Hygiene + privacy/injection (11.4)

| Check | Command | Result |
|---|---|---|
| Whitespace/conflict-marker | `git diff --check` | **clean (exit 0)** |
| Password privacy (focus never echoes value) | `cargo test -p kria-core --test gui_cognition_password_privacy_tests` | **5 passed** |
| Browser/OCR injection defense (data-only; plan unaffected) | `cargo test -p kria-core --test gui_cognition_injection_defense_tests` | **8 passed** |
| Audit ledger (append-only, sanitized, no secrets) | `cargo test -p kria-core --test gui_cognition_audit_ledger_tests` | **5 passed** |
| Verification truth (no false `verified`) | `cargo test -p kria-core --test gui_cognition_verification_tests` | **12 passed** |
| UI layman privacy scrub (no hashes/IDs/secrets in layman layer) | `ui` vitest `src/lib/guiCognitionSummary.test.ts` | **17 passed** |

---

## 5. Known-unrelated pre-existing failures (explicitly excluded)

Per the task brief these inline lib unit tests are pre-existing and unrelated to GUI Cognition
production work; they are excluded from this gate. Confirmed still present this run (full
`cargo test -p kria-core --lib`: **2373 passed**, the excluded failures below):

1. `agent::atspi_engine::tests::atspi_snapshot_request_defaults_are_bounded_for_gui_cognition`
   — asserts a narrower default a11y role set (left/right list mismatch); unrelated to the
   production upgrade surface.
2. `agent::loop_engine::tests::deterministic_dispatch_create_project_folder` — loop-engine
   deterministic dispatch fixture; unrelated.
3. `agent::continuation_reentry::tests::duplicate_continuation_is_rejected` — continuation reentry
   fixture; unrelated.

Additionally, `agent::gui_cognition::clipboard::tests::t2_second_session_waits_for_first_to_release`
may flake under parallel execution; it **passes when re-run isolated** (confirmed green this run
inside the gui_cognition lib name-matched run and the clipboard lib suite). It is a timing-sensitive
serialization test, not a product defect.

None of the above are in the 27 gui_cognition integration binaries (360 passed) or the broad
desktop suite (97 passed).

---

## 6. Feature-flag table (flag → env → default-ON → rollback)

All ten wave flags are constructed in `crates/kria-desktop/src/commands/gui_cognition.rs` via
`*::from_env_default_on()` for the live/desktop turn builder (server-side only — a client cannot
toggle them). Each is **ON unless its env var is an explicit opt-out** (`0`/`false`/`no`/`off`/
empty); an absent value keeps it ON. Verified default-ON this run; **no flips were required** (each
was flipped at its own wave gate).

| # | Flag | Env var | Construction (default-ON) | Rollback switch (falsy env → OFF) |
|---|---|---|---|---|
| 1 | `gui_cog_runtime_guards` | `KRIA_GUI_COG_RUNTIME_GUARDS` | `GuiRuntimeGuardConfig::from_env_default_on()` | `KRIA_GUI_COG_RUNTIME_GUARDS=0` (or `false`/`no`/`off`) → prior Step 1–12 behavior |
| 2 | `gui_cog_smart_planner` | `KRIA_GUI_COG_SMART_PLANNER` | `GuiSmartPlannerConfig::from_env_default_on()` | `KRIA_GUI_COG_SMART_PLANNER=0` → single-attempt planner |
| 3 | `gui_cog_reobserve` | `KRIA_GUI_COG_REOBSERVE` | `GuiReobserveConfig::from_env_default_on()` | `KRIA_GUI_COG_REOBSERVE=0` → prior re-observe behavior |
| 4 | `gui_cog_wayland_focus` | `KRIA_GUI_COG_WAYLAND_FOCUS` | `GuiWaylandFocusConfig::from_env_default_on()` | `KRIA_GUI_COG_WAYLAND_FOCUS=0` → prior SwitchWindow path |
| 5 | `gui_cog_step_completeness` | `KRIA_GUI_COG_STEP_COMPLETENESS` | `GuiStepCompletenessConfig::from_env_default_on()` | `KRIA_GUI_COG_STEP_COMPLETENESS=0` → plan-preserving (no post-process) |
| 6 | `gui_cog_primitives` | `KRIA_GUI_COG_PRIMITIVES` | `GuiPrimitivesConfig::from_env_default_on()` | `KRIA_GUI_COG_PRIMITIVES=0` → legacy executor mapping byte-for-byte |
| 7 | `gui_cog_browser` | `KRIA_GUI_COG_BROWSER` | `GuiBrowserConfig::from_env_default_on()` | `KRIA_GUI_COG_BROWSER=0` → prior executor/resolver path |
| 8 | `gui_cog_crossapp` | `KRIA_GUI_COG_CROSSAPP` | `GuiCrossAppConfig::from_env_default_on()` | `KRIA_GUI_COG_CROSSAPP=0` → prior path; clipboard never borrowed |
| 9 | `gui_cog_safety_polish` | `KRIA_GUI_COG_SAFETY_POLISH` | `GuiSafetyPolishConfig::from_env_default_on()` | `KRIA_GUI_COG_SAFETY_POLISH=0` → prior verification verdict byte-for-byte |
| 10 | `gui_cog_stream_ux` | `KRIA_GUI_COG_STREAM_UX` | `GuiStreamUxConfig::from_env_default_on()` (streaming also requires an `event_emitter`) | `KRIA_GUI_COG_STREAM_UX=0` → end-of-turn batch behavior |

The deterministic T2 fixture tier is unaffected by any rollback — those runtimes construct their
config explicitly and never read the env. The env-flag name constants are pinned by
`t1_env_flag_const_is_stable` tests in each module.

---

## 7. KRIA runtime-authority invariants self-check

All ten flags are ON and every authority invariant is preserved:

- **KRIA authoritative; substrates execution-only.** Every flag changes only how the runtime plans,
  re-observes, focuses, verifies, or reports — never *who* decides. KRIA owns the full
  **Intent → Capability → Policy → Substrate → Tool → Verification** flow; no substrate gains
  authority. KRIA remains a bounded-execution intelligence / capability-first orchestrator, **not**
  an AGI / tool-mesh / tool-first agent.
- **No Prompt→Tool shortcut.** The smart planner emits a strict schema-validated typed plan
  (lenient prose-scrape rejected); targets resolve against the live accessibility observation;
  read/summarize + OCR text are treated as **data only** and never influence planner/executor
  (injection_defense 8). No action is ever synthesized from raw prompt / OCR / LLM prose /
  coordinates.
- **No recursive / uncontrolled loops.** `TurnBudget` (max_steps, watchdog, per-step
  resolve/verify timeouts, max_reobserve) bounds every turn; recovery retries exactly **once** and
  only for idempotent actions; the smart planner does at most **one** repair-retry. Per-step
  re-observe and cross-app combos add no unbounded loop (runtime_guards 26, recovery 26,
  workflow_runtime 24).
- **Verifier-aware execution / verification truth.** Every executed step carries a type-correct
  `verification_strategy`; `ActionCompleted` (backend success) is never silently upgraded to
  `verified`; low-confidence / unreliable evidence yields honest `inconclusive`
  (verification_contract 4, verification 12).
- **Safety / confirmation gating.** Approval-gated actions pause and execute only on an approved +
  matching HITL decision (never deny/expired/mismatch); auto-approve is rejected outside the test
  substrate; ambiguity asks; boundaries block every destructive/state-changing action; the
  destructive-leak detector stays armed (safety_hitl 6, audit_ledger 5).
- **Cancellation propagation.** The visible Stop/Cancel control wires the Task 1 `CancelToken`
  (`cancel_gui_cognition_turn`); cancel and GlobalSafetyHalt stop before the next action
  (Playwright Stop test + runtime_guards).
- **Deterministic orchestration + privacy.** Streaming is passive additive telemetry (identical
  event content, delivery timing only); the append-only sanitized ledger records every executed
  action with no secrets / raw payloads; the layman UI layer is hard-scrubbed of hashes / IDs /
  coordinates / secrets (guiCognitionSummary 17, password_privacy 5).

---

## 8. PENDING — live numeric acceptance audit (§Reproduction)

The **only outstanding acceptance item** is the authoritative **live held-out capability audit**:
3-run median **overall ≥ 90%**, **every family ≥ 80%**, **0 BROKEN**, **0 destructive-leak**. It is
**PENDING a reachable live desktop API**. In this environment
`http://127.0.0.1:3001/api/health` returns `000` (unreachable); the audit **fails safe** (exit 2,
writes no report) rather than fabricating numbers. This mirrors how every prior wave gate
(`gui_cognition_wave2..wave8_*_gate.md`) handled the live audit.

### §Reproduction (on a machine with a live KRIA desktop session)

```bash
# 0. Stand up the desktop so /api/health returns 200
cargo run -p kria-desktop        # (or: cd crates/kria-desktop && cargo tauri dev)

# 1. Preflight (uinput / AT-SPI / focus / DISPLAY)
bash scripts/gui_cognition_desktop_preflight.sh

# 2. Frozen set intact (digest-locked)
python3 testing/tools/heldout_prompt_set.py --verify

# 3. NON-DESTRUCTIVE families on the REAL session (3 runs, gate on median).
#    All ten flags default ON; have a browser + text editor + file manager open
#    for the cross-app / fm-select / read-visible families.
python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --out planning_docs/gui_cognition_capability_audit_wave9_final.md

# 4. APPROVAL / DESTRUCTIVE families (C17 approval, C21 recovery, any destructive
#    verbs) ONLY in the TEST SUBSTRATE (Xvfb/headless seat), scratch profile +
#    clipboard save/restore; auto-approval fixtures are REJECTED on the real session.
scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
    python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --environment test_substrate \
    --out planning_docs/gui_cognition_capability_audit_wave9_final_substrate.md
```

**Close condition:** overall median **≥ 90%**, **every one of the 21 families ≥ 80%**, **0 BROKEN**
(no family < 40%), **0 destructive-leak** (audit exit 0 — no unrequested
delete/move/rename/submit/install/setting-change), and all CI-safe suites still green. Commit the
generated matrices alongside this report.

### CI-safe re-run (no live desktop needed — re-runnable here)

```bash
python3 testing/tools/heldout_prompt_set.py --verify
python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session
python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate
cargo build -p kria-core && cargo build -p kria-desktop
cargo test -p kria-desktop                       # 97 passed
cd ui && npm run build && npm run test:run        # build clean; 151 passed
cd ui && npm run dev -- --host 127.0.0.1 --port 1420   # background
cd testing/suites/playwright && \
  KRIA_UI_URL=http://127.0.0.1:1420 npx playwright test --project=e2e-tauri-mock --reporter=list  # 23 passed
git diff --check                                  # clean
```

---

## 9. Acceptance for Task 11

- [x] **11.1** Held-out frozen-set integrity verified; audit dry-runs green (real session +
      substrate); destructive-leak detector armed. Live numeric audit (overall ≥ 90% / family ≥ 80%
      / 0 BROKEN / 0 destructive-leak) documented PENDING with §Reproduction (no live API; no
      numbers fabricated).
- [x] **11.2** `cargo build -p kria-core` + `cargo build -p kria-desktop` clean; all gui_cognition
      core suites green (lib 201 + 27 integration binaries = 360 passed); broad `kria-desktop`
      suite green (97 passed); three known-unrelated failures excluded (§5).
- [x] **11.3** `npm run build` clean; vitest 151 passed (13 files); Playwright `e2e-tauri-mock`
      23 passed.
- [x] **11.4** `git diff --check` clean; privacy/no-leak (password_privacy 5, audit_ledger 5,
      verification 12, UI guiCognitionSummary 17) + injection (injection_defense 8) green.
- [x] **11.5** All ten wave flags verified default-ON via `from_env_default_on()`; rollback paths
      documented (§6); no flips required.
- [x] **11.6** This report written with before/after capability matrix, per-family ledger/evidence
      summary, full feature-flag table, aggregated CI-safe results, KRIA runtime-authority
      self-check, and the PENDING live-audit §Reproduction.
- [ ] **Live numeric acceptance audit** (overall ≥ 90% / every family ≥ 80% / 0 BROKEN / 0
      destructive-leak, 3-run median) — PENDING a reachable live desktop API (§8). No live
      percentages fabricated.

_Last recorded by Task 11. All CI-verifiable acceptance evidence is green across every wave and all
ten flags are ON with documented rollback. The single authoritative live numeric audit is gated on
a running desktop session and is the only outstanding item — exactly as recorded in every prior
wave gate doc. This closes Wave 9._
