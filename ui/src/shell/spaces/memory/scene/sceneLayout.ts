/**
 * memory/scene/sceneLayout — Deterministic layout hint computation.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Computes deterministic layout seeds from query hash + revision,
 * selects the appropriate layout strategy based on query context,
 * and returns a complete SemanticLayoutHint.
 *
 * Design invariants (F4.6): equal input → equal seed → equal layout hint.
 *
 * IDs: MGD-003, MGD-046; MG-M09–M11.
 */

import type { LayoutStrategy, SemanticLayoutHint } from "./semanticScene";

// ─── Public types ──────────────────────────────────────────────────────────────

export type QueryContext = {
  queryKind: 'search' | 'overview' | 'ego' | 'path' | 'temporal' | 'goal' | 'source';
  queryHash: string;        // hash of the query parameters
  graphRevision: number;    // current graph revision
  primaryItemId: string | null;  // ego node for 'ego', start for 'path'
  maxDepth: number | null;       // optional depth limit for path/ego
};

// ─── Strategy mapping ──────────────────────────────────────────────────────────

const QUERY_KIND_TO_STRATEGY = {
  search:   'search-treemap-grid',
  overview: 'search-treemap-grid',
  ego:      'ego-radial-rings',
  path:     'path-layered-dag',
  temporal: 'temporal-lanes',
  goal:     'goal-source-grouped-lane',
  source:   'goal-source-grouped-lane',
} as const satisfies Record<QueryContext['queryKind'], LayoutStrategy>;

// ─── Seed computation ──────────────────────────────────────────────────────────

/**
 * Computes a deterministic positive 32-bit integer seed from queryHash and
 * graphRevision using FNV-1a over the combined string "queryHash:revision".
 *
 * Same inputs always produce the same seed.
 */
export function computeLayoutSeed(queryHash: string, graphRevision: number): number {
  const input = queryHash + ":" + String(graphRevision);
  let hash = 0x811c9dc5; // FNV-1a 32-bit offset basis
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    // FNV prime 32-bit: 0x01000193; >>> 0 coerces to unsigned 32-bit
    hash = (Math.imul(hash, 0x01000193) >>> 0);
  }
  // Ensure positive: hash is already unsigned (>>> 0 above), but if it happens
  // to be 0 we still want a valid seed. The unsigned value is always in [0, 2^32).
  // Return as-is; zero is a valid seed but practically won't occur for real inputs.
  return hash >>> 0;
}

// ─── Strategy selection ────────────────────────────────────────────────────────

/**
 * Returns the LayoutStrategy for the given query kind.
 *
 * Pure function — no side effects.
 */
export function selectLayoutStrategy(queryKind: QueryContext['queryKind']): LayoutStrategy {
  return QUERY_KIND_TO_STRATEGY[queryKind];
}

// ─── Layout hint builder ───────────────────────────────────────────────────────

/**
 * Builds a complete SemanticLayoutHint from a QueryContext.
 *
 * The seed is deterministic: same queryHash + graphRevision always produce
 * the same seed, ensuring equal input → equal layout hint.
 *
 * Pure function — no side effects.
 */
export function buildLayoutHint(context: QueryContext): SemanticLayoutHint {
  return {
    seed: computeLayoutSeed(context.queryHash, context.graphRevision),
    strategy: selectLayoutStrategy(context.queryKind),
    primaryItemId: context.primaryItemId,
    maxDepth: context.maxDepth,
  };
}
