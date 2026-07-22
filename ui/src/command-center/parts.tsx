/**
 * Command Center — shared presentational parts.
 *
 * Reusable, self-contained building blocks used by every panel. Extracting them
 * here (Phase 1 architecture) lets each panel be relocated into the Command Deck
 * or Developer Observatory with a single import move — no rewrite. Pure
 * presentation: no stores, no services, no side effects beyond a local clock.
 */
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { CcIcon } from "./CcIcon";
import type { Gauge } from "./data";

/** Resolve reduced-motion (global kill-switch first, then OS query). */
export function reducedMotion(): boolean {
  if (typeof document !== "undefined" && document.documentElement?.dataset.reducedMotion === "on") return true;
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  }
  return false;
}

/** Live clock (demo: real system time), 1s tick, cleaned up on unmount. */
export function useClock() {
  const [now, setNow] = createSignal(new Date());
  onMount(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    onCleanup(() => clearInterval(id));
  });
  const date = () =>
    now().toLocaleDateString(undefined, { weekday: "long", day: "2-digit", month: "long", year: "numeric" });
  const time = () =>
    now().toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: true }).toLowerCase();
  return { date, time };
}

/** Decorative animated equaliser bar row. */
export function Waveform(props: { bars?: number; class?: string }) {
  const bars = () => props.bars ?? 22;
  return (
    <div class={`cc-wave ${props.class ?? ""}`.trim()} aria-hidden="true">
      <For each={Array.from({ length: bars() })}>{(_, i) => <span style={{ "--i": i() }} />}</For>
    </div>
  );
}

/** SVG radial gauge (0..100). */
export function RadialGauge(props: { gauge: Gauge }) {
  const R = 34;
  const C = 2 * Math.PI * R;
  const offset = () => C * (1 - props.gauge.value / 100);
  return (
    <div class="cc-gauge">
      <svg viewBox="0 0 80 80" width="80" height="80" aria-hidden="true">
        <circle class="cc-gauge__track" cx="40" cy="40" r={R} />
        <circle
          class="cc-gauge__value"
          cx="40"
          cy="40"
          r={R}
          stroke-dasharray={String(C)}
          stroke-dashoffset={String(offset())}
          transform="rotate(-90 40 40)"
        />
      </svg>
      <div class="cc-gauge__center">
        <span class="cc-gauge__label">{props.gauge.label}</span>
        <span class="cc-gauge__num">{props.gauge.value}%</span>
      </div>
    </div>
  );
}

/**
 * Panel — the standard framed surface (header + optional action + body).
 *
 * This is the relocatable container every migratable widget renders inside, so
 * a widget looks identical whether it lives on the homepage today or in the
 * Command Deck / Developer Observatory tomorrow.
 */
export function Panel(props: { title: string; action?: string; class?: string; children: JSX.Element }) {
  return (
    <section class={`cc-panel ${props.class ?? ""}`.trim()}>
      <header class="cc-panel__head">
        <h2 class="cc-panel__title">{props.title}</h2>
        <Show when={props.action}>
          <button type="button" class="cc-panel__action">
            {props.action} <CcIcon name="chevron" size={12} />
          </button>
        </Show>
      </header>
      <div class="cc-panel__body">{props.children}</div>
    </section>
  );
}
