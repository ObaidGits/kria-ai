/** Linux desktop presentation validation helpers (Req 18.5). */
export const LINUX_DESKTOP_MATRIX = [
  { desktop: "GNOME", session: "Wayland" },
  { desktop: "GNOME", session: "X11" },
  { desktop: "KDE", session: "Wayland" },
  { desktop: "KDE", session: "X11" },
] as const;

export interface BackingStoreSize {
  cssWidth: number;
  cssHeight: number;
  pixelWidth: number;
  pixelHeight: number;
  devicePixelRatio: number;
}

/**
 * Preserve CSS proportions while allocating integer device pixels for canvas.
 * DPR is bounded to avoid accidental huge allocations from invalid/hostile input.
 */
export function canvasBackingStoreSize(
  cssWidth: number,
  cssHeight: number,
  devicePixelRatio: number,
): BackingStoreSize {
  const width = Number.isFinite(cssWidth) && cssWidth > 0 ? cssWidth : 1;
  const height = Number.isFinite(cssHeight) && cssHeight > 0 ? cssHeight : 1;
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0
    ? Math.min(devicePixelRatio, 4)
    : 1;
  return {
    cssWidth: width,
    cssHeight: height,
    pixelWidth: Math.max(1, Math.round(width * ratio)),
    pixelHeight: Math.max(1, Math.round(height * ratio)),
    devicePixelRatio: ratio,
  };
}
