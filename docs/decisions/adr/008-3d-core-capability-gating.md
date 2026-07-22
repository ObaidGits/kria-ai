# ADR: 3D Core Capability-Gating + 2D-First

Status: Accepted
Date: 2026-05-27
Owner: Homepage Presence Redesign
Scope: `ui/src/platform/coreRenderMode.ts` (Core gate/resolver),
`ui/src/platform/renderMode.ts` (lens gate), `ui/src/components/CorePresence.tsx`,
`ui/src/platform/boot.ts`, `ui/src/styles/motion.css`
Requirements: 17 (motion/performance budget), 20 (Linux performance gates)

## 1) Decision

**2D is the default and permanent render path for the Core — never a fallback
afterthought.** The 3D Core is an enhancement gated on device capability. The
homepage Core has its **own** render-mode resolver (`coreRenderMode.ts`),
separate from the Memory/Capability 3D-lens gate (`renderMode.ts`).

`resolveCoreRenderMode` reads a stored preference (`2d | 3d | auto`) and a live
capability snapshot and resolves to a concrete `2d` or `3d`. 3D enables only when
the preference allows it, the capability gate passes, and no degrade trigger is
active. It auto-degrades to 2D whenever any trigger fires, in a fixed documented
order:

```text
reduced-motion → no-webgl → low-power → failed-gate → frame-drop
```

The 3D Core is a **single WebGL surface** (translucent shell + one filament layer
+ suspended motes + one tilted ring + aura + a *faked* static rim — no real
distortion), visually consistent with the 2D path, and the context is released
on unmount. Frame rate caps at 30–45 fps, rendering pauses on blur, and the
motion budget sheds particles → filament → parallax → breath under load.

## 2) Context

KRIA is Linux-primary and often runs on WebKitGTK, where WebGL is frequently
absent or weak, while the local AI simultaneously saturates CPU/GPU. A homepage
that assumes 3D would be broken or janky on the primary target. The Memory and
Capability lenses already have a 3D gate, but the Core is a different surface
with different budgets and must not share that gate.

## 3) Rationale

- **Linux reality.** Defaulting to 2D guarantees a correct, smooth homepage on
  WebKitGTK and under model load; 3D is layered on only where it demonstrably
  works.
- **Separate gates, separate surfaces.** The Core and the lenses have different
  render costs and lifecycles; a shared gate would couple unrelated decisions.
- **Measurable degradation.** A fixed trigger order makes degrade behavior
  deterministic and testable rather than heuristic.

## 4) Enforcement

`boot.ts` seeds both gates from live capability detection before surfaces mount,
starting in 2D. The resolver and trigger order are property-tested
(`ui/src/shell/spaces/home/perfInvariants.pbt.test.ts`,
`ui/src/platform/coreRenderMode.test.ts`, `renderMode.test.ts`). Linux-matrix
gate results are recorded in the spec's `core-3d-gate-matrix.md` and
`performance-matrix.md`; physical per-device runs on the single-dev laptop are
**gated/manual** and documented honestly there rather than claimed as automated.

## 5) Consequences

- **Positive:** the 2D homepage is smooth on the primary Linux target and under
  local-AI load; 3D never becomes a hard dependency; degradation is deterministic.
- **Negative / limitation:** the physical multi-device Linux matrix
  (GNOME/KDE × Wayland/X11 × NVIDIA/AMD/Intel) is validated manually on available
  hardware, not in automated CI on every device.

## 6) Alternatives Considered

- **3D-first with a 2D fallback.** Rejected: fails on WebKitGTK/no-WebGL and
  under load; makes 2D a second-class path.
- **Reuse the lens 3D gate for the Core.** Rejected: different surface, budget,
  and lifecycle; coupling would cause wrong decisions for one or the other.
