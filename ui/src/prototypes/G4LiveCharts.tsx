/**
 * G4 — uPlot live charts probe (design.md §11.3).
 * Goal: 5 live series @1 Hz; success = <5% CPU, smooth. Mounts a real uPlot
 * instance and pushes a new sample once per second, recording update cost.
 */
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { createSignal, onCleanup, onMount } from "solid-js";

export interface G4LiveChartsProps {
  /** Number of live series (G4 target: 5). */
  series?: number;
  /** Update interval in ms (G4 target: 1000 = 1 Hz). */
  intervalMs?: number;
  /** Ring-buffer window length (points kept on screen). */
  window?: number;
}

const SERIES_TOKENS = [
  "--color-info-solid",
  "--color-success-solid",
  "--color-warning-solid",
  "--color-danger-solid",
  "--color-accent-default",
  "--color-text-secondary",
] as const;

function resolveToken(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function G4LiveCharts(props: G4LiveChartsProps) {
  const seriesCount = () => props.series ?? 5;
  const intervalMs = () => props.intervalMs ?? 1000;
  const windowLen = () => props.window ?? 120;
  let host: HTMLDivElement | undefined;
  let plot: uPlot | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  const [lastUpdateMs, setLastUpdateMs] = createSignal(0);
  const [ticks, setTicks] = createSignal(0);

  onMount(() => {
    if (!host) return;
    const t0 = Math.floor(Date.now() / 1000);
    const data: uPlot.AlignedData = [
      [t0],
      ...Array.from({ length: seriesCount() }, () => [Math.random() * 100] as number[]),
    ] as uPlot.AlignedData;

    const colors = SERIES_TOKENS.map(resolveToken);
    const opts: uPlot.Options = {
      width: 640,
      height: 320,
      title: `G4 · ${seriesCount()} live series @${(1000 / intervalMs()).toFixed(1)} Hz`,
      scales: { x: { time: true } },
      series: [
        {},
        ...Array.from({ length: seriesCount() }, (_, i) => ({
          label: `s${i + 1}`,
          stroke: colors[i % colors.length],
          width: 1,
        })),
      ],
    };
    plot = new uPlot(opts, data, host);

    timer = setInterval(() => {
      if (!plot) return;
      const start = performance.now();
      const arr = plot.data.map((s) => s.slice()) as number[][];
      const nextT = arr[0][arr[0].length - 1] + 1;
      arr[0].push(nextT);
      for (let i = 1; i <= seriesCount(); i++) {
        const prev = arr[i][arr[i].length - 1] ?? 50;
        arr[i].push(Math.max(0, Math.min(100, prev + (Math.random() * 20 - 10))));
      }
      // Trim to the window length to bound memory/CPU.
      if (arr[0].length > windowLen()) {
        for (let i = 0; i < arr.length; i++) arr[i] = arr[i].slice(-windowLen());
      }
      plot.setData(arr as uPlot.AlignedData);
      setLastUpdateMs(performance.now() - start);
      setTicks((n) => n + 1);
    }, intervalMs());
  });

  onCleanup(() => {
    if (timer) clearInterval(timer);
    plot?.destroy();
  });

  return (
    <div style={{ font: "13px var(--font-family-text)", color: "var(--color-text-primary)" }}>
      <div style={{ "margin-bottom": "var(--space-2)", opacity: 0.8 }}>
        G4 · ticks {ticks()} · last setData {lastUpdateMs().toFixed(2)}ms (budget ≪16ms/frame)
      </div>
      <div ref={host} />
    </div>
  );
}

export default G4LiveCharts;
