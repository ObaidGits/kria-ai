import { beforeEach, describe, expect, it } from "vitest";
import {
  applyProbeResult,
  degradeToTwoD,
  initRenderMode,
  lensRenderMode,
  setReducedMotion,
  useLensRenderMode,
} from "./renderMode";
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";

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
  return { interactionFrameMs: 20, interactionFps: 50, idleQuiet: true, nodeCount: 1500, ...overrides };
}

describe("renderMode gate — 2D is the default", () => {
  beforeEach(() => {
    // Reset the shared store to a known capable-but-unprobed baseline.
    initRenderMode(snapshot());
  });

  it("defaults to 2D with 3D disabled before any probe runs", () => {
    const s = initRenderMode(snapshot());
    expect(s.mode).toBe("2d");
    expect(s.enable3D).toBe(false);
    expect(s.reason).toContain("not yet run");
  });

  it("stays 2D and static under reduced-motion even with a passing probe", () => {
    const s = initRenderMode(snapshot({ prefersReducedMotion: true, probe: passingProbe() }));
    expect(s.mode).toBe("2d");
    expect(s.enable3D).toBe(false);
    expect(s.isStatic).toBe(true);
  });

  it("stays 2D when WebGL is absent (WebKitGTK common case)", () => {
    const s = initRenderMode(snapshot({ hasWebGL: false, webglTier: "none" }));
    expect(s.enable3D).toBe(false);
    expect(s.mode).toBe("2d");
  });

  it("isStatic is false when motion is allowed", () => {
    const s = initRenderMode(snapshot());
    expect(s.isStatic).toBe(false);
  });
});

describe("applyProbeResult — the only path into 3D", () => {
  beforeEach(() => initRenderMode(snapshot()));

  it("enables 3D when a passing probe is applied on a capable device", () => {
    const s = applyProbeResult(passingProbe());
    expect(s.mode).toBe("3d");
    expect(s.enable3D).toBe(true);
    expect(lensRenderMode().enable3D).toBe(true);
  });

  it("keeps 2D when the probe is below the G2 threshold", () => {
    const s = applyProbeResult(passingProbe({ interactionFps: 22 }));
    expect(s.enable3D).toBe(false);
    expect(s.reason).toContain("below threshold");
  });

  it("keeps 2D when the probe is null (probe could not run)", () => {
    const s = applyProbeResult(null);
    expect(s.enable3D).toBe(false);
  });

  it("does not enable 3D under reduced-motion even with a passing probe", () => {
    initRenderMode(snapshot({ prefersReducedMotion: true }));
    const s = applyProbeResult(passingProbe());
    expect(s.enable3D).toBe(false);
    expect(s.mode).toBe("2d");
  });
});

describe("degradeToTwoD — auto-degrade hook", () => {
  beforeEach(() => initRenderMode(snapshot()));

  it("tears 3D back down to 2D and clears the probe", () => {
    applyProbeResult(passingProbe());
    expect(lensRenderMode().enable3D).toBe(true);
    const s = degradeToTwoD();
    expect(s.mode).toBe("2d");
    expect(s.enable3D).toBe(false);
    expect(s.snapshot.probe).toBeNull();
    expect(s.reason).toContain("degraded");
  });

  it("is idempotent when already 2D", () => {
    const before = lensRenderMode();
    const s = degradeToTwoD();
    expect(s.mode).toBe("2d");
    expect(s.enable3D).toBe(before.enable3D);
  });
});

describe("live reduced-motion kill-switch", () => {
  it("tears active 3D down to static 2D and restores only when motion is allowed", () => {
    initRenderMode(snapshot());
    applyProbeResult(passingProbe());
    expect(lensRenderMode().enable3D).toBe(true);

    const frozen = setReducedMotion(true);
    expect(frozen.mode).toBe("2d");
    expect(frozen.enable3D).toBe(false);
    expect(frozen.isStatic).toBe(true);

    const restored = setReducedMotion(false);
    expect(restored.mode).toBe("3d");
    expect(restored.enable3D).toBe(true);
    expect(restored.isStatic).toBe(false);
  });
});

describe("useLensRenderMode", () => {
  it("returns the shared reactive accessor", () => {
    initRenderMode(snapshot());
    const accessor = useLensRenderMode();
    expect(accessor().mode).toBe("2d");
    applyProbeResult(passingProbe());
    expect(accessor().enable3D).toBe(true);
  });
});
