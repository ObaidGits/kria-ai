import { describe, expect, it } from "vitest";
import {
  decideRenderMode,
  detectBackdropFilter,
  detectReducedMotion,
  detectWebGLTier,
  G2_MIN_INTERACTION_FPS,
  probePasses,
  shouldEnable3D,
  type CapabilitySnapshot,
  type ProbeResult,
} from "./capabilities";

function snapshot(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
  return {
    webglTier: "webgl2",
    hasWebGL: true,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
    ...overrides,
  };
}

function passingProbe(overrides: Partial<ProbeResult> = {}): ProbeResult {
  return {
    interactionFrameMs: 20,
    interactionFps: 50,
    idleQuiet: true,
    nodeCount: 1500,
    ...overrides,
  };
}

describe("detectWebGLTier", () => {
  it("returns 'none' when document is unavailable", () => {
    expect(detectWebGLTier(undefined)).toBe("none");
  });

  it("returns 'none' when canvas cannot produce a context (jsdom)", () => {
    // jsdom's canvas has no WebGL — this is the WebKitGTK-like default path.
    const fakeDoc = {
      createElement: () => ({ getContext: () => null }),
    } as unknown as Document;
    expect(detectWebGLTier(fakeDoc)).toBe("none");
  });

  it("prefers webgl2 when available", () => {
    const fakeDoc = {
      createElement: () => ({ getContext: (t: string) => (t === "webgl2" ? {} : null) }),
    } as unknown as Document;
    expect(detectWebGLTier(fakeDoc)).toBe("webgl2");
  });

  it("falls back to webgl1 when webgl2 is absent", () => {
    const fakeDoc = {
      createElement: () => ({ getContext: (t: string) => (t === "webgl" ? {} : null) }),
    } as unknown as Document;
    expect(detectWebGLTier(fakeDoc)).toBe("webgl1");
  });
});

describe("detectReducedMotion", () => {
  it("defaults to true (motion-off) when matchMedia is unavailable", () => {
    expect(detectReducedMotion(undefined)).toBe(true);
    expect(detectReducedMotion({} as unknown as Window)).toBe(true);
  });

  it("reflects the media query match", () => {
    const win = { matchMedia: (q: string) => ({ matches: q.includes("reduce") }) } as unknown as Window;
    expect(detectReducedMotion(win)).toBe(true);
    const winNo = { matchMedia: () => ({ matches: false }) } as unknown as Window;
    expect(detectReducedMotion(winNo)).toBe(false);
  });
});

describe("detectBackdropFilter", () => {
  it("returns false when CSS.supports is unavailable", () => {
    expect(detectBackdropFilter(undefined)).toBe(false);
  });

  it("detects standard and -webkit- prefixed support", () => {
    const std = { supports: (p: string) => p === "backdrop-filter" } as unknown as typeof CSS;
    expect(detectBackdropFilter(std)).toBe(true);
    const webkit = { supports: (p: string) => p === "-webkit-backdrop-filter" } as unknown as typeof CSS;
    expect(detectBackdropFilter(webkit)).toBe(true);
    const none = { supports: () => false } as unknown as typeof CSS;
    expect(detectBackdropFilter(none)).toBe(false);
  });
});

describe("probePasses (G2 thresholds)", () => {
  it("passes when interaction fps >= threshold AND idle is quiet", () => {
    expect(probePasses(passingProbe())).toBe(true);
    expect(probePasses(passingProbe({ interactionFps: G2_MIN_INTERACTION_FPS }))).toBe(true);
  });

  it("fails when interaction fps below threshold", () => {
    expect(probePasses(passingProbe({ interactionFps: G2_MIN_INTERACTION_FPS - 1 }))).toBe(false);
  });

  it("fails when idle is not quiet even if fps is high", () => {
    expect(probePasses(passingProbe({ idleQuiet: false }))).toBe(false);
  });
});

describe("decideRenderMode — 2D is the default", () => {
  it("2D under reduced-motion regardless of WebGL/probe", () => {
    const d = decideRenderMode(snapshot({ prefersReducedMotion: true, probe: passingProbe() }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.reason).toContain("reduced-motion");
  });

  it("2D when WebGL is absent (WebKitGTK common case)", () => {
    const d = decideRenderMode(snapshot({ hasWebGL: false, webglTier: "none", probe: passingProbe() }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.reason).toContain("WebGL");
  });

  it("2D when no probe has run yet (default until G2 passes)", () => {
    const d = decideRenderMode(snapshot({ probe: null }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.reason).toContain("not yet run");
  });

  it("2D when the probe is below the G2 threshold", () => {
    const d = decideRenderMode(snapshot({ probe: passingProbe({ interactionFps: 22, idleQuiet: true }) }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.reason).toContain("below threshold");
  });

  it("3D only when capability passes AND probe passes", () => {
    const d = decideRenderMode(snapshot({ probe: passingProbe() }));
    expect(d.mode).toBe("3d");
    expect(d.enable3D).toBe(true);
    expect(shouldEnable3D(snapshot({ probe: passingProbe() }))).toBe(true);
  });
});
