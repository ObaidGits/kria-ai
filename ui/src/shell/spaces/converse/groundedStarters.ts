/**
 * groundedStarters — cold/new-thread starter prompts GROUNDED in existing
 * enabled/available KRIA capabilities (task 6.4; UIE-L-002, Req 6.6).
 *
 * The Cold Start and Intentional New Thread empty states present a concise
 * orientation plus NO MORE THAN THREE starters. Those starters must demonstrate
 * KRIA-specific value rather than generic chat — but they must never promise a
 * capability that is not actually enabled/available (Req 6.6, UIE-L-002 risk:
 * "Starter promises unavailable capability").
 *
 * ── Grounding is READ-ONLY ───────────────────────────────────────────────────
 * Selection reads already-materialised `capabilityStore` signals (tools, skills,
 * MCP servers, generation availability). It NEVER dispatches a load, invokes a
 * tool, or triggers a side effect (Req 8.4 "no inference"). If a capability list
 * is empty/unknown we simply OMIT that capability-specific starter and fall back
 * to safe generic-but-truthful base starters (ask / remember) — we do not
 * fabricate a capability that is not enabled.
 *
 * Selecting a starter only STAGES an editable draft (handled by the caller); it
 * never sends, runs a tool, or grants approval (Req 6.5 / 6.6, hardened in 6.6).
 *
 * Requirements: 6.6, 5.4
 */
import { capabilityStore } from "../../../stores/capabilityStore";
import { evaluateOmission, getCapabilityFact } from "../../../stores/capabilityFieldMap";

/** A starter prompt — a curated draft staged into the composer when selected. */
export interface ExampleIntent {
  id: string;
  /** Lucide sprite icon id shown on the starter. */
  icon: string;
  /** Short human label shown on the starter (its accessible name). */
  label: string;
  /** Draft text staged into the composer when selected (user reviews first). */
  draft: string;
}

/** Cap enforced for the starter zone (design §11.6: "up to three"). */
export const MAX_STARTERS = 3;

interface StarterCandidate extends ExampleIntent {
  /**
   * READ-ONLY availability predicate. `true` means the capability this starter
   * demonstrates is actually enabled/available right now. Base starters that
   * describe core assistant behaviour (ask / remember) return `true` always and
   * act as the safe generic-but-truthful fallback.
   */
  available: () => boolean;
}

// ─── Grounding predicates (pure reads of already-loaded signals) ─────────────

/**
 * The global tools/MCP registry (F6). Native active tools + MCP servers — the
 * authoritative F6 source (skills are the SEPARATE F7 fact, surfaced by the
 * run-skill starter). Read-only; no per-turn set (M5).
 */
function toolsRegistryValue(): readonly unknown[] {
  return [
    ...capabilityStore.capabilities().filter((c) => c.status === "active"),
    ...capabilityStore.mcpServers(),
  ];
}

/**
 * Native tools / MCP tools give KRIA something to automate. Grounded through
 * the SHARED `capabilityFieldMap` F6 omission rule (task 10.6) so this starter,
 * the 10.5 links, and the 10.6 capability disclosure all ground on the SAME
 * authoritative rule (`show` when the registry is non-empty; else omit) instead
 * of a forked predicate.
 */
function toolsAvailable(): boolean {
  return evaluateOmission(getCapabilityFact("F6"), toolsRegistryValue()) === "show";
}

/** Image generation is only truthful when a backend reports itself available. */
function generateAvailable(): boolean {
  return capabilityStore.generateStatus()?.available === true;
}

/** An installed + enabled skill is required before we offer to run one. */
function skillsEnabled(): boolean {
  return capabilityStore.skills().some((s) => s.installed && s.enabled);
}

/**
 * Ordered starter candidates. Capability-specific starters (KRIA-specific value)
 * come first so they win the ≤3 slots when available; the two base starters are
 * always-truthful fallbacks describing core assistant behaviour.
 */
const STARTER_CANDIDATES: readonly StarterCandidate[] = [
  {
    id: "automate",
    icon: "workflow",
    label: "Automate a task on your computer",
    draft: "Set up an automation to ",
    available: toolsAvailable,
  },
  {
    id: "generate-image",
    icon: "sparkles",
    label: "Generate an image",
    draft: "Generate an image of ",
    available: generateAvailable,
  },
  {
    id: "run-skill",
    icon: "zap",
    label: "Run one of your skills",
    draft: "Run the skill that ",
    available: skillsEnabled,
  },
  // Base starters — core assistant behaviour, always truthful (never promise a
  // substrate). These are the safe generic-but-truthful fallback.
  {
    id: "remember",
    icon: "brain",
    label: "Remember something",
    draft: "Remember that ",
    available: () => true,
  },
  {
    id: "ask",
    icon: "message-circle",
    label: "Ask a question",
    draft: "What can you help me with?",
    available: () => true,
  },
];

/** Strip the internal predicate before handing a starter to presentation. */
function toIntent(candidate: StarterCandidate): ExampleIntent {
  const { available: _available, ...intent } = candidate;
  return intent;
}

/**
 * Never let a grounding read throw into render. A capability signal that is
 * momentarily unavailable simply counts as "not available" (omit the starter).
 */
function isAvailable(candidate: StarterCandidate): boolean {
  try {
    return candidate.available();
  } catch {
    return false;
  }
}

/**
 * Resolve the grounded starters for Cold Start / Intentional New Thread, capped
 * at {@link MAX_STARTERS}. Only starters whose capability is enabled/available
 * are surfaced; the always-truthful base starters guarantee a non-empty, safe
 * result even when no optional capability is available.
 */
export function groundedStarters(): ExampleIntent[] {
  const grounded = STARTER_CANDIDATES.filter(isAvailable).map(toIntent);
  return grounded.slice(0, MAX_STARTERS);
}

/**
 * The safe generic-but-truthful base starters (ask / remember) shown when no
 * capability-specific starter is available. Exported for stories/tests and
 * backward-compatible references.
 */
export const BASE_STARTERS: readonly ExampleIntent[] = STARTER_CANDIDATES.filter(
  (c) => c.id === "remember" || c.id === "ask",
).map(toIntent);
