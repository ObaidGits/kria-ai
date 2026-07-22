/**
 * Bounded-text helper (task 10.7, IU-07; UIE-H-002, UIE-M-011, UIE-M-018).
 *
 * The shared seam that gives every Task-10 surface truthful BOUNDED presentation:
 * a stable set of truncation class tokens plus `boundedTitle`, which surfaces the
 * FULL value for a `title` so a visually-truncated value stays recoverable — and
 * which NEVER fabricates (blank/absent → undefined, via the shared `nonEmpty`
 * omission discipline).
 */
import { describe, it, expect } from "vitest";
import { BOUNDED, BOUNDED_CLAMP_2, BOUNDED_CLAMP_3, boundedTitle } from "./boundedText";

describe("boundedText — shared bounded presentation seam (task 10.7)", () => {
  it("exposes the shared bounded-text class tokens", () => {
    expect(BOUNDED).toBe("kria-bounded");
    expect(BOUNDED_CLAMP_2).toBe("kria-bounded--2");
    expect(BOUNDED_CLAMP_3).toBe("kria-bounded--3");
  });

  it("returns the full value so a truncated value is recoverable via title", () => {
    const long =
      "an-extremely-long-model-provider-name-that-would-otherwise-overflow-the-lane";
    expect(boundedTitle(long)).toBe(long);
  });

  it("trims surrounding whitespace (matches the nonEmpty discipline)", () => {
    expect(boundedTitle("  Model name  ")).toBe("Model name");
  });

  it("never fabricates a title for an absent / blank value", () => {
    expect(boundedTitle(undefined)).toBeUndefined();
    expect(boundedTitle(null)).toBeUndefined();
    expect(boundedTitle("")).toBeUndefined();
    expect(boundedTitle("   ")).toBeUndefined();
  });
});
