import { Component, For, Show } from "solid-js";
import type { WorkflowCandidate, WorkflowSuggestionResponse } from "../stores/n8n";

interface Props {
  suggestion: WorkflowSuggestionResponse;
  busy?: boolean;
  onConfirm: (candidate: WorkflowCandidate) => void;
  onCreateDraft?: () => void;
  onCreateUpdatedCopy?: (candidate?: WorkflowCandidate) => void;
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
  const authoringStatus = () => ["create_workflow", "create_from_template", "update_workflow"].includes(props.suggestion.status);
  const isUpdate = () => props.suggestion.status === "update_workflow";
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
          <span class={`n8n-badge ${props.suggestion.can_auto_run ? "ok" : "danger"}`}>
            {props.suggestion.can_auto_run ? "safe auto-run eligible" : "no silent auto-run"}
          </span>
        </div>
      </div>

      <Show when={authoringStatus()}>
        <div class="n8n-authoring-card">
          <strong>{isUpdate() ? "Create updated draft copy" : "Create inactive n8n draft"}</strong>
          <p>
            {isUpdate()
              ? "KRIA will keep the original workflow unchanged and create an updated draft copy for review."
              : "KRIA will create an inactive n8n draft first. Test and approval are required before normal routing."}
          </p>
          <div class="n8n-suggestion-actions">
            <button
              type="button"
              class="btn-primary"
              disabled={props.busy}
              onClick={() => {
                if (isUpdate()) props.onCreateUpdatedCopy?.(props.suggestion.candidates[0]);
                else props.onCreateDraft?.();
              }}
            >
              {isUpdate() ? "Create updated copy" : "Create draft"}
            </button>
          </div>
        </div>
      </Show>

      <Show
        when={!authoringStatus() && props.suggestion.candidates.length > 0}
        fallback={!authoringStatus() ? <div class="n8n-empty">No approved workflow candidates matched this prompt.</div> : null}
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
                  <Show when={(candidate.blockers?.length ?? 0) > 0}>
                    <p class="n8n-candidate-warning">
                      Blocked: {candidate.blockers?.slice(0, 2).join("; ")}
                    </p>
                  </Show>
                  <Show when={(candidate.next_actions?.length ?? 0) > 0}>
                    <p class="n8n-candidate-next">
                      Next: {candidate.next_actions?.slice(0, 2).join(" · ")}
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
                    disabled={props.busy || (candidate.missing_inputs?.length ?? 0) > 0 || (candidate.blockers?.length ?? 0) > 0}
                    onClick={() => props.onConfirm(candidate)}
                  >
                    {props.suggestion.can_auto_run && !candidate.requires_confirmation ? "Run" : "Review first"}
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
