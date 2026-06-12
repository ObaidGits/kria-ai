# Issue 3 Control Fusion Report

## Implementation Complete

Implemented a control fusion layer inside GUI Cognition perception.

- Added identity, bounds, state, and executable confidence fields to `GuiControlSummary`.
- Added source tracking and stricter `is_executable_candidate()` gating.
- Fused AT-SPI controls with visual detections and OCR layout evidence.
- Context executable controls now use fused confidence instead of only `enabled && visible`.
- Resolver-facing matching now only returns executable candidates.
- UI and same-path responses show trusted, partial, not executable, and executable counts.

## Tests Executed

- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cargo check -p kria-desktop --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`

## Results

- Controlled fixture controls expose labels, bounds, state, confidence, quality, and executable status.
- Same-path scenario `gui_cognition.controls.fused_executable_surface` was added.

## Failures Found

- Visual label matching initially allowed a visual input named `Search` to match a button named `Search`, creating a duplicate visual-only button.

## Fixes Applied

- Added visual type to accessibility role compatibility checks before label/bounds matching.

## Remaining Risks

- Live app quality still depends on whether AT-SPI exposes bounds/state and whether the vision sidecar is available.
- Browser DOM and VS Code adapter evidence are not forced; they remain optional future authority sources.

## Ready For Next Issue?

Yes for fusion contract and safe executable gating.
