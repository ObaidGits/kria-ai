# Issue 2 Focus Authority Report

## Implementation Complete

Implemented real focus authority adapters for GUI Cognition selected-mode perception.

- Added canonical focus authority fields:
  - focused window/app
  - focused control id/label/role/bounds
  - editable target state
  - text cursor state
  - terminal-like state
  - source, confidence, reliability, adapter status, latency, failure chain
- Added live adapter priority path:
  1. GNOME bridge focus placeholder, reported unavailable unless bridge schema exposes focus fields
  2. Chrome/Chromium CDP active element
  3. Firefox WebDriver BiDi active element
  4. KRIA VS Code extension endpoint
  5. GNOME Terminal heuristic
  6. AT-SPI focused object/window/app fallback
  7. unavailable with precise blocker
- Added Chrome CDP metadata-only active element probe in `browser_cognition.rs`.
- Added Firefox WebDriver BiDi metadata-only active element probe in `browser_cognition.rs`.
- Browser focus snapshots now require either `document.hasFocus()` or a matching
  OS/bridge foreground-app hint before they can claim focus.
- Firefox BiDi sessions are closed after each probe so Firefox does not hit the
  "maximum active sessions" limit after repeated observations.
- Added a manual KRIA VS Code extension scaffold:
  - `crates/kria-desktop/vscode-extension/kria-focus-authority`
  - localhost-only `/focus` endpoint
  - reports editor/terminal metadata only
  - reports `window_focused` in the updated VSIX so background VS Code
    windows cannot claim desktop focus after restart/reload
  - never returns code contents, selected text contents, clipboard, or full workspace paths
- Added GNOME Terminal handling:
  - terminal focus is known keyboard focus
  - `terminal_like=true`
  - `editable_target_known=false` for normal GUI form typing
- Updated `ObservationCompleted`, `ContextBuilt`, UI store, and GUI panel.
- Added deterministic same-path fixtures for Chrome, Firefox, VS Code editor, VS Code terminal, GNOME Terminal, and unavailable adapters.

## Tests Executed

Targeted during this implementation:

- `cargo check -p kria-desktop --quiet`

- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel app.tool-choice HitlModal`
- `cd ui && npm run test:run`
- `cd ui && npm run build`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`
- `cd testing/suites/playwright && npm run typecheck`
- `cd testing/suites/playwright && npm run test:tauri-mock -- tests/gui-cognition-tool-mode.tauri-mock.e2e.spec.ts`
- `./testing/run.sh gui_cognition --profile ci`
- `./testing/run.sh gui_cognition --tag focus_authority --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast`
- `git diff --check`

Additional live adapter proof after installing the VS Code extension:

- Direct VS Code endpoint check:
  - `http://127.0.0.1:47323/focus`
  - returned `focused_app=VS Code`, `focused_control_role=editor`,
    `editable_target_known=true`, `confidence=0.95`
- Same-path VS Code prompt through `/api/testing/desktop-chat-command`:
  - path `send_manual_tool_message`
  - `focus_source=vscode_extension`
  - `focused_control_role=editor`
  - `editable_target_known=true`
  - `confidence=0.95`
- Live Chrome CDP prompt with isolated debug profile on port `9222`:
  - path `send_manual_tool_message`
  - `focus_source=chrome_cdp_active_element`
  - `focused_control_label=KRIA Chrome Search`
  - `focused_control_role=searchbox`
  - `editable_target_known=true`
  - `confidence=0.94`
- Live Firefox BiDi attempt with Snap Firefox `151.0.3`:
  - Firefox accepted the `--remote-debugging-port 9223` flag in process args
  - no listener was created on `127.0.0.1:9223`
  - KRIA reported `Firefox BiDi unavailable: IO error: Connection refused`
  - no false focus claim was made
- Live Firefox BiDi proof with local Mozilla Firefox `151.0.3`:
  - local binary installed at `~/.local/share/kria/tools/firefox-bidi/firefox`
  - BiDi listener opened on `127.0.0.1:9223`
  - same-path prompt returned `focus_source=firefox_bidi_active_element`,
    `focused_control_role=searchbox`, `focused_control_label=KRIA Firefox Search`,
    `editable_target_known=true`, `confidence=0.90`
  - repeated BiDi session creation succeeded after KRIA probes, confirming the
    session cleanup fix
- Live GNOME Terminal proof:
  - GNOME Terminal became foreground after the KRIA GNOME bridge was enabled
  - bridge D-Bus returned `app_name=Terminal`, `app_id=org.gnome.Terminal.desktop`,
    and `source=kria_gnome_shell_bridge`
  - selected-mode prompt returned `focus_source=gnome_bridge_focus`,
    `focused_control_role=terminal`, `terminal_like=true`,
    `editable_target_known=false`, `confidence=0.86`
  - KRIA did not emit `ActionStarted` or `ActionCompleted`
- KRIA GNOME Shell bridge install:
  - extension files are installed at
    `~/.local/share/gnome-shell/extensions/kria-active-window@kria.ai`
  - metadata now includes an explicit extension version and GNOME Shell 46 support
  - `gnome-extensions pack` validates the installed extension package
  - `org.gnome.shell enabled-extensions` includes the KRIA bridge without removing
    the user's existing enabled extensions
  - after package validation, `gnome-extensions enable kria-active-window@kria.ai`
    activated the extension in the current session
  - bridge D-Bus service is live at `ai.kria.ActiveWindow`

## Results

- Rust desktop compile check passed after adding live adapter code.
- Focus-authority same-path suite passed all 7 selected scenarios.
- Broad desktop-command suite passed: `77 passed, 0 failed, 5 skipped`.
- UI full test suite passed: `93 passed`.
- Playwright GUI Cognition mock E2E passed: `13 passed`.
- Same-path focus fixture scenarios now cover:
  - Chrome CDP search box focus
  - Firefox BiDi search box focus
  - VS Code editor focus
  - VS Code integrated terminal focus
  - GNOME Terminal focus
  - unavailable adapter fallback
  - observe-only no-action behavior
- Live VS Code and Chrome adapter paths were confirmed through the same
  selected-mode local API path.
- Live Firefox BiDi is implemented, fixture-tested, and live-confirmed with the
  local non-Snap Firefox binary. Snap Firefox GUI still does not expose the BiDi
  listener in this environment.

## Failures Found

- Previous live path treated terminal-like AT-SPI roles as editable targets.
- Previous focus failure chain only reported app adapters as not live, not as bounded authority attempts.
- Previous same-path focus coverage only had one generic fixture text-field scenario.
- During testing, an unavailable focus fixture was falsely upgraded by generic accessibility-control fallback. This was fixed before final verification.
- During live testing, the first VS Code selected-mode prompt returned focus
  unavailable because GNOME Wayland did not expose an active-app hint, and
  the app-adapter gate skipped VS Code before asking the extension.
- During review, CDP/BiDi page focus was identified as insufficient without
  foreground confirmation. Browser adapters now require `document.hasFocus()`
  or a matching OS/bridge foreground-app hint before they can claim focus.
- Firefox BiDi sessions were not closed after probing, which could leave
  Firefox at `Maximum number of active sessions`. Session cleanup is now
  performed after successful and failed post-session probes.
- The GNOME Shell bridge did not become visible until the package was validated
  and explicitly enabled with `gnome-extensions enable`.

## Fixes Applied

- Terminal-like roles now set `terminal_like=true` and `editable_target_known=false`.
- Live adapter attempts are bounded and recorded in `focus_failure_chain`.
- UI shows focus adapter status, latency, focused element, editable state, and terminal warning.
- Fixtures now represent production adapter sources instead of a generic focus fixture only.
- Failed focus probe payloads now preserve structured failure-chain data.
- The unavailable focus fixture no longer exposes a focused accessibility control that can be promoted by generic fallback.
- Browser adapters can be probed when the OS focus hint is unavailable, but
  they are rejected unless `document.hasFocus()` proves foreground page focus.
  If a trusted OS/bridge hint confirms the browser is foreground, the adapter
  may accept Firefox/Chrome active-element metadata even when the browser's
  `document.hasFocus()` signal is weak.
- VS Code can be probed when the OS focus hint is unavailable; the updated
  extension reports `window_focused` and KRIA rejects explicit background
  responses.
- The VS Code extension package now includes repository/license/files metadata
  and uses `onStartupFinished` activation instead of wildcard activation.
- Focus authority now uses the KRIA GNOME bridge as focus context when available.
  For terminal windows it reports `terminal_like=true` and
  `editable_target_known=false`, preserving blind-typing safety.
- Installed the KRIA GNOME Shell bridge extension files for the current user.
- Fixed the KRIA GNOME Shell bridge metadata, validated the package with
  `gnome-extensions pack`, and staged it in GNOME's enabled-extension settings
  for the next shell/session restart.
- Enabled the KRIA GNOME Shell bridge through `gnome-extensions enable`; the
  extension is now `Enabled: Yes`, `State: ACTIVE`, and the D-Bus bridge responds
  to `GetActiveWindow`.

## Remaining Risks

- Chrome focus requires Chrome/Chromium to be launched with CDP on port `9222`;
  background Chrome pages are rejected unless `document.hasFocus()` is true.
- Firefox focus requires a Firefox build/session that actually binds WebDriver
  BiDi on port `9223`; the tested Snap Firefox GUI process did not bind the
  port, while the local Mozilla Firefox binary does.
- VS Code focus requires manual installation/run of the KRIA VS Code extension.
  The updated `window_focused` field is available after VS Code reload/restart.
- GNOME Terminal focus now depends on the KRIA GNOME bridge remaining enabled.
  If the extension is disabled later, KRIA will fall back safely and report a
  precise blocker instead of guessing terminal focus.
- If apps do not expose focus authority and adapters are not running, KRIA remains fail-closed with a precise blocker.

## Ready For Next Issue?

Yes for Chrome CDP, Firefox BiDi through local Mozilla Firefox, VS Code
extension, GNOME Terminal through the KRIA GNOME bridge, AT-SPI fallback,
terminal-like handling, and fail-closed unavailable behavior.

Issue 2 Focus Unknown: PASS.
