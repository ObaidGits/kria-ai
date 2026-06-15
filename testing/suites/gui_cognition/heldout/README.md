# GUI Cognition — Frozen Held-Out Prompt Set

This directory holds the **frozen held-out prompt set** for the GUI Cognition
production upgrade (spec task 0.1). It is the authoritative scoring input for the
live capability audit (Requirement 17) and is the gate at every stage of the
upgrade.

## Files

| File | Purpose |
|---|---|
| `heldout_prompt_set.v1.json` | The prompts: **21 capability families, ≥ 5 prompts each** (105 total). |
| `heldout_prompt_set.v1.lock` | SHA-256 freeze lock over the scored prompt content + invariant counts. Committed. |
| `test_heldout_prompt_set.py`  | Guard tests: family coverage, ≥ 5/family, kinds, no duplicates, frozen-digest match, tamper detection. |

Tooling lives separately at `testing/tools/heldout_prompt_set.py` (loader,
verifier, freeze tool).

## The freeze rule (do not edit to pass a build)

This set **must never be edited to make a failing build pass.** It is stored
separately from the audit code and protected by a committed digest lock:

- Any change to a scored prompt changes the SHA-256 digest.
- `--verify` (and the guard test) fail when the digest no longer matches the
  lock, so an edit-to-pass is caught and blocks CI.
- Legitimate changes are **append-only via a version bump** (`v2`, …) with a
  deliberate, reviewed re-freeze. Cosmetic edits to `description`/`policy` text
  are allowed (not hashed); prompt edits are not.

## Capability families (21)

`C1_open_app, C2_switch_window, C3_focus_control, C4_type_text, C5_clear_select,
C6_clipboard, C7_key_press, C8_scroll, C9_click_button, C10_checkbox, C11_dialog,
C12_in_app_search, C13_multistep, C14_cross_app, C15_fm_select, C16_read_visible,
C17_approval, C18_ambiguity, C19_boundary, C20_verify_stop, C21_recovery`

Prompt kinds: `action` (execute + verify), `ask` (refuse-to-guess / clarify),
`boundary` (no destructive/state-changing execution). Locale is English for v1
(Requirement 26.3).

## Commands

```bash
# Verify the set is frozen + valid (used by CI / task gate)
python3 testing/tools/heldout_prompt_set.py --verify

# Inspect per-family counts
python3 testing/tools/heldout_prompt_set.py --stats

# Deliberately (re)generate the lock after a reviewed version bump — NOT to mask a failure
python3 testing/tools/heldout_prompt_set.py --freeze

# Run the guard tests
python3 -m pytest testing/suites/gui_cognition/heldout/test_heldout_prompt_set.py
```

Task 0.2 upgrades `gui_cognition_capability_audit.py` to load its prompts from
this frozen set via `testing.tools.heldout_prompt_set.load_prompts()` instead of
the inline list, so scoring always runs against the frozen surface.
