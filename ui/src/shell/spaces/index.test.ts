import { describe, it, expect } from "vitest";
import { SPACE_COMPONENTS, SPACE_META } from "./index";
import ConverseSpace from "./ConverseSpace";
import type { Space } from "../router";

/**
 * Load-strategy guard for the Space registry (task 7.4; IU-08; UIE-H-003).
 *
 * design.md §12 / §24.6: "Converse remains eager; the other six remain lazy
 * unless separate performance evidence approves change." Req 2.1 (instant open,
 * no lazy chunk on palette open) and Req 16 (startup budget: only shell +
 * Converse in the initial bundle) both depend on this split holding.
 *
 * This is a GUARD, not a behavior change: it pins the eager/lazy split so any
 * accidental drift (Converse made lazy, or a lazy Space made eager) fails CI.
 *
 * How the split is detected without loading chunks: solid-js `lazy()` returns a
 * component wrapper that carries a `.preload` function; an eagerly-imported
 * component is a plain function with no `.preload`. So:
 *   - Converse must be the same module reference as the static import AND have
 *     no `.preload` (eager).
 *   - The other six must each expose a `.preload` function (lazy wrapper).
 *
 * Validates: Requirements 2.1, 16
 */

const EAGER_SPACE: Space = "converse";
const LAZY_SPACES: Space[] = [
  "memory",
  "automations",
  "capabilities",
  "machines",
  "observatory",
  "settings",
];

type WithPreload = { preload?: unknown };

describe("SPACE_COMPONENTS load strategy (task 7.4) — Converse eager, six lazy", () => {
  it("registers exactly the seven canonical Spaces", () => {
    expect(Object.keys(SPACE_COMPONENTS).sort()).toEqual(
      [EAGER_SPACE, ...LAZY_SPACES].sort(),
    );
    // SPACE_META must cover the same set (Dock parity).
    expect(Object.keys(SPACE_META).sort()).toEqual(
      Object.keys(SPACE_COMPONENTS).sort(),
    );
  });

  it("Converse is eagerly imported (static module reference, no lazy wrapper)", () => {
    // Same reference as the top-level static import => in the initial bundle.
    expect(SPACE_COMPONENTS[EAGER_SPACE]).toBe(ConverseSpace);
    // Eager component is a plain function with no lazy `.preload` hook.
    expect((SPACE_COMPONENTS[EAGER_SPACE] as WithPreload).preload).toBeUndefined();
  });

  it("the other six Spaces are lazy() chunks (deferred, expose preload)", () => {
    for (const space of LAZY_SPACES) {
      const component = SPACE_COMPONENTS[space] as WithPreload;
      expect(
        typeof component.preload,
        `${space} must be a lazy() chunk (has .preload)`,
      ).toBe("function");
      // A lazy wrapper is never the eager Converse module.
      expect(SPACE_COMPONENTS[space]).not.toBe(ConverseSpace);
    }
  });
});
