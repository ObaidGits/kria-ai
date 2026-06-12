# GUI Cognition Perception Completion Report

## Issue 2: PASS

Focus authority now includes bounded real adapter paths for Chrome CDP, Firefox WebDriver BiDi, KRIA VS Code extension, GNOME Terminal, and AT-SPI fallback. Deterministic same-path fixtures cover each adapter source. Terminal-like focus is not treated as normal GUI editable input, unavailable focus remains fail-closed, and browser/VS Code adapters are gated by active/focused app hints before they can claim focus.

Additional live proof after the VS Code extension install confirmed:

- VS Code selected-mode focus: `focus_source=vscode_extension`, role `editor`, editable target known, confidence `0.95`.
- Chrome selected-mode focus with isolated CDP profile on port `9222`: `focus_source=chrome_cdp_active_element`, role `searchbox`, editable target known, confidence `0.94`.
- Firefox selected-mode live proof now passes with the local Mozilla Firefox binary at `~/.local/share/kria/tools/firefox-bidi/firefox`: `focus_source=firefox_bidi_active_element`, role `searchbox`, editable target known, confidence `0.90`. Snap Firefox GUI still does not bind BiDi.
- GNOME Terminal selected-mode live proof now passes through the KRIA GNOME bridge: `active_window_source=kria_gnome_shell_bridge`, `active_window_app=Terminal`, `focus_source=gnome_bridge_focus`, role `terminal`, `terminal_like=true`, `editable_target_known=false`, confidence `0.86`.
- Follow-up bridge staging fixed the extension metadata, validated the package, restored GNOME's enabled-extension setting so KRIA is enabled alongside the user's existing extensions, and enabled the extension in the current session.

Verdict is PASS for Issue 2. Firefox BiDi is live-confirmed through the local non-Snap Firefox path, and GNOME Terminal is live-confirmed through the KRIA GNOME bridge.

## Issue 3: PASS

Control fusion and executable confidence gating are implemented. Context and resolver now use fused executable candidates. Visual/OCR evidence cannot directly authorize execution.

## Issue 4: PARTIAL

Visual controls are integrated as supporting evidence through the existing OmniParser-compatible sidecar and fixture tests. Production-quality detection still depends on enabling a real model-backed sidecar or DOM adapters.

## Issue 5: PASS

Accessibility health scoring is implemented with status, confidence, app scores, stale/timeout counts, and remediation. Degraded accessibility is no longer treated as fully reliable.

## Issue 6: PARTIAL

Fast OCR cache and metrics are implemented. Sidecar/Tesseract paths are bounded and observable. Full changed-region OCR and RapidOCR/PaddleOCR production benchmarking remain optional follow-up work.

## Files Changed

- `crates/kria-core/src/agent/gui_cognition/perception.rs`
- `crates/kria-core/src/agent/gui_cognition/context.rs`
- `crates/kria-core/src/agent/gui_cognition/mod.rs`
- `crates/kria-core/src/agent/browser_cognition.rs`
- `crates/kria-core/src/agent/atspi_engine.rs`
- `crates/kria-desktop/src/commands/gui_cognition.rs`
- `crates/kria-desktop/vscode-extension/kria-focus-authority/`
- `ui/src/types/guiCognition.ts`
- `ui/src/stores/guiCognitionSession.ts`
- `ui/src/components/GuiCognitionPanel.tsx`
- `testing/harness/models.py`
- `testing/suites/gui_cognition/manifest.json`
- `testing/suites/gui_cognition/scenarios/focus_authority.json`
- `testing/suites/gui_cognition/scenarios/control_fusion.json`
- `testing/suites/gui_cognition/scenarios/visual_controls.json`
- `testing/suites/gui_cognition/scenarios/accessibility_health.json`
- `testing/suites/gui_cognition/scenarios/ocr_performance.json`
- `testing/suites/gui_cognition/scenarios/perception_completion.json`

## Tests Executed

- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_context_builder_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_backend_route_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_goal_contract_tests --quiet`
- `cargo test -p kria-core --test gui_cognition_llm_planner_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cargo check -p kria-desktop --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel`
- `cd ui && npm run test:run`
- `cd ui && npm run build`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`
- `cd testing/suites/playwright && npm run typecheck`
- `cd testing/suites/playwright && npm run test:tauri-mock -- tests/gui-cognition-tool-mode.tauri-mock.e2e.spec.ts`
- `./testing/run.sh gui_cognition --profile ci`
- `./testing/run.sh gui_cognition --tag focus_authority --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag control_fusion --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag visual_controls --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag accessibility_health --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag ocr_performance --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag perception_completion --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag perception --include-live --include-slow --fail-fast`
- `./testing/run.sh gui_cognition --tag desktop_command --include-live --include-slow --fail-fast`
- `git diff --check`

Broad live desktop-command result:

```text
77 passed, 0 failed, 5 skipped
```

## Failures Found

- Visual label-only matching could cross-match an input and a button with the same label.
- Desktop crate did not include `once_cell`, so adding it directly failed compile.
- Panel test had duplicate `reliable` text after adding focus reliability.
- Focus unavailable fixture was falsely upgraded by generic accessibility-control fallback.
- Browser CDP/BiDi page focus needed foreground proof to avoid claiming focus from a background browser.
- Active-app hint gating was too strict when GNOME Wayland exposed no active app/window; it skipped VS Code even though the extension had fresh focus metadata.
- Snap Firefox did not bind the WebDriver BiDi listener on `127.0.0.1:9223`.
- Firefox BiDi sessions were not explicitly ended after probe completion, causing a possible `Maximum number of active sessions` failure.
- Running GNOME Wayland Shell did not immediately hot-load the newly installed KRIA bridge extension until it was package-validated and explicitly enabled.

## Fixes Applied

- Added visual control type to accessibility role compatibility checks.
- Used `std::sync::LazyLock` for OCR cache instead of adding a dependency.
- Updated UI tests to expect multiple reliability labels.
- Preserved structured failure-chain data from failed focus probes.
- Removed focused controls from the unavailable focus fixture.
- Browser focus adapters now require `document.hasFocus()` or a matching OS/bridge foreground-app hint before accepting CDP/BiDi metadata.
- Firefox BiDi probes now end their WebDriver BiDi session after completion.
- Installed a local Mozilla Firefox binary at `~/.local/share/kria/tools/firefox-bidi/firefox` for a working non-Snap BiDi path.
- VS Code focus can be probed when OS focus hints are unavailable; the updated extension reports `window_focused`, and KRIA rejects explicit background-window responses.
- Repackaged the VS Code extension with repository/license/files metadata and `onStartupFinished` activation.
- Focus authority now consumes the KRIA GNOME bridge when available and maps terminal active windows to `terminal_like=true`, not normal editable GUI input.
- The KRIA GNOME bridge extension metadata was fixed, the installed package validates, and the bridge is staged in `org.gnome.shell enabled-extensions` for the next GNOME Shell/session restart.
- The KRIA GNOME bridge is now enabled and active in the current session; `ai.kria.ActiveWindow` responds and the live selected-mode terminal focus proof passed.

## Remaining Limitations

- Live production quality still depends on OS/app source exposure.
- Snap Firefox GUI still does not bind the remote debugging listener here; use the local Mozilla Firefox binary for BiDi testing/production until Snap behavior is resolved.
- The updated VS Code extension `window_focused` field requires VS Code reload/restart after VSIX reinstall.
- Live terminal focus depends on the KRIA GNOME Shell bridge remaining enabled; if disabled, KRIA falls back safely with a blocker instead of guessing.
- The system remains fail-closed: weak or unavailable sources are explicit and do not become executable authority.

## Overall GUI Cognition Readiness

PARTIAL
