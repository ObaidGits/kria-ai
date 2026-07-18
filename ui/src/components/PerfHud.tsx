/**
 * PerfHud — a lightweight, dev-gated performance HUD (design.md §1.22).
 *
 * Shows the most recent perf measures from `utils/perf` (space-switch,
 * palette-open, first-token, lens-mount, list-scroll, …) with their §5.6
 * budgets, flagging any that ran over budget. Later this lives inside
 * Observatory → Diagnostics; for now it can be dropped anywhere in dev.
 *
 * Production cost: ZERO. `PERF_HUD_ENABLED` is `import.meta.env.DEV`, which is a
 * static `false` in production builds, so the component early-returns null and
 * the subscription/DOM are tree-shaken/never created. Never gate on runtime
 * state alone — the static flag is what lets the bundler drop it.
 *
 * Styling uses design tokens only (Req 14.2) — no raw color literals.
 */
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { clearMeasures, getMeasures, subscribe, type PerfMeasure } from "../utils/perf";

/** Static dev gate — false in production builds, enabling dead-code elimination. */
export const PERF_HUD_ENABLED = import.meta.env.DEV;

const MAX_ROWS = 12;

function panelStyle(): JSX.CSSProperties {
  return {
    position: "fixed",
    bottom: "var(--space-3)",
    right: "var(--space-3)",
    "z-index": "var(--z-floating)",
    "min-width": "220px",
    "max-width": "320px",
    padding: "var(--space-3)",
    "border-radius": "var(--radius-md)",
    background: "var(--color-surface-3)",
    border: "1px solid var(--color-border-default)",
    "box-shadow": "var(--elevation-3)",
    "backdrop-filter": "blur(var(--blur-floating))",
    color: "var(--color-text-primary)",
    "font-family": "var(--font-family-mono)",
    "font-size": "var(--font-size-micro)",
    "line-height": "var(--font-line-height-normal)",
  };
}

function rowStyle(over: boolean): JSX.CSSProperties {
  return {
    display: "flex",
    "justify-content": "space-between",
    gap: "var(--space-2)",
    padding: "var(--space-1) 0",
    color: over ? "var(--color-danger-text)" : "var(--color-text-secondary)",
  };
}

function formatMs(n: number): string {
  return `${n.toFixed(1)}ms`;
}

/**
 * The perf HUD. Renders nothing in production. In dev it subscribes to perf
 * measures and shows the most recent ones with budget status.
 */
export function PerfHud(): JSX.Element {
  if (!PERF_HUD_ENABLED) return null;

  const [measures, setMeasures] = createSignal<PerfMeasure[]>(getMeasures().slice(-MAX_ROWS));
  const [open, setOpen] = createSignal(true);

  onMount(() => {
    const unsubscribe = subscribe((m) => {
      setMeasures((prev) => [...prev, m].slice(-MAX_ROWS));
    });
    onCleanup(unsubscribe);
  });

  const overBudgetCount = () => measures().filter((m) => m.overBudget).length;

  return (
    <aside
      aria-label="Performance HUD"
      role="complementary"
      style={panelStyle()}
      data-testid="perf-hud"
    >
      <header
        style={{
          display: "flex",
          "align-items": "center",
          "justify-content": "space-between",
          gap: "var(--space-2)",
          "margin-bottom": open() ? "var(--space-2)" : "0",
        }}
      >
        <strong style={{ "font-family": "var(--font-family-display)", color: "var(--color-text-primary)" }}>
          Perf HUD
          <Show when={overBudgetCount() > 0}>
            <span
              style={{ "margin-left": "var(--space-2)", color: "var(--color-danger-text)" }}
              aria-label={`${overBudgetCount()} over budget`}
            >
              ⚠ {overBudgetCount()}
            </span>
          </Show>
        </strong>
        <span style={{ display: "flex", gap: "var(--space-2)" }}>
          <button
            type="button"
            onClick={() => {
              clearMeasures();
              setMeasures([]);
            }}
            aria-label="Clear perf measures"
            style={{
              background: "transparent",
              border: "1px solid var(--color-border-default)",
              "border-radius": "var(--radius-sm)",
              color: "var(--color-text-secondary)",
              "font-size": "var(--font-size-micro)",
              cursor: "pointer",
              padding: "0 var(--space-2)",
            }}
          >
            clear
          </button>
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open()}
            aria-label={open() ? "Collapse perf HUD" : "Expand perf HUD"}
            style={{
              background: "transparent",
              border: "1px solid var(--color-border-default)",
              "border-radius": "var(--radius-sm)",
              color: "var(--color-text-secondary)",
              "font-size": "var(--font-size-micro)",
              cursor: "pointer",
              padding: "0 var(--space-2)",
            }}
          >
            {open() ? "–" : "+"}
          </button>
        </span>
      </header>

      <Show when={open()}>
        <Show
          when={measures().length > 0}
          fallback={
            <p style={{ margin: "0", color: "var(--color-text-muted)" }}>No measures yet.</p>
          }
        >
          <ul style={{ margin: "0", padding: "0", "list-style": "none" }}>
            <For each={measures().slice().reverse()}>
              {(m) => (
                <li style={rowStyle(m.overBudget)}>
                  <span>
                    {m.overBudget ? "● " : "○ "}
                    {m.name}
                  </span>
                  <span>
                    {formatMs(m.duration)}
                    <Show when={m.budgetMs != null}>
                      <span style={{ color: "var(--color-text-muted)" }}> / {formatMs(m.budgetMs!)}</span>
                    </Show>
                  </span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </aside>
  );
}

export default PerfHud;
