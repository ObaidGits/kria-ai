export interface KriaUiMetric {
  name: string;
  startTime: number;
  duration: number;
}

declare global {
  interface Window {
    __KRIA_UI_METRICS__?: KriaUiMetric[];
  }
}

const METRIC_PREFIX = "kria-ui";
const MAX_BUFFERED_METRICS = 200;

export function startUiMeasure(name: string): string {
  const markName = `${METRIC_PREFIX}:${name}:start:${performance.now().toFixed(3)}`;
  performance.mark(markName);
  return markName;
}

export function endUiMeasure(name: string, startMark: string): void {
  const endMark = `${METRIC_PREFIX}:${name}:end:${performance.now().toFixed(3)}`;
  const measureName = `${METRIC_PREFIX}:${name}`;

  performance.mark(endMark);
  performance.measure(measureName, startMark, endMark);

  const entries = performance.getEntriesByName(measureName);
  const entry = entries[entries.length - 1];
  if (!entry) return;

  const metric: KriaUiMetric = {
    name,
    startTime: entry.startTime,
    duration: entry.duration,
  };

  const buffer = (window.__KRIA_UI_METRICS__ ??= []);
  buffer.push(metric);
  if (buffer.length > MAX_BUFFERED_METRICS) {
    buffer.splice(0, buffer.length - MAX_BUFFERED_METRICS);
  }

  performance.clearMarks(startMark);
  performance.clearMarks(endMark);
  performance.clearMeasures(measureName);
}
