/**
 * Capability / context field map — a PURE, READ-ONLY descriptor table (IU-07;
 * UIE-H-002, UIE-M-018). Task 10.2.
 *
 * This module is the single source of truth for HOW each existing authoritative
 * frontend fact (F1–F12, inventoried in
 * `evidence/task-10.1-capability-field-inventory.md`) may be surfaced by the
 * Task 10 consumers (10.3 Homepage summary, 10.4 ContextRail enrichment, 10.5
 * detail links, 10.6 grounded descriptors, 10.7 bounded/omission presentation).
 *
 * It does NOT hold data, signals, setters, timers, or side effects. It only:
 *   • names the authoritative source accessor for each fact (read-only ref),
 *   • records the ONE authoritative owner surface (one-fact-one-home, §8.6),
 *   • classifies freshness/lifecycle and available-vs-used meaning,
 *   • gives a bounded display label,
 *   • encodes an omission rule as a pure predicate (present → "show",
 *     absent/unknown → "omit", optional-service offline → "unavailable"),
 *   • points at an EXISTING detail destination (Space + optional Inspector
 *     type / segment) — never a new route or dashboard,
 *   • and explicitly flags the §2 MUST-OMIT facts that have NO authoritative
 *     field so a future consumer that tries to read them is guided to omit.
 *
 * The omission predicates REUSE the existing `nonEmpty` discipline from
 * `currentWorkSummary.ts` (imported, not forked) so every surface omits
 * absent/unknown values identically (Req 8.4; design §20.1 read-only-projection
 * invariant). Encoding the omission discipline HERE is what stops 10.3–10.7
 * from fabricating a fact the runtime cannot prove.
 *
 * Resolves inventory gaps:
 *   • G1 — no per-fact label / omission rule / freshness: this table defines all
 *          three for every fact.
 *   • G7 — active model (F1) is CONFIGURED / available-next-turn, NOT the model
 *          that provably produced this answer: F1.kind === "available" and
 *          F1.freshness === "configured" encode that so presentation cannot
 *          imply per-turn consumption.
 *
 * Requirements: 8.1–8.7, 9.3, 13.3; design §8.6, §20.1; UIE-H-002, UIE-M-011,
 * UIE-M-018, UIE-M-019.
 */
import type { Space } from "../shell/router";
import { nonEmpty } from "./currentWorkSummary";

// ─── Classification vocabulary ──────────────────────────────────────────────────

/** Stable fact ids, matching the 10.1 inventory §1 table (F1–F12). */
export type CapabilityFactId =
  | "F1"
  | "F2"
  | "F3"
  | "F4"
  | "F5"
  | "F6"
  | "F7"
  | "F8"
  | "F9"
  | "F10"
  | "F11"
  | "F12";

/**
 * Freshness / lifecycle classification of the authoritative source:
 *   • "live"       — event-driven, updates within a turn (cleared on thread switch).
 *   • "loaded"     — fetched on section/Space load; may be stale until reloaded.
 *   • "static"     — set once per message/turn, never mutated afterwards.
 *   • "configured" — available / next-turn state, NOT per-turn consumed (the G7 class).
 */
export type FactFreshness = "live" | "loaded" | "static" | "configured";

/**
 * Available-vs-used meaning:
 *   • "available" — configured / installed / enabled; describes what CAN be used.
 *   • "used"      — provably consumed by this turn / message.
 *   • "n/a"       — neither applies (ambient store or a field that carries no
 *                   use-state, e.g. the context rail item shape).
 */
export type FactKind = "available" | "used" | "n/a";

/**
 * The outcome of an omission rule for a concrete authoritative value:
 *   • "show"        — the source provides the fact; surface it (bounded).
 *   • "omit"        — absent / unknown / empty; render NOTHING (never inferred).
 *   • "unavailable" — an optional service is present-but-offline; a surface MAY
 *                     show an explicit "unavailable" state rather than nothing.
 */
export type OmissionOutcome = "show" | "omit" | "unavailable";

/** An EXISTING detail destination — never a new route/dashboard (one-fact-one-home). */
export interface FactDetailDestination {
  /** The owning Space route (existing). */
  readonly space: Space;
  /** Optional existing Space segment (e.g. "models", "tools", "skills"). */
  readonly segment?: string;
  /**
   * Optional existing shared-Inspector target type (§inspectorRegistry). One of
   * the registered types only — the shell adds no new Inspector type here.
   */
  readonly inspectorType?:
    | "memory"
    | "capability"
    | "automation-node"
    | "device"
    | "observatory";
  /**
   * Optional existing non-Space surface that owns the fact (PresenceBar,
   * StatusLine, WorkLane, GuiCognitionPanel, ContextRail, Approval Center).
   * Present when the authoritative home is a shell surface, not a Space route.
   */
  readonly surface?: string;
}

/**
 * A single fact descriptor. `V` is the natural shape of the authoritative value
 * the omission rule inspects, so each rule stays type-checked at its definition
 * site while the table erases to a common supertype for iteration.
 */
export interface CapabilityFactDescriptor<V = unknown> {
  readonly id: CapabilityFactId;
  /** Human category, matching the 10.1 inventory. */
  readonly category: string;
  /**
   * The authoritative source accessor, referenced BY NAME (read-only). This map
   * never calls it or duplicates its data — consumers read the real accessor.
   */
  readonly sourceAccessor: string;
  /** The single authoritative store owner. */
  readonly owner: string;
  /** The single authoritative surface (one-fact-one-home). */
  readonly ownerSurface: string;
  readonly freshness: FactFreshness;
  readonly kind: FactKind;
  /** Concise, bounded human label to show. */
  readonly displayLabel: string;
  /**
   * Pure predicate: authoritative value → show / omit / unavailable. Mirrors the
   * `nonEmpty` / `deriveModel` discipline. Deterministic, side-effect free.
   */
  readonly omissionRule: (value: V) => OmissionOutcome;
  /** Existing detail destination for this fact. */
  readonly detailDestination: FactDetailDestination;
  /** True when `currentWorkSummary.ts` already projects this fact (dedupe hint). */
  readonly inSummary: boolean;
  /** Short clarifying note (e.g. the G7 rationale). */
  readonly note?: string;
}

/** Type-preserving descriptor constructor (keeps `V` at the definition site). */
function defineFact<V>(d: CapabilityFactDescriptor<V>): CapabilityFactDescriptor<V> {
  return d;
}

// ─── Minimal authoritative value shapes the omission rules inspect ──────────────
// Structural subsets of the real store types (10.1 §1) — enough for the rule to
// decide show/omit/unavailable without importing or duplicating the full types.

/** F1 subset of `capabilityStore.activeLlmRuntime()`. */
export interface ActiveModelValue {
  readonly providerId?: string | null;
  readonly activeModel?: string | null;
}
/** F7 subset: OpenClaw runtime + installed skill count. */
export interface OpenClawValue {
  readonly settings: { readonly runtimeActive?: boolean } | null;
  readonly skillCount: number;
}

// ─── Reusable omission predicates (built on the shared `nonEmpty` discipline) ───

/** Present when a countable collection is non-empty; else omit. */
const showIfNonEmpty = (count: number): OmissionOutcome => (count > 0 ? "show" : "omit");

// ─── F1–F12 descriptor table ────────────────────────────────────────────────────

/** F1 — Active model. AVAILABLE / configured (G7), NOT per-turn consumed. */
export const F1_ACTIVE_MODEL = defineFact<ActiveModelValue | null>({
  id: "F1",
  category: "Active model",
  sourceAccessor: "capabilityStore.activeLlmRuntime",
  owner: "capabilityStore",
  ownerSurface: "Capabilities Space → Models",
  freshness: "configured", // loaded + apply-status events; NOT per-turn (G7)
  kind: "available", // what WILL be used next turn — never "used" (G7)
  displayLabel: "Model",
  // Mirrors deriveModel: null OR (no providerId AND no activeModel) → omit; never
  // surface the source's "Not configured" placeholder as a fact.
  omissionRule: (v) => {
    if (!v) return "omit";
    const id = nonEmpty(v.providerId);
    const model = nonEmpty(v.activeModel);
    return id || model ? "show" : "omit";
  },
  detailDestination: { space: "capabilities", segment: "models", inspectorType: "capability" },
  inSummary: true,
  note: "G7: configured/available next-turn model, not proof of the model that produced this answer. Presentation must not imply per-turn consumption.",
});

/** F2 — Available context (rail). Empty in production; never auto-open (G3). */
export const F2_CONTEXT_RAIL = defineFact<readonly unknown[]>({
  id: "F2",
  category: "Available context (rail)",
  sourceAccessor: "converseStore.contextRail",
  owner: "converseStore",
  ownerSurface: "ContextRail lane (Converse)",
  freshness: "live", // set/cleared on thread switch; no production writer today
  kind: "available", // item shape carries no source/relevance/use-state field
  displayLabel: "Context",
  // Empty rail is the normal production state → omit; a consumer must NOT
  // auto-open an empty rail (UIE-H-002 regression).
  omissionRule: (items) => showIfNonEmpty(items.length),
  detailDestination: { space: "converse", surface: "ContextRail", inspectorType: "memory" },
  inSummary: true,
  note: "No production runtime writer; rail is empty by default. Available-vs-used is not representable on the item shape (routed to 10.4 field-only enrichment).",
});

/** F3 — Consumed / used memory context. The only per-turn USED provenance. */
export const F3_USED_MEMORY = defineFact<readonly string[] | undefined>({
  id: "F3",
  category: "Consumed / used context (memory)",
  sourceAccessor: "converseStore Message.usedMemoryIds",
  owner: "converseStore",
  ownerSurface: "Memory Space + Inspector (memory) via whyDidKriaAnswer",
  freshness: "static", // set at message creation; never mutated
  kind: "used", // memory-only per-answer consumed provenance
  displayLabel: "Used memory",
  // Optional/empty → hide the affordance (no fabricated link).
  omissionRule: (ids) => showIfNonEmpty(ids?.length ?? 0),
  detailDestination: { space: "memory", segment: "explorer", inspectorType: "memory" },
  inSummary: false,
  note: "Memory-only. Document/tool consumed provenance has no field → must be omitted, not synthesized from the rail.",
});

/** F4 — Memory facts / contribution. Knowledge store (not a per-turn used set). */
export const F4_MEMORY_FACTS = defineFact<readonly unknown[]>({
  id: "F4",
  category: "Memory facts / contribution",
  sourceAccessor: "memoryStore.facts",
  owner: "memoryStore",
  ownerSurface: "Memory Space",
  freshness: "loaded", // refreshed (debounced) on memory:updated/deleted
  kind: "n/a",
  displayLabel: "Memory",
  omissionRule: (facts) => showIfNonEmpty(facts.length),
  detailDestination: { space: "memory", inspectorType: "memory" },
  inSummary: false,
});

/** F5 — Tool activity (live). This turn's calls. */
export const F5_TOOL_ACTIVITY = defineFact<readonly unknown[]>({
  id: "F5",
  category: "Tool activity (live)",
  sourceAccessor: "converseStore.workBlocks (type: tool-call)",
  owner: "converseStore",
  ownerSurface: "WorkLane → WorkBlock",
  freshness: "live", // agent:tool-call / agent:tool-result; cleared on thread switch
  kind: "used",
  displayLabel: "Tool activity",
  omissionRule: (blocks) => showIfNonEmpty(blocks.length),
  detailDestination: { space: "converse", surface: "WorkLane" },
  inSummary: true,
});

/** F6 — Tools / MCP registry (available). */
export const F6_TOOLS_REGISTRY = defineFact<readonly unknown[]>({
  id: "F6",
  category: "Tools registry (available)",
  sourceAccessor: "capabilityStore.capabilities / capabilityStore.mcpServers",
  owner: "capabilityStore",
  ownerSurface: "Capabilities Space → Tools/Integrations",
  freshness: "loaded",
  kind: "available",
  displayLabel: "Tools",
  // Empty until loaded → omit (honest empty, guarded by loading()).
  omissionRule: (caps) => showIfNonEmpty(caps.length),
  detailDestination: { space: "capabilities", segment: "tools", inspectorType: "capability" },
  inSummary: false,
});

/** F7 — OpenClaw skills. Optional service → offline shows "unavailable". */
export const F7_OPENCLAW_SKILLS = defineFact<OpenClawValue>({
  id: "F7",
  category: "OpenClaw skills",
  sourceAccessor: "capabilityStore.skills / remoteSkills / openClawSettings",
  owner: "capabilityStore",
  ownerSurface: "Capabilities Space → Skills + Governance",
  freshness: "loaded",
  kind: "available",
  displayLabel: "Skills",
  // Optional service: settings null OR runtime not active → explicit "unavailable"
  // (offline optional-service state, not a fabricated fact). Runtime active but no
  // installed skills → omit. Installed skills present → show.
  omissionRule: ({ settings, skillCount }) => {
    if (!settings || settings.runtimeActive !== true) return "unavailable";
    return showIfNonEmpty(skillCount);
  },
  detailDestination: { space: "capabilities", segment: "skills", inspectorType: "capability" },
  inSummary: false,
  note: "Optional substrate: degrades to 'unavailable' when offline; never inferred as enabled.",
});

/** F8 — Automations / background work. USED (running) vs configured via status. */
export const F8_AUTOMATIONS = defineFact<readonly unknown[]>({
  id: "F8",
  category: "Automations / background work",
  sourceAccessor: "automationStore.workflows / runningWorkflowIds / runProgress",
  owner: "automationStore",
  ownerSurface: "Automations Space",
  freshness: "live", // n8n status subscription + automation:* events
  kind: "used",
  displayLabel: "Automations",
  omissionRule: (workflows) => showIfNonEmpty(workflows.length),
  detailDestination: { space: "automations", inspectorType: "automation-node" },
  inSummary: false,
  note: "Not read by currentWorkSummary today (G5); 10.3/10.5 extend the read-only summary and link here.",
});

/** F9 — Background workflow sessions. USED (running/paused). */
export const F9_WORKFLOW_SESSIONS = defineFact<readonly unknown[]>({
  id: "F9",
  category: "Background workflow sessions",
  sourceAccessor: "workflowStore.recentSessions / activeSession",
  owner: "workflowStore",
  ownerSurface: "Automations Space run views + Approval Center (HITL)",
  freshness: "live", // WorkflowTelemetry envelopes
  kind: "used",
  displayLabel: "Workflow runs",
  omissionRule: (sessions) => showIfNonEmpty(sessions.length),
  detailDestination: { space: "automations", surface: "Approval Center" },
  inSummary: false,
  note: "HITL routes to the single Approval Center; never auto-launched.",
});

/** F10 — Planning / reasoning. USED (this turn). */
export const F10_PLANNING = defineFact<readonly unknown[]>({
  id: "F10",
  category: "Planning / reasoning",
  sourceAccessor: "converseStore.workBlocks (reasoning|plan-compare) / guiCognitionSession",
  owner: "converseStore + guiCognitionSession + coreStore",
  ownerSurface: "WorkLane WorkBlock / GuiCognitionPanel",
  freshness: "live",
  kind: "used",
  displayLabel: "Planning",
  omissionRule: (blocks) => showIfNonEmpty(blocks.length),
  detailDestination: { space: "converse", surface: "WorkLane" },
  inSummary: true,
});

/** F11 — GUI cognition. USED (active turn); null when idle. */
export const F11_GUI_COGNITION = defineFact<unknown | null>({
  id: "F11",
  category: "GUI cognition",
  sourceAccessor: "activeGuiCognitionSession / guiCognitionRoutingStatus",
  owner: "guiCognitionSession",
  ownerSurface: "GuiCognitionPanel / WorkLane",
  freshness: "live", // gui-cognition:event; null when lifecycle === "idle"
  kind: "used",
  displayLabel: "GUI cognition",
  // null (idle) → omit; any live session → show.
  omissionRule: (session) => (session ? "show" : "omit"),
  detailDestination: { space: "converse", surface: "GuiCognitionPanel" },
  inSummary: true,
});

/** F12 — Active Space / activity narration. Space always known; error optional. */
export const F12_SPACE_ACTIVITY = defineFact<{ readonly error?: string | null } | null>({
  id: "F12",
  category: "Active Space / activity narration",
  sourceAccessor: "shellStore.activeSpace / coreStore.state|errorMessage|blockReason",
  owner: "shellStore + coreStore",
  ownerSurface: "PresenceBar / StatusLine (IU-06)",
  freshness: "live",
  kind: "n/a",
  displayLabel: "Activity",
  // Active Space is always known → always "show"; the optional error/block
  // message is handled by currentWorkSummary's error fact (omitted when absent).
  omissionRule: () => "show",
  detailDestination: { space: "converse", surface: "StatusLine" },
  inSummary: true,
  note: "Already flows through currentWorkSummary → PresenceBar/StatusLine; consumers LINK to the one owner, never restate (G9).",
});

/**
 * The complete F1–F12 map. Keyed by fact id. This is the single table 10.3–10.7
 * consume. Descriptors erase to the common supertype so the table is iterable;
 * each rule keeps its precise value type at its definition site above.
 */
export const CAPABILITY_FIELD_MAP: Readonly<
  Record<CapabilityFactId, CapabilityFactDescriptor>
> = {
  F1: F1_ACTIVE_MODEL as CapabilityFactDescriptor,
  F2: F2_CONTEXT_RAIL as CapabilityFactDescriptor,
  F3: F3_USED_MEMORY as CapabilityFactDescriptor,
  F4: F4_MEMORY_FACTS as CapabilityFactDescriptor,
  F5: F5_TOOL_ACTIVITY as CapabilityFactDescriptor,
  F6: F6_TOOLS_REGISTRY as CapabilityFactDescriptor,
  F7: F7_OPENCLAW_SKILLS as CapabilityFactDescriptor,
  F8: F8_AUTOMATIONS as CapabilityFactDescriptor,
  F9: F9_WORKFLOW_SESSIONS as CapabilityFactDescriptor,
  F10: F10_PLANNING as CapabilityFactDescriptor,
  F11: F11_GUI_COGNITION as CapabilityFactDescriptor,
  F12: F12_SPACE_ACTIVITY as CapabilityFactDescriptor,
};

/** Every fact id, in inventory order. */
export const ALL_CAPABILITY_FACT_IDS: readonly CapabilityFactId[] = [
  "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

// ─── MUST-OMIT facts (§2): NO authoritative field → NEVER surface ───────────────

/** A spec-mentioned fact that has NO authoritative frontend field today. */
export interface MustOmitFact {
  /** Stable id (M1–M6). */
  readonly id: string;
  readonly label: string;
  /** Spec touchpoint that mentions it. */
  readonly specTouchpoint: string;
  /** Why it must never be surfaced (would be fabrication). */
  readonly reason: string;
}

/**
 * The §2 must-omit list. A consumer that reaches for any of these MUST omit it
 * (or, for an optional service, show an explicit "unavailable") — never infer a
 * value. `NEVER_SURFACE` is the invariant: there is no authoritative field.
 */
export const MUST_OMIT_FACTS: readonly MustOmitFact[] = [
  {
    id: "M1",
    label: "Token / context-window budget (available/consumed tokens, prompt/completion length, context %)",
    specTouchpoint: "UIE-M-018 consumed context",
    reason: "No frontend field exists in any store. Inventing a number = fabrication.",
  },
  {
    id: "M2",
    label: "Available-vs-consumed use-state on rail items",
    specTouchpoint: "UIE-M-011 separate available from consumed",
    reason: "ContextRailItem has only {id,type,label,data} — no source/relevance/use field. Not representable without a field change (10.4, field-only).",
  },
  {
    id: "M3",
    label: "Populated ContextRail in production",
    specTouchpoint: "UIE-H-002 / UIE-M-011",
    reason: "No runtime writer (setContextRailItems is test-only). Rail is empty in production → never auto-open; show honest empty/unavailable.",
  },
  {
    id: "M4",
    label: "Consumed context beyond memory (documents/tool-results fed to the model this turn)",
    specTouchpoint: "UIE-H-011 used-context provenance",
    reason: "Only usedMemoryIds (memory) exists. Non-memory consumed provenance has no field → omit; never synthesize from the rail.",
  },
  {
    id: "M5",
    label: "Per-turn 'tools available to this turn'",
    specTouchpoint: "UIE-M-019",
    reason: "Only a global capabilities() registry exists, not a turn-scoped availability set. Ground cues in global enabled/installed state only.",
  },
  {
    id: "M6",
    label: "Model context length / max-tokens / quantization of the active runtime",
    specTouchpoint: "UIE-M-018",
    reason: "ActiveLlmRuntime has none of these. Omit unless surfacing local-model capabilities[] where authoritative.",
  },
];

/** The invariant outcome for any must-omit fact: it has no field, so never show. */
export const NEVER_SURFACE = "omit" as const satisfies OmissionOutcome;

/** Set of must-omit ids for quick membership checks by consumers/tests. */
export const MUST_OMIT_FACT_IDS: ReadonlySet<string> = new Set(
  MUST_OMIT_FACTS.map((f) => f.id),
);

// ─── Consumer helpers (read-only) ───────────────────────────────────────────────

/** Look up a descriptor by id. */
export function getCapabilityFact(id: CapabilityFactId): CapabilityFactDescriptor {
  return CAPABILITY_FIELD_MAP[id];
}

/**
 * Evaluate a fact's omission rule against an authoritative value. Thin, typed
 * pass-through so consumers apply the SAME rule the table defines (no forking).
 */
export function evaluateOmission<V>(
  descriptor: CapabilityFactDescriptor<V>,
  value: V,
): OmissionOutcome {
  return descriptor.omissionRule(value);
}
