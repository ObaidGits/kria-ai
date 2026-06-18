import { describe, expect, it } from "vitest";
import {
  applyPan,
  applyPinch,
  clampTransform,
  clientToSurfaceNorm,
  doubleTapToggle,
  fitScale,
  fitTransform,
  isFit,
  MAX_ZOOM,
  scaleRange,
  type Bounds,
} from "./viewTransform";

// Viewport 1000×500, surface 2000×1000 (same 2:1 aspect) → fitScale = 0.5.
const b: Bounds = { vw: 1000, vh: 500, sw: 2000, sh: 1000 };
// Wider surface than viewport aspect → letterbox top/bottom.
const bLetter: Bounds = { vw: 1000, vh: 1000, sw: 2000, sh: 1000 };

describe("viewTransform.fit", () => {
  it("fitScale picks the limiting dimension", () => {
    expect(fitScale(b)).toBe(0.5);
    expect(fitScale(bLetter)).toBe(0.5); // limited by width: 1000/2000
  });

  it("fitTransform fills exactly when aspect matches", () => {
    const t = fitTransform(b);
    expect(t.scale).toBe(0.5);
    expect(t.tx).toBe(0);
    expect(t.ty).toBe(0);
  });

  it("fitTransform centers letterbox when aspect differs", () => {
    const t = fitTransform(bLetter);
    // content = 2000*0.5 × 1000*0.5 = 1000×500 in a 1000×1000 viewport → centered vertically.
    expect(t.scale).toBe(0.5);
    expect(t.tx).toBe(0);
    expect(t.ty).toBe(250);
  });

  it("scaleRange spans fit→fit*MAX_ZOOM", () => {
    const r = scaleRange(b);
    expect(r.min).toBe(0.5);
    expect(r.max).toBe(0.5 * MAX_ZOOM);
  });
});

describe("viewTransform.clamp", () => {
  it("clamps scale into range", () => {
    expect(clampTransform({ scale: 0.1, tx: 0, ty: 0 }, b).scale).toBe(0.5);
    expect(clampTransform({ scale: 99, tx: 0, ty: 0 }, b).scale).toBe(0.5 * MAX_ZOOM);
  });

  it("keeps zoomed content covering the viewport (no gutters)", () => {
    // scale 1.0 → content 2000×1000 in 1000×500 viewport.
    const over = clampTransform({ scale: 1, tx: 9999, ty: 9999 }, b);
    expect(over.tx).toBe(0); // can't move past top-left
    const under = clampTransform({ scale: 1, tx: -9999, ty: -9999 }, b);
    expect(under.tx).toBe(1000 - 2000); // -1000 (right edge aligned)
    expect(under.ty).toBe(500 - 1000); // -500
  });
});

describe("viewTransform.pinch", () => {
  it("zooms keeping the focus point stationary on the surface", () => {
    const fit = fitTransform(b); // scale 0.5
    // Focus at container center (500,250) → surface (1000,500) i.e. norm (0.5,0.5).
    const z = applyPinch(fit, 500, 250, 2, b); // → scale 1.0
    expect(z.scale).toBe(1);
    const n = clientToSurfaceNorm(500, 250, 0, 0, z, b);
    expect(n.x).toBeCloseTo(0.5, 5);
    expect(n.y).toBeCloseTo(0.5, 5);
  });

  it("respects max zoom", () => {
    const fit = fitTransform(b);
    const z = applyPinch(fit, 0, 0, 100, b);
    expect(z.scale).toBe(0.5 * MAX_ZOOM);
  });
});

describe("viewTransform.pan", () => {
  it("translates and clamps", () => {
    const z = applyPinch(fitTransform(b), 500, 250, 2, b); // scale 1, content 2000×1000
    const panned = applyPan(z, -100, -50, b);
    expect(panned.tx).toBeLessThanOrEqual(0);
    expect(panned.ty).toBeLessThanOrEqual(0);
    // cannot pan beyond left/top edge
    const maxed = applyPan(z, 99999, 99999, b);
    expect(maxed.tx).toBe(0);
    expect(maxed.ty).toBe(0);
  });
});

describe("viewTransform.doubleTap", () => {
  it("toggles fit ↔ 2x at the tap point", () => {
    const fit = fitTransform(b);
    expect(isFit(fit, b)).toBe(true);
    const zoomed = doubleTapToggle(fit, 500, 250, b);
    expect(zoomed.scale).toBe(1); // 2× fit
    expect(isFit(zoomed, b)).toBe(false);
    const back = doubleTapToggle(zoomed, 500, 250, b);
    expect(isFit(back, b)).toBe(true);
  });
});

describe("viewTransform.clientToSurfaceNorm", () => {
  it("maps correctly at fit", () => {
    const t = fitTransform(b);
    expect(clientToSurfaceNorm(0, 0, 0, 0, t, b)).toEqual({ x: 0, y: 0 });
    expect(clientToSurfaceNorm(1000, 500, 0, 0, t, b)).toEqual({ x: 1, y: 1 });
    expect(clientToSurfaceNorm(500, 250, 0, 0, t, b)).toEqual({ x: 0.5, y: 0.5 });
  });

  it("accounts for container offset", () => {
    const t = fitTransform(b);
    const n = clientToSurfaceNorm(540, 290, 40, 40, t, b);
    expect(n.x).toBeCloseTo(0.5, 5);
    expect(n.y).toBeCloseTo(0.5, 5);
  });

  it("clamps to [0,1]", () => {
    const t = fitTransform(b);
    const n = clientToSurfaceNorm(-50, 9999, 0, 0, t, b);
    expect(n.x).toBe(0);
    expect(n.y).toBe(1);
  });

  it("stays correct under zoom + pan", () => {
    // Zoom to 2× at center, then the same screen center still maps to (0.5,0.5).
    const z = applyPinch(fitTransform(b), 500, 250, 2, b);
    const n = clientToSurfaceNorm(500, 250, 0, 0, z, b);
    expect(n.x).toBeCloseTo(0.5, 5);
    expect(n.y).toBeCloseTo(0.5, 5);
  });
});
