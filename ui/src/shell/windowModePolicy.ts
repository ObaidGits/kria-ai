import type { WindowMode } from "../stores/shellStore";
import type { Space } from "./router";

/** Explicit Space×mode composition contract from masterplan §52. */
export type SpaceComposition =
  | "conversation-only" | "search-peek" | "run-only" | "capability-lookup"
  | "fleet-glance" | "now-mini" | "settings-search" | "full"
  | "conversation-focus" | "graph-stage" | "builder-stage"
  | "constellation-stage" | "machine-stage" | "monitoring-wall" | "settings-centered";

const COMPOSITION: Record<WindowMode, Record<Space, SpaceComposition>> = {
  compact: {
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
};

export function spaceComposition(space: Space, mode: WindowMode): SpaceComposition {
  return COMPOSITION[mode][space];
}

export const SPACE_MODE_MATRIX = COMPOSITION;
