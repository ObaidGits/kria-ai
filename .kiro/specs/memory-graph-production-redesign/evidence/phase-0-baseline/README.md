# Legacy Phase 0 Memory Graph Baseline Instructions

**Status:** Historical capture instructions only. This directory contains no valid F0 Evidence Artifact manifest and proves no implementation or gate completion. New runs must use the canonical `evidence/F0/<run-id>/` contract in `validation.md`.

Historical task: `0.5 Capture deterministic baselines`

These instructions describe an initial current-state comparison baseline for MGR-027/MGR-028 and findings MG-H14/MG-M27. Any generated output is descriptive baseline material only, not proof that F0 or later release thresholds pass.

The seed, fixture names, and command below predate the v2 fixture contract. They may be used only to reproduce historical behavior; F0 implementation must use the versioned `mg-*-v2` fixtures and manifest tooling defined by the current specification.

## Reproduce legacy baseline

## Reproduce

From `ui/`:

```bash
npx playwright test e2e/memory-graph-baseline.spec.ts --project=webkit
```

The test uses seed `1263683905` (`0x4b524941`), authority sizes 100/1k/10k/100k, current visible cap 300, and viewports 640×480, 800×600, 1176×775, 1440×900, 1920×1080, and 2560×1080. Each run performs 5 API warmups plus 30 measured iterations.

## Evidence contract

Generated JSON records commit/worktree state, OS/CPU/RAM, browser, desktop session, scale, power mode, GPU/VRAM/power and battery when exposed, deterministic fixture identity, API p50/p95/p99, frame p50/p95/p99, paint/layout-shift/GC/long-task/event/mutation/idle counters, DOM/heap data, screenshots, keyboard route, and AT-facing semantics.

Metrics contain fixture IDs/counts only. No real labels, source text, query text, credentials, or private content are recorded. Unavailable host telemetry remains explicitly unavailable.

Screenshots are asserted for non-empty output and paired with semantic checks: rendered count, generated-navigation classification, absence of generated authority lines, status count, and AT labels/live regions.

## Interpretation limits

- API timings measure deterministic current v1 browser-bridge fixtures, not SQLite or production API acceptance. F3/F5 own production backend and release gates.
- CPU percentage/RSS describe the capture harness process; system RAM and browser JS heap are recorded separately.
- Chromium/WebKit expose different paint, GC, heap, and long-task observers; support is recorded per report.
- Native Orca speech cannot be truthfully automated in a headless browser. Report includes installed-version probe plus AT-SPI semantic proxy; human WebKitGTK listen-through remains required.
- Current one-focus-stop-per-visible-node keyboard route and discarded capped authority total are baseline findings, not accepted behavior.
- Initial capture has no prior accepted phase. Future phase reports must compare on same hardware/config and explain >10% regressions.
