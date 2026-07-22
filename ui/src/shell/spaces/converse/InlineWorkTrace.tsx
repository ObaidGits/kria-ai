/**
 * InlineWorkTrace — KRIA's per-turn activity trace, rendered inline in the
 * conversation right after the user message that started the turn (ChatGPT
 * "Thinking" / Gemini style). This replaces the standalone right-hand Work lane
 * so the conversation reclaims the full width while the work stays legible and
 * attached to its turn.
 *
 * Behavior (design intent):
 *   • While the turn runs (any block pending/running, or the turn is the active
 *     thinking turn): the trace is auto-EXPANDED and live-updating.
 *   • On success (all blocks terminal, none failed/stopped): it auto-COLLAPSES
 *     to a one-line summary — "Worked 3.2s · 2 tools · 1 plan". One click
 *     re-expands.
 *   • On failure / stopped: it stays EXPANDED so a blocker is never buried.
 *   • The user's explicit toggle always wins over the auto behavior.
 *
 * Runtime-authority invariant: presentation only. It reuses {@link WorkBlock},
 * whose per-block Stop dispatches the existing typed cancel request. It never
 * calls a tool, never drives a loop.
 *
 * Accessibility: a labeled, collapsible region. The summary control carries
 * aria-expanded/aria-controls; while running the body is a polite live region
 * so assistive tech hears progress without a focus steal.
 */
import { createMemo, createSignal, For, Show } from "solid-js";
import { Icon } from "../../../components/Icon";
import { converseStore } from "../../../stores";
import type { WorkBlock as WorkBlockData } from "../../../stores/converseStore";
import { WorkBlock } from "./WorkBlock";
import "./InlineWorkTrace.css";

export interface InlineWorkTraceProps {
  /** The turn (user-message id) whose work blocks this trace renders. */
  turnId: string;
}

const TYPE_NOUN: Record<WorkBlockData["type"], [string, string]> = {
  reasoning: ["reasoning step", "reasoning steps"],
  "tool-call": ["tool", "tools"],
  "plan-compare": ["plan", "plans"],
  "gui-cognition": ["GUI step", "GUI steps"],
  "workflow-run": ["workflow", "workflows"],
};

function pluralize(count: number, [one, many]: [string, string]): string {
  return `${count} ${count === 1 ? one : many}`;
}

export function InlineWorkTrace(props: InlineWorkTraceProps) {
  // Explicit user override of the auto expand/collapse behavior. `null` = follow
  // the derived default; a boolean = the user's sticky choice for this turn.
  const [override, setOverride] = createSignal<boolean | null>(null);
  const bodyId = `work-trace-${props.turnId}`;

  const blocks = createMemo(() => converseStore.workBlocksForTurn(props.turnId));
  const hasBlocks = createMemo(() => blocks().length > 0);

  const active = createMemo(() =>
    blocks().some((block) => block.status === "running" || block.status === "pending"),
  );
  const attention = createMemo(() =>
    blocks().some((block) => block.status === "failed" || block.status === "stopped"),
  );
  const isActiveTurn = createMemo(
    () => converseStore.thinking() && converseStore.currentTurnId() === props.turnId,
  );

  // Auto: expanded while the turn is doing work or needs attention; collapsed
  // once it finished cleanly. The explicit user toggle overrides this.
  const autoOpen = createMemo(() => active() || isActiveTurn() || attention());
  const open = createMemo(() => override() ?? autoOpen());

  const durationMs = createMemo(() => {
    const list = blocks();
    if (list.length === 0) return 0;
    const start = Math.min(...list.map((block) => block.startedAt));
    const end = active()
      ? Date.now()
      : Math.max(...list.map((block) => block.completedAt ?? block.startedAt));
    return Math.max(0, end - start);
  });

  const summaryLabel = createMemo(() => {
    const list = blocks();
    if (list.length === 0) return "No activity";
    const counts = new Map<WorkBlockData["type"], number>();
    for (const block of list) counts.set(block.type, (counts.get(block.type) ?? 0) + 1);
    const parts = [...counts.entries()].map(([type, count]) => pluralize(count, TYPE_NOUN[type]));
    const verb = active() ? "Working" : attention() ? "Finished with issues" : "Worked";
    const secs = (durationMs() / 1000).toFixed(1);
    const timing = active() ? "" : ` ${secs}s`;
    return `${verb}${timing} · ${parts.join(" · ")}`;
  });

  return (
    <Show when={hasBlocks()}>
      <section
        class="kria-inline-work"
        data-region="inline-work-trace"
        data-turn-id={props.turnId}
        data-open={open() ? "true" : "false"}
        data-active={active() ? "true" : "false"}
        data-attention={attention() ? "true" : "false"}
        aria-label="KRIA activity for this turn"
      >
        <button
          type="button"
          class="kria-inline-work__summary"
          aria-expanded={open()}
          aria-controls={bodyId}
          onClick={() => setOverride(!open())}
        >
          <Icon name={open() ? "chevron-down" : "chevron-right"} size={14} aria-hidden />
          <Show when={active()}>
            <Icon name="loader" size={13} class="kria-inline-work__spinner" aria-hidden />
          </Show>
          <span class="kria-inline-work__summary-text">{summaryLabel()}</span>
        </button>

        <Show when={open()}>
          <div
            id={bodyId}
            class="kria-inline-work__body"
            role="group"
            aria-label="Activity detail"
            aria-live={active() ? "polite" : "off"}
          >
            <For each={blocks()}>{(block) => <WorkBlock block={block} />}</For>
          </div>
        </Show>
      </section>
    </Show>
  );
}

export default InlineWorkTrace;
