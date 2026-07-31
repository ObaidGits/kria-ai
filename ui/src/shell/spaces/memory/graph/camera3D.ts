/**
 * camera3D.ts — F6 isolated technical spike: pure camera state machine.
 *
 * Pure TypeScript module — no JSX, no DOM, no WebGL, no side effects.
 *
 * This module implements the camera state machine for the 3D spike (task 6.2.4).
 * All state is immutable; every function returns a new state value.
 *
 * Exports:
 *   • Camera3DState          — immutable camera value type.
 *   • DEFAULT_CAMERA_3D      — canonical default framing.
 *   • ZOOM_MIN / ZOOM_MAX    — zoom bounds from design.md §10.3 ([0.25, 4.0]).
 *   • HISTORY_BOUND          — max history entries (20).
 *   • MARGIN_FRACTION        — 25% margin for fit operations.
 *
 *   Fit operations:
 *   • fitVisible             — fit all positions with 25% margin.
 *   • fitSelection           — fit selected node positions.
 *   • fitNeighborhood        — fit neighborhood positions.
 *   • resetCamera            — return to default framing.
 *
 *   History:
 *   • pushHistory            — append state to history, bounded to 20, drop future.
 *   • goBack                 — navigate history back (no-op at start).
 *   • goForward              — navigate history forward (no-op at end).
 *
 *   Keyboard:
 *   • handleKeyboard         — 'f' fit, 'r' reset, 'ArrowLeft'/'ArrowRight' history.
 *
 *   Touch:
 *   • pinchZoom              — update zoom by scale factor, clamped to [0.25, 4.0].
 *   • twoFingerPan           — pan within 25% margin bounds.
 *
 *   Depth comfort:
 *   • depthComfort           — map raw z ∈ [0,1] to comfortable camera depth.
 *
 *   Focus / offscreen:
 *   • isOffscreen            — true if world position is off-screen given camera.
 *   • offscreenMarker        — {angle, distance} for offscreen indicator placement.
 *
 *   List sync:
 *   • listSyncState          — {action: 'none'|'scroll'|'reframe', itemId: string|null}.
 *
 * Design invariants (frozen per task 6.2.4):
 *   • zoom ∈ [0.25, 4.0] — design.md §10.3.
 *   • fit operations add 25% margin on all sides.
 *   • history is bounded to 20 entries; push drops all future entries.
 *   • All functions are pure — no side effects, no globals, no DOM.
 *
 * IDs: MGR-001, MGR-004, MGR-012, MGR-015, MGR-026; MGD-003, MGD-026, MGD-046;
 *      task 6.2.4 (F6 pre-production spike only — not a shipped renderer path).
 */

// ─── Constants ────────────────────────────────────────────────────────────────

/** Zoom range from design.md §10.3. */
export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 4.0;

/** Maximum history entries (bounded per design.md §10.3 and A6). */
export const HISTORY_BOUND = 20;

/**
 * Pan/fit margin fraction — 25% of scene bounds on each side.
 * From design.md §10.3: "pan bounds = scene bounds plus 25% viewport margin."
 */
export const MARGIN_FRACTION = 0.25;

/**
 * Default screen half-width used for off-screen projection.
 * Represents a normalised viewport half-extent in the same world-unit scale
 * as positions in the packed buffer (range roughly [-1, 1]).
 */
const DEFAULT_VIEWPORT_HALF_X = 1.0;
const DEFAULT_VIEWPORT_HALF_Y = 1.0;

// ─── Camera state ─────────────────────────────────────────────────────────────

/**
 * Immutable camera state value type for the 3D spike.
 *
 * eye    — camera position in world space [x, y, z].
 * target — look-at point in world space [x, y, z].
 * zoom   — zoom level, clamped to [ZOOM_MIN, ZOOM_MAX] = [0.25, 4.0].
 * history      — bounded ring of previous camera states (max 20 entries).
 * historyIndex — current position in history (0 = oldest in ring, length-1 = newest).
 *
 * History semantics:
 *   history[historyIndex] is the "current" committed state.
 *   Entries after historyIndex are "future" (valid for goForward).
 *   pushHistory appends a new entry and discards all future entries.
 */
export interface Camera3DState {
  readonly eye: [number, number, number];
  readonly target: [number, number, number];
  readonly zoom: number;
  readonly history: ReadonlyArray<Camera3DSnapshot>;
  readonly historyIndex: number;
}

/**
 * A lightweight snapshot stored in history (no recursive history field).
 */
export interface Camera3DSnapshot {
  readonly eye: [number, number, number];
  readonly target: [number, number, number];
  readonly zoom: number;
}

/**
 * Default camera framing — looking at the origin from slightly above and in front.
 * zoom=1.0 (identity), eye slightly above and behind origin.
 */
export const DEFAULT_CAMERA_3D: Camera3DState = {
  eye: [0, 0.5, 3.0],
  target: [0, 0, 0],
  zoom: 1.0,
  history: [],
  historyIndex: -1,
};

// ─── Internal helpers ─────────────────────────────────────────────────────────

/** Clamp a number to [min, max]. */
function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Extract a snapshot (no history) from a full Camera3DState.
 */
function toSnapshot(s: Camera3DState): Camera3DSnapshot {
  return { eye: s.eye, target: s.target, zoom: s.zoom };
}

/**
 * Compute bounding box of a list of 3-tuples.
 * Returns null if positions is empty.
 */
function boundingBox(
  positions: ReadonlyArray<[number, number, number]>,
): { min: [number, number, number]; max: [number, number, number] } | null {
  if (positions.length === 0) return null;
  let minX = Infinity, minY = Infinity, minZ = Infinity;
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity;
  for (const [x, y, z] of positions) {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] };
}

/**
 * Compute a camera state that frames the given positions with a 25% margin.
 * The camera looks at the centroid of the bounding box from the +z side.
 * Zoom is adjusted so the widest extent fits in view with margin.
 *
 * Returns the input state unchanged when positions is empty.
 */
function cameraFitPositions(
  state: Camera3DState,
  positions: ReadonlyArray<[number, number, number]>,
): Camera3DState {
  const bb = boundingBox(positions);
  if (bb === null) return state;

  const cx = (bb.min[0] + bb.max[0]) / 2;
  const cy = (bb.min[1] + bb.max[1]) / 2;
  const cz = (bb.min[2] + bb.max[2]) / 2;

  // Extent in x and y
  const extentX = (bb.max[0] - bb.min[0]) * (1 + MARGIN_FRACTION * 2);
  const extentY = (bb.max[1] - bb.min[1]) * (1 + MARGIN_FRACTION * 2);
  const maxExtent = Math.max(extentX, extentY, 0.001); // avoid zero

  // Scale zoom so the scene fits in the normalised [-1,1]^2 view
  const newZoom = clamp(2.0 / maxExtent, ZOOM_MIN, ZOOM_MAX);

  // Place eye above-and-behind the centroid on the +z axis
  const eyeDistance = Math.max(3.0, maxExtent * 2);
  const newEye: [number, number, number] = [cx, cy, cz + eyeDistance];
  const newTarget: [number, number, number] = [cx, cy, cz];

  return { ...state, eye: newEye, target: newTarget, zoom: newZoom };
}

// ─── Fit operations ───────────────────────────────────────────────────────────

/**
 * Move camera to fit all visible node positions with a 25% margin.
 *
 * @param state     — current camera state.
 * @param positions — world positions of all visible nodes.
 * @returns new camera state that frames all positions, or `state` if empty.
 */
export function fitVisible(
  state: Camera3DState,
  positions: ReadonlyArray<[number, number, number]>,
): Camera3DState {
  return cameraFitPositions(state, positions);
}

/**
 * Fit selected nodes with a 25% margin.
 *
 * @param state       — current camera state.
 * @param positions   — Map from node id → world position.
 * @param selectedIds — ids of selected nodes.
 * @returns new camera state framing selected nodes, or `state` if none selected.
 */
export function fitSelection(
  state: Camera3DState,
  positions: ReadonlyMap<string, [number, number, number]>,
  selectedIds: ReadonlyArray<string>,
): Camera3DState {
  const pts: Array<[number, number, number]> = [];
  for (const id of selectedIds) {
    const p = positions.get(id);
    if (p !== undefined) pts.push(p);
  }
  return cameraFitPositions(state, pts);
}

/**
 * Fit a neighborhood of nodes (e.g. direct neighbours of selected node).
 *
 * @param state        — current camera state.
 * @param positions    — Map from node id → world position.
 * @param neighborIds  — ids of nodes in the neighborhood.
 * @returns new camera state framing neighborhood, or `state` if empty.
 */
export function fitNeighborhood(
  state: Camera3DState,
  positions: ReadonlyMap<string, [number, number, number]>,
  neighborIds: ReadonlyArray<string>,
): Camera3DState {
  const pts: Array<[number, number, number]> = [];
  for (const id of neighborIds) {
    const p = positions.get(id);
    if (p !== undefined) pts.push(p);
  }
  return cameraFitPositions(state, pts);
}

/**
 * Return to the default camera framing, preserving history fields.
 *
 * @param state — current camera state.
 * @returns new camera at DEFAULT_CAMERA_3D eye/target/zoom but with same history.
 */
export function resetCamera(state: Camera3DState): Camera3DState {
  return {
    ...state,
    eye: DEFAULT_CAMERA_3D.eye,
    target: DEFAULT_CAMERA_3D.target,
    zoom: DEFAULT_CAMERA_3D.zoom,
  };
}

// ─── History ──────────────────────────────────────────────────────────────────

/**
 * Append `newState` to the history of `state`, bounded to HISTORY_BOUND (20).
 *
 * Behaviour:
 *   - All future entries (after historyIndex) are discarded first.
 *   - The snapshot of `state` is appended (or becomes entry 0 when history is empty).
 *   - If the resulting history would exceed HISTORY_BOUND, the oldest entry is dropped.
 *   - historyIndex is updated to point to the last entry.
 *   - The returned state has the eye/target/zoom of `newState`.
 *
 * @param state    — current camera state (its snapshot is recorded as the "before" point).
 * @param newState — the camera state to transition to (its snapshot is NOT stored yet;
 *                   only `state`'s snapshot is appended — `newState` is the live state).
 */
export function pushHistory(
  state: Camera3DState,
  newState: Camera3DSnapshot,
): Camera3DState {
  // Drop all "future" entries beyond current historyIndex.
  const trimmed: Camera3DSnapshot[] =
    state.historyIndex >= 0
      ? (state.history as Camera3DSnapshot[]).slice(0, state.historyIndex + 1)
      : [];

  // Append the current state's snapshot as the "committed before" entry.
  const appended: Camera3DSnapshot[] = [...trimmed, toSnapshot(state)];

  // Bound to HISTORY_BOUND by dropping the oldest when over.
  const bounded: Camera3DSnapshot[] =
    appended.length > HISTORY_BOUND ? appended.slice(appended.length - HISTORY_BOUND) : appended;

  return {
    eye: newState.eye,
    target: newState.target,
    zoom: newState.zoom,
    history: bounded,
    historyIndex: bounded.length - 1,
  };
}

/**
 * Navigate back in history.
 *
 * Returns the state at `historyIndex - 1`.
 * No-op (returns `state` unchanged) when already at the start (historyIndex ≤ 0).
 */
export function goBack(state: Camera3DState): Camera3DState {
  if (state.historyIndex <= 0) return state;
  const prevIndex = state.historyIndex - 1;
  const snap = state.history[prevIndex];
  if (snap === undefined) return state;
  return {
    eye: snap.eye,
    target: snap.target,
    zoom: snap.zoom,
    history: state.history,
    historyIndex: prevIndex,
  };
}

/**
 * Navigate forward in history.
 *
 * Returns the state at `historyIndex + 1`.
 * No-op (returns `state` unchanged) when at the end of history.
 */
export function goForward(state: Camera3DState): Camera3DState {
  const lastIndex = state.history.length - 1;
  if (state.historyIndex >= lastIndex) return state;
  const nextIndex = state.historyIndex + 1;
  const snap = state.history[nextIndex];
  if (snap === undefined) return state;
  return {
    eye: snap.eye,
    target: snap.target,
    zoom: snap.zoom,
    history: state.history,
    historyIndex: nextIndex,
  };
}

// ─── Keyboard actions ─────────────────────────────────────────────────────────

/**
 * Handle keyboard input — returns a new camera state.
 *
 * Supported keys:
 *   'f'          — fit all visible nodes (fitVisible).
 *   'r'          — reset camera to default framing.
 *   'ArrowLeft'  — go back in history.
 *   'ArrowRight' — go forward in history.
 *
 * Any other key returns `state` unchanged.
 *
 * @param state       — current camera state.
 * @param key         — keyboard key string (e.g. 'f', 'r', 'ArrowLeft').
 * @param positions   — world positions of all visible nodes (for fit).
 * @param selectedIds — currently selected node ids (unused for now; reserved).
 */
export function handleKeyboard(
  state: Camera3DState,
  key: string,
  positions: ReadonlyArray<[number, number, number]>,
  selectedIds: ReadonlyArray<string>,
): Camera3DState {
  switch (key) {
    case 'f':
      return fitVisible(state, positions);
    case 'r':
      return resetCamera(state);
    case 'ArrowLeft':
      return goBack(state);
    case 'ArrowRight':
      return goForward(state);
    default:
      return state;
  }
}

// ─── Touch policy ─────────────────────────────────────────────────────────────

/**
 * Update zoom by a pinch scale factor.
 *
 * newZoom = clamp(state.zoom * scale, ZOOM_MIN, ZOOM_MAX).
 *
 * @param state — current camera state.
 * @param scale — pinch scale factor (e.g. 1.2 = zoom in, 0.8 = zoom out).
 * @returns new camera state with clamped zoom.
 */
export function pinchZoom(state: Camera3DState, scale: number): Camera3DState {
  const newZoom = clamp(state.zoom * scale, ZOOM_MIN, ZOOM_MAX);
  return { ...state, zoom: newZoom };
}

/**
 * Pan the camera by (dx, dy) in world-unit offsets, clamped to scene margin.
 *
 * Pan bounds: the camera target is kept within the scene bounding box plus a
 * MARGIN_FRACTION (25%) margin. Without a scene reference the bounds are
 * defined as [-(1 + MARGIN_FRACTION), (1 + MARGIN_FRACTION)] per axis —
 * matching the packed position range of [-1, 1] plus 25% on each side.
 *
 * Both eye and target are translated identically so the look-direction is preserved.
 *
 * @param state — current camera state.
 * @param dx    — world-unit pan in x (positive = right).
 * @param dy    — world-unit pan in y (positive = up).
 * @returns new camera state with eye and target translated.
 */
export function twoFingerPan(state: Camera3DState, dx: number, dy: number): Camera3DState {
  const bound = 1.0 + MARGIN_FRACTION; // = 1.25

  const newTargetX = clamp(state.target[0] + dx, -bound, bound);
  const newTargetY = clamp(state.target[1] + dy, -bound, bound);

  // Preserve the eye offset relative to target
  const offsetX = state.eye[0] - state.target[0];
  const offsetY = state.eye[1] - state.target[1];
  const offsetZ = state.eye[2] - state.target[2];

  const newEye: [number, number, number] = [
    newTargetX + offsetX,
    newTargetY + offsetY,
    state.eye[2], // z is unaffected by x/y pan — depth comfort owns z
  ];
  const newTarget: [number, number, number] = [newTargetX, newTargetY, state.target[2]];

  // Correct eye z to maintain offset
  newEye[2] = newTarget[2] + offsetZ;

  return { ...state, eye: newEye, target: newTarget };
}

// ─── Depth comfort ────────────────────────────────────────────────────────────

/**
 * Map a raw z-value from the z-axis mapper to a comfortable camera depth distance.
 *
 * Input:  z ∈ [0, 1] (from computeNodeZ: z = (1 − cosine_similarity) / 2).
 *   z=0 — origin node (most similar, closest semantically).
 *   z=1 — most distant node.
 *
 * Output: camera depth distance > 0 that:
 *   - Keeps nodes away from the near plane (minimum distance = 0.5).
 *   - Maps z=0 → 1.0 (close but comfortable).
 *   - Maps z=1 → 6.0 (far back for context).
 *   - Is monotonically increasing.
 *   - Avoids the near-plane collapse artefact (nodes never closer than 0.5).
 *
 * Formula: depth = 1.0 + z * 5.0  (linear; simple and predictable for the spike).
 *
 * @param z — raw z-value in [0, 1].
 * @returns camera depth distance.
 */
export function depthComfort(z: number): number {
  // Clamp input defensively
  const zClamped = clamp(z, 0, 1);
  // Linear mapping: [0,1] → [1.0, 6.0]
  // Near-plane floor: 0.5 (enforced but redundant given minimum of 1.0)
  return 1.0 + zClamped * 5.0;
}

// ─── Focus indication / offscreen ────────────────────────────────────────────

/**
 * Test whether a world position would be off-screen given the current camera.
 *
 * The camera's viewport is approximated as a 2D window centred on the target,
 * with half-extents computed from the zoom level:
 *
 *   half_x = DEFAULT_VIEWPORT_HALF_X / zoom
 *   half_y = DEFAULT_VIEWPORT_HALF_Y / zoom
 *
 * A position is considered on-screen when:
 *   |position.x - target.x| ≤ half_x  AND
 *   |position.y - target.y| ≤ half_y
 *
 * The z-axis is intentionally excluded from the 2D off-screen test:
 * depth compositing is handled separately by depthComfort.
 *
 * @param state    — current camera state.
 * @param position — world position to test.
 * @returns true if the position is off-screen.
 */
export function isOffscreen(
  state: Camera3DState,
  position: [number, number, number],
): boolean {
  const halfX = DEFAULT_VIEWPORT_HALF_X / state.zoom;
  const halfY = DEFAULT_VIEWPORT_HALF_Y / state.zoom;

  const dx = position[0] - state.target[0];
  const dy = position[1] - state.target[1];

  return Math.abs(dx) > halfX || Math.abs(dy) > halfY;
}

/**
 * Compute the angle and distance for an off-screen indicator.
 *
 * Returns the polar angle (radians, measured from +x axis) and the
 * Euclidean distance from the screen centre (target projection) to the
 * off-screen position, in world units.
 *
 * Use `angle` to place an arrow indicator around the viewport edge pointing
 * toward the off-screen node.
 *
 * @param state    — current camera state.
 * @param position — world position of the off-screen node.
 * @returns { angle: number, distance: number }
 */
export function offscreenMarker(
  state: Camera3DState,
  position: [number, number, number],
): { angle: number; distance: number } {
  const dx = position[0] - state.target[0];
  const dy = position[1] - state.target[1];

  const angle = Math.atan2(dy, dx);
  const distance = Math.sqrt(dx * dx + dy * dy);

  return { angle, distance };
}

// ─── List synchronization ─────────────────────────────────────────────────────

/**
 * Result of a list-sync decision.
 *
 * action:
 *   'none'    — no action required; the list is already showing the right item.
 *   'scroll'  — the selected node is on-screen; the list should scroll to itemId.
 *   'reframe' — the selected node is off-screen; the camera should reframe to it.
 * itemId:
 *   The relevant item id, or null when action is 'none'.
 */
export interface ListSyncResult {
  readonly action: 'none' | 'scroll' | 'reframe';
  readonly itemId: string | null;
}

/**
 * Determine whether the list should scroll to a selected node or the camera
 * should reframe to bring it into view.
 *
 * Rules:
 *   1. If `selectedId` is null/empty → action='none', itemId=null.
 *   2. If the selected node's position is not in `positions` → action='none', itemId=null.
 *   3. If the selected node is on-screen (isOffscreen returns false) →
 *      action='scroll', itemId=selectedId.
 *   4. If the selected node is off-screen (isOffscreen returns true) →
 *      action='reframe', itemId=selectedId.
 *
 * @param state      — current camera state.
 * @param selectedId — id of the currently selected node (or null).
 * @param positions  — Map from node id → world position.
 * @returns ListSyncResult.
 */
export function listSyncState(
  state: Camera3DState,
  selectedId: string | null,
  positions: ReadonlyMap<string, [number, number, number]>,
): ListSyncResult {
  if (!selectedId) {
    return { action: 'none', itemId: null };
  }

  const pos = positions.get(selectedId);
  if (pos === undefined) {
    return { action: 'none', itemId: null };
  }

  if (isOffscreen(state, pos)) {
    return { action: 'reframe', itemId: selectedId };
  }

  return { action: 'scroll', itemId: selectedId };
}
