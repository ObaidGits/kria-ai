/**
 * memory/knowledge/cameraHistory — Camera history with Back/Forward navigation
 * and fit-to-items helper.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Provides an immutable history stack so the knowledge graph can navigate
 * back/forward between camera states (e.g. after a query or manual pan/zoom).
 *
 * fitItems positions the camera so all supplied item positions are visible
 * with a small padding margin.
 *
 * IDs: MGD-003; task 4.7.7.
 */

import { type CameraState, type ViewportSize, ZOOM_MIN, ZOOM_MAX } from "./camera";

// ─── Types ────────────────────────────────────────────────────────────────────

export interface CameraHistoryEntry {
  /** Camera state at this history point. */
  camera: CameraState;
  /** Associated query ID, if any (used for revision-compatible requery). */
  queryId: string | null;
  /** Graph revision at this point, if known. */
  revision: number | null;
}

export interface CameraHistory {
  entries: CameraHistoryEntry[];
  /** Index of the current entry; -1 when history is empty. */
  currentIndex: number;
}

// ─── Constants ────────────────────────────────────────────────────────────────

/** Padding fraction applied around the bounding box when fitting items (10%). */
const FIT_PADDING = 0.10;

// ─── Public API ───────────────────────────────────────────────────────────────

/**
 * Create an empty camera history.
 */
export function createCameraHistory(): CameraHistory {
  return { entries: [], currentIndex: -1 };
}

/**
 * Push a new entry onto the history.
 *
 * If the current position is not the latest entry (i.e. the user has gone
 * back), all forward entries are discarded before appending.
 *
 * Returns a new CameraHistory — does not mutate the input.
 */
export function pushEntry(
  history: CameraHistory,
  entry: CameraHistoryEntry,
): CameraHistory {
  // Discard any entries after currentIndex (truncate forward history).
  const kept = history.entries.slice(0, history.currentIndex + 1);
  const entries = [...kept, entry];
  return { entries, currentIndex: entries.length - 1 };
}

/**
 * Navigate backward in history.
 *
 * Returns the updated history and the entry now at the current position,
 * or null if already at the beginning.
 */
export function goBack(
  history: CameraHistory,
): { history: CameraHistory; entry: CameraHistoryEntry | null } {
  if (!canGoBack(history)) {
    return { history, entry: null };
  }
  const newIndex = history.currentIndex - 1;
  const newHistory: CameraHistory = { entries: history.entries, currentIndex: newIndex };
  return { history: newHistory, entry: history.entries[newIndex] };
}

/**
 * Navigate forward in history.
 *
 * Returns the updated history and the entry now at the current position,
 * or null if already at the end.
 */
export function goForward(
  history: CameraHistory,
): { history: CameraHistory; entry: CameraHistoryEntry | null } {
  if (!canGoForward(history)) {
    return { history, entry: null };
  }
  const newIndex = history.currentIndex + 1;
  const newHistory: CameraHistory = { entries: history.entries, currentIndex: newIndex };
  return { history: newHistory, entry: history.entries[newIndex] };
}

/**
 * True when there is at least one entry before the current position.
 */
export function canGoBack(history: CameraHistory): boolean {
  return history.currentIndex > 0;
}

/**
 * True when there is at least one entry after the current position.
 */
export function canGoForward(history: CameraHistory): boolean {
  return history.currentIndex < history.entries.length - 1;
}

/**
 * Compute a camera state that fits all item positions within the viewport.
 *
 * The resulting camera centres the bounding box of all items and adjusts the
 * zoom so the bounding box (plus a 10% padding margin) fits within the
 * viewport.  Zoom is clamped to [ZOOM_MIN, ZOOM_MAX].
 *
 * If `itemPositions` is empty, the original camera is returned unchanged.
 *
 * Pure function — does not mutate inputs.
 */
export function fitItems(
  camera: CameraState,
  viewport: ViewportSize,
  itemPositions: Map<string, { x: number; y: number }>,
): CameraState {
  if (itemPositions.size === 0) {
    return camera;
  }

  const positions = Array.from(itemPositions.values());

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;

  for (const { x, y } of positions) {
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
  }

  const centreX = (minX + maxX) / 2;
  const centreY = (minY + maxY) / 2;

  const contentW = maxX - minX;
  const contentH = maxY - minY;

  let zoom: number;

  if (contentW === 0 && contentH === 0) {
    // Single point — keep existing zoom (or reset to 1 if out of bounds).
    zoom = Math.min(Math.max(camera.zoom, ZOOM_MIN), ZOOM_MAX);
  } else {
    const padW = viewport.width * (1 - FIT_PADDING);
    const padH = viewport.height * (1 - FIT_PADDING);

    const zoomX = contentW > 0 ? padW / contentW : Infinity;
    const zoomY = contentH > 0 ? padH / contentH : Infinity;

    zoom = Math.min(zoomX, zoomY);
    zoom = Math.min(Math.max(zoom, ZOOM_MIN), ZOOM_MAX);
  }

  return { x: centreX, y: centreY, zoom };
}
