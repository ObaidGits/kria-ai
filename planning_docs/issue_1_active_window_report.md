# Issue 1 Active Window Report

## Implementation Complete

Implemented the GUI Cognition selected-mode Active Window Authority Layer for Issue #1 only.

The observation path remains:

```text
GUI Cognition dropdown
 -> send_manual_tool_message
 -> observe
 -> gui_cognition:event
 -> UI panel
```

No focus-provider expansion, control-fusion work, OCR optimization, OpenApp, click/type/hotkey execution, global planner, or normal-mode routing changes were added for this issue.

Implemented pieces:

- Added richer active-window fields to `GuiActiveWindowSummary`: app id, pid, workspace, monitor, fullscreen/minimized flags, observed timestamp, authority status, and GNOME bridge status.
- Preserved source-specific active-window reliability and confidence instead of flattening every successful source into the generic `get_active_window` source.
- Added a bounded KRIA GNOME Shell bridge probe as the primary GNOME Wayland authority source.
- Added fallback priority through existing GNOME Shell Eval/compositor probes, X11/XWayland, AT-SPI focused window, AT-SPI focused app, and single-window best-effort fallback.
- Added stale bridge timestamp rejection/downgrade behavior.
- Added active-window authority fields to canonical `ObservationCompleted` event payloads and response JSON.
- Updated the GUI Cognition store/panel to render the authority source, confidence, status, and GNOME bridge state.
- Added a KRIA-owned GNOME Shell extension package at:
  - `crates/kria-desktop/gnome-shell/extensions/kria-active-window@kria.ai/metadata.json`
  - `crates/kria-desktop/gnome-shell/extensions/kria-active-window@kria.ai/extension.js`
- Added same-path active-window fixtures and scenarios for bridge success, bridge fallback, AT-SPI fallback, single-window best effort, precise failure chain, and secret redaction.

## Tests Executed

Rust:

```bash
cargo check -p kria-desktop --quiet
cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet
cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet
cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet
cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet
cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet
cargo test -p kria-desktop gui_cognition --quiet
```

UI:

```bash
cd ui && npm run check
cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel app.tool-choice HitlModal
cd ui && npm run test:run
cd ui && npm run build
```

Harness and Playwright:

```bash
python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py
cd testing/suites/playwright && npm run typecheck
cd testing/suites/playwright && npm run test:tauri-mock -- tests/gui-cognition-tool-mode.tauri-mock.e2e.spec.ts
```

Same-path evals through `/api/testing/desktop-chat-command`:

```bash
./testing/run.sh gui_cognition --tag active_window --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --tag perception --include-live --include-slow --fail-fast
./testing/run.sh gui_cognition --profile ci
./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast
```

## Results

- Core active-window/perception tests: 16 passed.
- Core backend route tests: 16 passed.
- Core context builder tests: 5 passed.
- Core goal contract tests: 5 passed.
- Core LLM planner tests: 7 passed.
- Desktop GUI Cognition tests: 2 passed.
- UI targeted tests: 53 passed.
- UI full tests: 92 passed.
- Playwright GUI Cognition mock E2E: 13 passed.
- Harness manifest/inventory tests: 16 passed.
- GUI Cognition CI profile: passed.
- Active-window same-path suite: 6 active-window scenarios passed.
- Perception same-path suite: passed.
- Broad live desktop-command suite: passed.

Active-window same-path scenarios verified:

- `gui_cognition.active_window.gnome_bridge_reliable`
- `gui_cognition.active_window.bridge_missing_fallback`
- `gui_cognition.active_window.atspi_fallback`
- `gui_cognition.active_window.single_window_best_effort`
- `gui_cognition.active_window.failure_chain_precise`
- `gui_cognition.active_window.no_raw_secret_leakage`

## Failures Found

- Existing controlled perception scenario initially failed against an already-running old KRIA binary, because the local API had not been restarted after the final source-preservation change. Restarting KRIA with the rebuilt binary fixed the mismatch.
- GNOME Wayland in the current live environment can still report active-window unavailable when the KRIA GNOME Shell bridge is not installed/enabled and GNOME does not expose focus through the fallback probes.

## Fixes Applied

- Restarted KRIA before same-path evals so `/api/testing/desktop-chat-command` used the updated active-window authority code.
- Preserved `active_window.data.source` in the core observation summary so fixture, GNOME bridge, compositor, and AT-SPI authority sources keep their own confidence and reliability.
- Updated the controlled-surface fixture expectation to match the preserved `gui_cognition_test_fixture` source.
- Added source-specific confidence/reliability mapping and stale GNOME bridge rejection.

## Remaining Risks

- The KRIA GNOME Shell extension package is present, but live GNOME bridge accuracy still requires installing/enabling the extension in the user GNOME session. Until then, GNOME Wayland may still degrade to existing fallback probes or an explicit unavailable blocker.
- Manual window-switch tests across VS Code, Firefox, Terminal, fullscreen, minimize/restore, multiple monitors, and browser tab switching were not fully automated in this run. The bridge behavior is covered by deterministic same-path fixtures; a final manual/live bridge pass should be run after enabling the extension.
- GNOME Shell extension APIs can vary across GNOME versions. The extension should be smoke-tested on the target production GNOME versions before marking this source as fully deployed.

## Ready For Next Issue?

yes, with one operational caveat: Issue #1 code, contracts, UI, and same-path tests are complete, but production deployment should install/enable the KRIA GNOME Shell bridge and run the manual live app-switching matrix once on the target GNOME session.
