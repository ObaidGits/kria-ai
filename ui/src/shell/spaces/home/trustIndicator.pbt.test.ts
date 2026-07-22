/**
 * Property-based tests for the Trust projection (design.md §9, Requirement 9).
 *
 * These verify the behavior-first trust INVARIANTS hold across every possible
 * input (all connectivity states × all Core states), using fast-check:
 *
 *   P1 stays-lit-offline  — `resolveTrustView(...).lit === true` for EVERY input
 *                           (offline never shows an error/unlit state). (Req 9.1)
 *   P2 muted              — `tone === "muted"` for EVERY input (never emerald).
 *                           (Req 9.2)
 *   P3 bounded reach cue  — `reach === isDesktopAction(coreState)`: a reach cue
 *                           appears iff KRIA is acting on the device, and it is a
 *                           pure function of the current Core state (bounded to
 *                           the action, not free-running). (Req 9.1)
 *
 * Validates: Requirements 9.1, 9.2, 9.3
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import {
  resolveTrustView,
  isDesktopAction,
  DESKTOP_ACTION_STATES,
  TRUST_SETTINGS_ROUTE,
} from "./trustIndicator";
import type { CoreState } from "../../../stores/coreStore";

/** Every Core state — the full input space for the reach projection. */
const ALL_CORE_STATES: readonly CoreState[] = [
  "idle",
  "listening",
  "thinking",
  "planning",
  "speaking",
  "responding",
  "acting",
  "running-automation",
  "watching",
  "remembering",
  "reflecting",
  "learning",
  "waiting",
  "blocked",
  "error",
  "recovering",
];

/** Smart generator: connectivity × the full Core-state space. */
const arbInput = fc.record({
  online: fc.boolean(),
  coreState: fc.constantFrom(...ALL_CORE_STATES),
});

describe("trustIndicator projection — properties (Req 9)", () => {
  it("P1: stays lit for every input — offline is never an error/unlit state (Req 9.1)", () => {
    fc.assert(
      fc.property(arbInput, (input) => {
        expect(resolveTrustView(input).lit).toBe(true);
      }),
    );
  });

  it("P2: the confirmation is always muted, never emerald (Req 9.2)", () => {
    fc.assert(
      fc.property(arbInput, (input) => {
        expect(resolveTrustView(input).tone).toBe("muted");
      }),
    );
  });

  it("P3: the reach cue appears iff KRIA is acting on the device (Req 9.1)", () => {
    fc.assert(
      fc.property(arbInput, (input) => {
        const view = resolveTrustView(input);
        expect(view.reach).toBe(isDesktopAction(input.coreState));
        // Reach is purely a function of the Core state — connectivity never
        // changes it (bounded to the action, not the network).
        expect(view.reach).toBe(DESKTOP_ACTION_STATES.has(input.coreState));
      }),
    );
  });

  it("P3-corollary: connectivity does not affect the reach cue (Req 9.1)", () => {
    fc.assert(
      fc.property(fc.constantFrom(...ALL_CORE_STATES), (coreState) => {
        const onlineView = resolveTrustView({ online: true, coreState });
        const offlineView = resolveTrustView({ online: false, coreState });
        expect(onlineView.reach).toBe(offlineView.reach);
      }),
    );
  });

  it("connectivity word tracks the online flag but never becomes an error (Req 9.1)", () => {
    fc.assert(
      fc.property(arbInput, (input) => {
        const view = resolveTrustView(input);
        expect(view.connectivity).toBe(input.online ? "online" : "offline");
        // Both connectivity states remain lit + muted (no error branch exists).
        expect(view.lit).toBe(true);
        expect(view.tone).toBe("muted");
      }),
    );
  });

  it("the routing target is always the Memory & Privacy Settings group (Req 9.3)", () => {
    // Invariant, input-independent: detail routes to Settings only.
    expect(TRUST_SETTINGS_ROUTE).toEqual({ space: "settings", segment: "memory-privacy" });
  });
});
