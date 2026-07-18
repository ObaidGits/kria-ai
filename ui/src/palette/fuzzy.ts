/**
 * Tiny in-house fuzzy matcher for the Command Palette (design.md §1.12).
 *
 * kbar/cmdk are React; there is no mature Solid equivalent, so we ship a small,
 * dependency-free subsequence matcher with fzf-style scoring. It is intentionally
 * simple (linear in target length, no DP table) — the palette ranks at most a few
 * hundred items per keystroke, so the priority is predictable, well-tested
 * ordering over theoretical optimality.
 *
 * Scoring rewards, in order of impact:
 *   • a match at the very start of the target        (prefix bonus)
 *   • matches on word / camelCase boundaries          (boundary bonus)
 *   • consecutive matched characters                  (streak bonus)
 * and penalizes:
 *   • leading gap before the first match              (distance penalty)
 *   • gaps between matched characters                 (gap penalty)
 *   • unmatched trailing length                       (length penalty)
 *
 * An empty query matches everything with a neutral score of 0, so callers can
 * fall back to recency / base ordering when the user has not typed anything.
 *
 * Requirements: 2.1 (fuzzy search), 2.3 (ranking).
 */

export interface FuzzyResult {
  /** Whether every query character was found as an ordered subsequence. */
  matched: boolean;
  /** Relative score; higher is a better match. 0 for an empty query. */
  score: number;
  /** Indices into `target` that matched, in order (for highlighting). */
  indices: number[];
}

const NO_MATCH: FuzzyResult = { matched: false, score: -Infinity, indices: [] };

// Score weights — tuned for the orderings asserted in fuzzy.test.ts.
const PREFIX_BONUS = 20;
const BOUNDARY_BONUS = 12;
const STREAK_BONUS = 8;
const FIRST_GAP_PENALTY = 3; // per char before the first match
const GAP_PENALTY = 1; // per skipped char between matches
const LENGTH_PENALTY = 0.1; // per unmatched target char (favours tighter targets)

function isBoundary(target: string, index: number): boolean {
  if (index === 0) return true;
  const prev = target[index - 1];
  const cur = target[index];
  // Separator before this char (space, dash, underscore, slash, dot).
  if (/[\s\-_/.:]/.test(prev)) return true;
  // camelCase / PascalCase boundary: lower→Upper or digit boundary.
  const lowerToUpper = prev === prev.toLowerCase() && cur === cur.toUpperCase() && cur !== cur.toLowerCase();
  return lowerToUpper;
}

/**
 * Match `query` against `target` (case-insensitive) as an ordered subsequence
 * and return a score + matched indices. Non-matches return `matched: false`.
 */
export function fuzzyMatch(query: string, target: string): FuzzyResult {
  const q = query.trim();
  if (q.length === 0) return { matched: true, score: 0, indices: [] };
  if (target.length === 0) return NO_MATCH;

  const ql = q.toLowerCase();
  const tl = target.toLowerCase();

  const indices: number[] = [];
  let score = 0;
  let ti = 0;
  let prevMatch = -1;

  for (let qi = 0; qi < ql.length; qi++) {
    const ch = ql[qi];
    // Skip whitespace in the query — treat "go home" like "gohome".
    if (ch === " ") continue;

    const found = tl.indexOf(ch, ti);
    if (found === -1) return NO_MATCH;

    indices.push(found);

    if (prevMatch === -1) {
      // First matched char: reward a prefix, penalize a long leading gap.
      if (found === 0) score += PREFIX_BONUS;
      else score -= found * FIRST_GAP_PENALTY;
      if (isBoundary(target, found)) score += BOUNDARY_BONUS;
    } else {
      const gap = found - prevMatch - 1;
      if (gap === 0) score += STREAK_BONUS; // consecutive
      else score -= gap * GAP_PENALTY;
      if (isBoundary(target, found)) score += BOUNDARY_BONUS;
    }

    prevMatch = found;
    ti = found + 1;
  }

  // Favour tighter matches: subtract for target chars left unmatched.
  score -= (target.length - indices.length) * LENGTH_PENALTY;

  return { matched: true, score, indices };
}

/** Convenience: score-only (returns -Infinity for a non-match). */
export function fuzzyScore(query: string, target: string): number {
  return fuzzyMatch(query, target).score;
}
