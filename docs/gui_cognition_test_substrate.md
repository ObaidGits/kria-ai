# GUI Cognition TestSubstrate (spec task 0.3)

The **TestSubstrate** is the isolated environment where destructive and approval
GUI Cognition **live** tests run without touching the user's real session or
data (Requirement 20). It is the structural data-loss-safety boundary for the
live capability audit.

## What it provides

| Primitive | Guarantee | Implementation |
|---|---|---|
| Separate display | Live tests never drive the real desktop | nested compositor (`weston`) on a real session, or `Xvfb` for headless/CI |
| Scratch sandbox | Destructive file actions confined to throw-away files (R20.2) | scratch `HOME` with `Downloads`/`Documents` + sample files |
| Clipboard save/restore | The user's clipboard survives the run (R7.2, R20.2) | capture before, restore at teardown (best effort, Wayland/X11) |
| Substrate marker | Auto-approval fixtures rejected outside the substrate (R20.3) | `KRIA_GUI_TEST_SUBSTRATE=1` read server-side |

## The auto-approval gate (Requirement 20.3)

Auto-approval (HITL decision) fixtures are the only way a destructive/approval
prompt can execute under test. They are **rejected on the real session** and
honored **only** inside the substrate.

The marker is derived **server-side** from the KRIA desktop process environment,
never from the request payload — a client cannot claim "I'm a substrate" over the
wire to coax the real session into auto-approving. The runtime reads it via
`GuiExecutionEnvironment::from_env()`:

- `crates/kria-core/src/agent/gui_cognition/execution_environment.rs` — the
  `GuiExecutionEnvironment { RealSession | TestSubstrate { scratch_dir,
  restore_clipboard } }` type and the `from_env` gate.
- `crates/kria-core/src/agent/gui_cognition/mod.rs` — `run_turn` emits
  `HitlFixtureRejected` and leaves the action gated when an authorizing fixture
  arrives outside the substrate.

Env contract (mirrored in `testing/tools/gui_cognition_substrate.py`):

| Variable | Meaning |
|---|---|
| `KRIA_GUI_TEST_SUBSTRATE` | `1`/`true`/`yes`/`on` → substrate; anything else → real session |
| `KRIA_GUI_TEST_SUBSTRATE_SCRATCH_DIR` | scratch root destructive actions are confined to |
| `KRIA_GUI_TEST_SUBSTRATE_RESTORE_CLIPBOARD` | `0` to disable clipboard restore (default on) |

## Usage

The substrate marker is read by whichever KRIA process runs the GUI Cognition
runtime — for the live audit that is the **desktop app** serving
`/api/testing/desktop-chat-command`. So the desktop app must be **started inside
the substrate**; the audit client then targets it normally.

Start the desktop app inside the substrate (CI / headless seat):

```bash
scripts/gui_cognition_test_substrate.sh --mode xvfb --keep -- \
  cargo run -p kria-desktop --release
# then, in another shell, run the audit against it:
python3 testing/tools/gui_cognition_capability_audit.py \
  --environment test_substrate --runs 3
```

Or wrap both the app launch and the audit in a single substrate session script.

Nested compositor on a developer's real desktop (does not touch the real seat):

```bash
scripts/gui_cognition_test_substrate.sh --mode nested -- <command>
```

Print the substrate env to source into the current shell (no display started):

```bash
eval "$(scripts/gui_cognition_test_substrate.sh --scratch-dir /tmp/kria-gui-substrate)"
```

The launcher refuses any display that looks like the real session (`:0`/`:1`).

## Tests

- `crates/kria-core/.../execution_environment.rs` (`#[cfg(test)]`) — env gate.
- `crates/kria-core/tests/gui_cognition_backend_route_tests.rs`
  (`auto_approve_fixture_is_rejected_on_real_session`,
  `auto_approve_fixture_is_honored_in_test_substrate`) — full-runtime gate.
- `testing/suites/gui_cognition/test_substrate.py` — scratch isolation +
  safety refusals, clipboard save/restore round-trip, env mapping, launcher
  smoke.

## Environment limitations / CI path

A real **nested compositor** (`weston`) requires a host Wayland/X11 session and
the `weston` binary. Where that is unavailable (containers, headless CI, or this
build environment), use `--mode xvfb` (the CI path), which only needs the `Xvfb`
binary. Where neither is available, the **deterministic fixture tier** (spec task
0.4) provides a no-display T2 path instead of live execution.
