/**
 * Presence Onboarding — pure, deterministic logic (design.md §17 "Onboarding",
 * Requirement 19.1/19.2/19.3).
 *
 * The cross-page cascade replaces any old application-shaped onboarding with the
 * presence model's first-run teaching (design §17):
 *   • a one-time **Core whisper** — "This is KRIA. Talk, type, or click me.";
 *   • a one-time **Orbit capability reveal** (memory / automation / desktop /
 *     local) shown the first time the user engages and the Orbit lights;
 *   • a one-time **Hidden Dock discovery hint** (⌘K / screen edge).
 *
 * It is deliberately **not a tour** and it **never repeats** (design §17): each
 * hint is an independent, in-context one-time cue, not a forced sequential
 * walkthrough. Permanence reuses the EXISTING one-time coach-hint ledger in
 * `adaptive/presentationRanking` (`shouldShowCoachHint` / `retireCoachHint`,
 * persisted in localStorage) — the onboarding invents no new persistence.
 *
 * This module is pure: it decides WHICH hints qualify given a "retired"
 * predicate and the live homepage context. It performs no side effects, no
 * storage writes, no navigation, and never sends/executes or writes `coreStore`
 * (KRIA runtime-authority invariant / guardrails.md). The component
 * (`PresenceOnboarding.tsx`) is thin presentation over these helpers, so the
 * correctness properties (never re-show a retired hint; Orbit reveal only once
 * engaged; canonical order; terminates) are unit- and property-testable in
 * isolation.
 */

/** The three first-run teaching moments (design §17). */
export type OnboardingHintId = "core-whisper" | "orbit-reveal" | "dock-hint";

/**
 * Coach-hint ledger ids (the durable one-time keys in
 * `adaptive/presentationRanking`). Namespaced so they never collide with the
 * adaptive-ranking zones' feature ids.
 */
export const ONBOARDING_COACH_IDS: Readonly<Record<OnboardingHintId, string>> = {
  "core-whisper": "home.onboarding.core-whisper",
  "orbit-reveal": "home.onboarding.orbit-reveal",
  "dock-hint": "home.onboarding.dock-hint",
} as const;

/** The capabilities the one-time Orbit reveal names (design §17). */
export const ORBIT_REVEAL_CAPABILITIES = [
  "Memory",
  "Automation",
  "Desktop",
  "Local",
] as const;

export interface OnboardingHint {
  readonly id: OnboardingHintId;
  /** The durable one-time ledger key for this hint. */
  readonly coachId: string;
  /** The single calm sentence KRIA "says" for this teaching moment. */
  readonly message: string;
  /** Optional short supporting detail (never a paragraph — one calm line). */
  readonly detail?: string;
}

/**
 * The canonical hint definitions in canonical order (Core → Orbit → Dock). The
 * copy is truthful and generic — it never fabricates personalization (Req 24.6)
 * and describes only what the presence homepage genuinely offers.
 */
export const ONBOARDING_HINTS: readonly OnboardingHint[] = [
  {
    id: "core-whisper",
    coachId: ONBOARDING_COACH_IDS["core-whisper"],
    message: "This is KRIA. Talk, type, or click me.",
  },
  {
    id: "orbit-reveal",
    coachId: ONBOARDING_COACH_IDS["orbit-reveal"],
    message: "The light around me is what I can reach.",
    detail: ORBIT_REVEAL_CAPABILITIES.join(" · "),
  },
  {
    id: "dock-hint",
    coachId: ONBOARDING_COACH_IDS["dock-hint"],
    message: "Press ⌘K, or reach the left edge, to move around.",
  },
] as const;

/** Look a hint definition up by id (exhaustive over {@link OnboardingHintId}). */
export function onboardingHint(id: OnboardingHintId): OnboardingHint {
  const hint = ONBOARDING_HINTS.find((candidate) => candidate.id === id);
  // Total by construction: every OnboardingHintId has a definition above.
  return hint!;
}

/** Live homepage context that gates the in-context (non-Core) hints. */
export interface OnboardingContext {
  /**
   * Whether the Contextual Orbit is currently engaged/lit. The Orbit reveal is
   * a one-time cue shown the FIRST time the user engages and the ring appears —
   * it is never shown at rest (Orbit is absent at rest; guardrails.md).
   */
  readonly orbitEngaged: boolean;
}

/**
 * Predicate: does a hint still need showing? A hint qualifies only while it has
 * NOT been retired (one-time), and, for the in-context hints, while its context
 * gate is satisfied:
 *   • `core-whisper` — always eligible on first run (anchors the Core).
 *   • `orbit-reveal` — only once the Orbit is engaged (never at rest).
 *   • `dock-hint`    — always eligible on first run (teaches navigation).
 *
 * `isRetired` is injected (the live layer passes `shouldShowCoachHint`'s
 * negation) so this stays pure and testable.
 */
export function hintQualifies(
  id: OnboardingHintId,
  isRetired: (coachId: string) => boolean,
  ctx: OnboardingContext,
): boolean {
  if (isRetired(ONBOARDING_COACH_IDS[id])) return false;
  if (id === "orbit-reveal") return ctx.orbitEngaged;
  return true;
}

/**
 * Resolve every onboarding hint to show right now, in canonical order. The
 * result is always a (possibly empty) SUBSET of {@link ONBOARDING_HINTS} with no
 * retired hint and no duplicates — so once every hint is retired the onboarding
 * renders nothing and never returns (design §17 "never repeats").
 *
 * Pure: derived solely from the injected `isRetired` predicate + context.
 */
export function visibleOnboardingHints(
  isRetired: (coachId: string) => boolean,
  ctx: OnboardingContext,
): readonly OnboardingHint[] {
  return ONBOARDING_HINTS.filter((hint) => hintQualifies(hint.id, isRetired, ctx));
}

/**
 * Whether onboarding is fully complete (every hint retired). When true the
 * component renders nothing — the first-run teaching never repeats.
 */
export function onboardingComplete(isRetired: (coachId: string) => boolean): boolean {
  return ONBOARDING_HINTS.every((hint) => isRetired(hint.coachId));
}
