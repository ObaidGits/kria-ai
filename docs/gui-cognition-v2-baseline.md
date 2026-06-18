# GUI Cognition V2 — Real-Verify Baseline Report (Task 10)

This is the recorded baseline that gated the V2 default flip (Task 12 / A6).
It is produced by the live, external-truth harness `scripts/gui_cog_e2e_v2.py`
(re-run it to regenerate `scripts/gui_cog_v2_baseline_<brain>.{md,json}`).

## Method

- **Pipeline:** V2 Sight/Brain/Hands loop (`KRIA_GUI_COG_V2=1`), real execution
  (`hitl_decision_fixture=approve`, `execution_mode=execute_live`) inside a
  `test_substrate`.
- **Brain:** `qwen` (text-first, default). The A/B `ui_tars` brain is run with
  `KRIA_GUI_COG_V2_BRAIN=ui_tars` and recorded to a separate report file.
- **External ground truth (outside KRIA):** GNOME extension
  `GetFocusedWindow` / `ListWindows` + `pgrep`. A reply that claims success while
  reality disagrees is flagged **MISMATCH**; anything the environment cannot
  verify is **INCONCLUSIVE** (never fake-passed).
- **Prompts:** held-out, UNSEEN phrasings (different wording from the prompts
  used while tuning), including multi-step open + standard-follow-up chains.

## Recorded baseline (qwen brain)

Held-out run result that gated the flip:

| Metric | Count |
|--|--|
| PASS | 6 |
| FAIL | 0 |
| MISMATCH | 0 |
| BLOCKED | 0 |
| INCONCLUSIVE | 0 |

**Clean pass** — every held-out case executed for real and was confirmed by the
external compositor/pgrep truth. Safety/HITL (A3), per-step re-observe
verification (A4), cancel / no-progress, audit, uinput, and model-swap remained
intact.

### Notable fixes surfaced by this harness
- `open the files manager` was BLOCKED until `app_registry::normalize_alias`
  was taught to treat `_` as a space (the Brain emits snake_case
  `OpenApp{app:"files_manager"}`). Externally verified live: nautilus `0 -> 1`.
- A FAILING action now arms the no-progress guard (so an unresolvable app name
  stops instead of running to the step cap).

## Known quality gap (tracked, not a flip regression)

Multi-action follow-up (e.g. "open chrome AND new tab/reload/close tab") is
inconsistent on the 7B Brain: it sometimes returns `needs_clarification` after
the open instead of chaining the follow-up `Key`. The deterministic
`apply_followup_assist` mitigates the common cases (covered by unit tests).
App-launch and single-action turns are solid. This is the hardening target
before the V1 over-built pipeline is deleted (Task 13).

## Regenerating

```bash
# default qwen brain
KRIA_GUI_COG_V2=1 python3 scripts/gui_cog_e2e_v2.py

# UI-TARS A/B (requires a vision model available to the router)
KRIA_GUI_COG_V2=1 KRIA_GUI_COG_V2_BRAIN=ui_tars python3 scripts/gui_cog_e2e_v2.py
```

Each run writes `scripts/gui_cog_v2_baseline_<brain>.md` and `.json`.
