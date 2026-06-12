# Issue 4 Visual Controls Report

## Implementation Complete

Added `VisualControlDetector` support as safe supporting evidence.

- Added `GuiVisualControlDetection` contract.
- Added `detect_visual_controls()` to `GuiPerceptionProvider`.
- Live desktop path calls the existing OmniParser-compatible vision sidecar `/parse_screen` with a bounded timeout.
- Fixture path returns deterministic visual buttons/inputs/tabs.
- Visual-only controls remain non-executable unless safer sources later confirm state and identity.
- UI shows visual controls detected, button-like count, matched/unmatched count, and false-positive risk.

## Tests Executed

- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cargo check -p kria-desktop --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`

## Results

- Same-path scenario `gui_cognition.visual.fixture_buttons_supporting_evidence` was added.
- Vision data can now support button/link/toggle/menu/tab evidence without bypassing safety.

## Failures Found

- Same visual label collision bug found during control fusion testing.

## Fixes Applied

- Role/type compatibility was added to reject cross-type visual matches.

## Remaining Risks

- The current sidecar remains an OmniParser-compatible scaffold. Real model-backed OmniParser/Rapid UI parsing must be enabled separately for production visual detection rates.
- Browser DOM extraction is not a mandatory live dependency in this pass.

## Ready For Next Issue?

Yes for supporting-evidence integration. Production visual model quality remains partial.
