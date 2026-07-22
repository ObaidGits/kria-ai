/**
 * Control criticality tiers + status priority (design.md §29 invariants,
 * §11.5, §20.3 overflow row; UIE-H-007, UIE-M-002/003).
 *
 * This module is the single, testable source of truth for WHICH shell controls
 * may move into a labelled overflow and WHICH must always stay directly
 * reachable. It encodes two design orderings as explicit data:
 *
 *   • Affordance priority (§29): "Composer input + Send/scoped Stop + approvals
 *     + recovery never disappear." Plus critical status. → tier `critical`.
 *   • Status priority (§29): "approval/error/scoped control → active work →
 *     relevant context → idle facts." → StatusPriority ranking.
 *
 * It owns NO runtime lifecycle: it is a read-only classification + partition
 * helper. Sub-task 8.6 consumes {@link partitionControls} to adapt the
 * conversation toolbar and Composer per Width Profile; 8.8 consumes the tiers +
 * status ranking for Mini critical disclosure. This task (8.5) only defines
 * the model and the one overflow primitive ({@link ./OverflowControl}).
 *
 * Requirements: 11.1, 11.2, 10.1–10.3 (via consumers), 16.3–16.5
 */

/**
 * Criticality tier for an interactive control.
 * - `critical`  — never placed in overflow; always directly reachable.
 * - `primary`   — prefer inline; may overflow only when space forces it, and
 *                 only AFTER every `secondary` control has already overflowed.
 * - `secondary` — first to move into the labelled overflow.
 */
export type CriticalityTier = "critical" | "primary" | "secondary";

/** Lower rank = higher priority = kept inline first. */
export const TIER_RANK: Record<CriticalityTier, number> = {
  critical: 0,
  primary: 1,
  secondary: 2,
};

/** A control tagged with its tier. `label` feeds the overflow menu item. */
export interface TieredControl {
  /** Stable control id (also the overflow MenuItem id). */
  id: string;
  tier: CriticalityTier;
  /** Human label used when the control is rendered inside the overflow menu. */
  label?: string;
}

/**
 * Comparator ordering controls by criticality (critical → primary → secondary).
 * Stable for equal tiers (returns 0), so callers keep source order within a tier.
 */
export function compareByTier(a: TieredControl, b: TieredControl): number {
  return TIER_RANK[a.tier] - TIER_RANK[b.tier];
}

export interface PartitionResult {
  /** Controls that stay directly reachable, in original order. */
  inline: TieredControl[];
  /** Controls moved into the labelled overflow, in original order. */
  overflow: TieredControl[];
}

/**
 * Partition controls into `inline` vs `overflow` for a given inline capacity.
 *
 * Invariants (design §29 "Affordance priority" + §Disclosure-overuse risk):
 *  1. `critical` controls are NEVER placed in overflow — even when
 *     `maxInline` is 0 or smaller than the number of critical controls.
 *  2. `secondary` controls move into overflow BEFORE any `primary` control.
 *  3. Original relative order is preserved within each partition.
 *
 * `maxInline` is a capacity hint (how many controls fit directly). It bounds
 * only NON-critical controls; criticals are always inline regardless.
 */
export function partitionControls(
  controls: readonly TieredControl[],
  maxInline: number,
): PartitionResult {
  const criticals = controls.filter((c) => c.tier === "critical");
  const nonCritical = controls.filter((c) => c.tier !== "critical");

  // Remaining inline slots after criticals are seated (never negative).
  const remaining = Math.max(0, maxInline - criticals.length);

  // Fill remaining slots preferring `primary` over `secondary`, so `secondary`
  // is the first to overflow (invariant 2). Stable within each tier.
  const rankedNonCritical = [...nonCritical].sort(compareByTier);
  const keptInline = new Set<string>(
    rankedNonCritical.slice(0, remaining).map((c) => c.id),
  );

  const inline: TieredControl[] = [];
  const overflow: TieredControl[] = [];
  for (const c of controls) {
    if (c.tier === "critical" || keptInline.has(c.id)) {
      inline.push(c);
    } else {
      overflow.push(c);
    }
  }
  return { inline, overflow };
}

/**
 * Status fact priority (design §29 "Status priority"):
 * approval/error/scoped control → active work → relevant context → idle facts.
 */
export type StatusPriority = "critical" | "active-work" | "context" | "idle";

/** Lower rank = higher priority = surfaced/kept first. */
export const STATUS_RANK: Record<StatusPriority, number> = {
  critical: 0,
  "active-work": 1,
  context: 2,
  idle: 3,
};

/** Comparator ordering status facts by the design status priority. */
export function compareStatusPriority(a: StatusPriority, b: StatusPriority): number {
  return STATUS_RANK[a] - STATUS_RANK[b];
}

/**
 * Ids of the affordances that §29 declares must never disappear. Exposed so
 * consumers and tests can assert the invariant data-drivenly.
 */
export const CRITICAL_CONTROL_IDS = [
  "composer-input",
  "send-stop",
  "approvals",
  "error-recovery",
  "critical-status",
] as const;

export type CriticalControlId = (typeof CRITICAL_CONTROL_IDS)[number];

/**
 * Canonical Converse control map (toolbar + Composer), from the task-8.1
 * inventory. This is the data 8.6 partitions by Width Profile.
 *
 * critical  — Composer input, Send⇄Stop, approvals access, error/recovery,
 *             critical status. Always directly reachable.
 * primary   — active toggles + Composer tools 8.6 must preserve (context-rail
 *             toggle, mode chip, attach, voice).
 * secondary — convenience actions that overflow first (export, detach,
 *             open-sidebar).
 */
export const CONVERSE_CONTROLS: readonly TieredControl[] = [
  // Critical affordances (§29 "never disappear").
  { id: "composer-input", tier: "critical", label: "Message" },
  { id: "send-stop", tier: "critical", label: "Send" },
  { id: "approvals", tier: "critical", label: "Approvals" },
  { id: "error-recovery", tier: "critical", label: "Retry" },
  { id: "critical-status", tier: "critical", label: "Status" },
  // Primary: active toggles + preserved Composer tools.
  { id: "context-rail-toggle", tier: "primary", label: "Toggle context rail" },
  { id: "mode-chip", tier: "primary", label: "Mode" },
  { id: "attach", tier: "primary", label: "Attach" },
  { id: "voice", tier: "primary", label: "Voice" },
  // Secondary: convenience actions, first to overflow.
  { id: "export", tier: "secondary", label: "Export" },
  { id: "detach", tier: "secondary", label: "Detach current thread" },
  { id: "open-sidebar", tier: "secondary", label: "Open thread sidebar" },
];

/** Tier lookup for a control id in {@link CONVERSE_CONTROLS} (undefined if unknown). */
export function controlTier(id: string): CriticalityTier | undefined {
  return CONVERSE_CONTROLS.find((c) => c.id === id)?.tier;
}
