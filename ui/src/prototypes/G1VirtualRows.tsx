/**
 * G1 — WebKitGTK baseline probe (design.md §11.3).
 * Goal: shell + 5k virtualized rows; success = 60fps scroll, idle CPU/GPU ~0,
 * no blank screen. Uses @tanstack/solid-virtual (mandatory virtualization per
 * §11.2) and records `list-scroll` measures via src/utils/perf.ts.
 */
import { createVirtualizer } from "@tanstack/solid-virtual";
import { createSignal, For, onCleanup } from "solid-js";
import { endMeasure, getMeasures, startMeasure, type PerfHandle } from "../utils/perf";

export interface G1VirtualRowsProps {
  /** Row count to virtualize (G1 target: 5000). */
  count?: number;
  /** Fixed row height in px. */
  rowHeight?: number;
}

/** Synthetic row model — cheap, representative of a memory/log/chat row. */
function rowLabel(i: number): string {
  return `Row ${i} · memory·auto·capability · status ok · ${(i * 37) % 1000}ms`;
}

export function G1VirtualRows(props: G1VirtualRowsProps) {
  const count = () => props.count ?? 5000;
  const rowHeight = () => props.rowHeight ?? 36;
  let scrollEl: HTMLDivElement | undefined;
  const [lastFrameMs, setLastFrameMs] = createSignal(0);

  const virtualizer = createVirtualizer({
    get count() {
      return count();
    },
    getScrollElement: () => scrollEl ?? null,
    estimateSize: () => rowHeight(),
    overscan: 8,
  });

  // Record a `list-scroll` measure per scroll event so the perf HUD / metrics
  // buffer captures frame cost during interaction.
  let scrollHandle: PerfHandle | null = null;
  const onScroll = () => {
    if (scrollHandle) {
      const m = endMeasure("list-scroll", scrollHandle);
      if (m) setLastFrameMs(m.duration);
    }
    scrollHandle = startMeasure("list-scroll");
  };

  onCleanup(() => {
    if (scrollHandle) endMeasure("list-scroll", scrollHandle);
  });

  return (
    <div style={{ font: "13px/1.4 var(--font-family-text)", color: "var(--color-text-primary)" }}>
      <div style={{ "margin-bottom": "var(--space-2)", opacity: 0.8 }}>
        G1 · {count()} virtualized rows · last frame {lastFrameMs().toFixed(1)}ms · measures{" "}
        {getMeasures().length}
      </div>
      <div
        ref={scrollEl}
        onScroll={onScroll}
        style={{
          height: "420px",
          overflow: "auto",
          border: "1px solid var(--color-border-default)",
          "border-radius": "var(--radius-sm)",
          background: "var(--color-neutral-1)",
          contain: "strict",
        }}
      >
        <div style={{ height: `${virtualizer.getTotalSize()}px`, position: "relative", width: "100%" }}>
          <For each={virtualizer.getVirtualItems()}>
            {(vrow) => (
              <div
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: `${vrow.size}px`,
                  transform: `translateY(${vrow.start}px)`,
                  display: "flex",
                  "align-items": "center",
                  padding: "0 var(--space-3)",
                  "box-sizing": "border-box",
                  "border-bottom": "1px solid var(--color-border-default)",
                }}
              >
                {rowLabel(vrow.index)}
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

export default G1VirtualRows;
