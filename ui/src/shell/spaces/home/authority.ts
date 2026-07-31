/**
 * AI vs Rules vs User — Decision Authority Framework (design.md §31, Req 29).
 *
 * The executable, machine-checkable encoding of "who decides what" on the
 * homepage. This is the runtime counterpart of the permanent code-review
 * guideline in `.kiro/specs/homepage-presence-redesign/ai-rules-user-authority-framework.md`
 * and the "Authority (who decides)" section of `guardrails.md`. A violation of
 * any principle here is a RELEASE BLOCKER, not a preference (Req 30.2 posture).
 *
 * It reuses the existing enforcement points rather than rebuilding them:
 *   • {@link RiskLevel} + {@link resolvePermissionMode} (Permission UX, task 8.5)
 *     own the GREEN/YELLOW/RED presentation — this module only asks them
 *     "is this tier auto-actable?" and "does it report?".
 *   • the Focus engine (`homeFocusStore`) is a pure read-model that emits chips
 *     of kind `"stage" | "route"` — never `"send"`/`"execute"` — which this
 *     module classifies as `staged`/`route`, never `auto-execute`.
 *   • navigation is owned by the user / palette / Navigation Rail (Phase 6); the
 *     Focus engine never calls `navigate` (verified by the static guardrail
 *     lint), which this module models as `NavSource` never being `"ai"`.
 *
 * Everything here is a PURE, total function of its inputs — no clock, no store,
 * no side effects — so the framework can be exhaustively property-tested.
 *
 * Requirements: 29.1 (explicit user wins; AI never overrides navigation),
 * 29.2 (rules are deterministic/auditable), 29.3 (AI outputs staged; GREEN
 * auto-acts + reports).
 */

import type { RiskLevel } from "../../../stores/approvalStore";
import { resolvePermissionMode, type PermissionMode } from "./permissionUx";

// ─── Actors + precedence (design §31) ─────────────────────────────────────────

/** The three decision authorities. */
export type AuthorityActor = "user" | "rules" | "ai";

/**
 * Authority precedence, highest first (design §31). When more than one actor
 * lays claim to the same decision, the earliest actor in this list wins:
 *   • the User's explicit action ALWAYS wins;
 *   • deterministic Rules win over an AI suggestion;
 *   • the AI only "wins" when it is the sole claimant (a pure suggestion).
 * This ordering is the single source of truth for {@link resolveConflict}.
 */
export const AUTHORITY_PRECEDENCE: readonly AuthorityActor[] = ["user", "rules", "ai"] as const;

// ─── Decision domains + their canonical owner (design §31) ────────────────────

/** Every decision domain the framework governs, tagged with its owner below. */
export type DecisionDomain =
  // User decides (explicit, always wins) — §31 bullet 1.
  | "navigation"
  | "send"
  | "approve"
  | "opt-in"
  | "mode"
  | "pin-dismiss"
  // Rules decide (deterministic, auditable) — §31 bullet 2.
  | "ranking-precedence"
  | "interruptibility"
  | "risk-classification"
  | "layout"
  | "dwell"
  | "tier-degradation"
  // AI suggests (never auto-acts on home) — §31 bullet 3.
  | "focus-subject"
  | "chip"
  | "starter"
  | "greeting"
  | "learned-fact";

/**
 * The canonical owner of each decision domain (design §31). This table is the
 * auditable mapping a reviewer checks a new feature against: any new homepage
 * decision must slot into exactly one of these domains under the right owner.
 */
export const DECISION_OWNER: Readonly<Record<DecisionDomain, AuthorityActor>> = {
  // User (explicit, always wins).
  navigation: "user",
  send: "user",
  approve: "user",
  "opt-in": "user",
  mode: "user",
  "pin-dismiss": "user",
  // Rules (deterministic).
  "ranking-precedence": "rules",
  interruptibility: "rules",
  "risk-classification": "rules",
  layout: "rules",
  dwell: "rules",
  "tier-degradation": "rules",
  // AI (suggests, staged).
  "focus-subject": "ai",
  chip: "ai",
  starter: "ai",
  greeting: "ai",
  "learned-fact": "ai",
};

/** The set of domains owned by each actor (derived from {@link DECISION_OWNER}). */
export function domainsOwnedBy(actor: AuthorityActor): DecisionDomain[] {
  return (Object.keys(DECISION_OWNER) as DecisionDomain[]).filter(
    (d) => DECISION_OWNER[d] === actor,
  );
}

// ─── Principle 1: explicit user actions ALWAYS win (Req 29.1) ─────────────────

/** A set of actors laying claim to a single decision (a "conflict"). */
export type AuthorityClaims = Partial<Record<AuthorityActor, boolean>>;

/**
 * Resolve who wins when multiple authorities claim the same decision. Returns
 * the highest-precedence actor present (User > Rules > AI). Returns `null` only
 * when no actor claims the decision at all.
 *
 * This is the executable statement of §31 "the User's explicit action always
 * wins": for ANY claim set that includes `user`, the result is `user`.
 */
export function resolveConflict(claims: AuthorityClaims): AuthorityActor | null {
  for (const actor of AUTHORITY_PRECEDENCE) {
    if (claims[actor]) return actor;
  }
  return null;
}

// ─── Principle 5: AI never overrides navigation (Req 29.1) ────────────────────

/**
 * Where a navigation command legitimately originates. Navigation is owned by
 * the user and the surfaces the user drives directly — the Command Palette
 * (searchable authority), the Navigation Rail (deliberate), a message action, or a
 * `route` chip the user clicked. Crucially it is NEVER `"ai"`/`"focus-engine"`:
 * the AI read-model may *declare a routing target*, but only an explicit user
 * gesture executes navigation.
 */
export type NavSource =
  | "user"
  | "palette"
  | "navigation-rail"
  | "message-action"
  | "chip-route"
  | "ai"
  | "focus-engine";

/** Nav sources that are user-owned (authoritative). Never includes AI sources. */
export const USER_OWNED_NAV_SOURCES: readonly NavSource[] = [
  "user",
  "palette",
  "navigation-rail",
  "message-action",
  "chip-route",
] as const;

/** AI-side nav sources — these must NEVER drive a navigation (§31, Req 29.1). */
export const FORBIDDEN_NAV_SOURCES: readonly NavSource[] = ["ai", "focus-engine"] as const;

/** True when a navigation source is user-owned (authoritative). */
export function isNavAuthoritative(source: NavSource): boolean {
  return (USER_OWNED_NAV_SOURCES as readonly string[]).includes(source);
}

/** Thrown when the AI attempts to override navigation (release-blocking). */
export class NavigationAuthorityError extends Error {
  constructor(source: NavSource) {
    super(
      `AI must never override navigation (design §31, Req 29.1): navigation from ` +
        `source "${source}" is forbidden — navigation is owned by the user / palette / dock`,
    );
    this.name = "NavigationAuthorityError";
  }
}

/**
 * Assert a navigation command is user-owned. Throws {@link NavigationAuthorityError}
 * for an AI/focus-engine source. This is the runtime guard behind "AI never
 * overrides explicit user navigation or intent" (§31).
 */
export function assertNavAuthoritative(source: NavSource): void {
  if (!isNavAuthoritative(source)) throw new NavigationAuthorityError(source);
}

// ─── Principle 3: AI outputs are STAGED, never auto-executed (Req 29.3) ───────

/** The kinds of output the homepage AI (Focus engine) can produce. */
export type AiOutputKind =
  | "focus-subject"
  | "chip-stage"
  | "chip-route"
  | "starter"
  | "greeting"
  | "learned-fact";

/**
 * How an AI output reaches the world. The homepage AI is suggestive only, so
 * its outputs are `informational` (a Voice Line / greeting / learned fact),
 * `staged` (a reviewable draft in the Composer the user commits), or `route`
 * (a routing target the user activates). It is NEVER `auto-execute` — the AI
 * never sends or executes from a home suggestion without review (§31, Req 29.3).
 */
export type AiOutputCommitMode = "informational" | "staged" | "route" | "auto-execute";

/**
 * Map an AI output kind to how it commits. Total and exhaustive — a new output
 * kind is a compile error, not a silent `auto-execute`. No branch returns
 * `auto-execute`: that is the whole point (Req 29.3).
 */
export function aiOutputCommitMode(kind: AiOutputKind): AiOutputCommitMode {
  switch (kind) {
    case "chip-stage":
      return "staged"; // reviewable draft in the Composer; user sends.
    case "chip-route":
      return "route"; // routing target; user activates it.
    case "focus-subject":
    case "starter":
    case "greeting":
    case "learned-fact":
      return "informational"; // pure presentation; the user acts, not the AI.
  }
}

/** True iff an AI output can reach the world without explicit user review. */
export function aiOutputIsAutoExecuted(kind: AiOutputKind): boolean {
  return aiOutputCommitMode(kind) === "auto-execute";
}

// ─── Principle 4: GREEN auto-acts then reports (Req 29.3) ─────────────────────

/**
 * The single risk tier on which KRIA may auto-act: only GREEN reversible,
 * pre-permitted actions execute without asking (§31 "AI may auto-act only… on
 * GREEN"). Every other tier requires an explicit user decision (HITL).
 */
export const AUTO_ACTABLE_RISK: RiskLevel = "green";

/**
 * True iff the risk tier may be auto-acted (GREEN only). YELLOW/RED/BLACK must
 * ask the user (Permission UX). Deterministic (§31 bullet 2 / Req 29.2).
 */
export function isAutoActable(risk: RiskLevel): boolean {
  return risk === AUTO_ACTABLE_RISK;
}

/**
 * True iff an auto-acted tier must REPORT (via the Voice Line + undo). GREEN
 * auto-acts then reports; this reuses the Permission UX mapping so the two
 * cannot drift: a GREEN action presents as a `report` (task 8.5). Any tier that
 * is auto-actable must present as a report.
 */
export function autoActReports(risk: RiskLevel): boolean {
  return isAutoActable(risk) && resolvePermissionMode(risk) === "report";
}

// ─── Principle 2: rules are deterministic (Req 29.2) ──────────────────────────
//
// "Rules are deterministic" is a property, not a value: a rule resolver, given
// the same input, must always produce the same output (no AI fuzzing, no
// randomness, no clock). The rule resolvers this framework depends on are the
// pure functions above ({@link resolvePermissionMode}, {@link resolveConflict},
// {@link aiOutputCommitMode}) — they are exhaustive `switch`/table lookups. The
// determinism property is asserted over them in `authority.test.ts` (Property 2).

/**
 * A convenience wrapper naming the Permission UX risk→mode map as the canonical
 * deterministic RULE for risk classification presentation. Re-exported so the
 * authority framework has one referenced rule resolver to property-test.
 */
export function riskPresentationRule(risk: RiskLevel): PermissionMode {
  return resolvePermissionMode(risk);
}
