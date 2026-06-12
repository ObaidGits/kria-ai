# Issue 6 OCR Performance Report

## Implementation Complete

Implemented a lightweight fast OCR pipeline surface without requiring heavyweight model downloads.

- Added OCR diagnostics:
  - fast path
  - cache hit
  - ROI count
  - changed region count
  - cold/warm start timing
  - benchmark summary
- Desktop live OCR now caches successful OCR output by screenshot hash for a short 1.5 second TTL.
- OCR still reuses the shared screenshot bytes and remains bounded.
- Sidecar OCR and Tesseract fallback now report fast-path and benchmark metadata.
- UI shows OCR fast path, cache state, ROI count, and changed-region count.

## Tests Executed

- `cargo test -p kria-core --test gui_cognition_observation_perception_tests --quiet`
- `cargo test -p kria-desktop gui_cognition --quiet`
- `cargo check -p kria-desktop --quiet`
- `cd ui && npm run check`
- `cd ui && npm run test:run -- guiCognitionSession GuiCognitionPanel`
- `python3 -m pytest testing/harness/tests/test_manifest_validation.py testing/harness/tests/test_inventory.py`

## Results

- Same-path scenario `gui_cognition.ocr.fixture_fast_path` was added.
- Repeated observe/plan prompts can reuse recent OCR output by screen hash.

## Failures Found

- No compile/runtime regressions after adding OCR cache and metrics.

## Fixes Applied

- Used `std::sync::LazyLock` instead of adding a new desktop crate dependency for `once_cell`.

## Remaining Risks

- True changed-region OCR and real ROI cropping are represented as metrics/hooks but not a full production CV pipeline yet.
- RapidOCR/PaddleOCR remain optional adapters; they are reported as unavailable/not configured unless installed and wired later.

## Ready For Next Issue?

Yes for bounded cache/metrics. Production sub-second OCR quality remains partial until a faster OCR backend is enabled.
