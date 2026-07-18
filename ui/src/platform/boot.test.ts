import { afterEach, describe, expect, it } from "vitest";
import { applyBootAttributes, decideBlurTreatment, disposePlatform, initPlatform } from "./boot";
import type { CapabilitySnapshot } from "./capabilities";

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
