/**
 * Homepage design guardrails — runtime validators (release-blocking).
 *
 * These are the executable counterparts of the canonical, published checklist
 * in `.kiro/specs/homepage-presence-redesign/guardrails.md` ("Never" section).
 * A guardrail violation is a RELEASE BLOCKER, not a preference (Req 30.2).
 *
 * This module enforces the four guardrails wired by task 0.5 that are cheap and
 * reliable to check on a `FocusFrame`-shaped value at runtime:
 *   1. Single Adaptive Context Surface — never two ACS at once.
 *   2. ≤ 3 contextual chips.
 *   3. `coreHint` is advisory only — see the static lint for write-back
 *      enforcement (`scripts/guardrail-lint.mjs`); here we simply keep the field
 *      typed as advisory and never surface it as authoritative.
 *   4. (accent-on-room-base is a CSS concern, enforced by the static lint.)
 *
 * The Focus engine (`homeFocusStore`, task 3.x) will produce the real
 * `FocusFrame`; this module defines the minimal structural shape it must honor
 * so the guardrails can be asserted the moment that frame exists. Keeping the
 * shape structural (not a hard import) avoids a build-order dependency while the
 * Focus engine is still being built.
 *
 * This module ALSO enforces the **resting-calm** guardrail (task 1.4, Req 1.5):
 * the resting homepage is Core + optional greeting only — it must NEVER render
 * placeholder widgets, empty cards, stat tiles, charts, an activity feed, or an
 * empty Adaptive Context Surface (guardrails.md "Never add dashboard widgets…",
 * regression alarms "renders a placeholder/empty card when idle" and "ACS shows
 * … an empty box instead of dissolving"). Because those are *rendered-DOM*
 * regressions, they are checked at runtime here — the static lint
 * (`scripts/guardrail-lint.mjs`) owns the *source-level* guardrails (coreStore
 * write-back, accent-on-room-base) and does not overlap.
 *
 * Requirements: 30.2 (guardrails are release-blocking), 8.1/8.4 (single ACS),
 * 5.1 (≤3 chips), 30.3 (coreHint advisory only), 1.5 (resting calm — no filler).
 */

/** Maximum number of contextual chips the homepage may show (Req 5.1). */
export const MAX_CHIPS = 3;

/** A contextual chip (structural subset of design §5 `Chip`). */
export interface GuardrailChip {
  id: string;
  label: string;
  kind: "stage" | "route";
}

/**
 * Structural subset of the Focus engine's `FocusFrame` (design §13 / §5) that
 * the guardrails inspect. `acs` is a single optional subject; passing an array
 * is itself a violation (there must never be two surfaces).
 */
export interface GuardrailFocusFrame {
  /** Adaptive Context Surface — at most ONE. `null`/absent means it dissolved. */
  acs?: unknown | null;
  /** Ranked next-action chips — length must be ≤ MAX_CHIPS. */
  chips?: readonly GuardrailChip[];
  /** Advisory Core hint only; never authoritative, never written to coreStore. */
  coreHint?: string;
}

/** A single guardrail violation. `rule` matches a guardrails.md "Never" item. */
export interface GuardrailViolation {
  rule:
    | "single-acs"
    | "chip-budget"
    | "corehint-advisory"
    | "resting-filler"
    | "empty-surface"
    | "duplicate-capability-awareness";
  message: string;
}

/** Thrown by {@link assertFocusFrame} when a release-blocking guardrail fails. */
export class GuardrailViolationError extends Error {
  readonly violations: readonly GuardrailViolation[];
  constructor(violations: readonly GuardrailViolation[]) {
    super(
      `Homepage guardrail violation (release-blocker): ${violations
        .map((v) => v.message)
        .join("; ")}`,
    );
    this.name = "GuardrailViolationError";
    this.violations = violations;
  }
}

/**
 * Single-ACS guardrail (guardrails.md: "Never render more than one Adaptive
 * Context Surface"). An array-valued `acs` with more than one entry is the
 * classic two-surface regression.
 */
export function checkSingleAcs(frame: GuardrailFocusFrame): GuardrailViolation | null {
  const { acs } = frame;
  if (Array.isArray(acs) && acs.length > 1) {
    return {
      rule: "single-acs",
      message: `at most one Adaptive Context Surface is allowed, got ${acs.length}`,
    };
  }
  return null;
}

/**
 * Chip-budget guardrail (guardrails.md: cognitive-load budget; Req 5.1 ≤3).
 */
export function checkChipBudget(frame: GuardrailFocusFrame): GuardrailViolation | null {
  const count = frame.chips?.length ?? 0;
  if (count > MAX_CHIPS) {
    return {
      rule: "chip-budget",
      message: `at most ${MAX_CHIPS} contextual chips are allowed, got ${count}`,
    };
  }
  return null;
}

/**
 * `coreHint` advisory-only guardrail (Req 30.3). The value may exist as a hint,
 * but it must be a plain advisory string — never an object masquerading as an
 * authoritative coreStore command. This complements the static write-back lint.
 */
export function checkCoreHintAdvisory(frame: GuardrailFocusFrame): GuardrailViolation | null {
  if (frame.coreHint !== undefined && typeof frame.coreHint !== "string") {
    return {
      rule: "corehint-advisory",
      message: "coreHint must be an advisory string, never an authoritative command",
    };
  }
  return null;
}

/** Run every FocusFrame guardrail and collect violations (empty = clean). */
export function checkFocusFrame(frame: GuardrailFocusFrame): GuardrailViolation[] {
  const checks = [checkSingleAcs, checkChipBudget, checkCoreHintAdvisory];
  const violations: GuardrailViolation[] = [];
  for (const check of checks) {
    const violation = check(frame);
    if (violation) violations.push(violation);
  }
  return violations;
}

/**
 * Assert a FocusFrame honors every release-blocking guardrail. Throws
 * {@link GuardrailViolationError} on any violation. Intended for the Focus
 * engine's presentation boundary + tests (fail loud, never ship a violation).
 */
export function assertFocusFrame(frame: GuardrailFocusFrame): void {
  const violations = checkFocusFrame(frame);
  if (violations.length > 0) throw new GuardrailViolationError(violations);
}

// ─── Resting-calm guardrails (task 1.4, Req 1.5) ─────────────────────────────
//
// These operate on the *rendered* homepage DOM (a root element), not a
// FocusFrame, because the regressions they catch are visual: a placeholder
// widget, an empty card, a stat tile / chart / activity feed, or an Adaptive
// Context Surface that renders an empty box instead of dissolving. At rest the
// homepage is Core + optional greeting only (design §1); everything else is
// ambient/on-demand, never a standing panel.

/**
 * Selectors for dashboard-style filler that must NEVER appear on the resting
 * homepage (guardrails.md "Never add dashboard widgets, stat tiles, charts, or
 * an activity feed"). These are intentional *contract* markers: any homepage
 * element that is one of these things must carry the matching `data-*` marker,
 * so the guardrail deterministically catches the regression regardless of the
 * element's styling/classes.
 */
export const RESTING_FILLER_SELECTORS = [
  "[data-widget]",
  "[data-stat-tile]",
  "[data-chart]",
  "[data-activity-feed]",
  "[data-dashboard-card]",
  "[data-placeholder]",
] as const;

/**
 * Marker attribute for a surface that MUST recede/dissolve (not render an empty
 * container) when it has no subject (Req 8.3, guardrails.md regression alarm
 * "the ACS shows … an empty box instead of dissolving"). The Adaptive Context
 * Surface (task 4.2) is the primary carrier: when empty it must be REMOVED from
 * the DOM, never rendered as a blank box.
 */
export const DISSOLVES_WHEN_EMPTY_ATTR = "data-dissolves-when-empty";

/** Minimal DOM surface this guard needs — keeps it usable in any DOM env. */
interface QueryableRoot {
  querySelectorAll(selectors: string): ArrayLike<Element>;
}

/**
 * Find dashboard-style filler present anywhere under `root`. Any match is a
 * resting-calm violation (Req 1.5): the resting homepage carries no widgets,
 * stat tiles, charts, activity feeds, dashboard cards, or placeholders.
 */
export function findRestingFiller(root: QueryableRoot): GuardrailViolation[] {
  const violations: GuardrailViolation[] = [];
  for (const selector of RESTING_FILLER_SELECTORS) {
    const count = root.querySelectorAll(selector).length;
    if (count > 0) {
      violations.push({
        rule: "resting-filler",
        message: `resting homepage must render no filler, found ${count} \`${selector}\` element(s)`,
      });
    }
  }
  return violations;
}

/**
 * An element that declares it dissolves-when-empty is "empty" when it has no
 * non-whitespace text content and no element children — or when it explicitly
 * marks itself `data-empty="true"`. Such an element present in the DOM is a
 * violation: it should have been removed, not rendered as a blank box.
 */
function isEmptySurface(element: Element): boolean {
  if (element.getAttribute("data-empty") === "true") return true;
  const hasChildElements = element.childElementCount > 0;
  const hasText = (element.textContent ?? "").trim().length > 0;
  return !hasChildElements && !hasText;
}

/**
 * Find dissolvable surfaces that rendered *empty* instead of dissolving
 * (Req 8.3). A present-but-empty `[data-dissolves-when-empty]` element is the
 * classic "empty ACS box" regression.
 */
export function findEmptyStandingSurfaces(root: QueryableRoot): GuardrailViolation[] {
  const violations: GuardrailViolation[] = [];
  const surfaces = root.querySelectorAll(`[${DISSOLVES_WHEN_EMPTY_ATTR}]`);
  for (let i = 0; i < surfaces.length; i += 1) {
    const surface = surfaces[i];
    if (isEmptySurface(surface)) {
      violations.push({
        rule: "empty-surface",
        message:
          "a dissolvable surface (e.g. the Adaptive Context Surface) rendered empty; it must dissolve, never show an empty box",
      });
    }
  }
  return violations;
}

/**
 * Run every resting-calm guardrail against a rendered homepage `root`
 * (empty = calm). Combines the no-filler and no-empty-box checks (Req 1.5/8.3).
 */
export function checkRestingCalm(root: QueryableRoot): GuardrailViolation[] {
  return [...findRestingFiller(root), ...findEmptyStandingSurfaces(root)];
}

// ─── Single capability-awareness system (task 6.2, Req 6.5) ──────────────────
//
// The Contextual Orbit SUBSUMES the former "capability sparks" concept: there
// must be EXACTLY ONE capability-awareness system on the homepage, and no
// duplicate/legacy sparks UI (guardrails.md; Req 6.5). Any capability-awareness
// surface marks itself `[data-capability-awareness]` (the Orbit uses
// `"orbit"`); a second such surface, or any legacy sparks marker, is a
// release-blocking violation.

/** Marker attribute a capability-awareness surface must carry (Req 6.5). */
export const CAPABILITY_AWARENESS_ATTR = "data-capability-awareness";

/**
 * Selectors for a legacy/duplicate "capability sparks" UI that must NEVER exist
 * — the Orbit is the single capability-awareness system (Req 6.5). Any match is
 * a violation regardless of styling.
 */
export const LEGACY_SPARKS_SELECTORS = [
  "[data-capability-sparks]",
  "[data-sparks]",
  "[data-spark]",
  ".capability-sparks",
  ".kria-sparks",
] as const;

/**
 * Assert there is exactly one capability-awareness system and no legacy sparks
 * UI (Req 6.5). Returns violations (empty = clean):
 *   • more than one `[data-capability-awareness]` region, OR
 *   • any legacy sparks marker present.
 * When the Orbit is at rest (unmounted) there are zero regions, which is fine —
 * the guardrail forbids a *second* system, not the calm absence of the first.
 */
export function checkSingleCapabilityAwareness(root: QueryableRoot): GuardrailViolation[] {
  const violations: GuardrailViolation[] = [];

  const systems = root.querySelectorAll(`[${CAPABILITY_AWARENESS_ATTR}]`).length;
  if (systems > 1) {
    violations.push({
      rule: "duplicate-capability-awareness",
      message: `exactly one capability-awareness system is allowed, found ${systems}`,
    });
  }

  for (const selector of LEGACY_SPARKS_SELECTORS) {
    const count = root.querySelectorAll(selector).length;
    if (count > 0) {
      violations.push({
        rule: "duplicate-capability-awareness",
        message: `legacy "capability sparks" UI is forbidden (Orbit is the single system), found ${count} \`${selector}\` element(s)`,
      });
    }
  }

  return violations;
}

/**
 * Assert the resting homepage is calm (Core + optional greeting only): no
 * placeholder widgets, empty cards, stat tiles, charts, activity feeds, or an
 * empty Adaptive Context Surface. Throws {@link GuardrailViolationError} on any
 * violation (release-blocking — fail loud, never ship filler at rest).
 */
export function assertRestingCalm(root: QueryableRoot): void {
  const violations = checkRestingCalm(root);
  if (violations.length > 0) throw new GuardrailViolationError(violations);
}
