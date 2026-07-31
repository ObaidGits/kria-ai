/**
 * graphCanvas3DSpike — F6 isolated technical spike helpers.
 *
 * Pure TypeScript module — no JSX, no DOM, no WebGL, no side effects.
 *
 * This module is the ONLY new logic for the F6.2 3D spike.  It provides:
 *
 *   • `SemanticInput3D` — the props contract that GraphCanvas3D must accept
 *     when operating as a pure SemanticScene consumer.  Identical in kind to
 *     what Graph2D accepts (same SemanticScene, same SceneCapabilities).
 *
 *   • `computeSemanticCollectionHash` — deterministic hash over the semantic
 *     item collection (items × {id, kind, truthState, graphRevision}).
 *     Returns the same value whether called from Graph2D or GraphCanvas3D
 *     for the same SemanticScene input.
 *
 *   • `computeAuthorizedActionHash` — deterministic hash over the authorized
 *     action set (actions × {targetItemId, kind}).
 *     Returns the same value whether called from Graph2D or GraphCanvas3D
 *     for the same SemanticScene input.
 *
 *   • `SemanticSceneItem3D` — extends SemanticSceneItem with optional z_value.
 *
 *   • `computeNodeZ` — maps a raw cosine similarity score to z-axis depth
 *     using the preregistered formula: z = (1 − cosine_similarity) / 2.
 *     Returns null for absent scores (node has no vector score).
 *
 *   • `mapZValues` — pure function that maps a SemanticSceneItem[] + a
 *     vectorScores Map → a Map<id, z | null> per the frozen protocol:
 *       - origin node (first isInPath=true item) gets z=0 exactly
 *       - nodes with a score use z = (1 − score) / 2
 *       - nodes without a score or cycle placeholders (kind='navigation-container')
 *         get z=null (Unavailable)
 *
 *   • `packNodePositions` — packs items + z-values into a contiguous
 *     Float32Array of shape [nodeIndex, x, y, z, hasZ] per item.
 *     x/y are computed deterministically from the id hash (no simulation).
 *     The buffer is transferable (no SharedArrayBuffer).
 *
 * Critical constraints (task_6_1_5 preregistration):
 *   • The 3D renderer is a PURE CONSUMER of the existing SemanticScene.
 *   • It must NOT maintain its own truth, policy, layout state, or fetch data.
 *   • The hashes produced here must equal those produced for the 2D/list
 *     condition for the same snapshot/session/capabilities.
 *   • z formula is preregistered and frozen: z = (1 − cosine_similarity) / 2.
 *   • No per-path rescaling; no global normalization; no numeric defaults for
 *     absent scores.
 *
 * Design invariants:
 *   • Equal SemanticScene input → equal hashes (deterministic, stable).
 *   • Hash algorithm is FNV-1a 32-bit — identical to sceneBuilder.ts.
 *   • No rendering logic here; this module may be imported by tests without
 *     any browser globals.
 *
 * IDs: MGR-001, MGR-002, MGR-004, MGR-012, MGR-026; MGD-003, MGD-026, MGD-046;
 *      task 6.2.1 and 6.2.2 (F6 pre-production spike only — not a shipped renderer path).
 */

import type { SemanticScene, SemanticSceneItem, SemanticSceneAction } from '../scene/semanticScene';
import type { SceneCapabilities } from '../scene/sceneActions';

// ─── Spike input contract ─────────────────────────────────────────────────────

/**
 * Props contract for GraphCanvas3D when acting as a pure SemanticScene
 * consumer.  Mirrors Graph2DProps in kind:
 *   • scene — the same SemanticScene produced by sceneBuilder / pathScene.
 *   • capabilities — the same SceneCapabilities used by Graph2D (sceneActions).
 *   • onAction — same typed dispatch callback.
 *
 * The 3D component accepts these props alongside its existing rendering options
 * (`static`) without removing them.  During the spike, the rendering itself is
 * NOT implemented (task 6.2.2+); only the input wiring is wired.
 */
export interface SemanticInput3D {
  /** The canonical SemanticScene — same type as Graph2D receives. */
  scene: SemanticScene;
  /**
   * Capability set that governed which actions were built into scene.actions.
   * Passed through for parity assertion; the 3D consumer must not derive
   * additional capabilities from it.
   */
  capabilities: SceneCapabilities;
  /** Typed action dispatch — same signature as Graph2D.onAction. */
  onAction: (itemId: string, kind: SemanticSceneAction['kind']) => void;
}

// ─── FNV-1a 32-bit hash (same algorithm as sceneBuilder.ts) ──────────────────

/**
 * FNV-1a 32-bit hash over a UTF-16 string.
 *
 * Algorithm is identical to the one in sceneBuilder.ts — this ensures that
 * any hash computed here can be compared directly to a sceneHash from
 * sceneBuilder without conversion.
 *
 * Returns a hex string prefixed with "fnv1a-".
 */
function fnv1aHash(input: string): string {
  let hash = 0x811c9dc5; // FNV offset basis (32-bit)
  for (let i = 0; i < input.length; i++) {
    hash ^= input.charCodeAt(i);
    hash = (Math.imul(hash, 0x01000193) >>> 0);
  }
  return 'fnv1a-' + hash.toString(16).padStart(8, '0');
}

// ─── Semantic collection hash ─────────────────────────────────────────────────

/**
 * Computes a deterministic hash over the semantic item collection.
 *
 * Hash inputs per item (sorted by id for determinism):
 *   { id, kind, truthState, graphRevision }
 *
 * This is the same projection used by sceneBuilder.ts for sceneHash, but
 * restricted to the semantic-meaning-bearing fields so that visual token
 * changes (shape, color) do not affect the parity oracle.
 *
 * Two calls with the same SemanticScene.items (same ids, kinds, truthStates,
 * revisions) MUST return equal hashes regardless of which renderer calls them.
 */
export function computeSemanticCollectionHash(items: SemanticSceneItem[]): string {
  // Sort by id for determinism — same as sceneBuilder.ts step 3.
  const sorted = [...items].sort((a, b) =>
    a.id < b.id ? -1 : a.id > b.id ? 1 : 0,
  );
  const payload = JSON.stringify(
    sorted.map((i) => ({
      id: i.id,
      kind: i.kind,
      truthState: i.truthState,
      rev: i.graphRevision,
    })),
  );
  return fnv1aHash(payload);
}

// ─── Authorized action hash ───────────────────────────────────────────────────

/**
 * Computes a deterministic hash over the authorized action set.
 *
 * Hash inputs per action (sorted by targetItemId then kind):
 *   { targetItemId, kind }
 *
 * Only actions present in scene.actions are hashed — the same set that the
 * capability-filtered sceneActions.buildAuthorizedActions would produce.
 *
 * Two calls with the same SemanticScene.actions MUST return equal hashes
 * regardless of which renderer calls them.
 */
export function computeAuthorizedActionHash(actions: SemanticSceneAction[]): string {
  // Sort by (targetItemId, kind) for determinism — same order as sceneBuilder.ts step 5.
  const sorted = [...actions].sort((a, b) => {
    if (a.targetItemId < b.targetItemId) return -1;
    if (a.targetItemId > b.targetItemId) return 1;
    if (a.kind < b.kind) return -1;
    if (a.kind > b.kind) return 1;
    return 0;
  });
  const payload = JSON.stringify(
    sorted.map((a) => ({ itemId: a.targetItemId, kind: a.kind })),
  );
  return fnv1aHash(payload);
}

// ─── Combined spike parity snapshot ──────────────────────────────────────────

/**
 * Snapshot of the two hashes for a single SemanticScene.
 *
 * Used by parity tests to assert that the same SemanticScene produces
 * identical hashes when presented to Graph2D and GraphCanvas3D.
 */
export interface SemanticParitySnapshot {
  semanticCollectionHash: string;
  authorizedActionHash: string;
}

/**
 * Computes both parity hashes for a SemanticScene in a single call.
 *
 * Pure function — no side effects, no rendering, no DOM.
 *
 * Both Graph2D and GraphCanvas3D spike call this function with the SAME
 * SemanticScene input.  The test asserts the snapshots are equal.
 */
export function computeSemanticParitySnapshot(scene: SemanticScene): SemanticParitySnapshot {
  return {
    semanticCollectionHash: computeSemanticCollectionHash(scene.items),
    authorizedActionHash: computeAuthorizedActionHash(scene.actions),
  };
}

// ─── Z-axis types and mapping (task 6.2.2) ────────────────────────────────────

/**
 * Extends SemanticSceneItem with an authority-backed z-axis depth value.
 *
 * z_value is the preregistered formula: z = (1 − cosine_similarity) / 2.
 *   - z=0.0 — origin node (semantically identical to itself).
 *   - z∈(0,1] — node at semantic distance from origin.
 *   - z=null — no vector score available (Unavailable); must NOT be defaulted.
 *
 * Protocol: preregistered in task_6_1_2, frozen. No per-path rescaling,
 * no global normalization, no numeric defaults for absent scores.
 */
export interface SemanticSceneItem3D extends SemanticSceneItem {
  /**
   * Authority-backed z-axis depth, or null when no vector score is available.
   *
   * null means "Unavailable" — the renderer must handle this gracefully
   * (e.g. distinct tier, omit from depth channel) but must NOT substitute
   * any numeric value (0.5, hop index, etc.) for a missing score.
   */
  z_value: number | null;
}

/**
 * Maps a raw cosine similarity score to the preregistered z-axis depth.
 *
 * Formula (frozen in preregistration task_6_1_2):
 *   z = (1 − cosine_similarity) / 2
 *
 * Input range: cosine_similarity ∈ [−1.0, 1.0]
 * Output range: z ∈ [0.0, 1.0]
 *   - cosine=1.0  → z=0.0 (identical direction; origin node)
 *   - cosine=0.0  → z=0.5 (orthogonal)
 *   - cosine=-1.0 → z=1.0 (maximally opposite)
 *
 * Returns null when cosineScore is null (absent score → Unavailable).
 *
 * Protocol constraints (no deviation permitted):
 *   - MUST NOT default null to any numeric value (e.g. 0.5).
 *   - MUST NOT apply per-path or global rescaling.
 *   - Division constant is exactly 2.0 — not 1.0 or any other value.
 */
export function computeNodeZ(cosineScore: number | null): number | null {
  if (cosineScore === null) return null;
  return (1.0 - cosineScore) / 2.0;
}

/**
 * Maps a SemanticSceneItem collection to authority-backed z-values.
 *
 * Rules (preregistered, frozen):
 *   1. The origin node is the FIRST item with `isInPath: true`.
 *      - If none is found, the first item in the array is treated as origin.
 *      - The origin always gets z=0.0 (exact), regardless of its score.
 *   2. Non-origin items with a vector score use z = (1 − score) / 2.
 *   3. Items without a vector score receive z=null (Unavailable).
 *   4. Cycle placeholders (kind === 'navigation-container') always get z=null.
 *      Copying the z-value of an originating node to its cycle placeholder
 *      is a protocol violation.
 *
 * Parameters:
 *   items        — the SemanticSceneItem array from the SemanticScene.
 *   vectorScores — Map<itemId, cosine_similarity∈[−1,1]> from
 *                  retrieval_trace_items.strategy_score where strategy='vector'.
 *                  Items absent from this map have no vector score.
 *
 * Returns a Map<itemId, z | null> for every item in the array.
 */
export function mapZValues(
  items: SemanticSceneItem[],
  vectorScores: Map<string, number>,
): Map<string, number | null> {
  const result = new Map<string, number | null>();

  // Identify origin: first item with isInPath=true, falling back to first item.
  const originItem = items.find((item) => item.isInPath) ?? items[0];
  const originId = originItem?.id ?? null;

  for (const item of items) {
    // Cycle placeholders always get z=null — no score, no borrowing.
    if (item.kind === 'navigation-container') {
      result.set(item.id, null);
      continue;
    }

    // Origin node is z=0 exactly (cosine of a vector with itself is 1.0).
    if (item.id === originId) {
      result.set(item.id, 0.0);
      continue;
    }

    // Non-origin: look up the vector score.
    const score = vectorScores.get(item.id);
    result.set(item.id, score !== undefined ? computeNodeZ(score) : null);
  }

  return result;
}

// ─── Packed transferable buffer (task 6.2.2) ─────────────────────────────────

/**
 * Number of Float32 values per node in the packed buffer.
 *
 * Layout per node (stride=5):
 *   [0] nodeIndex — 0-based index into the items array (float for alignment)
 *   [1] x         — deterministic x position from id hash
 *   [2] y         — deterministic y position from id hash
 *   [3] z         — z-axis depth (0.0 when hasZ=0)
 *   [4] hasZ      — 1.0 if z is valid, 0.0 if z is null/unavailable
 */
export const PACKED_NODE_STRIDE = 5;

/**
 * Deterministic x/y position from a string id hash.
 *
 * Uses two independent FNV-1a 32-bit hash passes (one seeded with 'x:',
 * one with 'y:') to produce independent x and y coordinates in [−1, 1].
 *
 * This is NOT a force-simulation — positions are computed once, purely from
 * the node id, and are stable across renders.
 */
function deterministicXY(id: string): { x: number; y: number } {
  let hx = 0x811c9dc5;
  const xInput = 'x:' + id;
  for (let i = 0; i < xInput.length; i++) {
    hx ^= xInput.charCodeAt(i);
    hx = (Math.imul(hx, 0x01000193) >>> 0);
  }

  let hy = 0x811c9dc5;
  const yInput = 'y:' + id;
  for (let i = 0; i < yInput.length; i++) {
    hy ^= yInput.charCodeAt(i);
    hy = (Math.imul(hy, 0x01000193) >>> 0);
  }

  // Map uint32 → [−1, 1]
  const x = (hx / 0xffffffff) * 2.0 - 1.0;
  const y = (hy / 0xffffffff) * 2.0 - 1.0;
  return { x, y };
}

/**
 * Packs node positions into a contiguous Float32Array suitable for postMessage
 * with transfer (zero-copy).
 *
 * Buffer layout (PACKED_NODE_STRIDE=5 floats per node):
 *   [i*5 + 0] nodeIndex  — 0-based float
 *   [i*5 + 1] x          — deterministic x from id hash, in [−1, 1]
 *   [i*5 + 2] y          — deterministic y from id hash, in [−1, 1]
 *   [i*5 + 3] z          — z-axis depth; 0.0 when hasZ=0
 *   [i*5 + 4] hasZ       — 1.0 if z is authority-backed, 0.0 if unavailable
 *
 * z=null items produce z=0.0, hasZ=0.0.  The renderer must check hasZ before
 * using the z value — a hasZ=0 item must be treated as Unavailable, not z=0.
 *
 * The returned Float32Array owns its own ArrayBuffer and may be transferred via
 * postMessage({ positions }, [positions.buffer]) without copying.
 */
export function packNodePositions(
  items: SemanticSceneItem[],
  zValues: Map<string, number | null>,
): Float32Array {
  const buf = new Float32Array(items.length * PACKED_NODE_STRIDE);

  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    const { x, y } = deterministicXY(item.id);
    const zRaw = zValues.get(item.id);
    const hasZ = zRaw !== null && zRaw !== undefined ? 1.0 : 0.0;
    const z = hasZ === 1.0 ? (zRaw as number) : 0.0;

    const base = i * PACKED_NODE_STRIDE;
    buf[base + 0] = i;
    buf[base + 1] = x;
    buf[base + 2] = y;
    buf[base + 3] = z;
    buf[base + 4] = hasZ;
  }

  return buf;
}


// ─── Task 6.2.2: SemanticScene → renderable graph model ───────────────────────

import { isEdgeItem, isNodeItem } from '../scene/semanticScene';
import {
  COMMUNITY_COLOR_TOKENS,
  type GraphEdge,
  type GraphNode,
} from './graphModel';
import type { PositionedNode } from './layoutSettle';

/**
 * Convert a `SemanticScene` into the renderable `{ nodes, edges }` model that
 * `GraphScene.setGraph` consumes.
 *
 * PURE CONSUMER — derives nothing new:
 *   • node id / label / kind come straight from the scene item
 *   • `community` is a deterministic bucket derived from the item **kind**
 *     (a display grouping, NOT an authority claim — see MGR-011 honest
 *     analytics vocabulary: this is a colour bucket, not a detected community)
 *   • `centrality` is the item's evidenceCount when present, else 1
 *     (a display size proxy, NOT a computed centrality metric)
 *
 * Both derived fields are presentation-only and are never surfaced as
 * analytical claims in the UI.
 */
export function sceneToGraphModel(scene: SemanticScene): {
  nodes: GraphNode[];
  edges: GraphEdge[];
} {
  const nodeItems = scene.items.filter(isNodeItem);
  const edgeItems = scene.items.filter(isEdgeItem);

  // Stable kind → colour-bucket index so the same kind always gets the same
  // colour across renders (deterministic, no random assignment).
  const kindOrder: string[] = [];
  for (const item of nodeItems) {
    if (!kindOrder.includes(item.kind)) kindOrder.push(item.kind);
  }

  const nodes: GraphNode[] = nodeItems.map((item) => ({
    id: item.id,
    label: item.label ?? item.id,
    community:
      kindOrder.indexOf(item.kind) % COMMUNITY_COLOR_TOKENS.length,
    centrality: Math.max(1, item.evidenceCount ?? 1),
  }));

  const nodeIds = new Set(nodes.map((n) => n.id));

  // Only keep edges whose BOTH endpoints are present as rendered nodes —
  // prevents dangling lines pointing at absent geometry.
  const edges: GraphEdge[] = edgeItems
    .filter(
      (e) =>
        e.sourceEndpointId != null &&
        e.targetEndpointId != null &&
        nodeIds.has(e.sourceEndpointId) &&
        nodeIds.has(e.targetEndpointId),
    )
    .map((e) => ({
      source: e.sourceEndpointId as string,
      target: e.targetEndpointId as string,
      relType: e.label ?? undefined,
      predicted: false,
    }));

  return { nodes, edges };
}

/**
 * Deterministic 3D layout via a Fibonacci sphere.
 *
 * Produces stable, well-separated positions without running a force
 * simulation, so the scene is immediately visible and reproducible
 * (equal input → equal positions). The layout worker may later refine
 * these positions, but the scene is never empty while waiting.
 *
 * `radius` scales with node count so density stays roughly constant.
 */
export function computeDeterministicPositions(
  nodes: GraphNode[],
): PositionedNode[] {
  const n = nodes.length;
  if (n === 0) return [];
  if (n === 1) return [{ id: nodes[0].id, x: 0, y: 0, z: 0 }];

  // Radius grows with sqrt(n) so spacing stays comfortable as the graph grows.
  const radius = Math.max(12, Math.sqrt(n) * 6);
  const goldenAngle = Math.PI * (3 - Math.sqrt(5)); // ≈2.39996 rad

  return nodes.map((node, i) => {
    // Fibonacci sphere: even distribution over the sphere surface.
    const y = 1 - (i / (n - 1)) * 2; // 1 → -1
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const theta = goldenAngle * i;
    return {
      id: node.id,
      x: Math.cos(theta) * r * radius,
      y: y * radius,
      z: Math.sin(theta) * r * radius,
    };
  });
}
