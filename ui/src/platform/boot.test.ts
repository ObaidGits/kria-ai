import { afterEach, describe, expect, it } from "vitest";
import {
  applyBootAttributes,
  decideBlurTreatment,
  disposePlatform,
  initPlatform,
  runCoreGateAndApply,
} from "./boot";
import type { CapabilitySnapshot, ProbeResult } from "./capabilities";
import { coreRenderMode, initCoreRenderMode } from "./coreRenderMode";

function snapshot(overrides: Partial<CapabilitySnapshot> = {}): CapabilitySnapshot {
  return {
    webglTier: "none",
    hasWebGL: false,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
    ...overrides,
  };
}

afterEach(() => {
  disposePlatform();
  document.documentElement.removeAttribute("data-reduced-motion");
});

describe("decideBlurTreatment — aura-glass gate (§11.2/§11.3 G8)", () => {
  it("enables blur only with backdrop-filter support AND motion allowed", () => {
    expect(decideBlurTreatment(snapshot({ supportsBackdropFilter: true }))).toBe("on");
  });

  it("falls back to solid when backdrop-filter is unsupported", () => {
    expect(decideBlurTreatment(snapshot({ supportsBackdropFilter: false }))).toBe("off");
  });

  it("falls back to solid under reduced-motion regardless of support", () => {
    expect(
      decideBlurTreatment(snapshot({ supportsBackdropFilter: true, prefersReducedMotion: true })),
    ).toBe("off");
  });
});

describe("applyBootAttributes", () => {
  it("stamps data-blur=on and no reduced-motion when blur is available", () => {
    applyBootAttributes(document, snapshot({ supportsBackdropFilter: true }));
    const root = document.documentElement;
    expect(root.getAttribute("data-blur")).toBe("on");
    expect(root.getAttribute("data-render-mode")).toBe("2d");
    expect(root.hasAttribute("data-reduced-motion")).toBe(false);
  });

  it("stamps data-blur=off and reduced-motion under reduced-motion", () => {
    applyBootAttributes(document, snapshot({ supportsBackdropFilter: true, prefersReducedMotion: true }));
    const root = document.documentElement;
    expect(root.getAttribute("data-blur")).toBe("off");
    expect(root.getAttribute("data-reduced-motion")).toBe("on");
  });
});

describe("initPlatform", () => {
  it("returns a capability snapshot and stamps the root without throwing", () => {
    const caps = initPlatform(document);
    expect(caps).toHaveProperty("hasWebGL");
    expect(["on", "off"]).toContain(document.documentElement.getAttribute("data-blur"));
  });

  it("is a no-op on the DOM when document is undefined", () => {
    expect(() => initPlatform(undefined)).not.toThrow();
  });
});

describe("runCoreGateAndApply — Core-3D gate probe → resolver (Req 20.2/20.3)", () => {
  const capable = (): CapabilitySnapshot => ({
    webglTier: "webgl2",
    hasWebGL: true,
    prefersReducedMotion: false,
    supportsBackdropFilter: true,
    probe: null,
  });
  const passingProbe = (): ProbeResult => ({
    interactionFrameMs: 16,
    interactionFps: 60,
    idleQuiet: true,
    nodeCount: 400,
  });

  it("enables 3D on a passing probe for a capable device", async () => {
    initCoreRenderMode(capable(), "auto");
    expect(coreRenderMode().enable3D).toBe(false); // 2D until the gate passes
    await runCoreGateAndApply(async () => passingProbe());
    expect(coreRenderMode().enable3D).toBe(true);
  });

  it("stays on the permanent 2D path when the probe is null (WebGL absent)", async () => {
    initCoreRenderMode(capable(), "auto");
    await runCoreGateAndApply(async () => null);
    expect(coreRenderMode().enable3D).toBe(false);
    expect(coreRenderMode().triggers).toContain("failed-gate");
  });

  it("treats a probe error as a failed gate (2D)", async () => {
    initCoreRenderMode(capable(), "auto");
    await runCoreGateAndApply(async () => {
      throw new Error("probe blew up");
    });
    expect(coreRenderMode().enable3D).toBe(false);
  });
});
