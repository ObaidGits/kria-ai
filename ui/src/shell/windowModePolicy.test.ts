import { describe, expect, it } from "vitest";
import { ALL_SPACES } from "./router";
import { SPACE_MODE_MATRIX, spaceComposition } from "./windowModePolicy";

/** Validates: Requirements 15.1, 15.2, 13.1 */
describe("Space×window-mode composition matrix", () => {
  it("defines one intentional composition for every Space in every mode", () => {
    for (const mode of ["mini", "standard", "immersive", "companion"] as const) {
      expect(Object.keys(SPACE_MODE_MATRIX[mode]).sort()).toEqual([...ALL_SPACES].sort());
      for (const space of ALL_SPACES) expect(spaceComposition(space, mode)).toBeTruthy();
    }
  });

  it("curates Mini and expands Immersive instead of reusing Standard", () => {
    for (const space of ALL_SPACES) {
      expect(spaceComposition(space, "mini")).not.toBe("full");
      expect(spaceComposition(space, "immersive")).not.toBe("full");
      expect(spaceComposition(space, "standard")).toBe("full");
    }
  });

  // Task 8.7 (design.md §29): Companion is the detached ember only — every Space
  // collapses to the SAME distinct `companion-ember` composition, not a reuse of
  // Mini's per-Space rows and never `full`.
  it("collapses every Space to the distinct companion-ember composition in Companion mode", () => {
    for (const space of ALL_SPACES) {
      expect(spaceComposition(space, "companion")).toBe("companion-ember");
      expect(spaceComposition(space, "companion")).not.toBe("full");
      // Distinct from Mini's curated per-Space rows (no placeholder mirroring).
      expect(spaceComposition(space, "companion")).not.toBe(spaceComposition(space, "mini"));
    }
  });
});
