import { describe, expect, it } from "vitest";
import {
  FONT_SCALE_STEPS,
  accessibilityPreferences,
  applyAccessibilityPreferences,
  normalizeFontScale,
} from "./accessibilityPreferences";

/** Validates: Requirements 17.4 */
describe("accessibility runtime preferences", () => {
  it("maps config values to document attributes used by CSS and rendering gates", () => {
    const root = document.createElement("html");
    const result = applyAccessibilityPreferences({
      ui: { high_contrast: true, reduce_motion: true, steady_lighting: true, font_scale: 1.5 },
    }, root);

    expect(result).toEqual({
      highContrast: true,
      reducedMotion: true,
      steadyLighting: true,
      fontScale: "1.5",
    });
    expect(root).toHaveAttribute("data-high-contrast", "true");
    expect(root).toHaveAttribute("data-font-scale", "1.5");
    expect(root).toHaveAttribute("data-reduced-motion", "on");
    expect(root).toHaveAttribute("data-steady-lighting", "true");
  });

  it("uses safe defaults for absent or malformed config", () => {
    expect(accessibilityPreferences({ ui: "invalid" })).toEqual({
      highContrast: false,
      reducedMotion: false,
      steadyLighting: false,
      fontScale: "1.0",
    });
  });

  it("defaults steady-lighting off and reflects it as a document attribute", () => {
    const root = document.createElement("html");
    const result = applyAccessibilityPreferences({ ui: {} }, root);
    expect(result.steadyLighting).toBe(false);
    expect(root).toHaveAttribute("data-steady-lighting", "false");
  });

  it("normalizes every numeric input to a supported bounded scale", () => {
    for (const value of [-100, 0, 0.81, 1.09, 1.39, 1.99, 100, NaN, Infinity]) {
      expect(FONT_SCALE_STEPS).toContain(normalizeFontScale(value));
    }
    expect(normalizeFontScale(1.39)).toBe("1.5");
  });
});
