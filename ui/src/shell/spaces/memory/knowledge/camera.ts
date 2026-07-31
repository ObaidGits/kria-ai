/**
 * memory/knowledge/camera — Camera world-coordinates, zoom, and pan.
 *
 * Pure TypeScript module — no JSX, no DOM, no side effects.
 *
 * Manages a 2-D canvas camera with:
 *   - Zoom clamped to [ZOOM_MIN, ZOOM_MAX]
 *   - Zoom-at-point that pivots around the cursor
 *   - Pan with 25% viewport-margin bounds enforcement
 *   - World ↔ screen coordinate transforms
 *
 * IDs: MGD-003; task 4.7.6.
 */

// ─── Constants ────────────────────────────────────────────────────────────────

export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 4.0;
/** Pan margin as a fraction of the viewport dimension (25%). */
export const PAN_MARGIN = 0.25;

// ─── Types ────────────────────────────────────────────────────────────────────

export interface CameraState {
  /** World-space center X that maps to the viewport centre. */
  x: number;
  /** World-space center Y that maps to the viewport centre. */
  y: number;
  /** Current zoom level – always in [ZOOM_MIN, ZOOM_MAX]. */
  zoom: number;
}

export interface ViewportSize {
  width: number;
  height: number;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Clamp a value to [min, max].
 */
function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

// ─── Public API ───────────────────────────────────────────────────────────────

/**
 * Create an initial camera centred at (viewport.width/2, viewport.height/2)
 * with zoom = 1.
 */
export function createCamera(viewport: ViewportSize): CameraState {
  return {
    x: viewport.width / 2,
    y: viewport.height / 2,
    zoom: 1,
  };
}

/**
 * Clamp zoom to [ZOOM_MIN, ZOOM_MAX].
 */
export function clampZoom(zoom: number): number {
  return clamp(zoom, ZOOM_MIN, ZOOM_MAX);
}

/**
 * Zoom at a specific screen-space point (screenPx, screenPy).
 *
 * The world point under the cursor stays fixed: we adjust the camera's
 * world-space centre so that the same world coordinate continues to sit
 * beneath the cursor after the zoom change.
 *
 * zoomDelta is a multiplicative scale factor (e.g. 1.1 to zoom in, 0.9 to
 * zoom out).  The resulting zoom is clamped to [ZOOM_MIN, ZOOM_MAX].
 */
export function zoomAt(
  camera: CameraState,
  viewport: ViewportSize,
  screenPx: number,
  screenPy: number,
  zoomDelta: number,
): CameraState {
  const newZoom = clampZoom(camera.zoom * zoomDelta);

  // World point currently under the cursor:
  const worldUnderCursor = screenToWorld(camera, viewport, screenPx, screenPy);

  // After applying newZoom, adjust the camera centre so that worldUnderCursor
  // still maps back to (screenPx, screenPy).
  //
  // screenToWorld formula (inverted):
  //   worldX = (screenX - vw/2) / zoom + cameraX
  // Therefore for the new zoom:
  //   worldUnderCursor.x = (screenPx - vw/2) / newZoom + newCameraX
  //   => newCameraX = worldUnderCursor.x - (screenPx - vw/2) / newZoom
  const halfW = viewport.width / 2;
  const halfH = viewport.height / 2;

  return {
    x: worldUnderCursor.x - (screenPx - halfW) / newZoom,
    y: worldUnderCursor.y - (screenPy - halfH) / newZoom,
    zoom: newZoom,
  };
}

/**
 * Pan camera by (dx, dy) in **screen** space and enforce 25% margin bounds.
 *
 * The pan margin means the camera centre must stay within the world bounds
 * expanded by ±(PAN_MARGIN × viewport dimension / zoom) on each axis, so that
 * the user can always pan a little past the content edges but not lose it
 * entirely.
 *
 * dx/dy are in screen pixels; they are divided by camera.zoom to convert to
 * world-space displacement.
 */
export function panCamera(
  camera: CameraState,
  viewport: ViewportSize,
  worldBounds: { minX: number; minY: number; maxX: number; maxY: number },
  dx: number,
  dy: number,
): CameraState {
  // Convert screen-space delta to world-space delta.
  const newX = camera.x - dx / camera.zoom;
  const newY = camera.y - dy / camera.zoom;

  // Compute the margin in world units.
  const marginX = (PAN_MARGIN * viewport.width) / camera.zoom;
  const marginY = (PAN_MARGIN * viewport.height) / camera.zoom;

  // Clamp so the camera stays within the world bounds + margin.
  const clampedX = clamp(newX, worldBounds.minX - marginX, worldBounds.maxX + marginX);
  const clampedY = clamp(newY, worldBounds.minY - marginY, worldBounds.maxY + marginY);

  return { x: clampedX, y: clampedY, zoom: camera.zoom };
}

/**
 * Transform a world-space point to screen-space coordinates.
 *
 * The camera.x / camera.y is the world point that maps to the screen centre.
 *
 *   screenX = (worldX - camera.x) * zoom + viewportWidth / 2
 */
export function worldToScreen(
  camera: CameraState,
  viewport: ViewportSize,
  worldX: number,
  worldY: number,
): { x: number; y: number } {
  return {
    x: (worldX - camera.x) * camera.zoom + viewport.width / 2,
    y: (worldY - camera.y) * camera.zoom + viewport.height / 2,
  };
}

/**
 * Transform a screen-space point to world-space coordinates.
 *
 *   worldX = (screenX - viewportWidth / 2) / zoom + camera.x
 */
export function screenToWorld(
  camera: CameraState,
  viewport: ViewportSize,
  screenX: number,
  screenY: number,
): { x: number; y: number } {
  return {
    x: (screenX - viewport.width / 2) / camera.zoom + camera.x,
    y: (screenY - viewport.height / 2) / camera.zoom + camera.y,
  };
}
