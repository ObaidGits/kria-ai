/**
 * Relationship Evolution — non-manipulative content scaling (task 8.8, design
 * §27, Req 27.1/27.2/27.3).
 *
 * KRIA's homepage presence deepens with the relationship over time — from a
 * brand-new **first-launch** user through **first-week → first-month →
 * power-user → long-term**. Across that arc the CONTENT scales (greeting
 * verbosity, whether starters ground in real usage, whether rare learned-fact
 * remarks are earned, whether chips habituate) while the STRUCTURE stays
 * IDENTICAL (design §27, L3 "Adapt content, never structure"). This module is
 * the single, pure source of truth for that scaling — it layers on top of the
 * existing greeting/Focus content pipeline (task 3.4, {@link homeFocusStore}),
 * it does NOT build a parallel one.
 *
 * ── Hard ethical constraints (Req 27.2, guardrails.md) ───────────────────────
 * The relationship is NEVER an engagement lever. This module refuses to emit or
 * pass through content that:
 *   • **fabricates emotion** — KRIA never claims feelings it does not have
 *     ("I missed you", "I feel", "I'm so happy to see you"); warmth = competence
 *     + memory + restraint, not sentiment; OR
 *   • **guilt-trips / manipulates** — no "you haven't talked to me in a while
 *     😢", no streaks/urgency/"come back", no manufactured intimacy.
 * {@link isRelationshipContentSafe} is the deterministic gate; unsafe content is
 * dropped (silence is a valid premium output, §5.7), never rewritten to sneak
 * past. The Focus engine runs every surfaced greeting / learned-fact remark
 * through it, and the property test asserts NO stage / history / text input can
 * produce a guilt/manipulation marker or a fabricated-emotion claim.
 *
 * ── Capped learned-facts (Req 27.3, §5.6) ────────────────────────────────────
 * The number of learned facts used for personalization is a BOUNDED SET, never
 * unbounded/creepy. {@link MAX_LEARNED_FACTS} is the absolute hard cap; each
 * stage earns a smaller per-stage cap ({@link stageLearnedFactCap}) so a
 * brand-new user gets none and familiarity unlocks a *few*, capped, remarks.
 * {@link capLearnedFacts} enforces the bound structurally.
 *
 * Everything here is a PURE function of its inputs (usage signals / text) — no
 * clock, no storage, no domain writes, no `coreStore` — so the Focus engine
 * stays a pure read-model (Property 1).
 *
 * Requirements: 27.1, 27.2, 27.3.
 */

import type { FocusGreeting } from "./homeFocusStore";

// ─── Relationship stage model (design §27) ───────────────────────────────────

/**
 * The five relationship stages (design §27). Ordered from newest to
 * longest-standing; content scales along this arc while structure is identical.
 */
export type RelationshipStage =
  | "first-launch"
  | "first-week"
  | "first-month"
  | "power-user"
  | "long-term";

/** All stages in evolution order (index = maturity rank). */
export const RELATIONSHIP_STAGES: readonly RelationshipStage[] = [
  "first-launch",
  "first-week",
  "first-month",
  "power-user",
  "long-term",
] as const;

/**
 * Session-count band edges that separate the stages. Derived from the SAME
 * persisted usage signal the greeting familiarity-scaling reads (task 3.4), so
 * the relationship model and the greeting pipeline never diverge. `sessionCount`
 * is inclusive of the cold-start visit (0 = brand-new / first launch).
 */
export const STAGE_FIRST_WEEK_MAX_SESSIONS = 7;
export const STAGE_FIRST_MONTH_MAX_SESSIONS = 30;
export const STAGE_POWER_USER_MAX_SESSIONS = 200;

/** The signals the stage is derived from (a subset of the greeting input). */
export interface RelationshipSignals {
  /** Prior sessions/visits; 0 = brand-new (first launch). */
  sessionCount: number;
  /** Consecutive-day streak (context only; never a retention lever). */
  dayStreak?: number;
}

/**
 * Derive the relationship stage from real usage signals. Pure, deterministic,
 * and MONOTONIC in `sessionCount`: more sessions never yields an *earlier*
 * stage. Negative/NaN session counts clamp to 0 (first launch).
 */
export function deriveRelationshipStage(signals: RelationshipSignals): RelationshipStage {
  const sessions =
    typeof signals.sessionCount === "number" && Number.isFinite(signals.sessionCount)
      ? Math.max(0, Math.floor(signals.sessionCount))
      : 0;
  if (sessions === 0) return "first-launch";
  if (sessions <= STAGE_FIRST_WEEK_MAX_SESSIONS) return "first-week";
  if (sessions <= STAGE_FIRST_MONTH_MAX_SESSIONS) return "first-month";
  if (sessions <= STAGE_POWER_USER_MAX_SESSIONS) return "power-user";
  return "long-term";
}

/** Maturity rank of a stage (0 = first-launch … 4 = long-term). */
export function stageRank(stage: RelationshipStage): number {
  return RELATIONSHIP_STAGES.indexOf(stage);
}

// ─── Capped learned-facts (Req 27.3, §5.6) ───────────────────────────────────

/**
 * ABSOLUTE hard cap on learned facts used for personalization at ANY stage
 * (Req 27.3). A bounded set — never let it grow unbounded/creepy. Every
 * per-stage cap is ≤ this value.
 */
export const MAX_LEARNED_FACTS = 3;

/**
 * Per-stage learned-fact cap (design §27's arc): a brand-new user gets NONE (no
 * memory to ground in yet — guidance, not personalization), and familiarity
 * unlocks a *few* capped remarks, saturating at {@link MAX_LEARNED_FACTS}.
 * Always ∈ [0, MAX_LEARNED_FACTS].
 */
const STAGE_LEARNED_FACT_CAP: Record<RelationshipStage, number> = {
  "first-launch": 0,
  "first-week": 1,
  "first-month": 2,
  "power-user": MAX_LEARNED_FACTS,
  "long-term": MAX_LEARNED_FACTS,
};

/** The learned-fact cap earned by a stage (clamped to [0, MAX_LEARNED_FACTS]). */
export function stageLearnedFactCap(stage: RelationshipStage): number {
  return Math.min(MAX_LEARNED_FACTS, Math.max(0, STAGE_LEARNED_FACT_CAP[stage] ?? 0));
}

/**
 * Enforce the bounded learned-fact set (Req 27.3). Returns at most
 * {@link stageLearnedFactCap} facts for the stage, and never more than
 * {@link MAX_LEARNED_FACTS} regardless. Preserves input order (callers pass the
 * facts pre-ranked by worth/recency), so it is a pure prefix — no reordering,
 * no fabrication. When `stage` is omitted the absolute hard cap applies.
 */
export function capLearnedFacts<T>(facts: readonly T[], stage?: RelationshipStage): T[] {
  const cap = stage === undefined ? MAX_LEARNED_FACTS : stageLearnedFactCap(stage);
  return facts.slice(0, Math.max(0, cap));
}

// ─── Greeting verbosity ceiling per stage (design §27 / §5.5) ────────────────

type Verbosity = FocusGreeting["verbosity"];

/** Numeric rank for verbosity (none < short < full) — for monotone clamping. */
export function verbosityRank(v: Verbosity): number {
  return v === "full" ? 2 : v === "short" ? 1 : 0;
}

/** The less-verbose of two verbosities (used to clamp to a stage ceiling). */
export function minVerbosity(a: Verbosity, b: Verbosity): Verbosity {
  return verbosityRank(a) <= verbosityRank(b) ? a : b;
}

/**
 * The MOST verbose greeting a stage permits (design §27): first-launch/first-week
 * may be FULL, first-month tightens to SHORT, and power-user/long-term default
 * to NONE (lead with substance; KRIA is "silent-but-ready"). The greeting
 * pipeline clamps its familiarity verbosity to this ceiling so the relationship
 * model — not a parallel rule — governs how the greeting scales. Rare milestone
 * greetings are an explicit exception applied upstream (they may exceed the
 * ceiling, but are still frequency-gated and non-manipulative).
 */
export function stageGreetingCeiling(stage: RelationshipStage): Verbosity {
  switch (stage) {
    case "first-launch":
    case "first-week":
      return "full";
    case "first-month":
      return "short";
    case "power-user":
    case "long-term":
      return "none";
  }
}

// ─── Content-scale descriptor (design §27) ───────────────────────────────────

/**
 * The full content-scaling profile for a stage. STRUCTURE is identical at every
 * stage (same slots: greeting, starters, learned-fact remark, chips) — only
 * these CONTENT knobs change (L3 "Adapt content, never structure").
 */
export interface RelationshipContentScale {
  stage: RelationshipStage;
  /** Most verbose greeting permitted at this stage. */
  greetingCeiling: Verbosity;
  /**
   * Whether starters should ground in the user's real prior usage yet. A
   * brand-new user has no history to ground in → generic-but-truthful base
   * starters only (never a fabricated "based on your work" claim).
   */
  groundStartersInHistory: boolean;
  /** Bounded learned-fact remark cap earned at this stage (≤ MAX_LEARNED_FACTS). */
  maxLearnedFacts: number;
  /** Whether chips may tighten toward learned habits (bounded, never structural). */
  habitualChips: boolean;
}

/** Resolve the content-scale profile for a stage. Pure. */
export function relationshipContentScale(stage: RelationshipStage): RelationshipContentScale {
  return {
    stage,
    greetingCeiling: stageGreetingCeiling(stage),
    // History grounding is unlocked once there is real usage to ground in.
    groundStartersInHistory: stage !== "first-launch",
    maxLearnedFacts: stageLearnedFactCap(stage),
    // Habits only emerge once patterns exist (first-month onward, §27).
    habitualChips: stageRank(stage) >= stageRank("first-month"),
  };
}

/** Convenience: resolve the content-scale profile directly from usage signals. */
export function contentScaleForSignals(signals: RelationshipSignals): RelationshipContentScale {
  return relationshipContentScale(deriveRelationshipStage(signals));
}

// ─── Non-manipulation / no-fake-emotion guard (Req 27.2) ─────────────────────

/**
 * Guilt / manipulation / dark-pattern markers (Req 27.2, guardrails.md "Never
 * use streaks, guilt, artificial urgency, or fake emotion/friendship for
 * retention"). Case-insensitive. These catch the classic engagement dark
 * patterns:
 *   • absence-guilt ("haven't talked to me in a while", "where have you been",
 *     "come back", "don't leave", "I missed you", "we miss you", "you forgot me"),
 *   • manufactured urgency ("act now", "last chance", "hurry", "don't miss out",
 *     "limited time", "expires soon", "final call", "before it's too late"),
 *   • streak/retention pressure ("keep your streak", "don't break your streak",
 *     "keep it going", "on a roll", "day streak"),
 *   • pleading / sad emoji used to tug at the user.
 * Kept deliberately PRECISE so legitimate factual content is never a false
 * positive: e.g. "I kept your Linux tooling in mind" (competence + memory),
 * "5 workflows finished", or the milestone "100 days together." are all safe.
 */
export const MANIPULATION_MARKERS: readonly RegExp[] = [
  // Absence guilt.
  /\bmiss(?:ed|ing)?\s+you\b/i,
  /\bwe\s+miss(?:ed)?\s+you\b/i,
  /\bhaven'?t\s+(?:talked|heard|seen|chatted|spoken)\b/i,
  /\bwhere\s+have\s+you\s+been\b/i,
  /\blong\s+time\s+no\s+see\b/i,
  /\bit'?s\s+been\s+(?:a\s+while|too\s+long|forever|ages)\b/i,
  /\bcome\s+back\b/i,
  /\b(?:don'?t|do\s+not)\s+(?:leave|go)\b/i,
  /\bplease\s+(?:stay|come|don'?t)\b/i,
  /\byou\s+(?:forgot|abandoned|neglected|left)\s+me\b/i,
  /\byou\s+owe\b/i,
  // Manufactured urgency.
  /\bact\s+now\b/i,
  /\blast\s+chance\b/i,
  /\bhurry\b/i,
  /\bdon'?t\s+miss\s+out\b/i,
  /\blimited\s+time\b/i,
  /\bexpires?\s+soon\b/i,
  /\bfinal\s+call\b/i,
  /\bbefore\s+it'?s\s+too\s+late\b/i,
  /\brunning\s+out\b/i,
  // Streak / retention pressure.
  /\bstreak\b/i,
  /\bkeep\s+it\s+going\b/i,
  /\bon\s+a\s+roll\b/i,
  // Pleading / sad emoji.
  /[\u{1F622}\u{1F62D}\u{1F97A}\u{1F614}\u{1F625}\u{1F63F}\u{1F494}]/u,
];

/**
 * Fabricated-emotion markers (Req 27.2 "SHALL NOT claim emotions … manufacture
 * intimacy"). These match KRIA speaking in the FIRST PERSON about feelings it
 * does not have. Precise on first-person constructions so third-party facts
 * ("you love coffee", "the user was excited") and functional first-person
 * ("I found", "I can help", "I kept … in mind", "I noticed") never trip:
 */
export const FABRICATED_EMOTION_MARKERS: readonly RegExp[] = [
  /\bi\s+(?:feel|felt)\b/i,
  /\bi'?m\s+feeling\b/i,
  /\bi\s+(?:miss|missed|love|loved|adore|cherish)\b/i,
  /\bi\s+(?:need|want)\s+you\b/i,
  /\bi'?m\s+(?:so\s+)?(?:sad|lonely|heartbroken|thrilled|excited|proud|worried|hurt|delighted|overjoyed)\b/i,
  /\bi\s+was\s+(?:worried|sad|lonely|scared)\b/i,
  /\bi\s+care\s+(?:about|for)\s+you\b/i,
  /\bmakes?\s+me\s+(?:happy|sad|feel)\b/i,
  /\bmy\s+(?:heart|feelings)\b/i,
  /\bi'?ve\s+missed\b/i,
];

/** List every guilt/manipulation marker found in `text` (for diagnostics/tests). */
export function findManipulationMarkers(text: string): string[] {
  if (typeof text !== "string" || text.length === 0) return [];
  const hits: string[] = [];
  for (const re of MANIPULATION_MARKERS) {
    const m = text.match(re);
    if (m) hits.push(m[0]);
  }
  return hits;
}

/** Whether `text` contains a guilt/manipulation/dark-pattern marker (Req 27.2). */
export function hasManipulation(text: string): boolean {
  return findManipulationMarkers(text).length > 0;
}

/** Whether `text` contains a fabricated first-person emotion claim (Req 27.2). */
export function hasFabricatedEmotion(text: string): boolean {
  if (typeof text !== "string" || text.length === 0) return false;
  return FABRICATED_EMOTION_MARKERS.some((re) => re.test(text));
}

/**
 * The single deterministic safety gate (Req 27.2): relationship-scaled content
 * is SAFE iff it contains NO guilt/manipulation marker AND NO fabricated-emotion
 * claim. The Focus engine runs every surfaced greeting / learned-fact remark
 * through this; unsafe content is dropped (never rewritten). Empty/whitespace
 * text is trivially safe (it surfaces nothing).
 */
export function isRelationshipContentSafe(text: string): boolean {
  return !hasManipulation(text) && !hasFabricatedEmotion(text);
}

/**
 * Return `text` only if it is safe to surface as relationship content; otherwise
 * `undefined` (omit — silence is a valid premium output, §5.7). Never mutates or
 * "cleans" the text: manipulation/fake-emotion is dropped wholesale, not
 * laundered into a softer manipulation.
 */
export function safeRelationshipContent(text: string | undefined): string | undefined {
  if (text === undefined) return undefined;
  return isRelationshipContentSafe(text) ? text : undefined;
}
