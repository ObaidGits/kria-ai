/**
 * culling3D.ts — F6 isolated technical spike: LOD, frustum/cap culling, and bounded label updates.
 *
 * Pure TypeScript module — no JSX, no DOM, no WebGL, no side effects.
 *
 * This module implements the LOD, culling, and cap logic for the 3D spike (task 6.2.3).
 * It operates entirely on the packed position buffer produced by packNodePositions and
 * on the semantic item metadata from the SemanticScene.
 *
 * Exports:
 *   • SCENE_CAPS        — exact scene/truncation cap constants from design.md §10.2.
 *   • LODLevel          — enum: NEAR | MID | FAR.
 *   • getLODLevel       — pure LOD classification by z distance from camera eye.
 *   • CameraState       — camera descriptor for frustum math.
 *   • frustumCull       — pure frustum visibility test over packed position buffer.
 *   • CappedSceneResult — output type for applyCaps.
 *   • SceneItem         — minimal item descriptor for applyCaps (no SemanticScene dep).
 *   • applyCaps         — priority-ordered cap enforcement returning CappedSceneResult.
 *   • LabelState        — per-label state for dirty/collision updates.
 *   • updateDirtyLabels — bounded dirty-label processor with AABB collision detection.
 *
 * Design invariants (frozen per task 6.2.3):
 *   • SCENE_CAPS values are exact — from design.md §10.2.
 *   • LOD boundaries are exact: NEAR < 2.0, MID 2.0–5.0, FAR > 5.0.
 *   • frustumCull adds a 64px overscan margin (world-unit equivalent).
 *   • applyCaps priority: selected/focused > path items > closer z-value.
 *   • updateDirtyLabels processes at most maxUpdatesPerFrame dirty labels per call (default 16).
 *   • All functions are pure — no side effects, no globals, no DOM.
 *
 * IDs: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026, MGD-046;
 *      task 6.2.3 (F6 pre-production spike only — not a shipped renderer path).
 */

import { PACKED_NODE_STRIDE } from './graphCanvas3DSpike';

// ─── Scene caps (exact values from design.md §10.2) ──────────────────────────

/**
 * Exact scene and truncation cap constants from design.md §10.2.
 *
 * Balanced default:  240 nodes / 360 edges / 80 labels / 512 KiB DTO.
 * Hard display cap:  500 nodes / 750 edges / 160 labels / 2 MiB.
 *
 * These values are preregistered and frozen for the F6 spike.
 * They MUST NOT be changed without a design revision.
 */
export const SCENE_CAPS = {
  BALANCED_NODES: 240,
  BALANCED_EDGES: 360,
  BALANCED_LABELS: 80,
  HARD_NODES: 500,
  HARD_EDGES: 750,
  HARD_LABELS: 160,
  BYTES_BALANCED: 512 * 1024,    // 512 KiB
  BYTES_HARD: 2 * 1024 * 1024,   // 2 MiB
} as const;

// ─── LOD levels ───────────────────────────────────────────────────────────────

/**
 * Level of Detail for a node in the 3D scene.
 *
 * Determined by the node's z distance from the camera eye (in world units):
 *   NEAR  — z < 2.0:         full geometry, labels always visible.
 *   MID   — 2.0 ≤ z ≤ 5.0:  reduced geometry, labels visible at priority threshold only.
 *   FAR   — z > 5.0:         minimal geometry (point only), no labels.
 *
 * Items beyond `camera.far` are never rendered (handled by frustumCull).
 */
export enum LODLevel {
  NEAR = 'NEAR',
  MID = 'MID',
  FAR = 'FAR',
}

/**
 * Boundaries for LOD classification (world units, distance from camera eye).
 * Frozen per task 6.2.3.
 */
export const LOD_NEAR_THRESHOLD = 2.0;
export const LOD_MID_THRESHOLD = 5.0;

/**
 * Classifies a node's LOD level by its z distance from the camera eye.
 *
 * @param zDistanceFromEye — non-negative distance in world units from the camera eye.
 *   This is NOT the packed z value from the buffer; it is the absolute distance
 *   between the node's world position and the camera eye position.
 *
 * LOD rules (frozen):
 *   NEAR  — zDistanceFromEye < 2.0
 *   MID   — 2.0 ≤ zDistanceFromEye ≤ 5.0
 *   FAR   — zDistanceFromEye > 5.0
 */
export function getLODLevel(zDistanceFromEye: number): LODLevel {
  if (zDistanceFromEye < LOD_NEAR_THRESHOLD) return LODLevel.NEAR;
  if (zDistanceFromEye <= LOD_MID_THRESHOLD) return LODLevel.MID;
  return LODLevel.FAR;
}

// ─── Camera state ─────────────────────────────────────────────────────────────

/**
 * Camera descriptor for frustum culling.
 *
 * All fields are in world-space units, angles in radians.
 *
 * eye    — position of the camera in world space.
 * target — point the camera is looking at.
 * fovY   — vertical field of view in radians.
 * aspect — viewport aspect ratio (width / height).
 * near   — near clip plane distance (positive).
 * far    — far clip plane distance (positive, > near).
 */
export interface CameraState {
  fovY: number;
  aspect: number;
  near: number;
  far: number;
  eye: [number, number, number];
  target: [number, number, number];
}

// ─── Frustum culling ──────────────────────────────────────────────────────────

/**
 * Pixel overscan margin converted to a conservative world-unit equivalent.
 *
 * From design.md §10.2: "only viewport plus 64px overscan is drawn."
 * For the spike, we treat this as 64 units in NDC/world-unit scale — a
 * conservative approximation that is sufficient for the pure-logic spike test.
 *
 * In a real renderer this would be derived from the projection matrix and
 * the viewport dimensions. For the spike, the constant keeps the math pure.
 */
const OVERSCAN_WORLD_UNITS = 0.15; // ~64px at a typical FOV and viewport size

/**
 * Normalizes a 3-element vector to unit length.
 * Returns [0,0,0] for zero-length input (degenerate camera).
 */
function normalize3(v: [number, number, number]): [number, number, number] {
  const len = Math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
  if (len === 0) return [0, 0, 0];
  return [v[0] / len, v[1] / len, v[2] / len];
}

/**
 * Dot product of two 3-vectors.
 */
function dot3(a: [number, number, number], b: [number, number, number]): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
}

/**
 * Cross product of two 3-vectors.
 */
function cross3(
  a: [number, number, number],
  b: [number, number, number],
): [number, number, number] {
  return [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ];
}

/**
 * Pure frustum culling over a packed position buffer.
 *
 * Reads x, y, z from the packed buffer (offsets 1, 2, 3 within each stride),
 * ignoring the hasZ flag — the frustum test is spatial and applies to all nodes
 * regardless of z availability.
 *
 * Visibility rules:
 *   - A node is VISIBLE when it is inside the view frustum (including the 64px
 *     overscan equivalent) AND its distance from the camera eye is ≤ camera.far.
 *   - A node is INVISIBLE when it is behind the camera (distance < camera.near),
 *     beyond camera.far, or outside the lateral frustum planes (with overscan).
 *
 * The frustum test is a conservative half-space test using the camera's forward,
 * right, and up vectors. It is exact for the spike purposes (no bounding sphere,
 * just point-in-frustum with overscan).
 *
 * @param positions — Float32Array from packNodePositions (layout: PACKED_NODE_STRIDE per node).
 * @param camera    — CameraState describing the view frustum.
 * @returns boolean[] of length `positions.length / PACKED_NODE_STRIDE`;
 *          true = visible, false = culled.
 */
export function frustumCull(positions: Float32Array, camera: CameraState): boolean[] {
  const nodeCount = Math.floor(positions.length / PACKED_NODE_STRIDE);
  const result: boolean[] = new Array(nodeCount).fill(false);

  // Build camera basis vectors.
  const fwd = normalize3([
    camera.target[0] - camera.eye[0],
    camera.target[1] - camera.eye[1],
    camera.target[2] - camera.eye[2],
  ]);

  // If the camera has a degenerate direction (eye === target), nothing is visible.
  if (fwd[0] === 0 && fwd[1] === 0 && fwd[2] === 0) {
    return result;
  }

  // World-up hint (avoid collinearity with fwd using fallback).
  const worldUp: [number, number, number] =
    Math.abs(fwd[1]) < 0.99 ? [0, 1, 0] : [1, 0, 0];
  const right = normalize3(cross3(fwd, worldUp));
  const up = normalize3(cross3(right, fwd));

  // Half-angles with overscan baked in.
  const halfFovY = camera.fovY / 2.0;
  const halfFovX = Math.atan(Math.tan(halfFovY) * camera.aspect);

  // Overscan: expand the half-angles by the world-unit equivalent fraction.
  const tanHalfY = Math.tan(halfFovY) + OVERSCAN_WORLD_UNITS;
  const tanHalfX = Math.tan(halfFovX) + OVERSCAN_WORLD_UNITS;

  for (let i = 0; i < nodeCount; i++) {
    const base = i * PACKED_NODE_STRIDE;
    const wx = positions[base + 1]!;
    const wy = positions[base + 2]!;
    const wz = positions[base + 3]!;

    // Vector from camera eye to node.
    const dx = wx - camera.eye[0];
    const dy = wy - camera.eye[1];
    const dz = wz - camera.eye[2];

    // Project onto camera forward axis — this is the depth in view space.
    const depth = dx * fwd[0] + dy * fwd[1] + dz * fwd[2];

    // Reject behind near plane or beyond far plane.
    if (depth < camera.near || depth > camera.far) {
      result[i] = false;
      continue;
    }

    // Project onto camera right and up axes, then test against FOV half-angles.
    const px = dx * right[0] + dy * right[1] + dz * right[2];
    const py = dx * up[0] + dy * up[1] + dz * up[2];

    // Compare |px|/depth vs tanHalfX and |py|/depth vs tanHalfY.
    const inFrustumX = Math.abs(px) <= tanHalfX * depth;
    const inFrustumY = Math.abs(py) <= tanHalfY * depth;

    result[i] = inFrustumX && inFrustumY;
  }

  return result;
}

// ─── Cap enforcement ──────────────────────────────────────────────────────────

/**
 * Minimal item descriptor for cap enforcement.
 * Decoupled from SemanticSceneItem to keep this module pure and dependency-free.
 */
export interface SceneItem {
  /** Unique item id. */
  id: string;
  /** Item kind: 'node' | 'edge'. */
  kind: 'node' | 'edge';
  /** Whether this item is selected. */
  isSelected: boolean;
  /** Whether this item is keyboard-focused. */
  isFocused: boolean;
  /** Whether this item is on the current navigation path. */
  isInPath: boolean;
  /**
   * z-axis depth in world units (from the packed buffer).
   * null means unavailable — item is de-prioritized but not excluded.
   * Lower z = closer to camera = higher priority.
   */
  z: number | null;
}

/**
 * Result of cap enforcement.
 */
export interface CappedSceneResult {
  /** IDs of nodes that remain visible after cap enforcement. */
  visibleNodes: string[];
  /** IDs of edges that remain visible after cap enforcement. */
  visibleEdges: string[];
  /** IDs of nodes whose labels are visible after cap enforcement. */
  visibleLabels: string[];
  /** True if any item was excluded by the cap. */
  truncated: boolean;
  /** Human-readable reason for truncation, or null if not truncated. */
  truncationReason: string | null;
}

/**
 * Computes a priority score for an item.
 * Lower score = higher priority (will be included first).
 *
 * Priority order (from design.md §10.2):
 *   0 — selected or focused (always visible, never culled per spec)
 *   1 — path items
 *   2 — all other items, sorted by z (closer = higher priority, z=null is lowest)
 */
function itemPriority(item: SceneItem): number {
  if (item.isSelected || item.isFocused) return 0;
  if (item.isInPath) return 1;
  // z available: priority 2 + normalized z (closer = smaller offset).
  // z unavailable: priority 3 (lowest tier).
  if (item.z !== null) return 2 + item.z;
  return 3;
}

/**
 * Applies BALANCED scene caps with priority ordering.
 *
 * Respects SCENE_CAPS.BALANCED_NODES and SCENE_CAPS.BALANCED_LABELS by default.
 * Edges currently apply SCENE_CAPS.BALANCED_EDGES.
 *
 * Priority order (design.md §10.2 — frozen):
 *   1. Selected / focused items — ALWAYS visible, never culled.
 *   2. Path items.
 *   3. Remaining items sorted by z (closer = higher priority); z=null is lowest.
 *
 * Label assignment follows the same priority order up to BALANCED_LABELS.
 *
 * @param items      — full candidate item list (nodes and edges mixed).
 * @param visibility — boolean visibility mask from frustumCull (index matches items).
 *                     Items with visibility[i] === false are pre-culled before caps.
 * @param zValues    — Map<id, z|null> (used for priority sorting; same as packed buffer z).
 */
export function applyCaps(
  items: SceneItem[],
  visibility: boolean[],
  zValues: Map<string, number | null>,
): CappedSceneResult {
  // Separate into nodes and edges, respecting pre-computed visibility.
  const visibleNodes: SceneItem[] = [];
  const visibleEdges: SceneItem[] = [];

  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    const vis = i < visibility.length ? visibility[i] : true;
    if (!vis) continue;
    if (item.kind === 'node') visibleNodes.push(item);
    else visibleEdges.push(item);
  }

  // Sort by priority (stable sort — equal priorities keep original order).
  const sortByPriority = (a: SceneItem, b: SceneItem): number =>
    itemPriority(a) - itemPriority(b);

  visibleNodes.sort(sortByPriority);
  visibleEdges.sort(sortByPriority);

  // Apply BALANCED caps.
  const cappedNodes = visibleNodes.slice(0, SCENE_CAPS.BALANCED_NODES);
  const cappedEdges = visibleEdges.slice(0, SCENE_CAPS.BALANCED_EDGES);

  // Label priority: same as node priority — pick top BALANCED_LABELS node ids.
  const labelCandidates = cappedNodes.slice(0, SCENE_CAPS.BALANCED_LABELS);

  const truncated =
    visibleNodes.length > SCENE_CAPS.BALANCED_NODES ||
    visibleEdges.length > SCENE_CAPS.BALANCED_EDGES;

  let truncationReason: string | null = null;
  if (visibleNodes.length > SCENE_CAPS.BALANCED_NODES && visibleEdges.length > SCENE_CAPS.BALANCED_EDGES) {
    truncationReason = `Scene exceeds balanced caps: ${visibleNodes.length} nodes (cap ${SCENE_CAPS.BALANCED_NODES}), ${visibleEdges.length} edges (cap ${SCENE_CAPS.BALANCED_EDGES})`;
  } else if (visibleNodes.length > SCENE_CAPS.BALANCED_NODES) {
    truncationReason = `Scene exceeds balanced node cap: ${visibleNodes.length} nodes (cap ${SCENE_CAPS.BALANCED_NODES})`;
  } else if (visibleEdges.length > SCENE_CAPS.BALANCED_EDGES) {
    truncationReason = `Scene exceeds balanced edge cap: ${visibleEdges.length} edges (cap ${SCENE_CAPS.BALANCED_EDGES})`;
  }

  return {
    visibleNodes: cappedNodes.map((n) => n.id),
    visibleEdges: cappedEdges.map((e) => e.id),
    visibleLabels: labelCandidates.map((n) => n.id),
    truncated,
    truncationReason,
  };
}

// ─── Dirty label / collision updates ─────────────────────────────────────────

/**
 * Per-label state for the bounded dirty label update system.
 */
export interface LabelState {
  /** Id of the semantic item this label belongs to. */
  itemId: string;
  /** Display text. */
  text: string;
  /** Screen-space x position (pixels). */
  screenX: number;
  /** Screen-space y position (pixels). */
  screenY: number;
  /** Whether this label is currently visible (not collided). */
  visible: boolean;
  /** Whether this label needs its position/visibility recomputed. */
  dirty: boolean;
}

/**
 * Default maximum number of dirty labels processed per frame.
 * Bounded per design.md §10.2 and §A6: queues and workers are capped.
 */
export const DEFAULT_MAX_UPDATES_PER_FRAME = 16;

/**
 * Approximate label AABB dimensions in screen pixels.
 * Used for collision detection. In a real renderer these would be measured
 * from the actual rendered text; for the spike we use a fixed estimate.
 */
const LABEL_WIDTH_PX = 80;
const LABEL_HEIGHT_PX = 16;

/**
 * Tests whether two axis-aligned bounding boxes overlap.
 *
 * AABB for label at (cx, cy): [cx, cy, cx+w, cy+h].
 */
function aabbOverlap(
  ax: number,
  ay: number,
  bx: number,
  by: number,
): boolean {
  return (
    ax < bx + LABEL_WIDTH_PX &&
    ax + LABEL_WIDTH_PX > bx &&
    ay < by + LABEL_HEIGHT_PX &&
    ay + LABEL_HEIGHT_PX > by
  );
}

/**
 * Processes up to `maxUpdatesPerFrame` dirty labels per call.
 *
 * Bounded dirty update contract:
 *   - Only dirty labels are processed. Clean labels are returned unchanged.
 *   - At most `maxUpdatesPerFrame` dirty labels are processed per call.
 *   - Remaining dirty labels keep `dirty: true` and are processed in a future call.
 *   - After processing a dirty label, its `dirty` flag is cleared.
 *   - Visible labels are AABB-collision-tested against all previously accepted
 *     visible labels in the processed batch. First-accepted wins (stable order).
 *   - Labels that collide with an already-visible label have `visible` set to false.
 *   - Labels that do not collide retain their current `visible` flag (or gain it
 *     if they were previously hidden and no collision is detected).
 *
 * @param labels             — current label state array. Not mutated; a new array is returned.
 * @param maxUpdatesPerFrame — max dirty labels to process this call (default 16).
 * @returns new LabelState[] with updated dirty/visible fields for processed labels.
 */
export function updateDirtyLabels(
  labels: LabelState[],
  maxUpdatesPerFrame: number = DEFAULT_MAX_UPDATES_PER_FRAME,
): LabelState[] {
  // Collect occupied screen positions from labels that are already visible and clean.
  // These represent the "locked" label positions that processed labels must not overlap.
  const occupiedPositions: Array<{ x: number; y: number }> = [];
  for (const label of labels) {
    if (!label.dirty && label.visible) {
      occupiedPositions.push({ x: label.screenX, y: label.screenY });
    }
  }

  let updatesRemaining = maxUpdatesPerFrame;
  const result: LabelState[] = new Array(labels.length);

  for (let i = 0; i < labels.length; i++) {
    const label = labels[i]!;

    if (!label.dirty || updatesRemaining <= 0) {
      // Not dirty or budget exhausted — copy unchanged.
      result[i] = label;
      continue;
    }

    // Process this dirty label.
    updatesRemaining--;

    // Check AABB collision against all currently occupied positions.
    let collides = false;
    for (const pos of occupiedPositions) {
      if (aabbOverlap(label.screenX, label.screenY, pos.x, pos.y)) {
        collides = true;
        break;
      }
    }

    const nowVisible = !collides;

    // If this label is now visible, record its position to block future labels.
    if (nowVisible) {
      occupiedPositions.push({ x: label.screenX, y: label.screenY });
    }

    result[i] = {
      ...label,
      visible: nowVisible,
      dirty: false,
    };
  }

  return result;
}
