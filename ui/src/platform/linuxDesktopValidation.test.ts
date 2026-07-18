import { describe, expect, it } from "vitest";
import baseCss from "../styles/base.css?raw";
import fontsCss from "../styles/fonts.css?raw";
import {
  canvasBackingStoreSize,
  LINUX_DESKTOP_MATRIX,
} from "./linuxDesktopValidation";

describe("Linux desktop validation matrix (Req 18.1/18.2/18.4/18.5)", () => {
  it("keeps all GNOME/KDE and Wayland/X11 cells explicit", () => {
    expect(LINUX_DESKTOP_MATRIX).toEqual([
      { desktop: "GNOME", session: "Wayland" },
      { desktop: "GNOME", session: "X11" },
      { desktop: "KDE", session: "Wayland" },
      { desktop: "KDE", session: "X11" },
    ]);
  });

  it("owns the root font stack and ships local font faces", () => {
    expect(baseCss).toContain("--font-sans: var(--font-family-text)");
    expect(baseCss).toContain("--font-mono: var(--font-family-mono)");
    expect(baseCss).toMatch(/body\s*\{[^}]*font-family:\s*var\(--font-sans\)/s);
    expect(fontsCss).toContain('font-family: "Space Grotesk"');
    expect(fontsCss).toContain('font-family: "IBM Plex Sans"');
    expect(fontsCss).toContain('font-family: "JetBrains Mono"');
    expect(fontsCss).toContain("url(\"/fonts/");
  });

  it("uses dynamic viewport sizing with a legacy fallback", () => {
    expect(baseCss).toMatch(/height:\s*100vh;\s*height:\s*100dvh;/);
    expect(baseCss).toContain("text-size-adjust: 100%");
  });
});

describe("fractional-scale canvas backing store (Req 18.5)", () => {
  it.each([1, 1.25, 1.5, 1.75, 2])("maps CSS pixels to integer device pixels at DPR %s", (dpr) => {
    const size = canvasBackingStoreSize(640, 480, dpr);
    expect(size.cssWidth).toBe(640);
    expect(size.cssHeight).toBe(480);
    expect(size.pixelWidth).toBe(Math.round(640 * dpr));
    expect(size.pixelHeight).toBe(Math.round(480 * dpr));
  });

  it("preserves proportions across representative mixed-DPI dimensions", () => {
    for (const width of [1, 137, 640, 1919]) {
      for (const height of [1, 91, 480, 1079]) {
        for (const dpr of [1, 1.25, 1.5, 1.75, 2, 2.5]) {
          const size = canvasBackingStoreSize(width, height, dpr);
          expect(size.cssWidth / size.cssHeight).toBe(width / height);
          expect(Math.abs(size.pixelWidth - width * dpr)).toBeLessThanOrEqual(0.5);
          expect(Math.abs(size.pixelHeight - height * dpr)).toBeLessThanOrEqual(0.5);
        }
      }
    }
  });

  it("fails safe for invalid dimensions and caps excessive DPR", () => {
    expect(canvasBackingStoreSize(0, Number.NaN, Number.POSITIVE_INFINITY)).toEqual({
      cssWidth: 1, cssHeight: 1, pixelWidth: 1, pixelHeight: 1, devicePixelRatio: 1,
    });
    expect(canvasBackingStoreSize(10, 10, 9).devicePixelRatio).toBe(4);
  });
});
