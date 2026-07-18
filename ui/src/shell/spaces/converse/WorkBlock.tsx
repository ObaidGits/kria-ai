/**
 * WorkBlock — a single typed unit of KRIA's visible work in the WorkLane
 * (Req 4.2, design.md §6.1 "WorkBlock(reason/tool/plan/gui/run)").
 *
 * Five typed variants — reasoning · tool-call · plan-compare · gui-cognition ·
 * workflow-run — each rendering:
 *   • status  — pending/running/completed/failed/stopped, conveyed by ICON +
 *     TEXT (never color alone — Req 17.3).
 *   • summary — plain-language, ALWAYS visible (Req 4.2).
 *   • details — a keyboard-operable disclosure (button + aria-expanded), with
 *     variant-specific content (reasoning trace / tool args+result / plan
 *     options / gui steps / run log).
 *   • evidence — the sources/artifacts KRIA used or produced.
 *   • an INDEPENDENT Stop — per-block, shown ONLY while running.
 *
 * Visually SECONDARY to the conversation (Req 4.3): caption type scale + muted
 * surface (see WorkBlock.css and the WorkLane container).
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Presentation + cancel-DISPATCH only. Stop routes through
 * `converseStore.cancelWorkBlock(id)`, which stages a typed per-block cancel
 * REQUEST on the event bus (consumed by the Tauri bridge → existing cancel
 * command). It is NEVER a direct tool call, NEVER a global stop, and there is no
 * orchestration or loop here. All untrusted text (tool args/result, evidence,
 * reasoning, details) is sanitized via the shared markdown sanitizer before it
 * reaches the DOM.
 *
 * Requirements: 4.2, 4.3, 17.3
 */
import { createSignal, createUniqueId, For, Show, type JSX } from "solid-js";
import { Icon } from "../../../components/Icon";
import { Badge, Button, Progress, ProvenanceCue } from "../../../kit";
import { renderMarkdown, sanitizeHtml } from "../../../lib/markdown";
import { PlanVisualization } from "./PlanVisualization";
import { converseStore } from "../../../stores";
import type {
  WorkBlock as WorkBlockData,
  WorkBlockStatus,
  WorkBlockType,
} from "../../../stores/converseStore";
import type { BadgeTone } from "../../../kit";
import "./WorkBlock.css";

// ── Type → label + icon (design.md §6.1) ────────────────────────────────────
const TYPE_META: Record<WorkBlockType, { label: string; icon: string }> = {
  reasoning: { label: "Reasoning", icon: "brain" },
  "tool-call": { label: "Tool call", icon: "terminal" },
  "plan-compare": { label: "Plan", icon: "git-branch" },
  "gui-cognition": { label: "GUI cognition", icon: "eye" },
  "workflow-run": { label: "Workflow run", icon: "workflow" },
};

// ── Status → label + icon + tone. Icon + text so status is never color-only
//    (Req 17.3). ──────────────────────────────────────────────────────────
const STATUS_META: Record<
  WorkBlockStatus,
  { label: string; icon: string; tone: BadgeTone }
> = {
  pending: { label: "Pending", icon: "clock", tone: "neutral" },
  running: { label: "Running", icon: "loader", tone: "info" },
  completed: { label: "Completed", icon: "check", tone: "success" },
  failed: { label: "Failed", icon: "x", tone: "danger" },
  stopped: { label: "Stopped", icon: "square", tone: "warning" },
};

export interface WorkBlockProps {
  block: WorkBlockData;
  /**
   * Independent Stop handler. Defaults to `converseStore.cancelWorkBlock` (the
   * typed per-block cancel path). Overridable for stories/tests only.
   */
  onStop?: (blockId: string) => void;
}

export function WorkBlock(props: WorkBlockProps) {
  const [open, setOpen] = createSignal(false);
  const detailsId = createUniqueId();

  const block = () => props.block;
  const typeMeta = () => TYPE_META[block().type];
  const statusMeta = () => STATUS_META[block().status];
  const isRunning = () => block().status === "running";

  const hasEvidence = () => (block().evidence?.length ?? 0) > 0;
  const hasDetails = () =>
    Boolean(
      block().details ||
        block().reasoning ||
        block().toolCall ||
        (block().planOptions?.length ?? 0) > 0 ||
        (block().guiSteps?.length ?? 0) > 0 ||
        block().workflowRun,
    );

  const stop = () => (props.onStop ?? converseStore.cancelWorkBlock)(block().id);

  return (
    <section
      class="kria-work-block"
      data-work-block-id={block().id}
      data-work-type={block().type}
      data-work-status={block().status}
      data-provenance="kria"
      role="group"
      aria-label={`${typeMeta().label}: ${block().summary}`}
    >
      {/* ── Header: type · status · independent Stop ──────────────────────── */}
      <header class="kria-work-block__header">
        <ProvenanceCue source="kria" label="KRIA action" />
        <span class="kria-work-block__type">
          <Icon name={typeMeta().icon} size={14} />
          <span>{typeMeta().label}</span>
        </span>

        {/* Status — icon + text (Req 17.3), never color alone. */}
        <Badge tone={statusMeta().tone} class="kria-work-block__status">
          <Icon name={statusMeta().icon} size={12} />
          <span>{statusMeta().label}</span>
        </Badge>

        {/* Independent Stop — only while running (Req 4.2). Cancels THIS block
            via the typed per-block cancel path (never a global stop). */}
        <Show when={isRunning()}>
          <Button
            variant="danger"
            size="sm"
            class="kria-work-block__stop"
            aria-label={`Stop ${typeMeta().label.toLowerCase()}`}
            onClick={stop}
          >
            <Icon name="square" size={12} />
            <span>Stop</span>
          </Button>
        </Show>
      </header>

      {/* ── Summary — always visible, plain language (Req 4.2) ────────────── */}
      <p class="kria-work-block__summary">{block().summary}</p>

      {/* ── Details disclosure — keyboard-operable (Req 17.1) ─────────────── */}
      <Show when={hasDetails()}>
        <button
          type="button"
          class="kria-work-block__disclosure"
          aria-expanded={open()}
          aria-controls={detailsId}
          onClick={() => setOpen((v) => !v)}
        >
          <Icon
            name="chevron-right"
            size={14}
            class="kria-work-block__disclosure-caret"
            data-open={open() ? "true" : "false"}
          />
          <span>{open() ? "Hide details" : "Show details"}</span>
        </button>

        <Show when={open()}>
          <div id={detailsId} class="kria-work-block__details" data-region="work-details">
            <WorkBlockDetails block={block()} />
          </div>
        </Show>
      </Show>

      {/* ── Evidence — sources/artifacts KRIA used (Req 4.2) ──────────────── */}
      <Show when={hasEvidence()}>
        <section
          class="kria-work-block__evidence"
          data-region="work-evidence"
          aria-label="Evidence"
        >
          <h3 class="kria-work-block__evidence-title">Evidence</h3>
          <ul class="kria-work-block__evidence-list">
            <For each={block().evidence}>
              {(item) => (
                <li class="kria-work-block__evidence-item">
                  <Show
                    when={item.href}
                    fallback={<span class="kria-work-block__evidence-label">{item.label}</span>}
                  >
                    <a
                      class="kria-work-block__evidence-label"
                      href={item.href}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      {item.label}
                    </a>
                  </Show>
                  <Show when={item.detail}>
                    {/* Untrusted → sanitized before display. */}
                    <div
                      class="kria-work-block__evidence-detail"
                      innerHTML={sanitizeHtml(item.detail!)}
                    />
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
    </section>
  );
}

/**
 * Variant-specific details body. Each of the 5 typed variants renders its own
 * disclosure content; a generic `details` string is always appended when set.
 */
function WorkBlockDetails(props: { block: WorkBlockData }): JSX.Element {
  const b = () => props.block;
  return (
    <>
      {/* reasoning — the reasoning trace (sanitized markdown). */}
      <Show when={b().type === "reasoning" && b().reasoning}>
        <div class="kria-work-block__reasoning" innerHTML={renderMarkdown(b().reasoning!)} />
      </Show>

      {/* tool-call — invocation args + result. */}
      <Show when={b().type === "tool-call" && b().toolCall}>
        <div class="kria-work-block__tool">
          <div class="kria-work-block__tool-name">
            <Icon name="terminal" size={12} />
            <span>{b().toolCall!.name}</span>
          </div>
          <Show when={b().toolCall!.args}>
            <div class="kria-work-block__tool-field">
              <span class="kria-work-block__tool-label">Arguments</span>
              {/* Args are escaped as text (never executed). */}
              <pre class="kria-work-block__tool-args">{b().toolCall!.args}</pre>
            </div>
          </Show>
          <Show when={b().toolCall!.result}>
            <div class="kria-work-block__tool-field">
              <span class="kria-work-block__tool-label">Result</span>
              {/* Untrusted tool result → sanitized before display. */}
              <div
                class="kria-work-block__tool-result"
                innerHTML={sanitizeHtml(b().toolCall!.result!)}
              />
            </div>
          </Show>
        </div>
      </Show>

      {/* plan-compare — the revived PlanVisualization (task 3.7) mounts into
          this slot, rendering the candidate plans side-by-side (steps /
          tradeoffs / recommended). Plan-select routes through the existing
          approve/converse path via converseStore.selectPlanOption, never a
          tool call. */}
      <Show when={b().type === "plan-compare"}>
        <div class="kria-work-block__plan" data-region="plan-visualization-slot">
          <PlanVisualization block={b()} />
        </div>
      </Show>

      {/* gui-cognition — observed/acted steps with per-step status. */}
      <Show when={b().type === "gui-cognition" && (b().guiSteps?.length ?? 0) > 0}>
        <ol class="kria-work-block__gui-steps">
          <For each={b().guiSteps}>
            {(step) => {
              const meta = STATUS_META[step.status];
              return (
                <li class="kria-work-block__gui-step" data-status={step.status}>
                  <Icon name={meta.icon} size={12} />
                  <span class="kria-work-block__gui-step-label">{step.label}</span>
                  <span class="kria-work-block__gui-step-status">{meta.label}</span>
                </li>
              );
            }}
          </For>
        </ol>
      </Show>

      {/* workflow-run — progress + log. */}
      <Show when={b().type === "workflow-run" && b().workflowRun}>
        <div class="kria-work-block__run">
          <Show when={b().workflowRun!.progress != null}>
            <Progress
              label={runProgressLabel(b().workflowRun!)}
              value={Math.round((b().workflowRun!.progress ?? 0) * 100)}
              minValue={0}
              maxValue={100}
            />
          </Show>
          <Show when={(b().workflowRun!.log?.length ?? 0) > 0}>
            <ul class="kria-work-block__run-log">
              <For each={b().workflowRun!.log}>
                {(line) => <li class="kria-work-block__run-log-line">{line}</li>}
              </For>
            </ul>
          </Show>
        </div>
      </Show>

      {/* Generic details text — always appended when present. */}
      <Show when={b().details}>
        <div class="kria-work-block__details-text" innerHTML={renderMarkdown(b().details!)} />
      </Show>
    </>
  );
}

function runProgressLabel(run: { completed?: number; total?: number }): string {
  if (run.completed != null && run.total != null) {
    return `Step ${run.completed} of ${run.total}`;
  }
  return "Progress";
}

export default WorkBlock;
