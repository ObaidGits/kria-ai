/**
 * Recent-item ranking for the Command Palette (Req 2.3, Req 19.1).
 *
 * The palette boosts recently used items so a returning user finds their common
 * targets first — but only ever *reorders* within the result list; it never
 * hides or removes an item (Req 19.1: adaptation is predictable and everything
 * stays reachable via search). Recents persist to localStorage so ranking
 * survives relaunch.
 *
 * This is a plain module (not a Solid store) because ranking is read
 * synchronously during result scoring; there is no reactive UI that needs to
 * observe the recents list directly.
 */

import { adaptiveScore, recordAdaptiveUse, resetAdaptiveSuggestions } from "../adaptive";

const STORAGE_KEY = "kria_palette_recents";
/** How many distinct recent ids to remember (most-recent-first). */
const MAX_RECENTS = 40;

let recents: string[] = loadRecents();

function loadRecents(): string[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((x): x is string => typeof x === "string").slice(0, MAX_RECENTS);
  } catch {
    return [];
  }
}

function persist(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(recents));
  } catch {
    // storage full / unavailable — ranking simply won't persist
  }
}

/**
 * Record that an item was used. Moves it to the front of the recents list
 * (most-recent-first), de-duplicating any prior entry.
 */
export function recordUse(id: string): void {
  if (!id) return;
  recents = [id, ...recents.filter((r) => r !== id)].slice(0, MAX_RECENTS);
  recordAdaptiveUse("palette", id);
  persist();
}

/**
 * The recency rank of an item: 0 = most recent, 1 = next, … or -1 if unseen.
 */
export function recencyRank(id: string): number {
  return recents.indexOf(id);
}

/**
 * A non-negative score boost for an item based on recency. The most recent item
 * gets the largest boost, decaying linearly; unseen items get 0. The boost is
 * deliberately additive with the fuzzy score so a strong text match still wins,
 * but ties (and empty queries) break toward recent items (Req 2.3).
 */
export function recencyBoost(id: string): number {
  return adaptiveScore("palette", id);
}

/** Current recents snapshot (most-recent-first). Primarily for tests. */
export function getRecents(): readonly string[] {
  return recents;
}

/** Clear all recents (Req 19.3 resettable; also used by tests). */
export function clearRecents(): void {
  recents = [];
  resetAdaptiveSuggestions("palette");
  persist();
}
