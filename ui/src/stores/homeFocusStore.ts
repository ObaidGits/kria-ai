/**
 * Home Focus Store — the Homepage Intelligence Layer (Focus engine), §5.
 *
 * A **pure read-model** store that fuses authoritative signals from the existing
 * domain stores (approvals, conversations, automations, memory, notifications)
 * plus a pluggable desktop-awareness bridge, and emits a single reactive
 * {@link FocusFrame}: one greeting?, one Voice Line?, at most one Adaptive
 * Context Surface (ACS)?, ≤3 chips, the lit Orbit points, and an advisory
 * `coreHint`.
 *
 * ── Authority invariants (Req 12.5 / 24.1 / 29.3 / 30.3) ─────────────────────
 * This store is READ-ONLY over the domain. It performs:
 *   • NO writes to any domain store,
 *   • NO tool calls, NO sends, NO orchestration,
 *   • NO `coreStore` writes — `coreHint` is ADVISORY ONLY (`coreStore` remains
 *     the sole authority for Core state; the guardrail lint
 *     `scripts/guardrail-lint.mjs` statically enforces the no-write-back rule on
 *     this file). It imports `CoreState` as a TYPE only — never a mutator.
 * Deriving a `FocusFrame` is a pure function of the current signal values, so
 * reading the frame can never mutate anything (Property 1: read-model purity).
 *
 * ── Single-subject binding (Req 8.4 / 12.3) ──────────────────────────────────
 * When both the Voice Line and the ACS render they describe the SAME subject:
 * both are built from the single highest-ranked candidate, so
 * `voiceLine.subjectId === acs.subjectId` holds by construction (Property 2).
 *
 * ── Always-valid resting output (Req 12.5) ───────────────────────────────────
 * `deriveFocusFrame` always returns a structurally valid frame: `chips` and
 * `orbit` are always arrays, `chips.length ≤ 3`, `orbit` contains only lit
 * points, and when no subject qualifies the frame rests (no Voice Line/ACS).
 *
 * ── Staged pipeline (task 3.2, design §24) ───────────────────────────────────
 * `deriveFocusFrame` runs the pure staged Focus pipeline:
 *   Signals → Understanding → Confidence → Reasoning → Timing/Interruptibility
 *          → Personalization → Decision → Presentation → Feedback → Learning
 * with:
 *   • per-subject CONFIDENCE ∈ [0,1] = source-trust × raw reliability × recency
 *     aging × temporal imminence (§24 stage 3);
 *   • PRIORITY AGING — unacted decaying subjects lose confidence/emphasis over
 *     time (approvals never decay) so a stale subject stops being headline-worthy
 *     and eventually drops out (§24.2);
 *   • TEMPORAL REASONING — time-anchored subjects escalate as they near their
 *     moment and expire after it passes (§24.2);
 *   • TTL/EXPIRATION — every subject may carry a TTL/expiry; expired subjects are
 *     removed and never surface (§24.2);
 *   • the invariant **low-confidence → low-emphasis only**: a subject below
 *     {@link CONFIDENCE_MEDIUM} can only be a chip/Orbit glow and can NEVER be
 *     the Voice Line headline or drive a step-forward `coreHint` (Req 24.2).
 * The pipeline is PURE and DETERMINISTIC: same inputs (incl. the injected `now`
 * clock) → same frame, so Property 1 (read-model purity) and Property 2
 * (single-subject binding) still hold.
 *
 * ── Conflict resolution + live delivery (task 3.3, design §5.4) ──────────────
 * The pure `deriveFocusFrame` resolves conflicts with a fixed TOTAL order —
 * precedence → source-trust → recency → subjectId ({@link compareCandidates}) —
 * so a single winner always emerges and the Voice Line + ACS can never show two
 * competing subjects (Req 12.2/12.3/24.4/24.5). Notification suppression
 * ({@link notificationQualifies}) keeps low-value ambient chatter out of the
 * frame (Req 12.4).
 *
 * The LIVE frame ({@link createLiveFocusFrame}) wraps the pure derivation in two
 * stateful, clock-injectable, IDLE-QUIET layers so the homepage never thrashes
 * (Req 12.4 / 24.4):
 *   • {@link createRecomputeThrottle} — coalesces bursts of signal changes to
 *     ≤1 recompute per ~250 ms (leading + trailing; a timer is armed ONLY while
 *     a trailing recompute is pending, never a perpetual interval).
 *   • {@link createDwellStabilizer} — once a subject is the headline it holds for
 *     a minimum dwell (~6 s) before a LOWER/equal-precedence subject may replace
 *     it; a strictly higher-precedence subject (e.g. a needs-you approval)
 *     preempts immediately. Crossfade-only semantics: the store exposes the
 *     stable headline; the visual crossfade lives in the Voice Line / ACS
 *     components (task 4.x). Deterministic via an injected clock.
 * Both layers are pure of domain writes — the read-model purity invariant
 * (Property 1) and single-subject binding (Property 2) still hold.
 *
 * ── Interruptibility gate (task 3.9, design §26.3 / Req 26) ──────────────────
 * The Timing/Interruptibility stage ({@link applyInterruptibility}) now enforces
 * the DEFAULT-SILENT posture: in an interruptibility-BLOCKED context (screen
 * record/share, call mic/cam, presentation/fullscreen, game/focus/DND — derived
 * by {@link isInterruptibilityBlocked} from the awareness signals or an explicit
 * input) only RED approvals (risk red/black) surface; every other subject is
 * deferred. The surviving RED approval surfaces CALMLY — the pure frame sets
 * {@link FocusFrame.blockedContext} so presentation uses the ember and NEVER
 * audio (the engine has no audio output). The stateful "at-most-one gentle
 * re-surface, then age-out" rule (Req 26.4) lives in the clock-injectable
 * {@link createInterruptibilityGate} layer (like the dwell stabilizer), so the
 * pure derivation stays deterministic.
 *
 * ── Left as clean seams (owned by later tasks) ───────────────────────────────
 * Greeting familiarity-scaling (task 3.4), capability tiers (task 3.6), and the
 * real desktop-awareness bridge (task 3.7) are NOT implemented here; the
 * Personalization, Feedback, and Learning stages are explicit passthrough seams
 * those tasks fill in.
 *
 * Requirements: 12.1, 12.2, 12.3, 12.4, 12.5, 24.1, 24.2, 24.3, 24.4, 24.5,
 * 26.1, 26.2, 26.3, 26.4, 29.3, 30.3.
 */
import { createEffect, createSignal, onCleanup } from "solid-js";

import type { Route } from "../shell/router";
// Type-only import: the frame carries an ADVISORY Core-state hint. This store
// never imports or calls a coreStore mutator (authority invariant, Req 30.3).
import type { CoreState } from "./coreStore";
import type { ApprovalRequest } from "./approvalStore";
import type { Thread } from "./converseStore";
import type { Workflow } from "./automationStore";
import type { MemoryFact } from "./memoryStore";
import type { Notification } from "./notificationStore";

import { approvalStore } from "./approvalStore";
import { converseStore } from "./converseStore";
import { automationStore } from "./automationStore";
import { memoryStore } from "./memoryStore";
import { notificationStore } from "./notificationStore";
// Bounded preference learning REUSES the existing adaptive-ranking module — the
// Focus engine never invents its own learning system (task 3.4, Req 24.7).
import { listAdaptiveDismissed, rankAdaptiveSuggestions } from "../adaptive";
import { homeGreetingStore } from "./homeGreetingStore";
// Relationship-evolution content scaling (task 8.8, design §27, Req 27.1/2/3):
// the single pure source for how content scales with the relationship over time
// (first-launch → long-term) AND the non-manipulation / no-fake-emotion gate +
// the bounded learned-fact cap. Layered on top of THIS content pipeline (never
// a parallel one): the greeting derivation clamps its verbosity to the stage
// ceiling and drops any unsafe text, and the learned-fact source is capped to a
// bounded set per stage.
import {
  capLearnedFacts,
  deriveRelationshipStage,
  isRelationshipContentSafe,
  minVerbosity,
  stageGreetingCeiling,
  type RelationshipStage,
} from "./relationshipEvolution";

// ─── FocusFrame data model (design §5.2 / Data Models) ───────────────────────

/** Greeting whose verbosity scales with familiarity (task 3.4 owns scaling). */
export interface FocusGreeting {
  text: string;
  verbosity: "full" | "short" | "none";
}

/**
 * The persisted familiarity/greeting state the greeting derivation reads (task
 * 3.4). Passed as an INPUT so {@link deriveGreeting} / {@link deriveFocusFrame}
 * stay PURE and deterministic (Property 1). The live layer sources these from a
 * lightweight localStorage-backed signal ({@link homeGreetingStore}); the pure
 * function never touches storage or a clock of its own.
 *
 * Cold start (brand-new user) is `sessionCount === 0` with `name` undefined:
 * the greeting is truthful + generic and NEVER fabricates personalization
 * (Req 27.1/27.3 / 24.6).
 */
export interface GreetingInput {
  /** Prior sessions/visits. 0 = cold start (brand-new user). Drives verbosity. */
  sessionCount: number;
  /** Consecutive-day streak; drives rare milestone greetings (Req 27.1/12.6). */
  dayStreak: number;
  /** Local hour 0–23 for the time-of-day segment (deterministic; injected). */
  hourOfDay: number;
  /**
   * The user's real name IF they actually provided one. Undefined until known —
   * the greeting never invents a name (cold-start truthfulness, Req 27.3).
   */
  name?: string;
  /**
   * Text of the greeting shown last (for no-consecutive-repeat, Req 12.6). The
   * derivation never returns a greeting whose text equals this value.
   */
  lastGreetingText?: string;
}

/**
 * Presentation emphasis a subject earns from its confidence (design §24 stage 8:
 * Presentation). Only `high` earns the full step-forward Voice Line + ACS
 * treatment; `medium` is Voice Line only; `low` is chip/Orbit-glow only and can
 * NEVER become the Voice Line headline (Req 24.2 "low-confidence → low-emphasis
 * only"). Below the floor a subject earns `hidden` and does not surface at all.
 */
export type FocusEmphasis = "high" | "medium" | "low" | "hidden";

/** The Voice Line — one adaptive sentence beneath the Core (Req 3). */
export interface FocusVoiceLine {
  /** Id of the Focus subject this line describes. */
  subjectId: string;
  /** The sentence KRIA "says". Never fabricated — derived from a real signal. */
  text: string;
  /** Stable content key for no-consecutive-repeat + dwell (task 3.3 uses it). */
  key: string;
  /** Whether the subject references a navigable owner (routing-only deep link). */
  actionable: boolean;
  /** Optional routing-only deep link to the owning surface (no side effect). */
  link?: Route;
  /**
   * Fixed-precedence band of this subject ({@link FOCUS_PRIORITY}; design §5.3).
   * Exposed so the stateful anti-flicker dwell layer ({@link createDwellStabilizer})
   * can let a strictly higher-precedence subject preempt the incumbent while
   * holding lower/equal-priority challengers for the minimum dwell (§5.4). The
   * pure frame carries it as read-only metadata; presentation ignores it.
   */
  priority: number;
  /**
   * Per-subject confidence ∈ [0,1] (design §24 stage 3). Fused from source
   * trust × raw reliability × recency aging × temporal imminence. A headline
   * Voice Line always has confidence ≥ {@link CONFIDENCE_MEDIUM} (Req 24.2).
   */
  confidence: number;
  /** Presentation emphasis derived from {@link confidence} (never `hidden` here). */
  emphasis: Exclude<FocusEmphasis, "hidden">;
}

/** A single action offered by the ACS (verb + a routing/staging callback). */
export interface FocusAcsAction {
  label: string;
  /**
   * Runs the action. In this read-model an action only ever ROUTES or STAGES a
   * reviewable draft (KRIA runtime-authority invariant, Req 29.3) — it never
   * sends or executes. The concrete `run` is supplied by the presentation layer
   * (tasks 4.x); the engine only declares that an action exists.
   */
  run: () => void;
}

/** The Adaptive Context Surface — one subject, one detail, ≤1 action (Req 8). */
export interface FocusAcs {
  /** MUST equal `voiceLine.subjectId` when both render (Req 8.4 / 12.3). */
  subjectId: string;
  title: string;
  line: string;
  /** At most one action; deeper detail routes to `ownerRoute`. */
  action?: FocusAcsAction;
  /** Where "more detail" routes to (the owning Space). */
  ownerRoute: Route;
}

/** Capability an Orbit point represents (design Data Models; open-ended). */
export type OrbitCapability =
  | "memory"
  | "automation"
  | "desktop"
  | "local"
  | "approval"
  | "conversation"
  | (string & {});

/** A contextual next-action chip (≤3, ranked from live state — Req 5). */
export interface Chip {
  id: string;
  label: string;
  icon: string;
  /** `stage` places a reviewable draft; `route` navigates. Never executes. */
  kind: "stage" | "route";
  payload: string | Route;
}

/** A capability-awareness light-point around the Core (Req 6). */
export interface OrbitPoint {
  id: string;
  capability: OrbitCapability;
  /** Only lit points are emitted in the frame. */
  lit: boolean;
  label: string;
  /** Routing-only deep link when actionable. */
  route?: Route;
}

/**
 * The single frame emitted by the Focus engine (design §5.2 / Data Models).
 * Derived, never persisted. Invariants enforced by construction:
 * `voiceLine`/`acs` bind to the same `subjectId` when both render;
 * `chips.length ≤ 3`; `orbit` contains only lit points.
 */
export interface FocusFrame {
  greeting?: FocusGreeting;
  voiceLine?: FocusVoiceLine;
  acs?: FocusAcs;
  chips: Chip[];
  orbit: OrbitPoint[];
  /** Advisory only; `coreStore` stays the authority (Req 30.3). */
  coreHint?: CoreState;
  /**
   * True when this frame is emitted in an interruptibility-BLOCKED context
   * (screen recording/sharing, an active call's mic/cam, presentation/fullscreen,
   * game/focus, or Do-Not-Disturb — Req 26.2/26.3, design §26.3). It is the
   * engine's PRESENTATION CONTRACT: surface CALMLY via the Companion ember and
   * NEVER produce audio. The engine only sets the flag and outputs the subject +
   * its advisory (calm) `coreHint`; it has no audio output of its own, so it can
   * never *request* audio (Req 26.3 / §26.5). In a blocked context the pure
   * derivation has already suppressed every non-RED subject (task 3.9), so when
   * `blockedContext` is set any `voiceLine`/`acs` describes a RED approval only.
   * Absent/false in interruptible contexts (the default posture is unchanged).
   */
  blockedContext?: boolean;
}

/** Maximum contextual chips (Req 5.1 — mirrors guardrails.MAX_CHIPS). */
export const MAX_CHIPS = 3;

// ─── Desktop-awareness bridge seam (task 3.7 implements the real one) ────────

/**
 * One desktop-awareness signal mapped to a Focus candidate. The real bridge
 * (task 3.7) will populate these from portals/integrations (calendar, MPRIS,
 * editor, git, battery, downloads). Kept as an injectable interface seam here so
 * the Focus engine can rank awareness subjects the moment the bridge exists,
 * without a build-order dependency. Awareness carries its OWN priority so the
 * bridge controls where its subjects land in the fixed precedence.
 */
export interface AwarenessSignal {
  /** Stable id; becomes the Focus subjectId (namespaced `desktop:`). */
  id: string;
  capability: OrbitCapability;
  /** Priority within the fixed precedence (§5.3); see {@link FOCUS_PRIORITY}. */
  priority: number;
  /** Tiebreak recency (higher = more recent). */
  recency: number;
  /** The Voice Line sentence (already truthful/plain-language). */
  voiceText: string;
  /** Whether it deep-links to an owner (routing only). */
  actionable?: boolean;
  ownerRoute?: Route;
  /** Optional ACS expansion. */
  acsTitle?: string;
  acsLine?: string;
  /** Optional advisory Core hint. */
  coreHint?: CoreState;
  // ── Confidence / aging / temporal / TTL inputs (task 3.2, §24) ─────────────
  /**
   * Raw signal reliability ∈ [0,1] (how sure the sensor is). Defaults to
   * {@link DEFAULT_AWARENESS_CONFIDENCE}. The bridge MUST report honest
   * confidence — uncertain awareness stays low-emphasis (Req 24.2/25 §5).
   */
  confidence?: number;
  /** Source-trust weight ∈ [0,1] (down-weights unreliable sources, Req 24.5). */
  sourceTrust?: number;
  /** Whether this subject ages (loses emphasis while unacted). Defaults true. */
  decays?: boolean;
  /** Aging half-life in ms (emphasis halves each half-life). */
  agingHalfLifeMs?: number;
  /** Time-to-live in ms measured from {@link recency}; older → expired/removed. */
  ttlMs?: number;
  /** Absolute expiry timestamp (ms); at/after it the subject is removed. */
  expiresAt?: number;
  /** Moment the subject refers to (e.g. meeting start) for temporal imminence. */
  startsAt?: number;
  /** How far ahead of {@link startsAt} the subject begins escalating (ms). */
  leadWindowMs?: number;
  /** Grace window after {@link startsAt} before the subject expires (ms). */
  graceMs?: number;
  /**
   * Whether this signal, when present, marks an interruptibility-BLOCKED context
   * (Req 26.2, design §26.3). The desktop-awareness bridge (task 3.7) sets it on
   * the signals for screen recording/sharing, an active call's camera/mic,
   * presentation/fullscreen focus, gaming/focus, or Do-Not-Disturb. When any
   * opted-in signal reports it (or its source id is one of
   * {@link BLOCKED_CONTEXT_SOURCE_IDS}) the Focus engine adopts the default-silent
   * posture: only RED approvals surface, calmly, never audio. Ephemeral like the
   * signal itself — the block lasts only while the signal is present.
   */
  blocksInterruptibility?: boolean;
}

/**
 * The bridge contract: a pure signal source. `signals()` returns the currently
 * available awareness signals (empty when the subsystem is OFF/unavailable —
 * Req 25.3 "omit unavailable signals without error"). The Focus engine only
 * READS it; it never writes back.
 */
export interface DesktopAwarenessBridge {
  signals: () => readonly AwarenessSignal[];
}

const EMPTY_BRIDGE: DesktopAwarenessBridge = { signals: () => [] };

let awarenessBridge: DesktopAwarenessBridge = EMPTY_BRIDGE;

/** Wire the real desktop-awareness bridge (task 3.7). Read-model only. */
export function setAwarenessBridge(bridge: DesktopAwarenessBridge): void {
  awarenessBridge = bridge;
}

/** Detach the awareness bridge (returns to "no signals"). */
export function clearAwarenessBridge(): void {
  awarenessBridge = EMPTY_BRIDGE;
}

// ─── Fixed ranking precedence (design §5.3) ──────────────────────────────────

/**
 * Fixed, documented precedence (§5.3):
 *   needs-you (approval) > imminent world/desktop event > active continuable
 *   session > finished work worth knowing > resumable task/thread >
 *   rare learned fact > rest.
 * Higher number wins. Desktop-awareness signals supply their own priority so the
 * bridge can slot subjects into the imminent-event / active-session bands.
 */
export const FOCUS_PRIORITY = {
  needsYou: 100,
  imminentEvent: 80,
  activeSession: 60,
  finishedWork: 50,
  resumable: 40,
  learnedFact: 20,
  rest: 0,
} as const;

// ─── Confidence / emphasis thresholds (design §24 stages 3 & 8) ──────────────

/**
 * Confidence thresholds that map a subject's confidence ∈ [0,1] onto its
 * presentation emphasis (design §24 stage 8):
 *   confidence ≥ HIGH   → `high`   → Voice Line + ACS + advisory `coreHint`
 *   MEDIUM ≤ c < HIGH   → `medium` → Voice Line only (no ACS, no step-forward)
 *   FLOOR  ≤ c < MEDIUM → `low`    → chip / Orbit glow only (NEVER headline)
 *   c < FLOOR           → `hidden` → does not surface at all
 * The MEDIUM boundary is the headline gate: a subject below it is low-emphasis
 * only and can never be the Voice Line (Req 24.2).
 */
export const CONFIDENCE_FLOOR = 0.15;
export const CONFIDENCE_MEDIUM = 0.4;
export const CONFIDENCE_HIGH = 0.7;

/**
 * Per-source trust weights ∈ [0,1] (design §24 stage 3 / §24.2 signal
 * reliability). Authoritative, first-party signals (approvals) are fully
 * trusted; softer inferences (desktop awareness) start lower so uncertain
 * awareness stays low-emphasis (Req 24.2/24.5).
 */
export const SOURCE_TRUST = {
  approval: 1.0,
  automation: 0.9,
  conversation: 0.85,
  memory: 0.9,
  notification: 0.8,
  awareness: 0.8,
} as const;

/** Default aging half-life for decaying subjects (emphasis halves each period). */
export const AGING_HALF_LIFE_MS = 45 * 60 * 1000; // 45 min
/** Default TTL for finished-work / notice subjects (removed once older). */
export const FINISHED_WORK_TTL_MS = 12 * 60 * 60 * 1000; // 12 h
/** Default raw confidence for a desktop-awareness signal that omits one. */
export const DEFAULT_AWARENESS_CONFIDENCE = 0.75;
/** Default aging half-life for ephemeral awareness signals. */
export const DEFAULT_AWARENESS_HALF_LIFE_MS = 30 * 60 * 1000; // 30 min
/** Default lead window over which a time-anchored subject escalates. */
export const DEFAULT_LEAD_WINDOW_MS = 30 * 60 * 1000; // 30 min
/** Default grace after a temporal subject's moment before it expires. */
export const DEFAULT_TEMPORAL_GRACE_MS = 5 * 60 * 1000; // 5 min
/** Floor emphasis a far-away temporal subject keeps before it starts escalating. */
export const TEMPORAL_BASE_FACTOR = 0.6;

/** Classify a confidence value into its presentation emphasis (§24 stage 8). */
export function classifyEmphasis(confidence: number): FocusEmphasis {
  if (confidence >= CONFIDENCE_HIGH) return "high";
  if (confidence >= CONFIDENCE_MEDIUM) return "medium";
  if (confidence >= CONFIDENCE_FLOOR) return "low";
  return "hidden";
}

// ─── Capability Tier resolver (task 3.6, design §30 / Req 28.1/28.2) ─────────

/**
 * The homepage capability tiers (design §30). Higher tiers add PRESCIENCE; they
 * never define baseline value. Tier 0 is the always-usable floor.
 *   • Tier 0 — Core only: presence + Composer + talk. Fully usable, never below.
 *   • Tier 1 — + Conversation/Memory: greeting personalization, resume, facts.
 *   • Tier 2 — + Automation: "what finished / running" subjects.
 *   • Tier 3 — + Desktop Awareness (opt-in): session/world prescience.
 *   • Tier 4 — + Calendar/integrations: richer time-sensitive Focus.
 *   • Tier 5 — Everything: full prescience (all subsystems up).
 */
export type CapabilityTier = 0 | 1 | 2 | 3 | 4 | 5;

/**
 * The independently-degradable subsystems that FEED the Focus engine. Each maps
 * to the candidate source of the same name; a subsystem being unavailable
 * removes ONLY its own subjects (Req 28.2 "degrade each tier independently").
 *
 * `approval`/`notification` are core safety/attention signals (Tier 0 plumbing):
 * they sit at the always-usable floor and never gate Tier-0 usability, but they
 * are still modelled as subsystems so they can be toggled independently for
 * completeness. `desktop`/`calendar` both ride the (opt-in, OFF-by-default)
 * desktop-awareness bridge (Req 25) yet degrade independently of one another.
 */
export type CapabilitySubsystem =
  | "approval"
  | "notification"
  | "conversation"
  | "memory"
  | "automation"
  | "desktop"
  | "calendar";

/** Every modelled subsystem, in a stable order (used to build availability). */
export const ALL_SUBSYSTEMS: readonly CapabilitySubsystem[] = [
  "approval",
  "notification",
  "conversation",
  "memory",
  "automation",
  "desktop",
  "calendar",
] as const;

/**
 * Which tier each subsystem's VALUE belongs to (design §30's additive model).
 * `approval`/`notification` are Tier-0 core plumbing; conversation/memory add
 * Tier 1; automation Tier 2; desktop awareness Tier 3; calendar Tier 4. Tier 5
 * ("Everything") is the emergent state where every subsystem is available.
 */
export const SUBSYSTEM_TIER: Record<CapabilitySubsystem, CapabilityTier> = {
  approval: 0,
  notification: 0,
  conversation: 1,
  memory: 1,
  automation: 2,
  desktop: 3,
  calendar: 4,
} as const;

/** Availability of every subsystem (true = up/contributing its subjects). */
export type CapabilityAvailability = Record<CapabilitySubsystem, boolean>;

/**
 * The queryable capability-tier state (exposed for later UI/Settings — the
 * "what KRIA can sense" panel, task 3.8). Reports which subsystems are up, which
 * value tiers are consequently available, and the coarse highest available tier.
 * `tier0Usable` is a compile-time-true invariant: Tier 0 (Core + Composer + talk)
 * stays fully usable regardless of which higher tiers are missing (Req 28.1).
 */
export interface CapabilityTierState {
  /** Per-subsystem availability snapshot. */
  subsystems: CapabilityAvailability;
  /** Tier 0 is ALWAYS usable — the always-true floor invariant (Req 28.1). */
  tier0Usable: true;
  /** Ascending list of currently-available value tiers (always includes 0). */
  availableTiers: CapabilityTier[];
  /** Coarse label: the highest currently-available tier (≥ 0). */
  highestTier: CapabilityTier;
}

/**
 * Normalise a partial availability map into a full one. A subsystem omitted from
 * `partial` defaults to AVAILABLE (`true`): the pure engine treats "unspecified"
 * as "present" so existing callers/tests keep every source, and the live layer
 * passes the subsystems it knows to be off (e.g. desktop awareness OFF by
 * default). Pure + deterministic.
 */
export function resolveAvailability(
  partial?: Partial<CapabilityAvailability>,
): CapabilityAvailability {
  const out = {} as CapabilityAvailability;
  for (const subsystem of ALL_SUBSYSTEMS) out[subsystem] = partial?.[subsystem] ?? true;
  return out;
}

/** The subsystems whose VALUE lands in a given tier (design §30 additive map). */
export function subsystemsAtTier(tier: CapabilityTier): CapabilitySubsystem[] {
  return ALL_SUBSYSTEMS.filter((s) => SUBSYSTEM_TIER[s] === tier);
}

/**
 * CAPABILITY TIER RESOLVER (Req 28.1/28.2, design §30). A pure function of the
 * current subsystem availability. Guarantees:
 *   • **Tier 0 always usable** — `tier0Usable` is invariantly `true` and Tier 0
 *     is always in `availableTiers`, regardless of which higher tiers are down.
 *   • **Independent degradation** — a value tier (1–4) is "available" iff at
 *     least one of its own subsystems is up, computed purely from that tier's
 *     subsystems; a missing tier can never remove another tier's availability.
 *   • Tier 5 ("Everything") is available only when EVERY subsystem is up.
 * The result is a snapshot for UI/Settings; it never gates Tier-0 usability.
 */
export function resolveCapabilityState(
  partial?: Partial<CapabilityAvailability>,
): CapabilityTierState {
  const subsystems = resolveAvailability(partial);
  // Tier 0 is the always-usable floor (Core + Composer + talk) — unconditional.
  const availableTiers: CapabilityTier[] = [0];
  for (const tier of [1, 2, 3, 4] as const) {
    if (subsystemsAtTier(tier).some((s) => subsystems[s])) availableTiers.push(tier);
  }
  // Tier 5 = full prescience: only when every modelled subsystem is available.
  if (ALL_SUBSYSTEMS.every((s) => subsystems[s])) availableTiers.push(5);
  // availableTiers is built strictly ascending, so its last entry is the max.
  const highestTier = availableTiers[availableTiers.length - 1];
  return { subsystems, tier0Usable: true, availableTiers, highestTier };
}

// ─── Greeting familiarity-scaling (task 3.4, design §5.5 / Req 12.6/27.1/27.3) ─

/**
 * Familiarity thresholds that scale greeting verbosity down as the user grows
 * familiar (design §5.5 "familiarity reduces words"). New users get a FULL
 * greeting; regulars get a SHORT one; daily/power users get NONE (lead with
 * substance). Sessions are inclusive of the cold-start visit (0).
 */
export const GREETING_FULL_MAX_SESSIONS = 3;
export const GREETING_SHORT_MAX_SESSIONS = 15;

/**
 * Day-streak milestones that earn a RARE greeting even for daily/power users who
 * normally get none (design §5.5 "milestones only, rare"). Truthful and
 * frequency-gated: a milestone greeting is shown at most once (no-consecutive-
 * repeat) and never manufactures emotion (Req 27.2).
 */
export const GREETING_MILESTONES: readonly number[] = [7, 30, 100, 365, 730, 1000];

type DaySegment = "morning" | "afternoon" | "evening" | "night";

/** Deterministic time-of-day segment from a 0–23 hour (wraps defensively). */
function daySegment(hour: number): DaySegment {
  const h = ((Math.floor(hour) % 24) + 24) % 24;
  if (h >= 5 && h < 12) return "morning";
  if (h >= 12 && h < 17) return "afternoon";
  if (h >= 17 && h < 22) return "evening";
  return "night";
}

const FULL_WORD: Record<DaySegment, string> = {
  morning: "Good morning",
  afternoon: "Good afternoon",
  evening: "Good evening",
  night: "Hello",
};

const SHORT_WORD: Record<DaySegment, string> = {
  morning: "Morning.",
  afternoon: "Afternoon.",
  evening: "Evening.",
  night: "Hello.",
};

/**
 * Build the greeting for a target verbosity. `none` yields no greeting at all
 * (omission is a valid premium output, §5.7). A FULL greeting includes the name
 * ONLY when it is really known — a cold-start user with no name gets the
 * truthful generic form (never a fabricated "welcome back, <name>"), satisfying
 * Req 27.3 / 24.6 (no fabricated personalization).
 */
function buildGreeting(
  verbosity: FocusGreeting["verbosity"],
  segment: DaySegment,
  name: string | undefined,
): FocusGreeting | undefined {
  if (verbosity === "none") return undefined;
  if (verbosity === "short") return { text: SHORT_WORD[segment], verbosity: "short" };
  const word = FULL_WORD[segment];
  const trimmed = name?.trim();
  return { text: trimmed ? `${word}, ${trimmed}.` : `${word}.`, verbosity: "full" };
}

/**
 * GREETING DERIVATION (task 3.4, design §5.5). Pure and deterministic in its
 * input. Enforces four rules:
 *   • **Familiarity-scaling** — verbosity is monotonically non-increasing in
 *     `sessionCount`: FULL (new) → SHORT (regular) → NONE (daily/power).
 *   • **Milestone-only rare greetings** — a day-streak milestone earns a rare
 *     greeting even for a power user who normally gets none; it is shown at most
 *     once (never consecutively) and never fabricates emotion.
 *   • **No-consecutive-repeat** — the returned greeting text never equals
 *     `lastGreetingText`; on a collision the verbosity steps down one level
 *     (deterministic) until the text differs or the greeting is omitted.
 *   • **Cold-start truthfulness** — with no name the greeting stays generic and
 *     never invents personalization (Req 27.3 / 24.6). Never random filler:
 *     the text is a pure function of the (segment, verbosity, name) tuple.
 */
export function deriveGreeting(input: GreetingInput): FocusGreeting | undefined {
  const segment = daySegment(input.hourOfDay);
  const sessions = Math.max(0, Math.floor(input.sessionCount));
  const last = input.lastGreetingText;

  // Milestone (rare) overrides familiarity — but never repeats consecutively.
  // Milestones may exceed the stage's greeting ceiling (design §27: a rare
  // milestone surfaces even for a power user who normally gets none), yet stays
  // non-manipulative — it is still routed through the relationship safety gate.
  if (GREETING_MILESTONES.includes(input.dayStreak)) {
    const text = `${input.dayStreak} days together.`;
    if (text !== last && isRelationshipContentSafe(text)) return { text, verbosity: "full" };
    // Already shown last time (or unsafe) → fall through to familiarity greeting.
  }

  // Relationship-evolution content scaling (task 8.8, §27): the relationship
  // STAGE — derived from the same usage signals — sets the MOST verbose greeting
  // permitted. The familiarity verbosity is then clamped to that ceiling so the
  // greeting scales with the relationship (structure identical, content scales),
  // reusing this pipeline rather than a parallel one.
  const stage: RelationshipStage = deriveRelationshipStage({
    sessionCount: sessions,
    dayStreak: input.dayStreak,
  });
  const ceiling = stageGreetingCeiling(stage);

  // Familiarity tier → base verbosity (monotonic in sessionCount), clamped to
  // the stage ceiling (never MORE verbose than the relationship earns).
  const familiarity: FocusGreeting["verbosity"] =
    sessions <= GREETING_FULL_MAX_SESSIONS
      ? "full"
      : sessions <= GREETING_SHORT_MAX_SESSIONS
        ? "short"
        : "none";
  let verbosity: FocusGreeting["verbosity"] = minVerbosity(familiarity, ceiling);

  // No-consecutive-repeat: step verbosity down until the text differs (or omit).
  let greeting = buildGreeting(verbosity, segment, input.name);
  while (greeting && greeting.text === last) {
    verbosity = verbosity === "full" ? "short" : "none";
    greeting = buildGreeting(verbosity, segment, input.name);
  }
  // Non-manipulation / no-fake-emotion gate (Req 27.2): never surface a greeting
  // that would guilt-trip or fabricate emotion. Generated greetings are safe by
  // construction; this makes the guarantee hold for ALL inputs (silence is a
  // valid premium output rather than a laundered manipulation).
  if (greeting && !isRelationshipContentSafe(greeting.text)) return undefined;
  return greeting;
}

// ─── Owner routes (routing only — no side effects) ───────────────────────────

const ROUTE_CONVERSE: Route = { space: "converse" };
const ROUTE_AUTOMATIONS: Route = { space: "automations" };
const ROUTE_MEMORY: Route = { space: "memory" };

const threadRoute = (id: string): Route => ({ space: "converse", segment: "thread", entityId: id });

// ─── Inputs + internal candidate model ───────────────────────────────────────

/**
 * The authoritative signal snapshot the frame is derived from. Extracted as an
 * explicit input so the derivation is a pure, testable function (Property 1/2)
 * independent of the live SolidJS accessors.
 */
export interface FocusInputs {
  approvals: readonly ApprovalRequest[];
  threads: readonly Thread[];
  activeThreadId: string | null;
  /** A conversation turn is actively in flight (Core is "responding"). */
  conversing: boolean;
  workflows: readonly Workflow[];
  facts: readonly MemoryFact[];
  notifications: readonly Notification[];
  awareness: readonly AwarenessSignal[];
  /** Deterministic clock for recency ordering (injected for tests). */
  now: number;
  /**
   * Persisted familiarity/greeting state (task 3.4). When present the frame
   * carries a familiarity-scaled greeting; when absent no greeting is emitted.
   * Kept as an input so the derivation stays pure (Property 1).
   */
  greeting?: GreetingInput;
  /**
   * Subject ids the user has dismissed (bounded preference learning, Req 24.7).
   * Sourced from the existing adaptive-ranking module in the live layer. A
   * dismissed subject is suppressed from surfacing — exact-subject only; it
   * never reorders bands, changes layout, or auto-acts (staged/advisory).
   */
  dismissedSubjects?: readonly string[];
  /**
   * Learned-fact frequency-cap gate (Req 12.7/27.3, design §5.6). When `false`
   * the rare learned-fact subject is withheld this frame so it stays meaningful.
   * Defaults to allowed (`undefined`/`true`). Computed from persisted last-shown
   * state in the live layer; passed in to keep the derivation pure.
   */
  learnedFactAllowed?: boolean;
  /**
   * Capability-tier availability (task 3.6, Req 28.1/28.2, design §30). Gates
   * which subsystems contribute candidates: a subsystem set to `false` is
   * unavailable and its subjects are omitted — removing ONLY that subsystem's
   * subjects (independent degradation). Omitted/undefined subsystems default to
   * AVAILABLE, so an absent `availability` keeps every source (Tier 5 behaviour)
   * and the derivation stays a pure function of its inputs. Tier 0 (Core +
   * Composer + talk) stays fully usable at any availability because the frame
   * always resolves to a structurally valid (often resting) output.
   */
  availability?: Partial<CapabilityAvailability>;
  /**
   * Explicit interruptibility-blocked override (task 3.9, Req 26.2/26.3). When
   * `true` the context is treated as BLOCKED regardless of awareness signals, so
   * the engine adopts the default-silent posture (only RED approvals surface,
   * calmly, never audio). Normally the block is DERIVED from the awareness
   * signals (see {@link isInterruptibilityBlocked}); this input lets the live
   * layer or a test assert the blocked context directly while keeping the
   * derivation a pure function of its inputs. Defaults to `false`/interruptible.
   */
  interruptibilityBlocked?: boolean;
}

/**
 * Internal ranking record for a single candidate subject. The winner becomes the
 * Voice Line + (optionally) the ACS; the rest feed chips/orbit.
 */
interface FocusCandidate {
  subjectId: string;
  /**
   * The independently-degradable subsystem this subject belongs to (task 3.6).
   * Set at build time by its source so the capability-tier resolver can gate it
   * and so {@link capabilitySubjects} can report subject→tier membership. It is
   * the AUTHORITATIVE subsystem tag (never inferred from `capability`).
   */
  subsystem: CapabilitySubsystem;
  priority: number;
  recency: number;
  capability: OrbitCapability;
  voiceText: string;
  key: string;
  actionable: boolean;
  link?: Route;
  ownerRoute: Route;
  acsTitle?: string;
  acsLine?: string;
  coreHint?: CoreState;
  /**
   * True iff this candidate is a RED approval (risk `red`/`black`). RED approvals
   * are the ONLY subjects allowed to surface in an interruptibility-blocked
   * context (Req 26.2); every other subject is deferred by
   * {@link applyInterruptibility}. Set at build time by {@link approvalCandidates}.
   */
  redApproval?: boolean;
  /** Optional chip this candidate contributes (stage/route only). */
  chip?: Chip;
  // ── Confidence / aging / temporal / TTL model (task 3.2, §24) ──────────────
  /** Source-trust weight ∈ [0,1] (§24 stage 3). */
  sourceTrust: number;
  /** Raw signal reliability ∈ [0,1] (§24 stage 3). */
  rawConfidence: number;
  /** Whether the subject ages while unacted (approvals never decay, §24.2). */
  decays: boolean;
  /** Aging half-life in ms when {@link decays}. */
  agingHalfLifeMs: number;
  /** TTL in ms from {@link recency}; older → expired (§24.2). Undefined = never. */
  ttlMs?: number;
  /** Absolute expiry timestamp (ms). Undefined = none. */
  expiresAt?: number;
  /** Temporal anchor moment (ms) for imminence escalation (§24.2). */
  startsAt?: number;
  /** Lead window (ms) over which a temporal subject escalates toward its moment. */
  leadWindowMs?: number;
  /** Grace (ms) after {@link startsAt} before the temporal subject expires. */
  graceMs?: number;
}

/** A candidate scored by the Confidence stage; carries derived emphasis. */
interface ScoredCandidate extends FocusCandidate {
  /** Fused confidence ∈ [0,1] (source-trust × raw × aging × temporal). */
  confidence: number;
  /** Presentation emphasis (§24 stage 8); `hidden` candidates are dropped. */
  emphasis: FocusEmphasis;
}

// ─── Candidate builders (one per signal source; all pure) ────────────────────

function approvalCandidates(inputs: FocusInputs): FocusCandidate[] {
  return inputs.approvals
    .filter((a) => a.status === "pending")
    .map((a) => {
      const highRisk = a.risk === "red" || a.risk === "black";
      return {
        subjectId: `approval:${a.id}`,
        subsystem: "approval" as CapabilitySubsystem,
        // Approvals never decay; high-risk sorts first via recency bump.
        priority: FOCUS_PRIORITY.needsYou,
        recency: a.createdAt + (highRisk ? 1e12 : 0),
        capability: "approval" as OrbitCapability,
        voiceText: a.title,
        key: `approval:${a.id}`,
        actionable: true,
        link: ROUTE_CONVERSE,
        ownerRoute: ROUTE_CONVERSE,
        acsTitle: a.title,
        acsLine: a.description,
        coreHint: highRisk ? ("blocked" as CoreState) : ("waiting" as CoreState),
        // RED (red/black) approvals are the only subject that may pierce a
        // blocked context (Req 26.2); tag them so the interruptibility gate can
        // let them through while deferring everything else.
        redApproval: highRisk,
        // Approvals are fully trusted and NEVER decay or expire (§24.2): a
        // pending "needs-you" stays a confident headline until it is acted on.
        sourceTrust: SOURCE_TRUST.approval,
        rawConfidence: 1,
        decays: false,
        agingHalfLifeMs: AGING_HALF_LIFE_MS,
        chip: {
          id: `approval:${a.id}`,
          label: "Review",
          icon: "shield-alert",
          kind: "route",
          payload: ROUTE_CONVERSE,
        },
      } satisfies FocusCandidate;
    });
}

function automationCandidates(inputs: FocusInputs): FocusCandidate[] {
  const out: FocusCandidate[] = [];
  for (const wf of inputs.workflows) {
    if (wf.status === "running") {
      out.push({
        subjectId: `automation:${wf.id}`,
        subsystem: "automation",
        priority: FOCUS_PRIORITY.activeSession,
        recency: wf.lastRunAt ?? wf.createdAt,
        capability: "automation",
        voiceText: `${wf.name} is running.`,
        key: `automation:running:${wf.id}`,
        actionable: true,
        link: ROUTE_AUTOMATIONS,
        ownerRoute: ROUTE_AUTOMATIONS,
        acsTitle: wf.name,
        acsLine: wf.description,
        coreHint: "running-automation" as CoreState,
        // A live, running session does not decay while it runs (§24.2).
        sourceTrust: SOURCE_TRUST.automation,
        rawConfidence: 1,
        decays: false,
        agingHalfLifeMs: AGING_HALF_LIFE_MS,
        chip: {
          id: `automation:${wf.id}`,
          label: wf.name,
          icon: "workflow",
          kind: "route",
          payload: ROUTE_AUTOMATIONS,
        },
      });
    } else if (wf.status === "completed") {
      out.push({
        subjectId: `automation:${wf.id}`,
        subsystem: "automation",
        priority: FOCUS_PRIORITY.finishedWork,
        recency: wf.lastRunAt ?? wf.createdAt,
        capability: "automation",
        voiceText: `${wf.name} finished.`,
        key: `automation:done:${wf.id}`,
        actionable: true,
        link: ROUTE_AUTOMATIONS,
        ownerRoute: ROUTE_AUTOMATIONS,
        acsTitle: wf.name,
        acsLine: wf.description,
        // Finished work ages out of the headline and expires via TTL (§24.2):
        // a 3-hour-old "finished" is no longer headline-worthy.
        sourceTrust: SOURCE_TRUST.automation,
        rawConfidence: 1,
        decays: true,
        agingHalfLifeMs: AGING_HALF_LIFE_MS,
        ttlMs: FINISHED_WORK_TTL_MS,
      });
    }
  }
  return out;
}

function conversationCandidates(inputs: FocusInputs): FocusCandidate[] {
  const out: FocusCandidate[] = [];
  // Resumable threads: most-recent non-archived, non-active thread.
  const resumable = inputs.threads
    .filter((t) => !t.archived && t.id !== inputs.activeThreadId)
    .sort((a, b) => b.updatedAt - a.updatedAt);
  const top = resumable[0];
  if (top) {
    out.push({
      subjectId: `thread:${top.id}`,
      subsystem: "conversation",
      priority: FOCUS_PRIORITY.resumable,
      recency: top.updatedAt,
      capability: "conversation",
      voiceText: `Pick up "${top.title}"?`,
      key: `thread:${top.id}`,
      actionable: true,
      link: threadRoute(top.id),
      ownerRoute: threadRoute(top.id),
      acsTitle: top.title,
      acsLine: "Resume this conversation.",
      // A resumable thread is a stable, low-priority option: it does not age out
      // (it stays available until archived), so it never decays or expires here.
      sourceTrust: SOURCE_TRUST.conversation,
      rawConfidence: 1,
      decays: false,
      agingHalfLifeMs: AGING_HALF_LIFE_MS,
      chip: {
        id: `thread:${top.id}`,
        label: "Resume",
        icon: "message-square",
        kind: "route",
        payload: threadRoute(top.id),
      },
    });
  }
  return out;
}

function memoryCandidates(inputs: FocusInputs): FocusCandidate[] {
  // Rare learned-facts, most-recently-updated worthwhile ones first. The set is
  // BOUNDED per relationship stage (task 8.8, Req 27.3): a brand-new user earns
  // none, familiarity unlocks a few, capped at MAX_LEARNED_FACTS. The stage is
  // derived from the same usage signals the greeting reads; with no greeting
  // state the absolute hard cap applies (keeps the derivation pure + stable).
  const stage = inputs.greeting
    ? deriveRelationshipStage({
        sessionCount: inputs.greeting.sessionCount,
        dayStreak: inputs.greeting.dayStreak,
      })
    : undefined;
  const worthwhile = inputs.facts
    .filter((f) => f.worth > 0)
    // Non-fake-emotion / non-manipulation gate (Req 27.2): never surface a fact
    // as a remark that would read as KRIA fabricating emotion or guilt-tripping.
    .filter((f) => isRelationshipContentSafe(f.content))
    .sort((a, b) => b.updatedAt - a.updatedAt);
  // Enforce the bounded learned-fact set (Req 27.3) BEFORE building candidates,
  // so the number of learned facts used for personalization can never exceed the
  // cap regardless of how many worthwhile facts exist.
  const bounded = capLearnedFacts(worthwhile, stage);
  return bounded.map((fact) => ({
    subjectId: `fact:${fact.id}`,
    subsystem: "memory",
    priority: FOCUS_PRIORITY.learnedFact,
    recency: fact.updatedAt,
    capability: "memory",
    voiceText: fact.content,
    key: `fact:${fact.id}`,
    actionable: true,
    link: ROUTE_MEMORY,
    ownerRoute: ROUTE_MEMORY,
    // A learned fact carries the memory's own stored confidence as its raw
    // reliability. Facts are stable knowledge — they do not age out here.
    sourceTrust: SOURCE_TRUST.memory,
    rawConfidence: clamp01(fact.confidence),
    decays: false,
    agingHalfLifeMs: AGING_HALF_LIFE_MS,
  }));
}

/**
 * Notification tiers that are *inherently worth knowing* on the homepage: the
 * non-blocking attention tier (`needs-you`) and the tiers that signal something
 * went wrong (`warn`/`error`). The ambient `info`/`success` tiers are low-value
 * chatter and are surfaced ONLY when they carry a real action (see
 * {@link notificationQualifies}).
 */
const WORTH_KNOWING_LEVELS: ReadonlySet<Notification["level"]> = new Set([
  "needs-you",
  "warn",
  "error",
]);

/**
 * NOTIFICATION SUPPRESSION (Req 12.4, design §5.4): a notice may surface on the
 * homepage ONLY if it is still live (unread and not dismissed) AND it is either
 *   • **actionable** — it carries an action the user can take (a route to look
 *     at the thing), or
 *   • **genuinely worth knowing** — it belongs to an attention/problem tier
 *     ({@link WORTH_KNOWING_LEVELS}).
 * Non-actionable, low-value ambient chatter (`info`/`success` with no action)
 * NEVER surfaces — it stays in the Notification Center. This keeps the Focus
 * engine from turning quiet background completions into homepage noise.
 *
 * Exported so the suppression rule can be unit-tested directly (task 3.3).
 */
export function notificationQualifies(n: Notification): boolean {
  if (n.read || n.dismissedAt) return false;
  const actionable = Boolean(n.action?.route);
  const worthKnowing = WORTH_KNOWING_LEVELS.has(n.level);
  return actionable || worthKnowing;
}

function notificationCandidates(inputs: FocusInputs): FocusCandidate[] {
  // Notification suppression (Req 12.4): only actionable OR genuinely
  // worth-knowing notices qualify; low-value ambient chatter is dropped.
  return inputs.notifications
    .filter(notificationQualifies)
    .map((n) => ({
      subjectId: `notice:${n.id}`,
      subsystem: "notification" as CapabilitySubsystem,
      priority: FOCUS_PRIORITY.finishedWork,
      recency: n.updatedAt,
      capability: "local" as OrbitCapability,
      voiceText: n.message,
      key: `notice:${n.id}`,
      actionable: Boolean(n.action?.route),
      ownerRoute: ROUTE_CONVERSE,
      // Notices age out of the headline and expire via TTL (§24.2).
      sourceTrust: SOURCE_TRUST.notification,
      rawConfidence: 1,
      decays: true,
      agingHalfLifeMs: AGING_HALF_LIFE_MS,
      ttlMs: FINISHED_WORK_TTL_MS,
    }));
}

function awarenessCandidates(inputs: FocusInputs): FocusCandidate[] {
  return inputs.awareness.map((s) => ({
    subjectId: `desktop:${s.id}`,
    // Calendar signals belong to the Tier-4 `calendar` subsystem; every other
    // awareness signal is Tier-3 `desktop`. Both ride the same (opt-in) bridge
    // but degrade independently (task 3.6).
    subsystem: (s.capability === "calendar" ? "calendar" : "desktop") as CapabilitySubsystem,
    priority: s.priority,
    recency: s.recency,
    capability: s.capability,
    voiceText: s.voiceText,
    key: `desktop:${s.id}`,
    actionable: Boolean(s.actionable && s.ownerRoute),
    link: s.ownerRoute,
    ownerRoute: s.ownerRoute ?? ROUTE_CONVERSE,
    acsTitle: s.acsTitle,
    acsLine: s.acsLine,
    coreHint: s.coreHint,
    // Awareness reports its OWN (honest) confidence + trust so uncertain
    // awareness stays low-emphasis (Req 24.2/24.5). It is ephemeral, so it ages
    // and may carry a TTL / temporal window supplied by the bridge (§24.2).
    sourceTrust: clamp01(s.sourceTrust ?? SOURCE_TRUST.awareness),
    rawConfidence: clamp01(s.confidence ?? DEFAULT_AWARENESS_CONFIDENCE),
    decays: s.decays ?? true,
    agingHalfLifeMs: s.agingHalfLifeMs ?? DEFAULT_AWARENESS_HALF_LIFE_MS,
    ttlMs: s.ttlMs,
    expiresAt: s.expiresAt,
    startsAt: s.startsAt,
    leadWindowMs: s.leadWindowMs,
    graceMs: s.graceMs,
  }));
}

// ─── Stage helpers: Confidence, aging, temporal reasoning, TTL (§24) ─────────

/** Clamp a number into [0,1]. */
function clamp01(n: number): number {
  if (Number.isNaN(n)) return 0;
  return n < 0 ? 0 : n > 1 ? 1 : n;
}

/**
 * PRIORITY AGING (§24.2). A decaying, unacted subject loses emphasis as it ages:
 * the factor halves every `agingHalfLifeMs`. Non-decaying subjects (approvals,
 * live sessions, resumable threads, stable facts) return 1. Future-dated recency
 * (recency > now) clamps age to 0 so nothing is boosted above its base.
 */
function agingFactor(c: FocusCandidate, now: number): number {
  if (!c.decays) return 1;
  const age = Math.max(0, now - c.recency);
  const halfLife = c.agingHalfLifeMs > 0 ? c.agingHalfLifeMs : AGING_HALF_LIFE_MS;
  return Math.pow(0.5, age / halfLife);
}

/**
 * TEMPORAL REASONING (§24.2). A time-anchored subject escalates as its moment
 * nears: far outside the lead window it holds {@link TEMPORAL_BASE_FACTOR}; it
 * ramps to 1 as `now` approaches `startsAt`, then stays full through the grace
 * window (after which {@link isExpired} removes it). Subjects with no `startsAt`
 * are timeless and return 1.
 */
function temporalFactor(c: FocusCandidate, now: number): number {
  if (c.startsAt === undefined) return 1;
  const remaining = c.startsAt - now;
  if (remaining <= 0) return 1; // at/after the moment (still within grace)
  const lead = (c.leadWindowMs ?? DEFAULT_LEAD_WINDOW_MS) || DEFAULT_LEAD_WINDOW_MS;
  const proximity = 1 - Math.min(1, remaining / lead);
  return clamp01(TEMPORAL_BASE_FACTOR + (1 - TEMPORAL_BASE_FACTOR) * proximity);
}

/**
 * TTL / EXPIRATION (§24.2). A subject is expired — and MUST never surface — when
 * any of its bounds have passed: an absolute `expiresAt`, a `ttlMs` measured from
 * `recency`, or a temporal `startsAt` plus its grace window. Subjects without any
 * bound (e.g. approvals) never expire.
 */
function isExpired(c: FocusCandidate, now: number): boolean {
  if (c.expiresAt !== undefined && now >= c.expiresAt) return true;
  if (c.ttlMs !== undefined && now - c.recency > c.ttlMs) return true;
  if (c.startsAt !== undefined) {
    const grace = c.graceMs ?? DEFAULT_TEMPORAL_GRACE_MS;
    if (now >= c.startsAt + grace) return true;
  }
  return false;
}

/**
 * CONFIDENCE (§24 stage 3). Fuse source trust × raw reliability × recency aging ×
 * temporal imminence into a single confidence ∈ [0,1]. This is the value that
 * gates emphasis, so low-reliability / stale / not-yet-imminent subjects earn
 * lower emphasis and can never headline (Req 24.2).
 */
function computeConfidence(c: FocusCandidate, now: number): number {
  const base = clamp01(c.sourceTrust) * clamp01(c.rawConfidence);
  return clamp01(base * agingFactor(c, now) * temporalFactor(c, now));
}

// ─── Deterministic ranking (Decision stage, §24 stage 7) ─────────────────────

/**
 * CONFLICT RESOLUTION — the fixed, total, deterministic order over scored
 * candidates (Req 12.2/12.3/24.4/24.5, design §5.4):
 *
 *   1. **precedence** (`priority`) desc — the fixed §5.3 band always wins first
 *      (needs-you > imminent event > active session > finished work > resumable
 *      > learned fact > rest). This is the primary conflict resolver: two signals
 *      in different bands never compete.
 *   2. **source-trust** (`sourceTrust`) desc — within a band, a more trustworthy
 *      source outranks a less reliable one, down-weighting unreliable sources
 *      (Req 24.5 "resolve conflicts by source-trust then recency").
 *   3. **recency** (`recency`) desc — a fresher subject outranks a staler one
 *      (Req 12.3 "precedence then recency").
 *   4. **subjectId** asc — a final stable tiebreak so the order is TOTAL and the
 *      derivation is fully deterministic (Property 4): identical signals always
 *      yield an identical ranking, so the Voice Line / ACS never oscillate
 *      between two equally-ranked subjects.
 *
 * Because a single winner is chosen from this total order and both the Voice Line
 * and the ACS are built from it, the engine can NEVER render two competing
 * subjects (Req 12.3). Aging/temporal effects still change *emphasis* (via
 * `confidence`, gating whether a subject may headline at all — §24.2) but never
 * reshuffle the fixed precedence bands.
 */
function compareCandidates(a: ScoredCandidate, b: ScoredCandidate): number {
  if (a.priority !== b.priority) return b.priority - a.priority;
  if (a.sourceTrust !== b.sourceTrust) return b.sourceTrust - a.sourceTrust;
  if (a.recency !== b.recency) return b.recency - a.recency;
  return a.subjectId < b.subjectId ? -1 : a.subjectId > b.subjectId ? 1 : 0;
}

/**
 * Per-source isolation (task 3.5, Req 28.1 / design §30 "each tier degrades
 * independently"). Build the candidates from ONE signal source, degrading to
 * OMISSION if that source is missing/unavailable or throws: its subjects simply
 * don't surface this frame. A single broken or absent subsystem can therefore
 * NEVER break the whole frame — the remaining tiers keep working and the frame
 * still resolves to a structurally valid (often resting) output. This is the
 * guarantee that a valid empty/resting output holds at EVERY capability tier,
 * including Tier 0 (no stores wired) and partial availability.
 *
 * The guard also normalises a `null`/`undefined` return (an absent source) to
 * an empty candidate list, so an omitted signal source is indistinguishable
 * from one that produced nothing.
 */
function safeSource(
  build: (inputs: FocusInputs) => FocusCandidate[],
  inputs: FocusInputs,
): FocusCandidate[] {
  try {
    return build(inputs) ?? [];
  } catch {
    // Degrade-to-omission: a throwing/broken source contributes no subjects.
    return [];
  }
}

/**
 * Signals → Understanding: build every candidate subject from the signals,
 * gated by the CAPABILITY TIER RESOLVER (task 3.6, Req 28.1/28.2, design §30).
 *
 * Two layers of independent degradation combine here:
 *   1. Per-source isolation ({@link safeSource}, task 3.5): a missing/throwing
 *      subsystem degrades to OMISSION instead of breaking the frame.
 *   2. Explicit capability-tier availability: a subsystem the resolver reports
 *      as unavailable contributes NO subjects this frame. Because each candidate
 *      carries its authoritative {@link FocusCandidate.subsystem} tag and is
 *      surfaced/scored independently of every other subject, removing one
 *      subsystem removes ONLY its own subjects — never another tier's (the
 *      independence guarantee). Tier 0 (Core + Composer + talk) needs no source
 *      at all, so it stays fully usable even when every subsystem is off.
 *
 * `availability` defaults to all-available (undefined subsystems ⇒ present), so
 * the pure derivation is unchanged for callers that don't model tiers.
 */
function gatherRawCandidates(inputs: FocusInputs): FocusCandidate[] {
  const available = resolveAvailability(inputs.availability);
  return [
    ...safeSource(approvalCandidates, inputs),
    ...safeSource(awarenessCandidates, inputs),
    ...safeSource(automationCandidates, inputs),
    ...safeSource(notificationCandidates, inputs),
    ...safeSource(conversationCandidates, inputs),
    ...safeSource(memoryCandidates, inputs),
  ].filter((c) => available[c.subsystem]);
}

// ─── Capability-tier subject reporting (task 3.6, Req 28.1/28.2) ─────────────

/** A surfacing Focus subject tagged with its subsystem + tier (design §30). */
export interface CapabilitySubject {
  subjectId: string;
  subsystem: CapabilitySubsystem;
  tier: CapabilityTier;
}

/**
 * The surfacing Focus subjects under the given signal snapshot, each tagged with
 * its owning subsystem and capability tier (task 3.6). Pure and deterministic:
 * it runs the same staged pipeline the frame uses, so a subject appears here iff
 * it would surface in the frame's chips/Orbit/Voice-Line. Exposed so the
 * independent-degradation guarantee (removing a subsystem removes only its
 * subjects) is directly observable + testable, and so later UI/Settings can list
 * which subjects each tier currently contributes.
 */
export function capabilitySubjects(inputs: FocusInputs): CapabilitySubject[] {
  return runPipeline(inputs).map((c) => ({
    subjectId: c.subjectId,
    subsystem: c.subsystem,
    tier: SUBSYSTEM_TIER[c.subsystem],
  }));
}

/**
 * Reasoning (§24 stage 4): dedupe/merge candidates that describe the SAME subject
 * so a subject is never counted twice (e.g. an approval about a running
 * automation is one subject). Keeps the higher-priority (then more-recent) record
 * per `subjectId`. Deterministic and order-independent.
 */
function dedupeSubjects(candidates: FocusCandidate[]): FocusCandidate[] {
  const bySubject = new Map<string, FocusCandidate>();
  for (const c of candidates) {
    const existing = bySubject.get(c.subjectId);
    if (
      !existing ||
      c.priority > existing.priority ||
      (c.priority === existing.priority && c.recency > existing.recency)
    ) {
      bySubject.set(c.subjectId, c);
    }
  }
  return [...bySubject.values()];
}

/**
 * Desktop-awareness source ids whose PRESENCE marks an interruptibility-blocked
 * context (Req 26.2/26.3, design §26.3 + §25.1 catalog). These sources only emit
 * a signal while their blocking condition holds — screen recording/sharing, a
 * call's camera/mic in use, or a presentation/fullscreen/game/focus/DND state —
 * so a live signal from any of them means "stay silent" (their §25.1 degradation
 * notes: a silent portal ⇒ assume not blocked). A signal may also set
 * {@link AwarenessSignal.blocksInterruptibility} explicitly.
 */
export const BLOCKED_CONTEXT_SOURCE_IDS: ReadonlySet<string> = new Set([
  "screen-capture", // screen recording / sharing (pipewire portal)
  "camera-mic", // camera/microphone in an active call/meeting
  "idle-focus", // presentation / fullscreen / gaming / focus / Do-Not-Disturb
]);

/**
 * Whether the current context is interruptibility-BLOCKED (Req 26.2/26.3). Pure
 * and deterministic in `inputs`. The context is blocked when either:
 *   • the explicit {@link FocusInputs.interruptibilityBlocked} override is set, or
 *   • any awareness signal reports {@link AwarenessSignal.blocksInterruptibility}
 *     or carries an id in {@link BLOCKED_CONTEXT_SOURCE_IDS}.
 * When blocked the engine adopts the default-silent posture: only RED approvals
 * survive {@link applyInterruptibility}; everything else defers until it clears.
 */
export function isInterruptibilityBlocked(inputs: FocusInputs): boolean {
  if (inputs.interruptibilityBlocked === true) return true;
  // Reading the awareness source is guarded: a broken/throwing awareness
  // subsystem degrades to OMISSION (task 3.5), so it reports no block — it can
  // never break the frame (Req 28.1). Absent awareness ⇒ interruptible.
  try {
    return inputs.awareness.some(
      (s) => s.blocksInterruptibility === true || BLOCKED_CONTEXT_SOURCE_IDS.has(s.id),
    );
  } catch {
    return false;
  }
}

/**
 * Timing / Interruptibility (§24 stage 5, task 3.9, design §26.3). Applies the
 * interruptibility gate: in an INTERRUPTIBLE context this is an identity
 * passthrough (the default posture is untouched); in a BLOCKED context the
 * homepage adopts a DEFAULT-SILENT posture and only RED approvals (risk
 * red/black) may surface — every other subject is DEFERRED until the context
 * clears (Req 26.1/26.2, §26.3). Pure and deterministic in `inputs`.
 *
 * The surfacing RED approval carries only its subject + advisory (calm)
 * `coreHint`; the calm/via-ember/never-audio presentation is signalled by the
 * frame's {@link FocusFrame.blockedContext} flag (set in {@link deriveFocusFrame}).
 * The at-most-one gentle re-surface + age-out rule (Req 26.4) is stateful and
 * lives in {@link createInterruptibilityGate} (clock-injectable, like dwell),
 * keeping this stage pure.
 */
function applyInterruptibility(candidates: ScoredCandidate[], inputs: FocusInputs): ScoredCandidate[] {
  if (!isInterruptibilityBlocked(inputs)) return candidates;
  // Default-silent posture: defer everything but RED approvals (Req 26.2).
  return candidates.filter((c) => c.redApproval === true);
}

/**
 * Personalization (§24 stage 6, task 3.4). Applies BOUNDED learned preferences
 * without reordering the fixed precedence structure (Req 24.7): it only
 * *removes* subjects, never promotes one across a precedence band, changes
 * layout, or auto-acts.
 *
 *   • **Learned-fact frequency cap** (Req 12.7/27.3, §5.6) — when the cap gate is
 *     closed (`learnedFactAllowed === false`) the rare learned-fact band is
 *     withheld this frame so learned-facts stay meaningful.
 *   • **Dismiss preference** (Req 24.7) — a user-dismissed subject is suppressed
 *     from surfacing. Sourced from the existing adaptive-ranking module (the
 *     live layer maps its dismissed set into `inputs.dismissedSubjects`); the
 *     habitual/bounded chip reorder is applied on top in the live layer via the
 *     same module, keeping this stage pure and deterministic.
 *
 * Greeting familiarity (the other §24 stage-6 preference) is applied separately
 * in {@link deriveFocusFrame} from `inputs.greeting` (also pure).
 */
function applyPersonalization(candidates: ScoredCandidate[], inputs: FocusInputs): ScoredCandidate[] {
  let out = candidates;
  if (inputs.learnedFactAllowed === false) {
    out = out.filter((c) => c.priority !== FOCUS_PRIORITY.learnedFact);
  }
  const dismissed = inputs.dismissedSubjects;
  if (dismissed && dismissed.length > 0) {
    const set = new Set(dismissed);
    out = out.filter((c) => !set.has(c.subjectId));
  }
  return out;
}

/**
 * Run the staged pipeline over the signals and return the surfacing candidates,
 * ranked. Stages (design §24): Signals+Understanding (gather) → Reasoning
 * (dedupe) → TTL/expiration filter → Confidence (score + aging + temporal) →
 * Timing/Interruptibility (blocked-context gate, task 3.9) → Personalization
 * (seam) → drop below-floor → Decision (rank). Pure and deterministic in
 * `inputs` (incl. `inputs.now`).
 */
function runPipeline(inputs: FocusInputs): ScoredCandidate[] {
  const raw = gatherRawCandidates(inputs);
  const merged = dedupeSubjects(raw);
  // Cross-cutting TTL/expiration: expired subjects are removed before scoring so
  // they can never surface anywhere (Req 24.3).
  const live = merged.filter((c) => !isExpired(c, inputs.now));
  // Confidence stage: score + classify emphasis.
  const scored: ScoredCandidate[] = live.map((c) => {
    const confidence = computeConfidence(c, inputs.now);
    return { ...c, confidence, emphasis: classifyEmphasis(confidence) };
  });
  const timed = applyInterruptibility(scored, inputs);
  const personalized = applyPersonalization(timed, inputs);
  // Presentation floor: below-floor subjects surface nothing at all (§24 stage 8).
  const surfacing = personalized.filter((c) => c.emphasis !== "hidden");
  return surfacing.sort(compareCandidates);
}

// ─── Pure derivation ──────────────────────────────────────────────────────────

/**
 * Derive the single {@link FocusFrame} from a signal snapshot. Pure and
 * deterministic: same inputs → same frame, and calling it performs NO writes,
 * tool calls, or sends (Property 1). The Voice Line and ACS are always built
 * from the SAME top candidate, so they bind to the same subject (Property 2).
 * Always returns a structurally valid frame (Req 12.5).
 */
export function deriveFocusFrame(inputs: FocusInputs): FocusFrame {
  const ranked = runPipeline(inputs);
  // Interruptibility posture (Req 26.2/26.3): when blocked, the pipeline above
  // has already suppressed every non-RED subject; flag the frame so presentation
  // stays calm (via the ember, never audio). Pure — derived from inputs only.
  const blockedContext = isInterruptibilityBlocked(inputs);

  // Familiarity-scaled greeting (task 3.4). Pure in `inputs.greeting`; omitted
  // entirely when there is no greeting state or the scaling resolves to `none`.
  const greeting = inputs.greeting ? deriveGreeting(inputs.greeting) : undefined;

  // Chips: ranked, de-duplicated by chip id, capped at MAX_CHIPS (Req 5.1).
  // Low-emphasis subjects are allowed here — a chip / Orbit glow is exactly the
  // "low-emphasis only" treatment they earn (§24 stage 8 / Req 24.2).
  const chips: Chip[] = [];
  const seenChips = new Set<string>();
  for (const c of ranked) {
    if (!c.chip || seenChips.has(c.chip.id)) continue;
    seenChips.add(c.chip.id);
    chips.push(c.chip);
    if (chips.length >= MAX_CHIPS) break;
  }

  // Orbit: only LIT points, one per capability of a surfacing candidate (Req 6.2).
  const orbit: OrbitPoint[] = [];
  const seenCaps = new Set<string>();
  for (const c of ranked) {
    if (seenCaps.has(c.capability)) continue;
    seenCaps.add(c.capability);
    orbit.push({
      id: `orbit:${c.capability}`,
      capability: c.capability,
      lit: true,
      label: String(c.capability),
      route: c.actionable ? c.link : undefined,
    });
  }

  // Decision → Presentation (§24 stages 7–8). The Voice Line HEADLINE is the
  // highest-ranked subject that earns at least `medium` emphasis. Low-confidence
  // subjects are skipped here: they can never headline (Req 24.2), only feed the
  // chips/Orbit built above. (Feedback + Learning, §24 stages 9–10, are seams
  // owned by tasks 3.3/3.4 and record nothing here.)
  const headline = ranked.find((c) => c.emphasis === "high" || c.emphasis === "medium");

  // Resting frame: no headline-worthy subject → calm (Voice Line/ACS absent).
  // A familiarity-scaled greeting may still lead (design §5.7 resting output).
  if (!headline) {
    const rest: FocusFrame = { chips, orbit };
    if (greeting) rest.greeting = greeting;
    if (blockedContext) rest.blockedContext = true;
    return rest;
  }

  const frame: FocusFrame = {
    greeting,
    voiceLine: {
      subjectId: headline.subjectId,
      text: headline.voiceText,
      key: headline.key,
      actionable: headline.actionable,
      link: headline.link,
      priority: headline.priority,
      confidence: headline.confidence,
      // Narrowed by the `find` above to high|medium (never low/hidden).
      emphasis: headline.emphasis as "high" | "medium",
    },
    chips,
    orbit,
  };

  // Blocked-context presentation contract (Req 26.2/26.3): the only surfacing
  // subject here is a RED approval; flag the frame so it is shown calmly via the
  // ember and never as audio. The engine has no audio output of its own.
  if (blockedContext) frame.blockedContext = true;

  // Only a HIGH-confidence headline earns the step-forward treatment: the ACS
  // expansion (same subject — Req 8.4) and an advisory `coreHint`. A medium
  // headline is Voice Line only — never a "blaze" (§24 stage 8 / Req 24.2).
  if (headline.emphasis === "high") {
    frame.coreHint = headline.coreHint;
    if (headline.acsTitle && headline.acsLine) {
      frame.acs = {
        subjectId: headline.subjectId,
        title: headline.acsTitle,
        line: headline.acsLine,
        ownerRoute: headline.ownerRoute,
      };
    }
  }

  return frame;
}

// ─── Live read-model (reactive accessor over the real stores) ────────────────

/**
 * Read ONE live signal source, degrading to `fallback` when that subsystem is
 * unavailable or throws (task 3.5, Req 28.1 / design §30). At Tier 0 (Core +
 * Composer only, no stores wired) or when a subsystem errors, its signals are
 * simply absent — the live frame stays valid and calm rather than breaking. A
 * `null`/`undefined` value also degrades to the fallback so an absent source is
 * indistinguishable from an empty one.
 */
function safeRead<T>(read: () => T, fallback: T): T {
  try {
    const value = read();
    return value ?? fallback;
  } catch {
    return fallback;
  }
}

/**
 * Snapshot the authoritative signals from the live domain stores + the
 * awareness bridge. Reading accessors inside a tracking scope makes the derived
 * frame reactive; it performs no writes.
 *
 * Every source is read through {@link safeRead}, so a missing subsystem (Tier 0)
 * or a throwing one degrades independently to its empty/absent fallback (Req
 * 28.1) — the resulting {@link FocusInputs} is always structurally complete and
 * {@link deriveFocusFrame} always yields a valid, calm resting frame at every
 * tier.
 */
function readInputs(): FocusInputs {
  const now = Date.now();
  return {
    approvals: safeRead(() => approvalStore.queue(), []),
    threads: safeRead(() => converseStore.threads(), []),
    activeThreadId: safeRead(() => converseStore.activeThreadId(), null),
    conversing: safeRead(() => converseStore.thinking(), false),
    workflows: safeRead(() => automationStore.workflows(), []),
    facts: safeRead(() => memoryStore.facts(), []),
    notifications: safeRead(() => notificationStore.active(), []),
    awareness: safeRead(() => awarenessBridge.signals(), []),
    now,
    // Familiarity + bounded preference state (task 3.4). All PURE reads — the
    // greeting/adaptive stores own the persistence; reading them never mutates.
    // Guarded the same way so a missing/broken personalization subsystem degrades
    // to "no greeting / no learned preferences" rather than breaking the frame.
    greeting: safeRead(() => homeGreetingStore.readGreetingInput(now), undefined),
    dismissedSubjects: safeRead(() => listAdaptiveDismissed("empty-state"), []),
    learnedFactAllowed: safeRead(() => homeGreetingStore.learnedFactAllowed(now), true),
    // Capability-tier gating (task 3.6). Desktop awareness is OFF by default
    // (Req 25) so its subsystems only contribute once a real bridge is wired;
    // the first-party subsystems are present in the local single-user build.
    availability: safeRead(() => liveAvailability(), undefined),
  };
}

/**
 * The live subsystem availability (task 3.6, design §30). Desktop awareness is
 * OPTIONAL and OFF by default (Req 25): `desktop`/`calendar` only contribute once
 * a real awareness bridge is wired (opt-in). The first-party subsystems
 * (approvals, notifications, conversation, memory, automation) exist in the
 * local single-user build, so they report available; their per-source guards
 * ({@link safeRead}/{@link safeSource}) still degrade them independently if one
 * throws. Pure read — never mutates.
 */
function liveAvailability(): CapabilityAvailability {
  const bridgeWired = awarenessBridge !== EMPTY_BRIDGE;
  return resolveAvailability({ desktop: bridgeWired, calendar: bridgeWired });
}

/**
 * The current queryable {@link CapabilityTierState} (task 3.6). Exposed for later
 * UI/Settings (the "what KRIA can sense" panel, task 3.8) to show which tiers /
 * subsystems are up. Reading it performs no writes and never gates Tier-0
 * usability — Tier 0 stays fully usable regardless of the reported state.
 */
function capabilityState(): CapabilityTierState {
  return resolveCapabilityState(liveAvailability());
}

/**
 * Bounded, presentation-only chip personalization (task 3.4, Req 24.7). REUSES
 * the existing adaptive-ranking module (`empty-state` zone) to apply the user's
 * pinned/dismissed/habitual preferences to the chip order within its ±shift
 * bound. It only reorders/filters chips — it never changes layout, adds a chip,
 * or auto-acts. Applied in the LIVE layer only, so the pure `deriveFocusFrame`
 * stays deterministic for tests.
 */
export function personalizeFrame(frame: FocusFrame): FocusFrame {
  if (frame.chips.length === 0) return frame;
  const ranked = rankAdaptiveSuggestions("empty-state", frame.chips);
  // Ranking is order/subset only; preserve the ≤MAX_CHIPS guarantee.
  return { ...frame, chips: ranked.slice(0, MAX_CHIPS) };
}

/**
 * The single reactive `FocusFrame` accessor. Call it inside a SolidJS reactive
 * scope (component/memo/effect) to react to any upstream signal change. It is a
 * pure read: it never mutates a domain store, never calls a tool, never sends,
 * and never writes `coreStore` (Req 12.5 / 29.3 / 30.3).
 */
function frame(): FocusFrame {
  return deriveFocusFrame(readInputs());
}

// ─── Debounced / incremental recompute (Req 12.4 / 24.4) ─────────────────────

/** Recompute coalescing window: at most one recompute per this many ms (§24.4). */
export const RECOMPUTE_DEBOUNCE_MS = 250;
/** Minimum time a subject stays the headline before a lower one replaces it (§5.4). */
export const MIN_DWELL_MS = 6_000;

/** Opaque timer handle (browser `number` or Node `Timeout`). */
export type TimerHandle = ReturnType<typeof setTimeout>;

/** Injection seams for {@link createRecomputeThrottle} (deterministic in tests). */
export interface RecomputeThrottleOptions {
  /** Coalescing window in ms. Defaults to {@link RECOMPUTE_DEBOUNCE_MS}. */
  intervalMs?: number;
  /** Monotonic clock. Defaults to `Date.now`. */
  now?: () => number;
  /** Arm a one-shot timer. Defaults to `setTimeout`. */
  setTimer?: (fn: () => void, ms: number) => TimerHandle;
  /** Cancel a timer. Defaults to `clearTimeout`. */
  clearTimer?: (handle: TimerHandle) => void;
}

/** A running recompute throttle. */
export interface RecomputeThrottle {
  /** Request a recompute; bursts coalesce to ≤1 run per interval. */
  schedule: () => void;
  /** True while a trailing recompute is armed (an internal timer is pending). */
  readonly pending: boolean;
  /** Cancel any pending recompute and go idle (no timer left running). */
  dispose: () => void;
}

/**
 * INCREMENTAL + DEBOUNCED RECOMPUTE (Req 12.4 / 24.4). A leading-plus-trailing
 * throttle: the first `schedule()` after a quiet period runs `run` immediately
 * (responsive), and any further calls within the window coalesce into a SINGLE
 * trailing run at the window's end. Steady-state cadence is therefore ≤1 run per
 * `intervalMs`, so a burst of signal changes can never thrash the homepage.
 *
 * IDLE-QUIET (bounded resource, steering): a timer is armed ONLY while a trailing
 * run is pending and is cleared as soon as it fires — there is NO perpetual
 * `setInterval`. When no signals change, nothing runs and no timer exists.
 *
 * Pure of side effects beyond calling `run`; fully deterministic under the
 * injected `now`/timer seams.
 */
export function createRecomputeThrottle(
  run: () => void,
  options: RecomputeThrottleOptions = {},
): RecomputeThrottle {
  const intervalMs = options.intervalMs ?? RECOMPUTE_DEBOUNCE_MS;
  const now = options.now ?? (() => Date.now());
  const setTimer = options.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = options.clearTimer ?? ((handle) => clearTimeout(handle));

  let lastRun: number | null = null;
  let timer: TimerHandle | null = null;
  let pending = false;

  function fire(): void {
    timer = null;
    if (!pending) return; // nothing coalesced during the window → stay idle
    pending = false;
    lastRun = now();
    run();
  }

  function schedule(): void {
    // A trailing run is already armed → coalesce into it (no new work/timer).
    if (timer !== null) {
      pending = true;
      return;
    }
    const t = now();
    // Quiet period elapsed → leading-edge run now (responsive), open a window.
    if (lastRun === null || t - lastRun >= intervalMs) {
      lastRun = t;
      run();
      return;
    }
    // Inside the cooldown → arm exactly one trailing run for the remaining time.
    pending = true;
    timer = setTimer(fire, intervalMs - (t - lastRun));
  }

  function dispose(): void {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
    pending = false;
  }

  return {
    schedule,
    get pending() {
      return pending;
    },
    dispose,
  };
}

// ─── Anti-flicker dwell stabilizer (Req 12.4, design §5.4 / Property 5) ───────

/** Injection seams for {@link createDwellStabilizer} (deterministic in tests). */
export interface DwellStabilizerOptions {
  /** Minimum dwell per headline subject in ms. Defaults to {@link MIN_DWELL_MS}. */
  minDwellMs?: number;
  /** Monotonic clock. Defaults to `Date.now`. */
  now?: () => number;
}

/** A running anti-flicker dwell stabilizer. */
export interface DwellStabilizer {
  /**
   * Apply the dwell rule to a freshly-derived frame and return the STABLE frame
   * the homepage should show. Pass an explicit `now` to override the injected
   * clock for a single call (used by the live layer at emit time).
   */
  stabilize: (frame: FocusFrame, now?: number) => FocusFrame;
  /** Forget the held subject (next headline is adopted immediately). */
  reset: () => void;
}

interface HeldHeadline {
  subjectId: string;
  priority: number;
  /** Clock value at which this subject became the headline. */
  since: number;
  /** The frame this subject headlined (its bound Voice Line + ACS + coreHint). */
  frame: FocusFrame;
}

/**
 * ANTI-FLICKER DWELL (Req 12.4, design §5.4, Property 5). Wraps a stream of pure
 * frames with a minimum-dwell rule so the headline does not thrash:
 *   • the first headline is adopted immediately;
 *   • while the SAME subject stays the headline its content refreshes and the
 *     dwell clock keeps running (a continuous subject never re-arms dwell);
 *   • a DIFFERENT subject may take over immediately only if it is STRICTLY
 *     higher precedence (e.g. a needs-you approval preempts anything); a
 *     lower/equal-precedence challenger is HELD OFF until the incumbent has
 *     dwelled `minDwellMs`, at which point it takes over;
 *   • when a frame rests (no headline) the incumbent is released immediately —
 *     dwell prevents flicker between competing subjects, it never keeps showing
 *     a subject that no longer qualifies (Req 3.2 "no stale content").
 *
 * While an incumbent is held against a challenger the returned frame keeps the
 * latest greeting/chips/orbit but restores the incumbent's Voice Line + ACS +
 * coreHint TOGETHER, so they always describe the SAME subject (Property 2).
 *
 * Stateful but deterministic under the injected clock: same frame sequence +
 * same `now` values → same stabilized output. Performs no domain writes.
 */
export function createDwellStabilizer(options: DwellStabilizerOptions = {}): DwellStabilizer {
  const minDwellMs = options.minDwellMs ?? MIN_DWELL_MS;
  const clock = options.now ?? (() => Date.now());
  let held: HeldHeadline | null = null;

  function adopt(next: FocusVoiceLine, frame: FocusFrame, at: number): void {
    held = { subjectId: next.subjectId, priority: next.priority, since: at, frame };
  }

  function stabilize(frame: FocusFrame, nowArg?: number): FocusFrame {
    const at = nowArg ?? clock();
    const next = frame.voiceLine;

    // No incumbent yet → adopt whatever (if anything) headlines.
    if (!held) {
      if (next) adopt(next, frame, at);
      return frame;
    }

    // Frame rests → release immediately; never hold a no-longer-qualifying subject.
    if (!next) {
      held = null;
      return frame;
    }

    // Same subject continues → refresh content, keep the dwell clock running.
    if (next.subjectId === held.subjectId) {
      held = { subjectId: held.subjectId, priority: next.priority, since: held.since, frame };
      return frame;
    }

    // Different subject: strictly higher precedence preempts; otherwise it must
    // wait until the incumbent has dwelled long enough.
    const higherPriority = next.priority > held.priority;
    const dwellElapsed = at - held.since >= minDwellMs;
    if (higherPriority || dwellElapsed) {
      adopt(next, frame, at);
      return frame;
    }

    // Hold the incumbent (anti-flicker). Keep latest ambient content but restore
    // the incumbent's bound headline trio so Voice Line + ACS stay one subject.
    return {
      ...frame,
      voiceLine: held.frame.voiceLine,
      acs: held.frame.acs,
      coreHint: held.frame.coreHint,
    };
  }

  function reset(): void {
    held = null;
  }

  return { stabilize, reset };
}

// ─── Blocked-context re-surface gate (Req 26.4, design §26.3) ────────────────

/**
 * Total times a blocked RED subject may appear in the blocked-context foreground
 * before it ages out: the initial surface + at-most-one gentle re-surface
 * (Req 26.4 "re-surface … at most once (gentle) before it ages out; … SHALL NOT
 * nag"). A third appearance is suppressed (aged out) until the context clears.
 */
export const MAX_BLOCKED_SURFACES = 2;

/** Injection seams for {@link createInterruptibilityGate} (deterministic in tests). */
export interface InterruptibilityGateOptions {
  /**
   * Max foreground appearances per subject while blocked (initial + re-surfaces).
   * Defaults to {@link MAX_BLOCKED_SURFACES} (one gentle re-surface, then age-out).
   */
  maxSurfaces?: number;
}

/** A running interruptibility re-surface/age-out gate. */
export interface InterruptibilityGate {
  /**
   * Apply the blocked-context re-surface rule to a freshly-derived frame and
   * return the frame the homepage should show. In an interruptible context the
   * frame passes through untouched (and re-surface tracking resets, so the next
   * blocked context starts fresh). In a blocked context a RED subject may appear
   * at most {@link InterruptibilityGateOptions.maxSurfaces} times (counting only
   * rising edges — a subject that stays shown does not re-arm); once it exceeds
   * that it ages out (its Voice Line/ACS are dropped, the frame rests calmly)
   * while the block persists.
   */
  gate: (frame: FocusFrame) => FocusFrame;
  /** Forget all re-surface tracking (next blocked appearance counts as the first). */
  reset: () => void;
}

/**
 * BLOCKED-CONTEXT RE-SURFACE GATE (Req 26.4, design §26.3). A stateful,
 * deterministic layer (analogous to {@link createDwellStabilizer}) that enforces
 * "at-most-one gentle re-surface, then age-out" for the RED approvals the pure
 * {@link deriveFocusFrame} lets through in a blocked context. It counts only
 * RISING EDGES (a subject transitioning from not-shown → shown): the first
 * appearance and one re-surface are allowed; any further re-surface is suppressed
 * so KRIA never nags. A subject that simply stays the headline does not consume
 * its budget. When the context becomes interruptible again the tracking resets,
 * so a later block starts the count over.
 *
 * Stateful but performs NO domain writes — the read-model purity invariant
 * (Property 1) still holds; the pure derivation stays untouched for determinism.
 */
export function createInterruptibilityGate(
  options: InterruptibilityGateOptions = {},
): InterruptibilityGate {
  const maxSurfaces = Math.max(1, Math.floor(options.maxSurfaces ?? MAX_BLOCKED_SURFACES));
  /** Foreground-appearance count per subject (rising edges) within the current block. */
  let counts = new Map<string, number>();
  /** The subject shown in the previously-emitted blocked frame (for edge detection). */
  let shown: string | null = null;

  /** Drop the headline trio, leaving a calm resting frame (still blocked). */
  function rested(frame: FocusFrame): FocusFrame {
    const { voiceLine: _v, acs: _a, coreHint: _c, ...rest } = frame;
    return rest;
  }

  function gate(frame: FocusFrame): FocusFrame {
    // Interruptible context → passthrough; reset so the next block starts fresh.
    if (!frame.blockedContext) {
      if (counts.size > 0) counts = new Map();
      shown = null;
      return frame;
    }

    const subjectId = frame.voiceLine?.subjectId ?? null;

    // Blocked but resting (no RED subject) → nothing to gate; note nothing shown.
    if (!subjectId) {
      shown = null;
      return frame;
    }

    // Continuing the same subject → no new rising edge; keep showing it.
    if (subjectId === shown) return frame;

    // Rising edge (new appearance / re-surface): consume one from the budget.
    const next = (counts.get(subjectId) ?? 0) + 1;
    counts.set(subjectId, next);
    if (next > maxSurfaces) {
      // Aged out — suppress the foreground surface (calm rest) but stay blocked.
      shown = null;
      return rested(frame);
    }
    shown = subjectId;
    return frame;
  }

  function reset(): void {
    counts = new Map();
    shown = null;
  }

  return { gate, reset };
}

// ─── Live read-model: debounced + dwell-stabilized reactive frame ────────────

/** A live, disposable Focus-frame accessor. */
export interface LiveFocusFrame {
  /** Reactive accessor to the current stable frame (call inside a Solid scope). */
  frame: () => FocusFrame;
  /** Tear down the throttle timer (also runs automatically on scope cleanup). */
  dispose: () => void;
}

/**
 * Build the LIVE homepage Focus frame: a reactive accessor whose value is the
 * pure {@link deriveFocusFrame} output passed through the debounced recompute
 * throttle (≤1/~250 ms) and the anti-flicker dwell stabilizer (~6 s). Call this
 * inside a SolidJS reactive scope (component / `createRoot`); the throttle is
 * disposed automatically via `onCleanup`.
 *
 * The pure `deriveFocusFrame` / `frame` accessors stay untouched for
 * determinism + testing; this is the stateful delivery layer on top of them.
 */
export function createLiveFocusFrame(options: {
  intervalMs?: number;
  minDwellMs?: number;
} = {}): LiveFocusFrame {
  const stabilizer = createDwellStabilizer({ minDwellMs: options.minDwellMs });
  const interruptibility = createInterruptibilityGate();
  // Pure derivation → bounded adaptive chip personalization → blocked-context
  // re-surface/age-out gate (Req 26.4) → dwell stabilizer. The gate runs before
  // dwell so an aged-out (rested) blocked frame releases the incumbent cleanly.
  const derive = () =>
    stabilizer.stabilize(interruptibility.gate(personalizeFrame(deriveFocusFrame(readInputs()))));
  const [current, setCurrent] = createSignal<FocusFrame>(derive());

  const recompute = () => setCurrent(derive());
  const throttle = createRecomputeThrottle(recompute, { intervalMs: options.intervalMs });

  // Subscribe to every upstream signal; coalesce bursts into ≤1 recompute/window.
  createEffect(() => {
    readInputs(); // read accessors inside the tracking scope to subscribe
    throttle.schedule();
  });

  onCleanup(() => throttle.dispose());

  return { frame: current, dispose: () => throttle.dispose() };
}

export const homeFocusStore = {
  /** Reactive single-frame accessor (the pure Focus engine output). */
  frame,
  /** Pure derivation (exposed for tests + downstream composition). */
  deriveFocusFrame,
  /** Pure familiarity-scaled greeting derivation (task 3.4). */
  deriveGreeting,
  /** Bounded adaptive chip personalization (live layer; reuses adaptive-ranking). */
  personalizeFrame,
  /** Read the current signal snapshot (pure). */
  readInputs,
  /** Notification-suppression predicate (Req 12.4). */
  notificationQualifies,
  /** Queryable capability-tier state for UI/Settings (task 3.6, Req 28). */
  capabilityState,
  /** Pure capability-tier resolver over subsystem availability (task 3.6). */
  resolveCapabilityState,
  /** Normalise partial subsystem availability to a full map (task 3.6). */
  resolveAvailability,
  /** Surfacing subjects tagged with subsystem + tier (task 3.6). */
  capabilitySubjects,
  /** Debounced + dwell-stabilized live frame factory (Req 12.4 / 24.4). */
  createLiveFocusFrame,
  /** Anti-flicker dwell stabilizer factory (design §5.4). */
  createDwellStabilizer,
  /** Blocked-context re-surface/age-out gate factory (Req 26.4, §26.3). */
  createInterruptibilityGate,
  /** Pure blocked-context predicate (Req 26.2/26.3). */
  isInterruptibilityBlocked,
  /** Debounced recompute throttle factory (Req 24.4). */
  createRecomputeThrottle,
  /** Desktop-awareness bridge seam (task 3.7 wires the real one). */
  setAwarenessBridge,
  clearAwarenessBridge,
} as const;
