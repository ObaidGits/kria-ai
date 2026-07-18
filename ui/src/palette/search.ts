/**
 * Palette search + bounded adaptive ranking + grouping (Req 2.1, 2.3, 19.1).
 *
 * Pipeline: items → fuzzy-match each (title weighted highest, then subtitle,
 * then keywords) → deterministic relevance sort → bounded presentation-only
 * adaptation → group by type in canonical order. No action is invoked here.
 */
import { fuzzyMatch } from "./fuzzy";
import { rankPaletteCandidates } from "../adaptive";
import type { PaletteGroup, PaletteItem, PaletteItemType, PaletteResult } from "./types";

/** Sub-field score multipliers (title is the primary matched field). */
const SUBTITLE_WEIGHT = 0.6;
const KEYWORDS_WEIGHT = 0.4;

/** Display order + labels for result groups. */
const GROUP_LABELS: Record<PaletteItemType, string> = {
  space: "Spaces",
  command: "Commands",
  shortcut: "Shortcuts",
  setting: "Settings",
  memory: "Memories",
  workflow: "Workflows",
  capability: "Capabilities",
  model: "Models",
  thread: "Threads",
  device: "Devices",
};

const TYPE_ORDER: readonly PaletteItemType[] = [
  "command",
  "shortcut",
  "space",
  "setting",
  "thread",
  "memory",
  "workflow",
  "capability",
  "model",
  "device",
];

/**
 * Compute the best fuzzy match for an item against a query, considering its
 * title, subtitle and keywords with descending weight. Returns null if nothing
 * matched (and the query is non-empty).
 */
function matchItem(query: string, item: PaletteItem): { score: number; indices: number[] } | null {
  const title = fuzzyMatch(query, item.title);
  let best = title.matched ? { score: title.score, indices: title.indices } : null;

  if (item.subtitle) {
    const sub = fuzzyMatch(query, item.subtitle);
    if (sub.matched) {
      const scaled = sub.score * SUBTITLE_WEIGHT;
      if (!best || scaled > best.score) best = { score: scaled, indices: best?.indices ?? [] };
    }
  }
  if (item.keywords) {
    const kw = fuzzyMatch(query, item.keywords);
    if (kw.matched) {
      const scaled = kw.score * KEYWORDS_WEIGHT;
      if (!best || scaled > best.score) best = { score: scaled, indices: best?.indices ?? [] };
    }
  }
  return best;
}

/**
 * Rank items for a query. Text relevance defines the baseline; persisted
 * frequency/recency may only make bounded presentation shifts afterward.
 */
export function searchItems(items: PaletteItem[], query: string): PaletteResult[] {
  const q = query.trim();
  const results: PaletteResult[] = [];

  for (const item of items) {
    if (q.length === 0) {
      results.push({ item, score: 0, indices: [] });
      continue;
    }
    const m = matchItem(q, item);
    if (!m) continue;
    results.push({ item, score: m.score, indices: m.indices });
  }

  // Text relevance establishes baseline order. Adaptation may move an item by
  // at most MAX_ADAPTIVE_SHIFT and never removes or invokes it.
  results.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return a.item.title.localeCompare(b.item.title);
  });

  return rankPaletteCandidates(
    results.map((result) => ({ id: result.item.id, result })),
  ).map(({ result }) => result);
}

/** Group ranked results by type in the canonical display order. */
export function groupResults(results: PaletteResult[]): PaletteGroup[] {
  const byType = new Map<PaletteItemType, PaletteResult[]>();
  for (const r of results) {
    const arr = byType.get(r.item.type);
    if (arr) arr.push(r);
    else byType.set(r.item.type, [r]);
  }

  const groups: PaletteGroup[] = [];
  for (const type of TYPE_ORDER) {
    const arr = byType.get(type);
    if (arr && arr.length > 0) {
      groups.push({ type, label: GROUP_LABELS[type], results: arr });
    }
  }
  return groups;
}

/** Flatten groups back into a single ordered result list (keyboard nav order). */
export function flattenGroups(groups: PaletteGroup[]): PaletteResult[] {
  return groups.flatMap((g) => g.results);
}
