import type { WindowMode } from "../stores/shellStore";
import type { Space } from "./router";

/** Explicit Space×mode composition contract from masterplan §52. */
export type SpaceComposition =
  | "conversation-only" | "search-peek" | "run-only" | "capability-lookup"
  | "fleet-glance" | "now-mini" | "settings-search" | "full"
  | "conversation-focus" | "graph-stage" | "builder-stage"
  | "constellation-stage" | "machine-stage" | "monitoring-wall" | "settings-centered"
  // Companion (detached cross-application ember, design §29): the whole surface
  // collapses to the ember regardless of Space — Room is none, chips/ACS/orbit
  // are absent, and conversation "opens Mini/Standard". A single distinct
  // composition, NOT a reuse of Mini's per-Space rows.
  | "companion-ember";

const COMPOSITION: Record<WindowMode, Record<Space, SpaceComposition>> = {
  mini: {
    converse: "conversation-only", memory: "search-peek", automations: "run-only",
    capabilities: "capability-lookup", machines: "fleet-glance", observatory: "now-mini",
    settings: "settings-search",
  },
  standard: {
    converse: "full", memory: "full", automations: "full", capabilities: "full",
    machines: "full", observatory: "full", settings: "full",
  },
  immersive: {
    converse: "conversation-focus", memory: "graph-stage", automations: "builder-stage",
    capabilities: "constellation-stage", machines: "machine-stage", observatory: "monitoring-wall",
    settings: "settings-centered",
  },
  // Companion (detached cross-application ember, canonical View Mode axis —
  // Requirements 13.1/13.6). Finalized by task 8.7 against design.md §29: the
  // Companion surface is the ember ONLY (Room none, chips/ACS/orbit hidden,
  // conversation opens Mini/Standard), so every Space collapses to the SAME
  // ember composition rather than mirroring Mini's per-Space rows. The precise
  // per-element show/hide/persist responsibility lives in
  // `viewModeResponsibilityMatrix.ts` (the §29 element matrix); this row is its
  // Space×mode counterpart.
  companion: {
    converse: "companion-ember", memory: "companion-ember", automations: "companion-ember",
    capabilities: "companion-ember", machines: "companion-ember", observatory: "companion-ember",
    settings: "companion-ember",
  },
};

export function spaceComposition(space: Space, mode: WindowMode): SpaceComposition {
  return COMPOSITION[mode][space];
}

export const SPACE_MODE_MATRIX = COMPOSITION;
