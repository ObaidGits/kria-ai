import type { WindowMode } from "../../stores/shellStore";

export type WidthProfile = "focus" | "dual" | "assisted" | "full";
export type ConverseSecondaryLane = "threads" | "work" | "context";
export type LaneRelevance = Readonly<Record<ConverseSecondaryLane, boolean>>;

const SEMANTIC_ORDER: readonly ConverseSecondaryLane[] = ["threads", "work", "context"];
const RELEVANCE_PRIORITY: readonly ConverseSecondaryLane[] = ["work", "context", "threads"];
const PROFILE_CAPACITY: Readonly<Record<WidthProfile, number>> = {
  focus: 0,
  dual: 1,
  assisted: 2,
  full: 3,
};

export interface ConverseComposition {
  readonly id: string;
  readonly mode: WindowMode;
  readonly profile: WidthProfile;
  readonly relevantLanes: readonly ConverseSecondaryLane[];
  readonly visibleLanes: readonly ConverseSecondaryLane[];
  readonly threads: boolean;
  readonly work: boolean;
  readonly context: boolean;
}

export function widthProfileFor(width: number): WidthProfile {
  if (width >= 1440) return "full";
  if (width >= 1056) return "assisted";
  if (width >= 736) return "dual";
  return "focus";
}

export function resolveConverseComposition(
  mode: WindowMode,
  profile: WidthProfile,
  relevance: LaneRelevance,
): ConverseComposition {
  const relevantLanes = SEMANTIC_ORDER.filter((lane) => relevance[lane]);
  const selected = new Set(
    RELEVANCE_PRIORITY.filter((lane) => relevance[lane]).slice(0, PROFILE_CAPACITY[profile]),
  );
  const visibleLanes = SEMANTIC_ORDER.filter((lane) => selected.has(lane));
  const relevantBits = SEMANTIC_ORDER.map((lane) => Number(relevance[lane])).join("");
  const visibleId = visibleLanes.join("+") || "conversation";

  return {
    id: `${mode}:${profile}:r-${relevantBits}:v-${visibleId}`,
    mode,
    profile,
    relevantLanes,
    visibleLanes,
    threads: selected.has("threads"),
    work: selected.has("work"),
    context: selected.has("context"),
  };
}
