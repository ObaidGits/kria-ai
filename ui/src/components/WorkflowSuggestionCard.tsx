import { Component, For, Show } from "solid-js";
import type { WorkflowCandidate, WorkflowSuggestionResponse } from "../stores/n8n";

interface Props {
  suggestion: WorkflowSuggestionResponse;
  busy?: boolean;
  onConfirm: (candidate: WorkflowCandidate) => void;
  onCancel: () => void;
}

function badgeTone(candidate: WorkflowCandidate): string {
  if (candidate.confidence >= 0.9) return "ok";
  if (candidate.confidence >= 0.7) return "warn";
  return "neutral";
}

function riskTone(risk?: string): string {
  const value = String(risk ?? "").toLowerCase();
  if (value === "green") return "ok";
  if (value === "yellow") return "warn";
  if (value === "red" || value === "black") return "danger";
  return "neutral";
}

const WorkflowSuggestionCard: Component<Props> = (props) => {
  return (
    <section class="n8n-suggestion-card">
      <div class="n8n-suggestion-head">
        <div>
          <span class="n8n-hub-eyebrow">Workflow Routing</span>
          <h4>{props.suggestion.message}</h4>
          <small>{props.suggestion.prompt}</small>
        </div>
        <div class="n8n-suggestion-flags">
          <span class={`n8n-badge ${props.suggestion.hard_prompt ? "warn" : "neutral"}`}>
            {props.suggestion.hard_prompt ? "confirmation required" : "bounded"}
          </span>
          <span class="n8n-badge danger">no auto-run</span>
        </div>
      </div>

      <Show
        when={props.suggestion.candidates.length > 0}
        fallback={<div class="n8n-empty">No approved workflow candidates matched this prompt.</div>}
      >
        <div class="n8n-candidate-list">
          <For each={props.suggestion.candidates}>
            {(candidate) => (
              <article class="n8n-candidate-card">
                <div class="n8n-candidate-main">
                  <strong>{candidate.display_name}</strong>
                  <small>{candidate.workflow_id} · {candidate.category || "general"}</small>
                  <p>{candidate.reason || "Matched approved workflow metadata."}</p>
                  <div class="n8n-workflow-tags">
                    {candidate.matched_on.slice(0, 5).map((source) => (
                      <span>{source}</span>
                    ))}
                  </div>
                  <Show when={(candidate.missing_inputs?.length ?? 0) > 0}>
                    <p class="n8n-candidate-warning">
                      Missing input: {candidate.missing_inputs?.slice(0, 3).join(", ")}
                    </p>
                  </Show>
                </div>
                <div class="n8n-candidate-side">
                  <span class={`n8n-badge ${badgeTone(candidate)}`}>
                    {Math.round(candidate.confidence * 100)}%
                  </span>
                  <span class={`n8n-badge ${riskTone(candidate.risk_tier)}`}>
                    {candidate.risk_tier}
                  </span>
                  <button
                    type="button"
                    class="btn-primary"
                    disabled={props.busy || (candidate.missing_inputs?.length ?? 0) > 0}
                    onClick={() => props.onConfirm(candidate)}
                  >
                    Confirm
                  </button>
                </div>
              </article>
            )}
          </For>
        </div>
      </Show>

      <div class="n8n-suggestion-actions">
        <button type="button" class="btn-secondary" disabled={props.busy} onClick={props.onCancel}>
          Cancel
        </button>
      </div>
    </section>
  );
};

export default WorkflowSuggestionCard;
