import { describe, expect, it } from "vitest";
import { ALL_SPACES } from "./router";
import { SPACE_MODE_MATRIX, spaceComposition } from "./windowModePolicy";

/** Validates: Requirements 15.1, 15.2 */
describe("Space×window-mode composition matrix", () => {
  it("defines one intentional composition for every Space in every mode", () => {
    for (const mode of ["compact", "standard", "immersive"] as const) {
      expect(Object.keys(SPACE_MODE_MATRIX[mode]).sort()).toEqual([...ALL_SPACES].sort());
      for (const space of ALL_SPACES) expect(spaceComposition(space, mode)).toBeTruthy();
    }
  });

  it("curates Compact and expands Immersive instead of reusing Standard", () => {
    for (const space of ALL_SPACES) {
      expect(spaceComposition(space, "compact")).not.toBe("full");
      expect(spaceComposition(space, "immersive")).not.toBe("full");
      expect(spaceComposition(space, "standard")).toBe("full");
    }
  });
});
