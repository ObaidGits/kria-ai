# Prototype Validation Gates — Recorded Results

Source of truth: `design.md §11.3` (gate table) and `§11.2` (WebKitGTK Correction).
Task: `tasks.md 0.5`. Requirements: 16.1, 16.3, 5.5, 17.5, 18.5.

## How to reproduce

The gate harness lives in `ui/src/prototypes/` and the capability gate in
`ui/src/platform/capabilities.ts`. Run the interactive probes in the workbench:

```bash
cd ui && npm run story:dev      # open "Prototypes/Gates" stories
```

Pure logic (frame math, fuzzy timing, capability gate) is covered by unit tests:

```bash
cd ui && npx vitest run src/platform/capabilities.test.ts src/prototypes/gateProbes.test.ts
```

Each gate story mounts a probe that reuses `ui/src/utils/perf.ts` (task 0.3) so
measures land in the perf buffer / dev HUD.

## Critical design posture (design.md §11.2)

**On Linux/WebKitGTK the 2D Memory graph and Capability constellation are the
DEFAULT.** 3D is opt-in, enabled only when BOTH (a) capability detection passes
(WebGL present, not reduced-motion) AND (b) the on-device G2 probe passes. **G2
resolving as "2D-default, 3D-off" is an ACCEPTED PASS outcome, not a failure.**
The gate is implemented in `capabilities.ts::decideRenderMode` and consumed by
task 0.6.

## Environment reality

This work was performed in a **single-laptop dev environment** (per
`dev-context.md`). The full GNOME+KDE × Wayland+X11 × NVIDIA+AMD+Intel hardware
matrix cannot be physically exercised here. Deliverables are therefore a
**runnable, on-device gate harness + capability gate + this results ledger**:
the current device is measured where measurable; the rest of the matrix is
flagged **pending-on-hardware** with documented default fallbacks.

### This device (one matrix cell)

| Property | Value |
|---|---|
| Desktop | GNOME (`ubuntu:GNOME`) |
| Session | **Wayland** |
| GPU (integrated) | Intel UHD Graphics 770 (Alder Lake-HX) |
| GPU (discrete) | NVIDIA GeForce RTX 4050 Max-Q / Mobile |
| Renderer target | Tauri **WebKitGTK** (production shell); Storybook probes run in the dev browser engine |

> Matrix cell covered by direct measurement: **GNOME × Wayland × Intel/NVIDIA-hybrid**.
> Remaining cells (KDE; X11; AMD-only) are **pending-on-hardware**.

## Gate results

Legend: **PASS** (criteria met) · **FALLBACK** (fail → documented fallback taken)
· **PASS(2D)** (G2 accepted 2D-default outcome) · **PENDING-HW** (needs the
physical matrix cell to record a final figure).

| Gate | P | Goal | Success criteria | This-device measured result | Decision | Matrix-pending notes |
|---|---|---|---|---|---|---|
| **G1 — WebKitGTK baseline** | P0 | Shell + 5k virtualized rows | 60 fps scroll; idle main-thread <2% CPU / ~0 GPU; no blank screen | Harness renders 5000 rows via `@tanstack/solid-virtual` with only viewport rows in the DOM (`overscan 8`, `contain: strict`); `list-scroll` perf measures recorded per scroll. Full fps + idle-CPU/GPU figures require the WebKitGTK shell. | **PENDING-HW** (harness ready; virtualization mandatory per §11.2) | Record fps + idle CPU/GPU per cell in the WebKitGTK build. Fallback if failing: reduce DOM depth, cut blur, ship `WEBKIT_DISABLE_COMPOSITING_MODE` / env-flag guidance. |
| **G2 — 3D graph viability** | P0 | 1–2k instanced nodes, frozen-when-idle | interaction ≥30 fps AND idle ~0 | Probe (`runG2Probe`, 1500 WebGL points) + gate (`decideRenderMode`) implemented and unit-tested. Where WebGL is absent (common WebKitGTK software-raster case) the probe returns `null` → gate resolves **2D default**. | **PASS(2D)** by design; per-device 3D-on/off recorded by running the probe | 2D is the mandated default (§11.2). On capable cells (e.g. this NVIDIA 4050), run the G2 story to record fps and flip `enable3D` only if it passes. Fallback: do not enable 3D on that device; 2D graph is default. |
| **G3 — Core motion** | P1 | CSS/SVG Core across all states | idle GPU negligible; static under reduced-motion; reads clearly | Not built here — the Core presence is task 2.2. Reduced-motion detection (`detectReducedMotion`, defaults motion-OFF) is in place to drive the static-Core path. | **PENDING-HW / task 2.2** | Validate once Core lands; reduced-motion must render a static Core (Req 3.5, 16.3). Fallback: simpler CSS Core, no shader. |
| **G4 — uPlot live charts** | P1 | 5 live series @1 Hz | <5% CPU, smooth | Harness mounts a real `uPlot` with 5 series, pushes 1 sample/series/sec, trims to a 120-pt window, records per-update `setData` cost. CPU% requires the WebKitGTK shell + OS profiler. | **PENDING-HW** (harness ready) | Record CPU% per cell. Fallback: reduce update rate / series; static snapshots. |
| **G5 — Command palette** | P1 | fuzzy over ~5k items | open <100 ms; <16 ms/keystroke | **Measured on this device** (5000 items): index build (open) **0.41 ms** ≪ 100 ms; keystroke **max 1.27 ms / mean 0.65 ms** ≪ 16 ms. | **PASS** | Engine caveat: measured in Node V8; WebKitGTK JavaScriptCore expected similar for pure-JS subsequence scoring. Re-confirm in-shell. Fallback: precompute index; cap result set (already limited to top 50). |
| **G6 — Kobalte a11y** | P1 | dialog/menu/listbox/tooltip | keyboard + screen-reader pass | Not exercised here — Kobalte primitives + a11y tests were delivered in task 0.4 (`ui/src/kit/*`). | **PENDING-HW** (screen-reader pass) | Run keyboard + Orca/NVDA pass per cell. Fallback: swap the specific primitive to Ark UI. |
| **G7 — Multi-window detach** | P2 | Tauri windows on 2 monitors | detach works; approval mirrors to active window; Core per window | Not built here — detach is task 12.3 (needs Tauri window APIs + 2 monitors). | **PENDING-HW / task 12.3** | Requires physical dual-monitor setup. Fallback: disable detach; single-window only. |
| **G8 — Blur / aura-glass** | P1 | floating layers with blur | acceptable compositing cost on matrix | `assessBlurFeasibility` reports `backdrop-filter` support and picks the treatment; harness renders both the `backdrop-blur` layer and the mandated `solid-translucent` fallback side-by-side for visual compositing-cost judgement. | **PENDING-HW** (visual/GPU cost) | Record compositing cost per cell in the WebKitGTK shell. Fallback: solid translucent surfaces (no backdrop blur) — visual language must survive without blur (§11.2). |

## P0 Phase-0 exit criteria status

- **G1 (P0)** — harness ready and virtualization enforced; **final fps/idle
  figures pending the WebKitGTK shell on each matrix cell.**
- **G2 (P0)** — gate implemented and **resolved by design as 2D-default**; 3D is
  capability + on-device-probe gated. This satisfies the §11.2 posture. Per-cell
  3D-enable is recorded by running the G2 probe on that device.

Phase-0 dependent surfaces may proceed on the **2D-default** ladder now (per the
tasks.md note that 3D lenses never block their Space's 2D delivery). Recording
the remaining in-shell fps/CPU/GPU figures across the matrix is tracked as
**pending-on-hardware** and folds into task 12.5 / 17.1 (Linux-matrix validation).

## Deliverables produced by task 0.5

- `ui/src/platform/capabilities.ts` (+ `.test.ts`) — WebGL / reduced-motion /
  backdrop-filter detection + the `decideRenderMode` / `shouldEnable3D` gate
  (2D-default, 3D capability+probe-gated). Feeds task 0.6.
- `ui/src/prototypes/gateProbes.ts` (+ `.test.ts`) — frame-timing math, G2 WebGL
  probe, G5 fuzzy index + timing, G8 blur feasibility.
- `ui/src/prototypes/G1VirtualRows.tsx`, `G4LiveCharts.tsx`, `G2G5G8Probes.tsx`,
  `Gates.stories.tsx` — the interactive workbench harness (reuses
  `ui/src/utils/perf.ts`).
- Pinned deps: `@tanstack/solid-virtual@3.13.33`, `uplot@1.6.32` (exact).
