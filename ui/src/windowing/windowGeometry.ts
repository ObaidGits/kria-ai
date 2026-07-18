export interface WindowGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
}

export interface GeometryMonitor {
  workArea: {
    position: { x: number; y: number };
    size: { width: number; height: number };
  };
  scaleFactor: number;
}

const MIN_LOGICAL_WIDTH = 400;
const MIN_LOGICAL_HEIGHT = 500;

export function isWindowGeometry(value: unknown): value is WindowGeometry {
  if (!value || typeof value !== "object") return false;
  const item = value as Record<string, unknown>;
  return ["x", "y", "width", "height", "scaleFactor"].every(
    (key) => typeof item[key] === "number" && Number.isFinite(item[key]),
  ) && (item.width as number) > 0 && (item.height as number) > 0 && (item.scaleFactor as number) > 0;
}

function intersectionArea(geometry: WindowGeometry, monitor: GeometryMonitor): number {
  const left = Math.max(geometry.x, monitor.workArea.position.x);
  const top = Math.max(geometry.y, monitor.workArea.position.y);
  const right = Math.min(geometry.x + geometry.width, monitor.workArea.position.x + monitor.workArea.size.width);
  const bottom = Math.min(geometry.y + geometry.height, monitor.workArea.position.y + monitor.workArea.size.height);
  return Math.max(0, right - left) * Math.max(0, bottom - top);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

export function normalizeGeometry(
  geometry: WindowGeometry,
  monitors: readonly GeometryMonitor[],
): WindowGeometry | null {
  if (!isWindowGeometry(geometry) || monitors.length === 0) return null;
  const monitor = monitors.reduce((best, candidate) =>
    intersectionArea(geometry, candidate) > intersectionArea(geometry, best) ? candidate : best,
  );
  const work = monitor.workArea;
  const scaleRatio = monitor.scaleFactor / geometry.scaleFactor;
  const minWidth = Math.min(work.size.width, MIN_LOGICAL_WIDTH * monitor.scaleFactor);
  const minHeight = Math.min(work.size.height, MIN_LOGICAL_HEIGHT * monitor.scaleFactor);
  const width = clamp(Math.round(geometry.width * scaleRatio), minWidth, work.size.width);
  const height = clamp(Math.round(geometry.height * scaleRatio), minHeight, work.size.height);
  return {
    x: clamp(Math.round(geometry.x), work.position.x, work.position.x + work.size.width - width),
    y: clamp(Math.round(geometry.y), work.position.y, work.position.y + work.size.height - height),
    width,
    height,
    scaleFactor: monitor.scaleFactor,
  };
}
