/**
 * memory/knowledge/sceneCaps — Balanced and hard scene caps enforcement.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Enforces node/edge/label counts so the Canvas2D renderer stays within the
 * memory and frame-time budgets documented in F4.7.3:
 *
 *   Balanced: 240 nodes / 360 edges / 80 labels   (UI truncation controls shown)
 *   Hard:     500 nodes / 750 edges / 160 labels   (slice — do not error)
 *
 * applyBalancedCaps: returns which items survive the caps plus a reason flag.
 * applyHardCaps: purely defensive — slices arrays at hard limits.
 *
 * IDs: MGD-003; MG-M09, MG-O19.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

export const BALANCED_NODE_CAP = 240;
export const BALANCED_EDGE_CAP = 360;
export const BALANCED_LABEL_CAP = 80;

export const HARD_NODE_CAP = 500;
export const HARD_EDGE_CAP = 750;
export const HARD_LABEL_CAP = 160;

// ─── Types ────────────────────────────────────────────────────────────────────

/**
 * Primary reason why the scene was truncated.
 * 'none' means no cap was exceeded.
 */
export type TruncationReason =
  | "node-cap"
  | "edge-cap"
  | "label-cap"
  | "none";

/** Result of applying balanced caps to a set of scene item IDs. */
export interface CapsResult {
  /** Node IDs that survived the cap (at most BALANCED_NODE_CAP items). */
  nodeIds: string[];
  /** Edge IDs that survived the cap (at most BALANCED_EDGE_CAP items). */
  edgeIds: string[];
  /** Label IDs that will be shown (at most BALANCED_LABEL_CAP items). */
  labelIds: string[];
  /** true when any cap was exceeded. */
  truncated: boolean;
  /** The first cap that was exceeded, or 'none'. */
  truncationReason: TruncationReason;
  /** true when nodeIds.length was > BALANCED_NODE_CAP before capping. */
  nodeCapExceeded: boolean;
  /** true when edgeIds.length was > BALANCED_EDGE_CAP before capping. */
  edgeCapExceeded: boolean;
}

// ─── applyBalancedCaps ────────────────────────────────────────────────────────

/**
 * Apply balanced caps to a proposed set of node, edge, and label IDs.
 *
 * Inputs beyond the balanced cap limits are sliced (first N survive).
 * The CapsResult describes what was truncated so the renderer can display
 * the appropriate frontier/narrowing controls.
 *
 * Pure function — does not mutate the input arrays.
 */
export function applyBalancedCaps(
  nodeIds: string[],
  edgeIds: string[],
  requestedLabelIds: string[],
): CapsResult {
  const nodeCapExceeded = nodeIds.length > BALANCED_NODE_CAP;
  const edgeCapExceeded = edgeIds.length > BALANCED_EDGE_CAP;
  const labelCapExceeded = requestedLabelIds.length > BALANCED_LABEL_CAP;

  const truncated = nodeCapExceeded || edgeCapExceeded || labelCapExceeded;

  let truncationReason: TruncationReason = "none";
  if (nodeCapExceeded) {
    truncationReason = "node-cap";
  } else if (edgeCapExceeded) {
    truncationReason = "edge-cap";
  } else if (labelCapExceeded) {
    truncationReason = "label-cap";
  }

  return {
    nodeIds: nodeCapExceeded ? nodeIds.slice(0, BALANCED_NODE_CAP) : nodeIds.slice(),
    edgeIds: edgeCapExceeded ? edgeIds.slice(0, BALANCED_EDGE_CAP) : edgeIds.slice(),
    labelIds: labelCapExceeded
      ? requestedLabelIds.slice(0, BALANCED_LABEL_CAP)
      : requestedLabelIds.slice(),
    truncated,
    truncationReason,
    nodeCapExceeded,
    edgeCapExceeded,
  };
}

// ─── applyHardCaps ────────────────────────────────────────────────────────────

/**
 * Apply hard (absolute) caps to node and edge arrays.
 *
 * This is the last line of defence before the renderer. Items beyond the hard
 * cap are silently dropped — no error is thrown. Call this after
 * applyBalancedCaps if you need an extra safety guard.
 *
 * Pure function — does not mutate the input arrays.
 */
export function applyHardCaps(
  nodeIds: string[],
  edgeIds: string[],
): { nodeIds: string[]; edgeIds: string[] } {
  return {
    nodeIds:
      nodeIds.length > HARD_NODE_CAP ? nodeIds.slice(0, HARD_NODE_CAP) : nodeIds.slice(),
    edgeIds:
      edgeIds.length > HARD_EDGE_CAP ? edgeIds.slice(0, HARD_EDGE_CAP) : edgeIds.slice(),
  };
}
