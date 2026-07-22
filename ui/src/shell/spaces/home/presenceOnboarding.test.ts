/**
 * Presence Onboarding logic — unit + property tests (design.md §17 "Onboarding",
 * Requirement 19.1/19.2/19.3).
 *
 * Unit tests pin the concrete first-run behavior (Core whisper first; Orbit
 * reveal only once engaged; Dock hint copy; completion). Property tests prove
 * the universal invariants that make onboarding "one-time, never a tour, never
 * repeats":
 *
 *   O1 never re-show retired — a retired hint is NEVER returned, for any retired
 *      set × context (design §17 "never repeats"). Validates: Requirements 19.2
 *   O2 canonical subset      — the visible list is always a duplicate-free subset
 *      of ONBOARDING_HINTS in canonical order. Validates: Requirements 19.1
 *   O3 terminates            — once every hint is retired the visible list is
 *      empty and onboardingComplete is true (never a resting placeholder).
 *      Validates: Requirements 19.2
 *   O4 orbit gate            — orbit-reveal is visible ONLY when the Orbit is
 *      engaged (never at rest). Validates: Requirements 19.1
 */
import { describe, it, expect } from "vitest";
import fc from "fast-check";

import {
  ONBOARDING_HINTS,
  ONBOARDING_COACH_IDS,
  ORBIT_REVEAL_CAPABILITIES,
  onboardingComplete,
  onboardingHint,
  visibleOnboardingHints,
  type OnboardingContext,
  type OnboardingHintId,
} from "./presenceOnboarding";

const ALL_IDS: readonly OnboardingHintId[] = ["core-whisper", "orbit-reveal", "dock-hint"];

/** A "nothing retired" predicate. */
const noneRetired = (): boolean => false;
/** Build a retired-set predicate from a set of coach ids. */
function retiredFrom(coachIds: ReadonlySet<string>): (coachId: string) => boolean {
  return (coachId: string) => coachIds.has(coachId);
}

describe("presenceOnboarding — first-run teaching (design §17, Req 19)", () => {
  it("teaches the Core first with the canonical whisper copy", () => {
    const engaged: OnboardingContext = { orbitEngaged: false };
    const first = visibleOnboardingHints(noneRetired, engaged)[0];
    expect(first.id).toBe("core-whisper");
    expect(first.message).toBe("This is KRIA. Talk, type, or click me.");
  });

  it("hides the Orbit reveal at rest and shows it once the Orbit is engaged", () => {
    const retired = retiredFrom(new Set([ONBOARDING_COACH_IDS["core-whisper"]]));
    // At rest: orbit-reveal is gated out; the dock hint is the next cue.
    const atRest = visibleOnboardingHints(retired, { orbitEngaged: false });
    expect(atRest.map((h) => h.id)).not.toContain("orbit-reveal");
    // Engaged: the orbit reveal qualifies (and, being earlier in canonical
    // order than the dock hint, is the next cue shown).
    const engaged = visibleOnboardingHints(retired, { orbitEngaged: true });
    expect(engaged[0].id).toBe("orbit-reveal");
    expect(onboardingHint("orbit-reveal").detail).toBe(
      ORBIT_REVEAL_CAPABILITIES.join(" · "),
    );
  });

  it("teaches the Hidden Dock discovery hint after earlier hints retire", () => {
    const retired = retiredFrom(
      new Set([ONBOARDING_COACH_IDS["core-whisper"], ONBOARDING_COACH_IDS["orbit-reveal"]]),
    );
    const next = visibleOnboardingHints(retired, { orbitEngaged: true })[0];
    expect(next.id).toBe("dock-hint");
    expect(next.message).toContain("⌘K");
  });

  it("renders nothing once every hint is retired (never repeats)", () => {
    const allRetired = retiredFrom(
      new Set(ALL_IDS.map((id) => ONBOARDING_COACH_IDS[id])),
    );
    expect(visibleOnboardingHints(allRetired, { orbitEngaged: true })).toHaveLength(0);
    expect(onboardingComplete(allRetired)).toBe(true);
  });

  // ── Property tests ─────────────────────────────────────────────────────────
  const retiredSetArb = fc.subarray([...ALL_IDS]).map(
    (ids) => new Set(ids.map((id) => ONBOARDING_COACH_IDS[id])),
  );
  const ctxArb: fc.Arbitrary<OnboardingContext> = fc.record({
    orbitEngaged: fc.boolean(),
  });

  it("O1: never returns a retired hint (Req 19.2)", () => {
    fc.assert(
      fc.property(retiredSetArb, ctxArb, (retiredIds, ctx) => {
        const isRetired = retiredFrom(retiredIds);
        for (const hint of visibleOnboardingHints(isRetired, ctx)) {
          expect(retiredIds.has(hint.coachId)).toBe(false);
        }
      }),
    );
  });

  it("O2: output is a duplicate-free subset of ONBOARDING_HINTS in canonical order (Req 19.1)", () => {
    const canonicalIndex = new Map(ONBOARDING_HINTS.map((h, i) => [h.id, i] as const));
    fc.assert(
      fc.property(retiredSetArb, ctxArb, (retiredIds, ctx) => {
        const visible = visibleOnboardingHints(retiredFrom(retiredIds), ctx);
        const indices = visible.map((h) => canonicalIndex.get(h.id)!);
        // Subset: every visible hint is a known hint.
        for (const hint of visible) expect(canonicalIndex.has(hint.id)).toBe(true);
        // No duplicates.
        expect(new Set(visible.map((h) => h.id)).size).toBe(visible.length);
        // Canonical order preserved (strictly increasing indices).
        for (let i = 1; i < indices.length; i += 1) {
          expect(indices[i]).toBeGreaterThan(indices[i - 1]);
        }
      }),
    );
  });

  it("O3: terminates — all-retired ⇒ empty & complete (Req 19.2)", () => {
    fc.assert(
      fc.property(ctxArb, (ctx) => {
        const allRetired = retiredFrom(new Set(ALL_IDS.map((id) => ONBOARDING_COACH_IDS[id])));
        expect(visibleOnboardingHints(allRetired, ctx)).toHaveLength(0);
        expect(onboardingComplete(allRetired)).toBe(true);
      }),
    );
  });

  it("O4: orbit-reveal is visible only when the Orbit is engaged (Req 19.1)", () => {
    fc.assert(
      fc.property(retiredSetArb, fc.boolean(), (retiredIds, orbitEngaged) => {
        const withoutOrbit = new Set(retiredIds);
        withoutOrbit.delete(ONBOARDING_COACH_IDS["orbit-reveal"]);
        const visible = visibleOnboardingHints(retiredFrom(withoutOrbit), { orbitEngaged });
        const hasOrbit = visible.some((h) => h.id === "orbit-reveal");
        // Not retired here, so visibility is governed purely by the engage gate.
        expect(hasOrbit).toBe(orbitEngaged);
      }),
    );
  });
});
