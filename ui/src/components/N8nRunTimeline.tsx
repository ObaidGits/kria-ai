import { Component, For, Show } from "solid-js";
import { deriveN8nLifecycle, shortN8nId } from "../lib/n8nProgress";
import type { N8nRunState } from "../stores/n8n";

interface Props {
  runs: N8nRunState[];
  selectedCorrelationId?: string | null;
  onSelect?: (run: N8nRunState) => void;
  onReconcile?: (correlationId: string) => void;
  busy?: boolean;
}

const N8nRunTimeline: Component<Props> = (props) => {
  return (
    <div class="n8n-run-timeline">
      <Show when={props.runs.length > 0} fallback={<div class="n8n-empty">No workflow runs yet.</div>}>
        <For each={props.runs}>
          {(run) => {
            const progress = () => deriveN8nLifecycle(run);
            return (
              <button
                type="button"
                class={`n8n-run-row ${props.selectedCorrelationId === run.correlation_id ? "selected" : ""}`}
                onClick={() => props.onSelect?.(run)}
              >
                <span class={`n8n-status-dot ${progress().tone}`} aria-hidden="true" />
                <span class="n8n-run-main">
                  <strong>{run.workflow_id}</strong>
                  <small>{shortN8nId(run.correlation_id)} · elapsed {progress().elapsedLabel}</small>
                  <small>Evidence: {progress().lastEvidenceLabel}</small>
                </span>
                <span class={`n8n-run-status ${progress().tone}`}>
                  {progress().label}
                </span>
                <Show when={progress().warning}>
                  <span class="n8n-run-warning">{progress().warning}</span>
                </Show>
                <Show when={run.n8n_run_id && props.onReconcile}>
                  <span
                    class="n8n-run-reconcile"
                    onClick={(event) => {
                      event.stopPropagation();
                      props.onReconcile?.(run.correlation_id);
                    }}
                    aria-disabled={props.busy}
                  >
                    Reconcile
                  </span>
                </Show>
              </button>
            );
          }}
        </For>
      </Show>
    </div>
  );
};

export default N8nRunTimeline;
