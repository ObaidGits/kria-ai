/**
 * Pure zoom / pan / fit math for the in-app remote desktop view.
 *
 * No DOM access — these functions are unit-tested in isolation. The view layer
 * applies the resulting {@link ViewTransform} as a CSS
 * `transform: translate(tx,ty) scale(scale)` on the `<video>` element
 * (transform-origin: 0 0), so zoom/pan are GPU-composited with no per-frame JS.
 *
 * Coordinate model:
 *   - container (viewport): `vw × vh` screen px
 *   - surface (remote desktop intrinsic): `sw × sh` px (only the aspect matters)
 *   - `scale` is absolute: surface px → screen px (so content = sw*scale × sh*scale)
 *   - `tx,ty` is the content top-left offset within the container (screen px)
 *   - at "fit", `scale = fitScale(b)` and the content is letterbox-centered
 *
 * The backend input contract is unchanged: pointer coordinates are normalized
 * to `[0,1]` of the surface via {@link clientToSurfaceNorm}, which inverts the
 * active transform so a tap lands where the user sees it at any zoom/pan.
 */

export interface ViewTransform {
  /** Absolute scale: surface px → screen px. */
  scale: number;
  /** Content top-left X offset within the container (screen px). */
  tx: number;
  /** Content top-left Y offset within the container (screen px). */
  ty: number;
}

export interface Bounds {
  /** Container (viewport) width in screen px. */
  vw: number;
  /** Container (viewport) height in screen px. */
  vh: number;
  /** Surface (remote desktop) intrinsic width in px. */
  sw: number;
  /** Surface (remote desktop) intrinsic height in px. */
  sh: number;
}

/** Maximum zoom relative to the fit scale (e.g. 4× fit). */
export const MAX_ZOOM = 4;

const safe = (n: number, fallback = 1) =>
  Number.isFinite(n) && n > 0 ? n : fallback;

/** Scale at which the whole surface is visible (letterbox "contain" fit). */
export function fitScale(b: Bounds): number {
  const vw = safe(b.vw);
  const vh = safe(b.vh);
  const sw = safe(b.sw);
  const sh = safe(b.sh);
  return Math.min(vw / sw, vh / sh);
}

/** Min (fit) and max absolute scale for the given bounds. */
export function scaleRange(b: Bounds): { min: number; max: number } {
  const min = fitScale(b);
  return { min, max: min * MAX_ZOOM };
}

/** The fit transform: scale = fitScale, content letterbox-centered. */
export function fitTransform(b: Bounds): ViewTransform {
  const scale = fitScale(b);
  return clampTransform({ scale, tx: 0, ty: 0 }, b);
}

function clampAxis(offset: number, contentSize: number, viewSize: number): number {
  if (contentSize <= viewSize) {
    // Smaller than the viewport (letterbox) → center it.
    return (viewSize - contentSize) / 2;
  }
  // Larger than the viewport → keep content covering it (no empty gutters).
  const min = viewSize - contentSize; // content bottom/right-aligned (most negative)
  const max = 0; // content top/left-aligned
  return Math.min(max, Math.max(min, offset));
}

/**
 * Clamp scale to `[fitScale, fitScale*MAX_ZOOM]` and translation so the content
 * always covers the viewport (or is centered when smaller than it).
 */
export function clampTransform(t: ViewTransform, b: Bounds): ViewTransform {
  const { min, max } = scaleRange(b);
  const scale = Math.min(max, Math.max(min, safe(t.scale, min)));
  const contentW = b.sw * scale;
  const contentH = b.sh * scale;
  return {
    scale,
    tx: clampAxis(Number.isFinite(t.tx) ? t.tx : 0, contentW, b.vw),
    ty: clampAxis(Number.isFinite(t.ty) ? t.ty : 0, contentH, b.vh),
  };
}

/**
 * Zoom around a focus point (in container/screen coords) by `deltaScale`
 * (multiplicative). The surface point under the focus stays under the focus.
 */
export function applyPinch(
  t: ViewTransform,
  focusX: number,
  focusY: number,
  deltaScale: number,
  b: Bounds,
): ViewTransform {
  const { min, max } = scaleRange(b);
  const newScale = Math.min(max, Math.max(min, t.scale * safe(deltaScale)));
  // Fraction of the content under the focus, before scaling.
  const fx = (focusX - t.tx) / (b.sw * t.scale);
  const fy = (focusY - t.ty) / (b.sh * t.scale);
  const newContentW = b.sw * newScale;
  const newContentH = b.sh * newScale;
  return clampTransform(
    { scale: newScale, tx: focusX - fx * newContentW, ty: focusY - fy * newContentH },
    b,
  );
}

/** Translate the content by a screen-space delta, clamped to bounds. */
export function applyPan(t: ViewTransform, dx: number, dy: number, b: Bounds): ViewTransform {
  return clampTransform({ scale: t.scale, tx: t.tx + dx, ty: t.ty + dy }, b);
}

/** Is the transform effectively at fit scale (within a small epsilon)? */
export function isFit(t: ViewTransform, b: Bounds): boolean {
  return Math.abs(t.scale - fitScale(b)) < fitScale(b) * 0.02;
}

/**
 * Double-tap toggle: if at (or near) fit, zoom to 2× fit centered on the tap;
 * otherwise return to fit.
 */
export function doubleTapToggle(
  t: ViewTransform,
  x: number,
  y: number,
  b: Bounds,
): ViewTransform {
  if (isFit(t, b)) {
    const target = 2; // 2× the fit scale
    return applyPinch(t, x, y, target, b);
  }
  return fitTransform(b);
}

/**
 * Map a client (screen) point to a `[0,1]` position on the remote surface,
 * inverting the active transform. Clamped to `[0,1]`.
 */
export function clientToSurfaceNorm(
  clientX: number,
  clientY: number,
  containerLeft: number,
  containerTop: number,
  t: ViewTransform,
  b: Bounds,
): { x: number; y: number } {
  const contentW = b.sw * t.scale;
  const contentH = b.sh * t.scale;
  const x = contentW > 0 ? (clientX - containerLeft - t.tx) / contentW : 0;
  const y = contentH > 0 ? (clientY - containerTop - t.ty) / contentH : 0;
  return { x: Math.max(0, Math.min(1, x)), y: Math.max(0, Math.min(1, y)) };
}
