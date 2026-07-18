/**
 * PlanVisualization — the revived plan-comparison view (Req 20.3), mounted
 * inside the plan-compare WorkBlock's `data-region="plan-visualization-slot"`.
 *
 * KRIA's planner proposes several candidate plans; this component renders them
 * side-by-side so the operator can compare their VALUE: relative risk, model
 * score/confidence, plain-language tradeoffs, ordered steps, and which one KRIA
 * recommends. It replaces the legacy `components/PlanVisualization.tsx`
 * (god-store + raw-hex, live-but-unmounted) with a modular, token-only,
 * kit-based port following the redesign conventions.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Presentation / read-model ONLY. This VISUALIZES the plans KRIA proposed. When
 * the operator picks a plan, `onSelect` fires — its default,
 * `converseStore.selectPlanOption`, stages a typed REQUEST on the event bus that
 * the Tauri bridge routes through the EXISTING approve/converse path
 * (Intent→Capability→Policy / Approval Center). It is NEVER a direct tool call
 * and NEVER shortcuts prompt→tool. All model-authored text (summary, tradeoffs,
 * selection reason, outcome reason) is UNTRUSTED and sanitized via the shared
 * `sanitizeHtml` before it reaches the DOM.
 *
 * Requirements: 20.3, 4.2, 17.3
 */
import { For, Show, createMemo } from "solid-js";
import { Icon } from "../../../components/Icon";
import { Badge, Button, Progress } from "../../../kit";
import { sanitizeHtml } from "../../../lib/markdown";
import { converseStore } from "../../../stores";
import type {
  PlanCompareOption,
  PlanCompareStep,
  WorkBlock as WorkBlockData,
  WorkBlockStatus,
} from "../../../stores/converseStore";
import type { BadgeTone } from "../../../kit";
import "./PlanVisualization.css";

// ── Risk posture → label + icon + tone. Icon + TEXT so risk is never conveyed
//    by color alone (Req 17.3). ───────────────────────────────────────────────
const RISK_META: Record<
  NonNullable<PlanCompareOption["risk"]>,
  { label: string; icon: string; tone: BadgeTone }
> = {
  safe: { label: "Diagnose first", icon: "search", tone: "success" },
  moderate: { label: "Minimal risk", icon: "shield", tone: "warning" },
  aggressive: { label: "Aggressive", icon: "zap", tone: "danger" },
};

// ── Per-step status → icon + text (Req 17.3). ────────────────────────────────
const STEP_STATUS_META: Record<WorkBlockStatus, { label: string; icon: string }> = {
  pending: { label: "Pending", icon: "clock" },
  running: { label: "Running", icon: "loader" },
  completed: { label: "Done", icon: "check" },
  failed: { label: "Failed", icon: "x" },
  stopped: { label: "Stopped", icon: "square" },
};

// ── Goal-verification outcome → label + icon (icon + text, not color alone). ──
const OUTCOME_META: Record<
  NonNullable<WorkBlockData["planOutcome"]>["outcome"],
  { label: string; icon: string; tone: BadgeTone }
> = {
  achieved: { label: "Goal achieved", icon: "check", tone: "success" },
  failed: { label: "Goal failed", icon: "x", tone: "danger" },
  continue: { label: "Continuing", icon: "loader", tone: "info" },
};

export interface PlanVisualizationProps {
  /** The plan-compare work block whose candidate plans are visualized. */
  block: WorkBlockData;
  /**
   * Plan-select handler. Defaults to `converseStore.selectPlanOption` — the
   * typed request path that routes through the existing approve/converse
   * pipeline. Overridable for stories/tests only. NEVER a tool call.
   */
  onSelect?: (blockId: string, optionId: string) => void;
}

/** A single candidate-plan card. */
function PlanCard(props: {
  option: PlanCompareOption;
  onSelect: () => void;
}) {
  const risk = createMemo(() =>
    props.option.risk ? RISK_META[props.option.risk] : null,
  );
  const scorePct = createMemo(() =>
    props.option.score != null ? Math.round(props.option.score * 100) : null,
  );
  const confidencePct = createMemo(() =>
    props.option.confidence != null ? Math.round(props.option.confidence * 100) : null,
  );

  return (
    <li
      class="kria-plan-viz__card"
      data-plan-option-id={props.option.id}
      data-recommended={props.option.recommended ? "true" : undefined}
    >
      <div class="kria-plan-viz__card-head">
        <span class="kria-plan-viz__card-label">{props.option.label}</span>
        <Show when={props.option.recommended}>
          <Badge tone="accent" class="kria-plan-viz__recommended">
            <Icon name="star" size={12} />
            <span>Recommended</span>
          </Badge>
        </Show>
      </div>

      {/* Relative risk — icon + text (Req 17.3). */}
      <Show when={risk()}>
        {(meta) => (
          <Badge tone={meta().tone} class="kria-plan-viz__risk">
            <Icon name={meta().icon} size={12} />
            <span>{meta().label}</span>
          </Badge>
        )}
      </Show>

      {/* Plain-language summary — model-authored → sanitized. */}
      <Show when={props.option.summary}>
        <div
          class="kria-plan-viz__summary"
          innerHTML={sanitizeHtml(props.option.summary!)}
        />
      </Show>

      {/* Model score + confidence bars. */}
      <Show when={scorePct() != null}>
        <Progress
          label="Model score"
          value={scorePct()!}
          minValue={0}
          maxValue={100}
        />
      </Show>
      <Show when={confidencePct() != null}>
        <Progress
          label="Confidence"
          value={confidencePct()!}
          minValue={0}
          maxValue={100}
        />
      </Show>

      {/* Tradeoffs — model-authored → sanitized. */}
      <Show when={props.option.tradeoffs}>
        <div class="kria-plan-viz__tradeoffs">
          <span class="kria-plan-viz__section-title">Tradeoffs</span>
          <div
            class="kria-plan-viz__tradeoffs-body"
            innerHTML={sanitizeHtml(props.option.tradeoffs!)}
          />
        </div>
      </Show>

      {/* Ordered steps with optional per-step status. */}
      <Show when={(props.option.steps?.length ?? 0) > 0}>
        <div class="kria-plan-viz__steps">
          <span class="kria-plan-viz__section-title">Steps</span>
          <ol class="kria-plan-viz__step-list">
            <For each={props.option.steps}>
              {(step: PlanCompareStep) => {
                const meta = step.status ? STEP_STATUS_META[step.status] : null;
                return (
                  <li class="kria-plan-viz__step" data-status={step.status}>
                    <Show when={meta}>
                      {(m) => (
                        <span class="kria-plan-viz__step-status" title={m().label}>
                          <Icon name={m().icon} size={12} />
                          <span class="kria-plan-viz__step-status-label">{m().label}</span>
                        </span>
                      )}
                    </Show>
                    <span class="kria-plan-viz__step-label">{step.label}</span>
                    {/* Command detail is escaped as text (never executed). */}
                    <Show when={step.detail}>
                      <span class="kria-plan-viz__step-detail">{step.detail}</span>
                    </Show>
                    <Show when={step.outcome}>
                      <span class="kria-plan-viz__step-outcome">{step.outcome}</span>
                    </Show>
                  </li>
                );
              }}
            </For>
          </ol>
        </div>
      </Show>

      {/* Select — routes through the existing approve/converse path, never a
          tool call (KRIA runtime-authority invariant). */}
      <Button
        variant={props.option.recommended ? "primary" : "secondary"}
        size="sm"
        class="kria-plan-viz__select"
        aria-label={`Use plan: ${props.option.label}`}
        onClick={() => props.onSelect()}
      >
        <Icon name="check" size={12} />
        <span>Use this plan</span>
      </Button>
    </li>
  );
}

export function PlanVisualization(props: PlanVisualizationProps) {
  const block = () => props.block;
  const options = () => block().planOptions ?? [];
  const select = (optionId: string) =>
    (props.onSelect ?? converseStore.selectPlanOption)(block().id, optionId);

  return (
    <div class="kria-plan-viz" data-region="plan-visualization">
      {/* Goal-verification banner — icon + text (Req 17.3). */}
      <Show when={block().planOutcome}>
        {(outcome) => {
          const meta = OUTCOME_META[outcome().outcome];
          return (
            <div class="kria-plan-viz__outcome" data-outcome={outcome().outcome}>
              <Badge tone={meta.tone}>
                <Icon name={meta.icon} size={12} />
                <span>{meta.label}</span>
              </Badge>
              <Show when={outcome().reason}>
                {/* Model-authored → sanitized. */}
                <div
                  class="kria-plan-viz__outcome-reason"
                  innerHTML={sanitizeHtml(outcome().reason!)}
                />
              </Show>
            </div>
          );
        }}
      </Show>

      {/* Why KRIA recommends the highlighted plan — model-authored → sanitized. */}
      <Show when={block().planSelectionReason}>
        <div
          class="kria-plan-viz__selection-reason"
          innerHTML={sanitizeHtml(block().planSelectionReason!)}
        />
      </Show>

      {/* Candidate plans side-by-side, or an honest empty state. */}
      <Show
        when={options().length > 0}
        fallback={<p class="kria-plan-viz__empty">No plan options yet.</p>}
      >
        <ul class="kria-plan-viz__cards">
          <For each={options()}>
            {(option) => <PlanCard option={option} onSelect={() => select(option.id)} />}
          </For>
        </ul>
      </Show>
    </div>
  );
}

export default PlanVisualization;
