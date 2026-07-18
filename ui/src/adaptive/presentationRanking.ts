/**
 * Deterministic, bounded presentation adaptation (Req 19.1/19.2).
 *
 * This module ranks candidates only. It never invokes actions, changes runtime
 * state, or participates in orchestration/policy/approval/cancellation paths.
 */
import { createSignal } from "solid-js";

export type AdaptiveZone = "quick-actions" | "empty-state" | "palette";

export interface AdaptiveCandidate {
  id: string;
  /** False keeps this candidate at its exact baseline position. */
  adaptive?: boolean;
}

export interface UsageStat {
  count: number;
  lastSequence: number;
}

interface AdaptivePreferences {
  pinned: Record<string, true>;
  dismissed: Record<string, true>;
}

interface AdaptiveState {
  sequence: number;
  zones: Record<AdaptiveZone, Record<string, UsageStat>>;
  preferences: Record<AdaptiveZone, AdaptivePreferences>;
  retiredCoaches: Record<string, true>;
}

const STORAGE_KEY = "kria_adaptive_presentation_v1";
const ZONES: readonly AdaptiveZone[] = ["quick-actions", "empty-state", "palette"];
export const MAX_ADAPTIVE_SHIFT = 2;
export const MAX_ADAPTIVE_COUNT = 20;
export const MAX_TRACKED_ITEMS_PER_ZONE = 100;
const RECENCY_WINDOW = 40;
const [adaptiveRevision, setAdaptiveRevision] = createSignal(0);

function emptyPreferences(): AdaptivePreferences {
  return { pinned: {}, dismissed: {} };
}

function emptyState(): AdaptiveState {
  return {
    sequence: 0,
    zones: { "quick-actions": {}, "empty-state": {}, palette: {} },
    preferences: {
      "quick-actions": emptyPreferences(),
      "empty-state": emptyPreferences(),
      palette: emptyPreferences(),
    },
    retiredCoaches: {},
  };
}

function clampInteger(value: unknown, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, Math.floor(value)))
    : min;
}
function booleanRecord(value: unknown): Record<string, true> {
  if (!value || typeof value !== "object") return {};
  return Object.fromEntries(
    Object.entries(value)
      .filter(([id, enabled]) => id.length > 0 && enabled === true)
      .slice(0, MAX_TRACKED_ITEMS_PER_ZONE)
      .map(([id]) => [id, true] as const),
  );
}

function loadState(): AdaptiveState {
  const next = emptyState();
  if (typeof window === "undefined") return next;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return next;
    const parsed = JSON.parse(raw) as Partial<AdaptiveState>;
    next.sequence = clampInteger(parsed.sequence, 0, Number.MAX_SAFE_INTEGER);
    for (const zone of ZONES) {
      const source = parsed.zones?.[zone];
      if (source && typeof source === "object") {
        const valid = Object.entries(source)
          .filter(([id, stat]) => id.length > 0 && stat && typeof stat === "object")
          .map(([id, stat]) => [id, {
            count: clampInteger((stat as UsageStat).count, 0, MAX_ADAPTIVE_COUNT),
            lastSequence: clampInteger(
              (stat as UsageStat).lastSequence,
              0,
              next.sequence,
            ),
          }] as const)
          .sort((a, b) => b[1].lastSequence - a[1].lastSequence || a[0].localeCompare(b[0]))
          .slice(0, MAX_TRACKED_ITEMS_PER_ZONE);
        next.zones[zone] = Object.fromEntries(valid);
      }
      const preferences = parsed.preferences?.[zone];
      next.preferences[zone] = {
        pinned: booleanRecord(preferences?.pinned),
        dismissed: booleanRecord(preferences?.dismissed),
      };
    }
    next.retiredCoaches = booleanRecord(parsed.retiredCoaches);
  } catch {
    return emptyState();
  }
  return next;
}

let state = loadState();

function persist(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Storage is optional; in-memory ranking remains deterministic.
  }
}

function commit(): void {
  persist();
  setAdaptiveRevision((value) => value + 1);
}

/** Record presentation use only; callers remain responsible for executing actions. */
export function recordAdaptiveUse(zone: AdaptiveZone, id: string): void {
  if (!id) return;
  const stats = state.zones[zone];
  if (!stats[id] && Object.keys(stats).length >= MAX_TRACKED_ITEMS_PER_ZONE) {
    const oldest = Object.entries(stats).sort(
      (a, b) => a[1].lastSequence - b[1].lastSequence || a[0].localeCompare(b[0]),
    )[0];
    if (oldest) delete stats[oldest[0]];
  }

  state.sequence = state.sequence >= Number.MAX_SAFE_INTEGER ? 1 : state.sequence + 1;
  const previous = stats[id];
  stats[id] = {
    count: Math.min(MAX_ADAPTIVE_COUNT, (previous?.count ?? 0) + 1),
    lastSequence: state.sequence,
  };
  commit();
}

/** Frequency dominates; recency deterministically breaks equal-frequency ties. */
export function adaptiveScore(zone: AdaptiveZone, id: string): number {
  const stat = state.zones[zone][id];
  if (!stat) return 0;
  const age = Math.max(0, state.sequence - stat.lastSequence);
  const recency = Math.max(0, RECENCY_WINDOW - age);
  return stat.count * (RECENCY_WINDOW + 1) + recency;
}

/**
 * Rank from baseline order while preserving every candidate. Adjacent swaps are
 * allowed only when both candidates remain within ±MAX_ADAPTIVE_SHIFT. A
 * protected candidate (`adaptive: false`) is an immovable anchor.
 */
export function rankAdaptiveCandidates<T extends AdaptiveCandidate>(
  zone: AdaptiveZone,
  candidates: readonly T[],
): T[] {
  adaptiveRevision();
  const preferences = state.preferences[zone];
  const ranked = candidates.map((candidate, baseline) => ({
    candidate,
    baseline,
    pinned: preferences.pinned[candidate.id] === true,
    score: adaptiveScore(zone, candidate.id),
  }));

  let changed = true;
  while (changed) {
    changed = false;
    for (let index = 1; index < ranked.length; index += 1) {
      const left = ranked[index - 1];
      const right = ranked[index];
      if (left.candidate.adaptive === false || right.candidate.adaptive === false) continue;
      const shouldPromote = right.pinned !== left.pinned
        ? right.pinned
        : right.score > left.score;
      if (!shouldPromote) continue;
      if (index - 1 < right.baseline - MAX_ADAPTIVE_SHIFT) continue;
      if (index > left.baseline + MAX_ADAPTIVE_SHIFT) continue;
      ranked[index - 1] = right;
      ranked[index] = left;
      changed = true;
    }
  }

  return ranked.map(({ candidate }) => candidate);
}

/** Rank a suggestion zone after honoring explicit dismissals. */
export function rankAdaptiveSuggestions<T extends AdaptiveCandidate>(
  zone: AdaptiveZone,
  candidates: readonly T[],
): T[] {
  adaptiveRevision();
  return rankAdaptiveCandidates(
    zone,
    candidates.filter((candidate) => !isAdaptiveDismissed(zone, candidate.id)),
  );
}

export function rankQuickActions<T extends AdaptiveCandidate>(candidates: readonly T[]): T[] {
  return rankAdaptiveSuggestions("quick-actions", candidates);
}

export function rankEmptyStateCandidates<T extends AdaptiveCandidate>(candidates: readonly T[]): T[] {
  return rankAdaptiveSuggestions("empty-state", candidates);
}

export function rankPaletteCandidates<T extends AdaptiveCandidate>(candidates: readonly T[]): T[] {
  // Palette entries remain searchable even if another suggestion surface was
  // dismissed. Palette ranking is ordering-only and never hides an item.
  return rankAdaptiveCandidates("palette", candidates);
}

/** Pin or unpin a suggestion. Pinning also restores a dismissed suggestion. */
export function setAdaptivePinned(zone: AdaptiveZone, id: string, pinned: boolean): void {
  if (!id) return;
  const preferences = state.preferences[zone];
  if (pinned) {
    preferences.pinned[id] = true;
    delete preferences.dismissed[id];
  } else {
    delete preferences.pinned[id];
  }
  commit();
}

/** Dismiss a suggestion from its zone; it remains reachable elsewhere. */
export function dismissAdaptiveSuggestion(zone: AdaptiveZone, id: string): void {
  if (!id) return;
  const preferences = state.preferences[zone];
  preferences.dismissed[id] = true;
  delete preferences.pinned[id];
  commit();
}

export function isAdaptivePinned(zone: AdaptiveZone, id: string): boolean {
  adaptiveRevision();
  return state.preferences[zone].pinned[id] === true;
}

export function isAdaptiveDismissed(zone: AdaptiveZone, id: string): boolean {
  adaptiveRevision();
  return state.preferences[zone].dismissed[id] === true;
}

/** Plain-language reason shown beside every adaptive suggestion. */
export function explainAdaptiveSuggestion(zone: AdaptiveZone, id: string): string {
  adaptiveRevision();
  if (isAdaptivePinned(zone, id)) return "Pinned by you.";
  const stat = state.zones[zone][id];
  if (!stat) return "Default suggestion.";
  if (stat.count > 1) return "Suggested because you use it often.";
  return "Suggested because you used it recently.";
}

/** Reset ranking, pins, and dismissals while preserving retired coach hints. */
export function resetAdaptiveSuggestions(zone?: AdaptiveZone): void {
  const zones = zone ? [zone] : ZONES;
  for (const current of zones) {
    state.zones[current] = {};
    state.preferences[current] = emptyPreferences();
  }
  if (!zone) state.sequence = 0;
  commit();
}

/** Retire a first-run coach after feature use or explicit dismissal. */
export function retireCoachHint(featureId: string): void {
  if (!featureId || state.retiredCoaches[featureId]) return;
  state.retiredCoaches[featureId] = true;
  commit();
}

/** Coach hints are opt-out once and never reappear unsolicited after use. */
export function shouldShowCoachHint(featureId: string): boolean {
  adaptiveRevision();
  return !!featureId && state.retiredCoaches[featureId] !== true;
}

/** Clear usage signals only. Public legacy helper used by ranking tests. */
export function clearAdaptiveUsage(zone?: AdaptiveZone): void {
  if (zone) state.zones[zone] = {};
  else {
    for (const current of ZONES) state.zones[current] = {};
    state.sequence = 0;
  }
  commit();
}

/** Read-only snapshot used by diagnostics/tests. */
export function getAdaptiveUsage(zone: AdaptiveZone, id: string): Readonly<UsageStat> | undefined {
  adaptiveRevision();
  const stat = state.zones[zone][id];
  return stat ? { ...stat } : undefined;
}
