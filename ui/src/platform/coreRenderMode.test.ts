/**
 * coreRenderMode tests (task 0.3) — the homepage Core-3D capability gate and
 * runtime render-mode resolver. Verifies the 2D-first / 3D-as-gated contract:
 * the resolver defaults to the first-class permanent 2D path whenever ANY
 * degrade trigger fires (reduced-motion, no-WebGL, low-power, failed gate,
 * frame-drop), that `auto` reaches 3D only when the Core-3D gate passes and
 * nothing degrades, and that explicit "3d" bypasses only the perf gate while
 * hard + runtime-safety triggers still force 2D.
 *
 * Validates: Requirements 17.4, 20.2, 20.3, 20.4
 */
import { describe, it, expect, beforeEach } from "vitest";
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";
import {
  CORE_3D_MIN_SUSTAINED_FPS,
  activeCoreDegradeTriggers,
  applyCoreGateResult,
  coreGatePasses,
  coreRenderMode,
  initCoreRenderMode,
  reportCoreFrameDrop,
  resolveCoreRenderMode,
  setCoreGatePassed,
  setCoreLowPower,
  setCoreReducedMotion,
  setCoreRenderPreference,
  type CoreRenderInputs,
  type CoreRenderMode,
} from "./coreRenderMode";

/** A fully 3D-capable device snapshot (WebGL present, motion allowed). */
function caps(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
  return {
    webglTier: "webgl2",
    hasWebGL: true,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
    ...overrides,
  };
}

/** Resolver inputs for a 3D-capable device with the gate passed and no degrade. */
function inputs(overrides: Partial<CoreRenderInputs> = {}): CoreRenderInputs {
  return {
    preference: "auto",
    snapshot: caps(),
    gatePassed: true,
    lowPower: false,
    frameDrop: false,
    ...overrides,
  };
}

const passingProbe = (): ProbeResult => ({
  interactionFrameMs: 16,
  interactionFps: 60,
  idleQuiet: true,
  nodeCount: 400,
});

describe("coreGatePasses — Core-3D gate", () => {
  it("fails with no probe (2D-first default until a probe passes)", () => {
    expect(coreGatePasses(null)).toBe(false);
  });

  it("passes a probe at/above the sustained-fps floor with a quiet idle", () => {
    expect(coreGatePasses(passingProbe())).toBe(true);
    expect(
      coreGatePasses({ ...passingProbe(), interactionFps: CORE_3D_MIN_SUSTAINED_FPS }),
    ).toBe(true);
  });

  it("fails a probe below the sustained-fps floor", () => {
    expect(coreGatePasses({ ...passingProbe(), interactionFps: 22 })).toBe(false);
  });

  it("fails when the idle loop did not go quiet (budget violation)", () => {
    expect(coreGatePasses({ ...passingProbe(), idleQuiet: false })).toBe(false);
  });
});

describe("activeCoreDegradeTriggers — trigger detection", () => {
  it("reports no triggers for a capable device with the gate passed", () => {
    expect(activeCoreDegradeTriggers(inputs())).toEqual([]);
  });

  it("detects reduced-motion (Req 17.4)", () => {
    expect(
      activeCoreDegradeTriggers(inputs({ snapshot: caps({ prefersReducedMotion: true }) })),
    ).toContain("reduced-motion");
  });

  it("detects no-webgl", () => {
    expect(
      activeCoreDegradeTriggers(inputs({ snapshot: caps({ hasWebGL: false, webglTier: "none" }) })),
    ).toContain("no-webgl");
  });

  it("detects low-power", () => {
    expect(activeCoreDegradeTriggers(inputs({ lowPower: true }))).toContain("low-power");
  });

  it("detects failed-gate", () => {
    expect(activeCoreDegradeTriggers(inputs({ gatePassed: false }))).toContain("failed-gate");
  });

  it("detects frame-drop", () => {
    expect(activeCoreDegradeTriggers(inputs({ frameDrop: true }))).toContain("frame-drop");
  });

  it("reports every active trigger together, order-stable", () => {
    expect(
      activeCoreDegradeTriggers({
        preference: "auto",
        snapshot: caps({ prefersReducedMotion: true, hasWebGL: false, webglTier: "none" }),
        gatePassed: false,
        lowPower: true,
        frameDrop: true,
      }),
    ).toEqual(["reduced-motion", "no-webgl", "low-power", "failed-gate", "frame-drop"]);
  });
});

describe("resolveCoreRenderMode — auto preference", () => {
  it("resolves 3D only when the gate passes and nothing degrades", () => {
    const d = resolveCoreRenderMode(inputs());
    expect(d.mode).toBe("3d");
    expect(d.enable3D).toBe(true);
    expect(d.degraded).toBe(false);
  });

  it.each<[string, Partial<CoreRenderInputs>]>([
    ["reduced-motion", { snapshot: caps({ prefersReducedMotion: true }) }],
    ["no-webgl", { snapshot: caps({ hasWebGL: false, webglTier: "none" }) }],
    ["low-power", { lowPower: true }],
    ["failed-gate", { gatePassed: false }],
    ["frame-drop", { frameDrop: true }],
  ])("auto-degrades to the first-class 2D path on %s", (trigger, override) => {
    const d = resolveCoreRenderMode(inputs(override));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.degraded).toBe(true);
    expect(d.triggers).toContain(trigger as never);
  });
});

describe("resolveCoreRenderMode — explicit 2D (permanent path)", () => {
  it("stays 2D by preference even on a fully capable device", () => {
    const d = resolveCoreRenderMode(inputs({ preference: "2d" }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    // Chosen by preference, not a degrade.
    expect(d.degraded).toBe(false);
  });
});

describe("resolveCoreRenderMode — explicit 3D", () => {
  it("renders 3D even when the perf gate has NOT passed (bypasses only the gate)", () => {
    const d = resolveCoreRenderMode(inputs({ preference: "3d", gatePassed: false }));
    expect(d.mode).toBe("3d");
    expect(d.enable3D).toBe(true);
  });

  it.each<[string, Partial<CoreRenderInputs>]>([
    ["reduced-motion", { snapshot: caps({ prefersReducedMotion: true }) }],
    ["no-webgl", { snapshot: caps({ hasWebGL: false, webglTier: "none" }) }],
    ["low-power", { lowPower: true }],
    ["frame-drop", { frameDrop: true }],
  ])("is still forced to 2D by the hard/runtime trigger %s", (trigger, override) => {
    const d = resolveCoreRenderMode(inputs({ preference: "3d", gatePassed: true, ...override }));
    expect(d.mode).toBe("2d");
    expect(d.enable3D).toBe(false);
    expect(d.degraded).toBe(true);
    expect(d.triggers).toContain(trigger as never);
  });
});

describe("reactive store — init + live degrade triggers", () => {
  beforeEach(() => {
    // Reset to a known capable/gated posture for each test.
    initCoreRenderMode(caps(), "auto");
    setCoreGatePassed(true);
  });

  it("seeds 2D-first: fresh init keeps 2D until the gate passes", () => {
    initCoreRenderMode(caps(), "auto");
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().triggers).toContain("failed-gate");
  });

  it("reaches 3D once the gate passes with no degrade", () => {
    expect(coreRenderMode().mode).toBe("3d");
    expect(coreRenderMode().enable3D).toBe(true);
  });

  it("live reduced-motion flip degrades a mounted 3D Core to 2D (Req 17.4)", () => {
    expect(coreRenderMode().mode).toBe("3d");
    setCoreReducedMotion(true);
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().triggers).toContain("reduced-motion");
    setCoreReducedMotion(false);
    expect(coreRenderMode().mode).toBe("3d");
  });

  it("runtime frame-drop auto-degrades then restores when cleared (Req 20.4)", () => {
    reportCoreFrameDrop(true);
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().triggers).toContain("frame-drop");
    reportCoreFrameDrop(false);
    expect(coreRenderMode().mode).toBe("3d");
  });

  it("low-power posture degrades to 2D (Req 20.3)", () => {
    setCoreLowPower(true);
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().triggers).toContain("low-power");
  });

  it("applyCoreGateResult(null) keeps the failed-gate 2D default", () => {
    applyCoreGateResult(null);
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().triggers).toContain("failed-gate");
    applyCoreGateResult(passingProbe());
    expect(coreRenderMode().mode).toBe("3d");
  });

  it("explicit 2D preference is honored live and never enables 3D", () => {
    (["2d", "3d", "auto"] as CoreRenderMode[]).forEach((p) => setCoreRenderPreference(p));
    setCoreRenderPreference("2d");
    expect(coreRenderMode().mode).toBe("2d");
    expect(coreRenderMode().enable3D).toBe(false);
  });
});
