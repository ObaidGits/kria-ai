import { Component, Show, createEffect, createSignal, onCleanup } from "solid-js";
import {
  deriveN8nLifecycle,
  n8nGovernanceLabel,
  shortN8nId,
  type N8nProgressView,
} from "../lib/n8nProgress";
import type { N8nGovernanceDecision, N8nRunState, N8nWorkflow } from "../stores/n8n";

interface Props {
  run?: N8nRunState;
  workflow?: N8nWorkflow;
  governance?: N8nGovernanceDecision;
  compact?: boolean;
}

const N8nRunProgress: Component<Props> = (props) => {
  const [nowMs, setNowMs] = createSignal(Date.now());
  const progress = (): N8nProgressView => deriveN8nLifecycle(props.run, props.workflow, props.governance, nowMs());

  createEffect(() => {
    if (!props.run || props.run.terminal) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    onCleanup(() => window.clearInterval(timer));
  });

  return (
    <div class={`n8n-progress-card ${props.compact ? "compact" : ""} ${progress().tone}`}>
      <div class="n8n-progress-head">
        <span class={`n8n-status-dot ${progress().tone}`} aria-hidden="true" />
        <div>
          <strong>{progress().label}</strong>
          <small>{props.workflow?.display_name || props.run?.workflow_id || "Workflow"}</small>
        </div>
        <span class={`n8n-run-status ${progress().tone}`}>{progress().lifecycle}</span>
      </div>

      <div class="n8n-progress-facts">
        <div>
          <span>Correlation</span>
          <strong title={props.run?.correlation_id}>{progress().correlationLabel}</strong>
        </div>
        <div>
          <span>Elapsed</span>
          <strong>{progress().elapsedLabel}</strong>
        </div>
        <div>
          <span>Evidence</span>
          <strong>{progress().lastEvidenceLabel}</strong>
        </div>
        <Show when={props.run?.n8n_run_id}>
          <div>
            <span>n8n Run</span>
            <strong title={props.run?.n8n_run_id}>{shortN8nId(props.run?.n8n_run_id)}</strong>
          </div>
        </Show>
      </div>

      <Show when={props.governance}>
        <div class={`n8n-progress-governance ${progress().tone}`}>
          <span>{n8nGovernanceLabel(props.governance)}</span>
          <small>{props.governance?.explanation || "Governance decision pending final evidence."}</small>
        </div>
      </Show>

      <Show when={progress().warning}>
        <div class="n8n-progress-warning">{progress().warning}</div>
      </Show>
      <Show when={progress().finalSummary}>
        <div class="n8n-progress-result">{progress().finalSummary}</div>
      </Show>
      <Show when={progress().recoveryHint}>
        <div class="n8n-progress-hint">{progress().recoveryHint}</div>
      </Show>
    </div>
  );
};

export default N8nRunProgress;
