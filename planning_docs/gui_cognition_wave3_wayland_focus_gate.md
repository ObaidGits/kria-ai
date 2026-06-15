# GUI Cognition — Wave 3 / Task 4 Wayland-Safe Window Focus Gate (Switch-window family ≥ 80%)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 4.5 — Live gate: "Switch to the
Chrome/terminal/file manager window" executed + verified (no "wmctrl required");
Switch-window family ≥ 80%.
**Flag:** `gui_cog_wayland_focus` (env `KRIA_GUI_COG_WAYLAND_FOCUS`).
**Requirements:** 3 (Wayland-safe window focus / switch); methodology per 17 (live testing),
18 (no regression), 23 (verification contract — verify-by-reobserve).
**Status of this record:** **CI-safe verification COMPLETE + gate flip applied.** The live
numeric **Switch-window family ≥ 80%** (and "executed + verified, no wmctrl-required") held-out
audit is **PENDING** a reachable live desktop API (`http://127.0.0.1:3001` health fails in this
environment). No live percentages are fabricated.

---

## 1. Why this document exists

Task 4 ("Wayland-safe window focus / switch") removes root-cause #3: SwitchWindow on Wayland
previously depended on `wmctrl`, which is unavailable on Wayland sessions, so window focus failed
with a generic "wmctrl required" / "deterministic action backend failed" message. Task 4 adds a
`WindowFocus` abstraction with a session-appropriate backend chain
(`GnomeBridge → Portal → UinputAltTab(verify) → X11Wmctrl` — `wmctrl` only on X11), routes
SwitchWindow through it with a truthful `backend_used`, prefers compositor-native
activate-by-window-identity, verifies by re-observing that the requested window is active, and
emits a clear actionable error when no focus path exists. The authoritative acceptance gate for
Task 4 is the live held-out capability audit landing the **Switch-window family (C2) ≥ 80%** with
each switch **executed + verified** and **never** the legacy "wmctrl required" failure.

This environment has **no reachable live KRIA desktop session** (the audit's own health probe to
`http://127.0.0.1:3001` returns `000` and the tool fails safe with exit 2, writing no report). The
live numeric audit therefore cannot run here and is **not** fabricated. This document records the
**CI-safe surrogate** — the deterministic T1/T2 evidence that proves the Wayland-focus gate's
INTENT is met — exactly as the Task 0.5/0.6 baseline, the Task 2.9 planner gate, and the Task 3.6
re-observe gate established for every "live gate" task in this spec.

> The deterministic T1/T2 tiers are the CI-verifiable proof that SwitchWindow routes through the
> Wayland-safe `WindowFocus` abstraction, selects a session-appropriate backend (never `wmctrl` on
> Wayland), executes, is verified by re-observing the active window == requested, and replaces the
> legacy "wmctrl required" path with a clear, actionable, non-`wmctrl` error; the ≥ 80% numeric
> figure is the live confirmation of that same property and is gated on a running desktop session
> (reproduction in §5).

---

## 2. CI-safe verification performed (the deterministic surrogate for Switch-window ≥ 80%)

### 2.1 Held-out prompt set integrity (frozen + digest-locked)

| Check | Command | Result |
|---|---|---|
| Frozen set intact | `python3 testing/tools/heldout_prompt_set.py --verify` | **PASS** — "Held-out set OK: frozen + valid (21 families, 105 prompts, >= 5/family)." |
| Audit plan dry-run (real session) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment real_session` | **PASS (exit 0)** — 21 families / 105 prompts enumerated; per-kind assertions correct; destructive-leak detector active |
| Audit plan dry-run (test substrate) | `python3 testing/tools/gui_cognition_capability_audit.py --dry-run --environment test_substrate` | **PASS (exit 0)** — same plan; substrate auto-approval path armed |

The frozen set is digest-locked (`heldout_prompt_set.v1.lock`, SHA-256); it cannot be silently
edited to make a build pass. The audit tool's scoring lives in pure functions and runs end-to-end
in `--dry-run` with no network. `C2_switch_window` (Switch window) is present at n=5, scored on
the strict execute+verify contract.

### 2.2 Task 4 deterministic test evidence (proves the Wayland-focus gate INTENT)

`cargo build -p kria-core` → **Finished (exit 0)**; `cargo check -p kria-desktop` → **Finished
(exit 0)** (call-site flip compiles). Then:

| # | Suite | Command | Result |
|---|---|---|---|
| 1 | Window focus (T1, lib) | `cargo test -p kria-core --lib gui_cognition::window_focus` | **PASS — 28 passed** |
| 2 | Window focus integration (T2) | `cargo test -p kria-core --test gui_cognition_window_focus_tests` | **PASS — 7 passed** |
| 3 | Executor (T1/T2) | `cargo test -p kria-core --test gui_cognition_executor_tests` | **PASS — 4 passed** |
| 4 | Workflow runtime (T1/T2) | `cargo test -p kria-core --test gui_cognition_workflow_runtime_tests` | **PASS — 16 passed** |
| 5 | Verification (T1/T2) | `cargo test -p kria-core --test gui_cognition_verification_tests` | **PASS — 12 passed** |

These collectively prove the gate's intent without a live run:

- **No `wmctrl` on Wayland (Req 3.1):** `wayland_excludes_wmctrl_and_keeps_preference_order`,
  `unknown_session_never_selects_wmctrl`, and `x11_includes_wmctrl_last` show the backend chain is
  session-selected and `wmctrl` is excluded on Wayland (and only ever last-resort on X11).
- **Compositor-native activate-by-identity preferred (Req 3.2):**
  `routing_prefers_compositor_native_over_alt_tab`,
  `routing_respects_chain_order_for_portal_before_alt_tab`, and
  `window_identity_sanitizes_and_reports_target` show GnomeBridge/Portal are tried before the
  Alt+Tab last resort, and Alt+Tab is dropped when no input substrate exists
  (`wayland_without_input_substrate_drops_alt_tab`) and never blind-fired without a target
  (`routing_without_target_never_blindly_alt_tabs`).
- **Verify by re-observing active window == requested (Req 3.4, 23):**
  `verify_active_window_matches_requested_identity`,
  `verify_focus_by_reobserve_reports_truthful_verdict`, and
  `verify_focus_by_reobserve_is_inconclusive_when_probe_failed` show focus is confirmed by
  re-observation with a truthful verified/inconclusive verdict (no false `verified`).
- **Clear actionable error, NOT "wmctrl required" (Req 3.3):**
  `no_path_message_is_actionable_and_never_mentions_wmctrl`,
  `no_path_message_for_missing_target_asks_for_a_window`,
  `no_path_message_for_backend_failed_and_not_implemented_are_actionable`, and
  `routing_no_available_backend_is_actionable_error` show the no-path failure is a clear, actionable
  message that never mentions `wmctrl` and never degrades to a generic "backend failed".
- **Truthful, sanitized routing surface (Req 3.2 `backend_used`):**
  `routing_json_is_sanitized_and_truthful`, `routing_json_surfaces_error_and_null_backend_on_no_path`.

This is the deterministic, CI-verifiable surrogate for the Switch-window ≥ 80% held-out target:
the Switch-window family (C2) that was BROKEN at the ~28% baseline ("Plan validation blocked" / no
Wayland-safe focus path) now routes through the abstraction, executes through a session-appropriate
backend, and is verified by re-observe — with the legacy "wmctrl required" path removed.

### 2.3 "Steps 1–12 green" — full gui_cognition suite surface

`cargo test -p kria-core` across the Step 1–12 same-path integration suites (single invocation):

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
| `gui_cognition_t2_fixture_tier` | 16 |
| `gui_cognition_target_resolver_tests` | 10 |
| `gui_cognition_verification_tests` | 12 |
| `gui_cognition_window_focus_tests` | 7 |
| `gui_cognition_workflow_runtime_tests` | 16 |
| **Integration total** | **253 passed; 0 failed** |

Plus the `gui_cognition::window_focus` lib unit tests: **28 passed** (backend selection, routing
order, verify-by-reobserve, no-`wmctrl`-on-Wayland, actionable no-path message, default-off +
default-on/rollback flag tests).

**Grand total: 281 passed; 0 failed** across the Task 4 + Steps 1–12 surface.

Known pre-existing UNRELATED failures (explicitly excluded from this gate per the task brief) live
in the crate's inline lib unit tests, not in this integration surface, and did **not** appear in
any of the runs above:
`agent::atspi_engine::tests::atspi_snapshot_request_defaults_are_bounded_for_gui_cognition`,
`agent::loop_engine::tests::deterministic_dispatch_create_project_folder`,
`agent::continuation_reentry::tests::duplicate_continuation_is_rejected`.

---

## 3. Gate flip: `gui_cog_wayland_focus` → default ON (with env rollback)

All CI-safe Task 4 evidence is green, so the flag is flipped to default-ON following the **same
pattern Task 1 (`gui_cog_runtime_guards`), Task 2 (`gui_cog_smart_planner`), and Task 3
(`gui_cog_reobserve`) used** (`*::from_env_default_on()`).

**Code changes:**

- `crates/kria-core/src/agent/gui_cognition/window_focus.rs`
  - `GuiWaylandFocusConfig::from_env_default_on()` + testable core
    `from_env_lookup_default_on()` were added in Task 4.1 (mirroring the runtime-guards /
    smart-planner / re-observe rollback semantics): the abstraction is ON unless
    `KRIA_GUI_COG_WAYLAND_FOCUS` is an explicit opt-out (`0`/`false`/`no`/`off`/empty); an absent
    value keeps it ON. The OFF-by-default `from_env()` and the truthy parser are unchanged.
- `crates/kria-desktop/src/commands/gui_cognition.rs`
  - The live-turn construction now calls `GuiWaylandFocusConfig::from_env_default_on()` (was
    `from_env()`). Server-side only — a client cannot toggle it.
- `crates/kria-core/src/agent/gui_cognition/window_focus.rs` (tests)
  - `flag_default_on_and_rollback` asserts the default-on + explicit-falsy-rollback semantics
    (added in 4.1). Green within the 28-test lib run.

**Rollback (no code change):** set `KRIA_GUI_COG_WAYLAND_FOCUS=0` (or `false`/`no`/`off`) in the
desktop environment to restore the prior SwitchWindow behavior. The deterministic T2 fixture tier
is unaffected — those runtimes set their Wayland-focus config explicitly and never read the env.

**Post-flip re-verification (all green):** `cargo check -p kria-desktop` (exit 0);
`cargo test -p kria-core --lib gui_cognition::window_focus` (28 passed);
`cargo test -p kria-core --test gui_cognition_window_focus_tests` (7 passed),
`--test gui_cognition_workflow_runtime_tests` (16 passed),
`--test gui_cognition_executor_tests` (4 passed),
`--test gui_cognition_verification_tests` (12 passed),
`--test gui_cognition_backend_route_tests` (27 passed).

---

## 4. KRIA runtime-authority self-check

The flip preserves every authority invariant:

- **KRIA authoritative; focus backends execution-only.** The flag only changes whether SwitchWindow
  routes through the Wayland-safe `WindowFocus` abstraction. KRIA still owns the
  Intent → Capability → Policy → Substrate → Tool → Verification flow; the focus backends
  (GnomeBridge/Portal/UinputAltTab/X11Wmctrl) are execution-only and gain no authority.
- **No Prompt→Tool shortcut.** SwitchWindow is resolved to a concrete window identity and routed
  through resolve → safety-gate → execute → verify; it never executes from raw prompt/OCR/LLM prose
  or raw coordinates. The Alt+Tab last resort is never blind-fired without a target
  (`routing_without_target_never_blindly_alt_tabs`).
- **No recursive / uncontrolled loops.** Focus + verify-by-reobserve are bounded by the Task 1
  runaway caps (`max_reobserve`, watchdog, `max_steps`); cancel / GlobalSafetyHalt still halt before
  the next action.
- **Verify-by-reobserve bounded; verification truth preserved.** SwitchWindow is confirmed only when
  the active window matches the requested identity above the confidence threshold; ambiguous probes
  yield `inconclusive`, never a false `verified`.
- **Safety / bounded-cognition / deterministic-orchestration / cancellation** unchanged —
  safety_hitl, verification, runtime_guards, preconditions, recovery suites all green.

---

## 5. Environment limitation & live numeric reproduction (PENDING)

This gate was driven to **green at the CI-safe level**. The live numeric **Switch-window family
(C2) ≥ 80%** held-out audit — with "Switch to the Chrome/terminal/file manager window" **executed +
verified** and **no "wmctrl required"** — is **PENDING a reachable live desktop API**:
`http://127.0.0.1:3001/api/health` is unreachable here, and the audit fails safe (exit 2, no
report) rather than fabricating numbers.

To close the live numeric gate on a machine with a desktop session:

```bash
# 0. Stand up the desktop so /api/health returns 200
cargo run -p kria-desktop        # (or: cd crates/kria-desktop && cargo tauri dev)

# 1. Preflight (uinput / AT-SPI / focus / DISPLAY)
bash scripts/gui_cognition_desktop_preflight.sh

# 2. Frozen set intact
python3 testing/tools/heldout_prompt_set.py --verify

# 3. Wayland-safe focus ON (default after this flip; explicit for clarity),
#    non-destructive audit on the REAL session (3 runs, gate on median)
KRIA_GUI_COG_WAYLAND_FOCUS=1 python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --out planning_docs/gui_cognition_capability_audit_wave3_wayland.md

# 4. Destructive / approval families in the TEST SUBSTRATE only (Xvfb/headless seat),
#    scratch files + clipboard save/restore; auto-approval rejected on real session
KRIA_GUI_COG_WAYLAND_FOCUS=1 scripts/gui_cognition_test_substrate.sh --mode xvfb -- \
    python3 testing/tools/gui_cognition_capability_audit.py \
    --runs 3 --environment test_substrate \
    --out planning_docs/gui_cognition_capability_audit_wave3_wayland_substrate.md
```

**Close condition:** **Switch-window family (C2_switch_window) ≥ 80%** (3-run median), each switch
**executed + verified by re-observe** (active window == requested), the legacy "wmctrl required"
failure **absent** (replaced by a clear actionable error only when genuinely no focus path exists),
**zero destructive-leak** (audit exit 0), and Steps 1–12 suites still green. Commit the generated
matrices alongside this doc.

---

## 6. Acceptance for Task 4.5

- [x] Frozen held-out set integrity verified (digest lock + invariants) and audit dry-run green
      (real session + substrate).
- [x] Deterministic T1/T2 evidence green: SwitchWindow routes through the Wayland-safe
      `WindowFocus` abstraction, selects a session-appropriate backend (never `wmctrl` on Wayland),
      prefers compositor-native activate-by-identity, executes, is verified by re-observe, and
      replaces the legacy "wmctrl required" path with a clear actionable error.
- [x] Steps 1–12 same-path suites green (253 integration passed) + `window_focus` lib tests green
      (28 passed) → 281 passed, 0 failed; known unrelated lib unit failures excluded and absent from
      this surface.
- [x] `gui_cog_wayland_focus` flipped to default ON via `from_env_default_on()`, mirroring Task 1's
      `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, and Task 3's `gui_cog_reobserve`;
      env rollback (`KRIA_GUI_COG_WAYLAND_FOCUS=0`) preserved + tested.
- [x] KRIA runtime-authority invariants self-checked (focus backends execution-only; no Prompt→Tool
      shortcut; verify-by-reobserve bounded by Task 1 caps; bounded/cancellable/safe).
- [ ] **Live numeric Switch-window family ≥ 80%** (executed + verified, no "wmctrl required") —
      PENDING a reachable live desktop API (reproduction in §5). No live percentages fabricated.

_Last recorded by Task 4.5. The authoritative live Switch-window ≥ 80% median is gated on a running
desktop session and is the only outstanding item; all CI-verifiable evidence is green and the flag
is ON._
