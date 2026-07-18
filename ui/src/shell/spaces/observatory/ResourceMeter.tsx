import { createEffect, onCleanup, onMount, Show } from "solid-js";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { DataAuthority, TelemetryPoint } from "../../../stores";
import { HonestyBadge } from "./HonestyBadge";

export function ResourceMeter(props: {
  title: string;
  metric: string;
  unit: string;
  points: TelemetryPoint[];
  authority: DataAuthority;
}) {
  let host: HTMLDivElement | undefined;
  let plot: uPlot | undefined;
  const series = () => props.points.filter((point) => point.metric === props.metric).slice(-120);

  const latest = () => {
    const points = series();
    return points[points.length - 1];
  };

  function data(): uPlot.AlignedData {
    const points = series();
    return [points.map((point) => point.ts / 1000), points.map((point) => point.value)];
  }

  function mountPlot() {
    if (!host || plot || series().length === 0 || navigator.userAgent.includes("jsdom")) return;
    const styles = getComputedStyle(host);
    plot = new uPlot({
      width: Math.max(host.clientWidth, 280), height: 150,
      legend: { show: false }, cursor: { show: false },
      scales: { x: { time: true }, y: { range: (_u, min, max) => [Math.min(0, min), Math.max(100, max)] } },
      axes: [{ stroke: styles.getPropertyValue("--color-text-muted") }, { stroke: styles.getPropertyValue("--color-text-muted") }],
      series: [{}, { label: props.title, stroke: styles.getPropertyValue("--color-accent-default"), width: 2 }],
    }, data(), host);
  }

  onMount(mountPlot);
  createEffect(() => {
    props.points;
    mountPlot();
    if (plot && series().length > 0) plot.setData(data());
  });
  onCleanup(() => plot?.destroy());

  return (
    <section class="kria-observatory__meter" aria-label={`${props.title} resource meter`}>
      <div class="kria-observatory__card-head">
        <div><h3>{props.title}</h3><span>{latest()?.value ?? "—"} {props.unit}</span></div>
        <HonestyBadge authority={props.authority} />
      </div>
      <Show when={series().length > 0} fallback={
        <p role="status">Awaiting {props.title.toLowerCase()} samples.</p>
      }>
        <div ref={host} class="kria-observatory__chart" aria-hidden="true" />
        <span class="kria-observatory__sr-only">Latest {props.title}: {latest()?.value} {props.unit}</span>
      </Show>
    </section>
  );
}
