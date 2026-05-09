/**
 * ExecutiveDashboard — Real-time view of the Executive Controller.
 *
 * Shows:
 * - Active foreground task (P0/P1)
 * - Active background tasks (P3/P4)
 * - Queued tasks
 * - GPU lease status
 * - Recent task completions (virtualized)
 * - Preemption events
 *
 * Uses a virtualized list for the event log so only visible rows are in the DOM.
 */

import {
  Component,
  Show,
  For,
  createSignal,
  createMemo,
  createEffect,
  onMount,
  onCleanup,
} from "solid-js";
import { appStore } from "../stores/app";
import type {
  ExecutiveTask,
  ExecutiveTaskCompleted,
  TaskPriority,
  TaskState,
} from "../types/intelligence";

// ─── Priority Badge ─────────────────────────────────────────────────────────

const PRIORITY_COLORS: Record<TaskPriority, string> = {
  Voice: "#3bc975",
  Interactive: "#18a57a",
  HitlResponse: "#f3b54a",
  Background: "#7b919f",
  Maintenance: "#4a5568",
};

const PRIORITY_LABELS: Record<TaskPriority, string> = {
  Voice: "P0",
  Interactive: "P1",
  HitlResponse: "P2",
  Background: "P3",
  Maintenance: "P4",
};

function PriorityBadge(props: { priority: TaskPriority }) {
  return (
    <span
      class="executive-priority-badge"
      style={{
        "background-color": PRIORITY_COLORS[props.priority],
        color: "#fff",
        "font-size": "10px",
        "font-weight": "700",
        padding: "2px 6px",
        "border-radius": "4px",
        "letter-spacing": "0.5px",
      }}
    >
      {PRIORITY_LABELS[props.priority]}
    </span>
  );
}

// ─── State Badge ────────────────────────────────────────────────────────────

const STATE_COLORS: Record<TaskState, string> = {
  Queued: "#7b919f",
  Running: "#18a57a",
  Completed: "#3bc975",
  Failed: "#f86d6d",
  Cancelled: "#f3b54a",
  Preempted: "#e07b39",
};

function StateBadge(props: { state: TaskState }) {
  return (
    <span
      style={{
        "font-size": "11px",
        color: STATE_COLORS[props.state],
        "font-weight": "600",
      }}
    >
      {props.state}
    </span>
  );
}

// ─── Task Row ───────────────────────────────────────────────────────────────

function TaskRow(props: { task: ExecutiveTask; onCancel?: (id: string) => void }) {
  const sourceLabel = createMemo(() => {
    const src = props.task.source;
    if (typeof src === "string") return src;
    if (src && typeof src === "object" && "CompiledSkill" in src) {
      return `Skill: ${src.CompiledSkill}`;
    }
    return String(src);
  });

  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "12px",
        padding: "8px 12px",
        "border-bottom": "1px solid var(--border)",
        "font-size": "13px",
      }}
    >
      <PriorityBadge priority={props.task.priority} />
      <span style={{ flex: "1", color: "var(--text-primary)", "font-weight": "500" }}>
        {props.task.description}
      </span>
      <span style={{ color: "var(--text-muted)", "font-size": "11px" }}>
        {sourceLabel()}
      </span>
      <StateBadge state={props.task.state} />
      <Show when={props.task.duration_ms !== null}>
        <span style={{ color: "var(--text-muted)", "font-size": "11px", "min-width": "60px", "text-align": "right" }}>
          {props.task.duration_ms! < 1000
            ? `${props.task.duration_ms}ms`
            : `${(props.task.duration_ms! / 1000).toFixed(1)}s`}
        </span>
      </Show>
      <Show when={props.task.state === "Running" && props.onCancel}>
        <button
          onClick={() => props.onCancel!(props.task.id)}
          style={{
            background: "var(--danger-soft)",
            color: "var(--danger)",
            border: "1px solid var(--danger)",
            "border-radius": "4px",
            padding: "2px 8px",
            cursor: "pointer",
            "font-size": "11px",
          }}
        >
          Cancel
        </button>
      </Show>
    </div>
  );
}

// ─── Virtualized Event Log ──────────────────────────────────────────────────

const ROW_HEIGHT = 44; // px per row
const OVERSCAN = 5; // extra rows above/below viewport

function VirtualizedEventLog(props: { events: ExecutiveTaskCompleted[] }) {
  let containerRef: HTMLDivElement | undefined;
  const [scrollTop, setScrollTop] = createSignal(0);
  const [containerHeight, setContainerHeight] = createSignal(400);

  const totalHeight = createMemo(() => props.events.length * ROW_HEIGHT);

  const visibleRange = createMemo(() => {
    const start = Math.max(0, Math.floor(scrollTop() / ROW_HEIGHT) - OVERSCAN);
    const visibleCount = Math.ceil(containerHeight() / ROW_HEIGHT) + OVERSCAN * 2;
    const end = Math.min(props.events.length, start + visibleCount);
    return { start, end };
  });

  const visibleEvents = createMemo(() =>
    props.events.slice(visibleRange().start, visibleRange().end)
  );

  function onScroll(e: Event) {
    const target = e.target as HTMLDivElement;
    setScrollTop(target.scrollTop);
  }

  onMount(() => {
    if (containerRef) {
      const ro = new ResizeObserver((entries) => {
        for (const entry of entries) {
          setContainerHeight(entry.contentRect.height);
        }
      });
      ro.observe(containerRef);
      onCleanup(() => ro.disconnect());
    }
  });

  const formatTs = (ts: string) => {
    try {
      const d = new Date(ts);
      return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
    } catch {
      return ts;
    }
  };

  return (
    <div
      ref={containerRef}
      onScroll={onScroll}
      style={{
        overflow: "auto",
        flex: "1",
        "min-height": "200px",
        "max-height": "500px",
        position: "relative",
      }}
    >
      <div style={{ height: `${totalHeight()}px`, position: "relative" }}>
        <For each={visibleEvents()}>
          {(event, i) => {
            const idx = createMemo(() => visibleRange().start + i());
            return (
              <div
                style={{
                  position: "absolute",
                  top: `${idx() * ROW_HEIGHT}px`,
                  left: "0",
                  right: "0",
                  height: `${ROW_HEIGHT}px`,
                  display: "flex",
                  "align-items": "center",
                  gap: "12px",
                  padding: "0 12px",
                  "border-bottom": "1px solid var(--border)",
                  "font-size": "12px",
                  "box-sizing": "border-box",
                }}
              >
                <span style={{ color: "var(--text-muted)", "font-size": "11px", "min-width": "70px" }}>
                  {formatTs(event.ts)}
                </span>
                <span
                  style={{
                    color: event.success ? "var(--success)" : "var(--danger)",
                    "font-weight": "600",
                    "min-width": "20px",
                  }}
                >
                  {event.success ? "✓" : "✗"}
                </span>
                <span style={{ flex: "1", color: "var(--text-primary)", overflow: "hidden", "text-overflow": "ellipsis", "white-space": "nowrap" }}>
                  {event.output_summary || event.error || event.task_id.slice(0, 8)}
                </span>
                <span style={{ color: "var(--text-muted)", "font-size": "11px", "min-width": "50px", "text-align": "right" }}>
                  {event.duration_ms < 1000
                    ? `${event.duration_ms}ms`
                    : `${(event.duration_ms / 1000).toFixed(1)}s`}
                </span>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}

// ─── GPU Lease Indicator ────────────────────────────────────────────────────

function GpuLeaseIndicator() {
  const snapshot = createMemo(() => appStore.executiveSnapshot());
  const holder = createMemo(() => snapshot()?.gpu_lease_holder ?? null);
  const remaining = createMemo(() => snapshot()?.gpu_lease_remaining_ms ?? null);

  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "8px",
        padding: "8px 12px",
        background: holder() ? "var(--accent-soft)" : "var(--surface-1)",
        "border-radius": "var(--radius-sm)",
        border: holder() ? "1px solid var(--accent-border)" : "1px solid var(--border)",
      }}
    >
      <div
        style={{
          width: "8px",
          height: "8px",
          "border-radius": "50%",
          "background-color": holder() ? "var(--accent)" : "var(--text-muted)",
        }}
      />
      <span style={{ "font-size": "13px", "font-weight": "600", color: "var(--text-primary)" }}>
        GPU Lease
      </span>
      <Show
        when={holder()}
        fallback={
          <span style={{ "font-size": "12px", color: "var(--text-muted)" }}>Free</span>
        }
      >
        <span style={{ "font-size": "12px", color: "var(--accent)" }}>
          Held by {holder()!.slice(0, 8)}
        </span>
        <Show when={remaining()}>
          <span style={{ "font-size": "11px", color: "var(--text-muted)" }}>
            ({(remaining()! / 1000).toFixed(0)}s remaining)
          </span>
        </Show>
      </Show>
    </div>
  );
}

// ─── Stats Bar ──────────────────────────────────────────────────────────────

function StatsBar() {
  const snapshot = createMemo(() => appStore.executiveSnapshot());

  return (
    <div style={{ display: "flex", gap: "16px", "font-size": "12px", color: "var(--text-secondary)" }}>
      <div>
        <span style={{ "font-weight": "700", color: "var(--text-primary)" }}>
          {snapshot()?.active_background.length ?? 0}
        </span>{" "}
        active
      </div>
      <div>
        <span style={{ "font-weight": "700", color: "var(--text-primary)" }}>
          {snapshot()?.queued.length ?? 0}
        </span>{" "}
        queued
      </div>
      <div>
        <span style={{ "font-weight": "700", color: "var(--success)" }}>
          {snapshot()?.total_completed ?? 0}
        </span>{" "}
        completed
      </div>
      <div>
        <span style={{ "font-weight": "700", color: "var(--danger)" }}>
          {snapshot()?.total_failed ?? 0}
        </span>{" "}
        failed
      </div>
    </div>
  );
}

// ─── Main Component ─────────────────────────────────────────────────────────

const ExecutiveDashboard: Component = () => {
  const snapshot = createMemo(() => appStore.executiveSnapshot());
  const recentEvents = createMemo(() => appStore.executiveRecentEvents());

  onMount(() => {
    // Refresh on mount.
    void appStore.loadExecutiveSnapshot();
  });

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        height: "100%",
        gap: "12px",
        padding: "16px",
        "box-sizing": "border-box",
      }}
    >
      {/* Header */}
      <div style={{ display: "flex", "justify-content": "space-between", "align-items": "center" }}>
        <h2 style={{ margin: "0", "font-size": "18px", color: "var(--text-primary)" }}>
          Executive Controller
        </h2>
        <StatsBar />
      </div>

      {/* GPU Lease */}
      <GpuLeaseIndicator />

      {/* Foreground Task */}
      <Show when={snapshot()?.active_foreground}>
        <div>
          <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
            FOREGROUND TASK
          </div>
          <TaskRow task={snapshot()!.active_foreground!} />
        </div>
      </Show>

      {/* Background Tasks */}
      <Show when={snapshot() && snapshot()!.active_background.length > 0}>
        <div>
          <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
            BACKGROUND TASKS ({snapshot()!.active_background.length})
          </div>
          <div style={{ "max-height": "200px", overflow: "auto" }}>
            <For each={snapshot()!.active_background}>
              {(task) => (
                <TaskRow
                  task={task}
                  onCancel={(id) => void appStore.cancelExecutiveTask(id)}
                />
              )}
            </For>
          </div>
        </div>
      </Show>

      {/* Queued Tasks */}
      <Show when={snapshot() && snapshot()!.queued.length > 0}>
        <div>
          <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
            QUEUE ({snapshot()!.queued.length})
          </div>
          <div style={{ "max-height": "150px", overflow: "auto" }}>
            <For each={snapshot()!.queued}>
              {(task) => <TaskRow task={task} />}
            </For>
          </div>
        </div>
      </Show>

      {/* Virtualized Event Log */}
      <div style={{ flex: "1", display: "flex", "flex-direction": "column", "min-height": "0" }}>
        <div style={{ "font-size": "12px", color: "var(--text-muted)", "margin-bottom": "4px" }}>
          RECENT EVENTS ({recentEvents().length})
        </div>
        <VirtualizedEventLog events={recentEvents()} />
      </div>
    </div>
  );
};

export default ExecutiveDashboard;
