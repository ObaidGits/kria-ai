import { Component, For, Show } from "solid-js";
import { n8nGovernanceLabel, summarizeN8nEvidence } from "../lib/n8nProgress";
import type { N8nGovernanceDecision, N8nRunState } from "../stores/n8n";

interface Props {
  run?: N8nRunState;
  governance?: N8nGovernanceDecision;
}

function governanceTone(governance?: N8nGovernanceDecision): string {
  const action = String(governance?.continuation_action ?? "").toLowerCase();
  const verification = String(governance?.verification_status ?? "").toLowerCase();
  if (verification === "verified" && action === "continue_workflow") return "ok";
  if (verification === "failed" || action === "recover_workflow") return "danger";
  if (verification === "needs_more_evidence" || action === "pause_for_hitl") return "warn";
  return "neutral";
}

function latestEvidence(run?: N8nRunState): any | undefined {
  return run?.evidence_log?.[run.evidence_log.length - 1];
}

function scalarValue(value: any): string | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  return undefined;
}

function outputObject(evidence: any): Record<string, any> | undefined {
  const output = evidence?.output;
  if (!output) return undefined;
  if (Array.isArray(output)) {
    const first = output[0];
    return first && typeof first === "object" && !Array.isArray(first) ? first : undefined;
  }
  return typeof output === "object" && !Array.isArray(output) ? output : undefined;
}

function outputRows(evidence: any): { key: string; value: string }[] {
  const output = outputObject(evidence);
  if (!output) return [];
  const priority = [
    "Title",
    "Year",
    "Genre",
    "Director",
    "Actors",
    "Plot",
    "imdbRating",
    "Runtime",
    "Released",
    "Type",
  ];
  const keys = [
    ...priority.filter((key) => Object.prototype.hasOwnProperty.call(output, key)),
    ...Object.keys(output).filter((key) => !priority.includes(key)),
  ];
  return keys
    .map((key) => ({ key, value: scalarValue(output[key]) }))
    .filter((row): row is { key: string; value: string } => Boolean(row.value))
    .slice(0, 10);
}

const N8nEvidenceViewer: Component<Props> = (props) => {
  const evidence = () => latestEvidence(props.run);
  const rows = () => outputRows(evidence());

  return (
    <div class="n8n-evidence-viewer">
      <div class="n8n-evidence-summary">
        <span>{summarizeN8nEvidence(props.run)}</span>
      </div>

      <Show when={rows().length > 0}>
        <div class="n8n-output-preview">
          <div class="n8n-output-preview-head">
            <strong>Workflow output</strong>
            <span>{evidence()?.output_source || "output node"}</span>
          </div>
          <dl>
            <For each={rows()}>
              {(row) => (
                <>
                  <dt>{row.key}</dt>
                  <dd>{row.value}</dd>
                </>
              )}
            </For>
          </dl>
        </div>
      </Show>

      <Show when={props.governance}>
        {(governance) => (
          <div class={`n8n-governance-line ${governanceTone(governance())}`}>
            <strong>{n8nGovernanceLabel(governance())}</strong>
            <span>{governance().continuation_action}</span>
            <small>{governance().explanation || "No governance explanation recorded."}</small>
            <Show when={(governance().missing_evidence ?? []).length > 0}>
              <small>Missing evidence: {(governance().missing_evidence ?? []).join(", ")}</small>
            </Show>
          </div>
        )}
      </Show>

      <details class="n8n-technical-details">
        <summary>Technical details</summary>
        <pre>{JSON.stringify({ run: props.run ?? null, governance: props.governance ?? null }, null, 2)}</pre>
      </details>
    </div>
  );
};

export default N8nEvidenceViewer;
