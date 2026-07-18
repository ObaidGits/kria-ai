import { describe, expect, it } from "vitest";
import motionCss from "../styles/motion.css?raw";

const cssFiles = import.meta.glob("../**/*.css", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

describe("motion budget properties", () => {
  it("caps every CSS transition at 200ms, or 400ms in deliberate scope", () => {
    for (const [file, css] of Object.entries(cssFiles)) {
      for (const match of css.matchAll(/transition(?:-duration)?\s*:\s*([^;]+)/g)) {
        const declaration = match[1];
        const selectorStart = css.lastIndexOf("}", match.index) + 1;
        const selectorEnd = css.lastIndexOf("{", match.index);
        const selector = css.slice(selectorStart, selectorEnd);
        const deliberate = selector.includes('[data-motion-scope="deliberate"]');
        const cap = deliberate ? 400 : 200;
        if (declaration.includes("--motion-duration-slow")) expect(deliberate).toBe(true);
        for (const duration of declaration.matchAll(/(\d*\.?\d+)\s*(ms|s)\b/g)) {
          const milliseconds = Number(duration[1]) * (duration[2] === "s" ? 1000 : 1);
          expect(milliseconds, `${file}: ${declaration}`).toBeLessThanOrEqual(cap);
        }
      }
    }
  });

  it("defines attribute and OS-level kill-switches for all CSS motion", () => {
    const css = motionCss;
    expect(css).toContain(':root[data-reduced-motion="on"] *');
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css.match(/animation:\s*none\s*!important/g)?.length).toBeGreaterThanOrEqual(2);
    expect(css.match(/transition:\s*none\s*!important/g)?.length).toBeGreaterThanOrEqual(2);
  });
});