import { Component, Show } from "solid-js";
import { deriveN8nLifecycle } from "../lib/n8nProgress";
import type { N8nRunState, N8nWorkflow } from "../stores/n8n";

interface Props {
  workflow: N8nWorkflow;
  latestRun?: N8nRunState;
  deadLetterCount: number;
  running: boolean;
  isSample?: boolean;
  onRun: (workflow: N8nWorkflow) => void;
  onRunNow?: (workflow: N8nWorkflow) => void;
}

function normalized(value?: string): string {
  return String(value ?? "").trim().toLowerCase();
}

function workflowName(workflow: N8nWorkflow): string {
  return workflow.display_name?.trim() || workflow.workflow_id;
}

function statusTone(status?: string): string {
  const value = normalized(status);
  if (value === "approved" || value === "completed" || value === "green") return "ok";
  if (value === "draft" || value === "test" || value === "waiting_for_approval" || value === "yellow") return "warn";
  if (value === "disabled" || value === "deprecated" || value === "failed" || value === "rejected" || value === "red") return "danger";
  return "neutral";
}

function lifecycleTone(status?: string, severity?: string): string {
  const value = normalized(status);
  const level = normalized(severity);
  if (["copy_changed", "copy_missing", "needs_review", "needs_retest", "blocked"].includes(value) || level === "blocker") return "danger";
  if (["source_changed", "safe_refresh_available", "cleanup_available", "pending_recovery"].includes(value) || level === "warning") return "warn";
  if (value === "current") return "ok";
  return "neutral";
}

function lifecycleLabel(status?: string): string {
  const value = normalized(status);
  if (!value) return "";
  if (value === "current") return "Current";
  if (value === "safe_refresh_available") return "Refresh available";
  if (value === "source_changed") return "Source changed";
  if (value === "copy_changed") return "Copy changed";
  if (value === "copy_missing") return "Missing in n8n";
  if (value === "needs_retest") return "Needs retest";
  if (value === "needs_review") return "Changed in n8n";
  if (value === "pending_recovery") return "Pending setup";
  if (value === "cleanup_available") return "Cleanup available";
  return value.replace(/_/g, " ");
}

function lifecycleBlocksRun(workflow: N8nWorkflow): boolean {
  return ["copy_changed", "copy_missing", "needs_review", "needs_retest", "blocked"].includes(
    normalized(workflow.lifecycle_status),
  );
}

function runLabel(run?: N8nRunState): string {
  if (!run) return "No runs";
  const localError = String(run.local_error ?? "").toLowerCase();
  if (
    normalized(run.status) === "rejected" &&
    localError.includes("webhook") &&
    (localError.includes("not registered for post") || localError.includes("get request"))
  ) {
    return "Webhook needs POST";
  }
  if (normalized(run.status) === "rejected" && localError.includes("webhook") && localError.includes("not registered")) {
    return "Webhook inactive";
  }
  return deriveN8nLifecycle(run).label;
}

function runMethod(workflow: N8nWorkflow): string {
  if (workflow.requires_callback) return "Callback";
  if (normalized(workflow.trigger_strategy) === "webhook") return "Webhook polling";
  if (normalized(workflow.trigger_strategy) === "form_submit") return "Form submit polling";
  if (normalized(workflow.trigger_strategy) === "chat_trigger") return "Chat trigger polling";
  if (normalized(workflow.trigger_strategy) === "manual_api_execute") {
    return `Runner${workflow.runner_backend ? `: ${workflow.runner_backend.replace(/_/g, " ")}` : ""}`;
  }
  if (normalized(workflow.result_mode) === "monitor_only") return "Monitor only";
  return workflow.trigger_strategy ? workflow.trigger_strategy.replace(/_/g, " ") : "Run method unknown";
}

const N8nWorkflowCard: Component<Props> = (props) => {
  const approved = () => normalized(props.workflow.status) === "approved";
  const monitorOnly = () => normalized(props.workflow.result_mode) === "monitor_only";
  const latestRunTone = () => deriveN8nLifecycle(props.latestRun, props.workflow).tone;
  const lifecycleBlocked = () => lifecycleBlocksRun(props.workflow);

  return (
    <article class="n8n-workflow-card">
      <div class="n8n-workflow-card-head">
        <div class="n8n-workflow-title">
          <strong>{workflowName(props.workflow)}</strong>
          <small>{props.workflow.workflow_id} · {props.workflow.workflow_version}</small>
        </div>
        <div class="n8n-workflow-badges">
          <Show when={props.isSample}>
            <span class="n8n-badge sample" title="Bundled demo workflow added by KRIA for testing">Sample</span>
          </Show>
          <Show when={normalized(props.workflow.adaptation_strategy) === "input_aware_copy"}>
            <span class="n8n-badge ok" title={`Created from ${props.workflow.adapted_from_workflow_id || "another workflow"}`}>Input-aware copy</span>
          </Show>
          <Show when={props.workflow.lifecycle_status}>
            <span
              class={`n8n-badge ${lifecycleTone(props.workflow.lifecycle_status, props.workflow.lifecycle_severity)}`}
              title={(props.workflow.lifecycle_warnings ?? []).join(" ") || "Lifecycle status"}
            >
              {lifecycleLabel(props.workflow.lifecycle_status)}
            </span>
          </Show>
          <span class={`n8n-badge ${statusTone(props.workflow.status)}`}>
            {props.workflow.status}
          </span>
        </div>
      </div>

      <div class="n8n-workflow-meta">
        <span class={`n8n-badge ${statusTone(props.workflow.risk_tier)}`}>{props.workflow.risk_tier}</span>
        <span>{props.workflow.category || "general"}</span>
        <span>{props.workflow.environment}</span>
        <span>{props.workflow.timeout_class || "background"}</span>
        <span>{runMethod(props.workflow)}</span>
      </div>

      <div class="n8n-workflow-result">
        <span class={`n8n-run-status ${latestRunTone()}`}>
          {props.latestRun ? `Last run: ${runLabel(props.latestRun)}` : runLabel(props.latestRun)}
        </span>
        <Show when={props.deadLetterCount > 0}>
          <span class="n8n-dead-letter-pill">Dead letters {props.deadLetterCount}</span>
        </Show>
      </div>

      <details class="n8n-card-details">
        <summary>Details</summary>
        <Show when={props.workflow.description}>
          <p>{props.workflow.description}</p>
        </Show>
        <Show when={(props.workflow.tags?.length ?? 0) > 0 || (props.workflow.aliases?.length ?? 0) > 0}>
          <div class="n8n-workflow-tags">
            {(props.workflow.tags ?? []).map((tag) => (
              <span>{tag}</span>
            ))}
            {(props.workflow.aliases ?? []).map((alias) => (
              <small>{alias}</small>
            ))}
          </div>
        </Show>
        <div class="n8n-card-detail-grid">
          <span>Timeout: {props.workflow.timeout_class || "background"}</span>
          <span>HITL: {props.workflow.hitl_policy || "none"}</span>
          <span>Run method: {runMethod(props.workflow)}</span>
          <Show when={normalized(props.workflow.trigger_strategy) === "manual_api_execute"}>
            <span>Runner target: {props.workflow.runner_target || "local/default"}</span>
          </Show>
          <Show when={["webhook", "form_submit", "chat_trigger"].includes(normalized(props.workflow.trigger_strategy))}>
            <span>Trigger URL: {props.workflow.webhook_path || props.workflow.endpoint_path || "not configured"}</span>
          </Show>
          <span>Evidence: {(props.workflow.expected_evidence ?? []).join(", ") || "none"}</span>
          <Show when={normalized(props.workflow.adaptation_strategy).includes("input_aware_copy")}>
            <span>Created from: {props.workflow.adapted_from_workflow_id || props.workflow.adapted_from_n8n_workflow_id}</span>
          </Show>
          <Show when={props.workflow.lifecycle_status}>
            <span>Lifecycle: {lifecycleLabel(props.workflow.lifecycle_status)}</span>
          </Show>
        </div>
      </details>

      <div class="n8n-workflow-actions">
        <Show
          when={monitorOnly()}
          fallback={
            <>
              <button
                type="button"
                class="btn-primary"
                disabled={!approved() || props.running || lifecycleBlocked()}
                title={
                  lifecycleBlocked()
                    ? "This workflow changed in n8n. Refresh/review before running."
                    : approved()
                      ? "Review workflow before running"
                      : "Only approved workflows can be used"
                }
                onClick={() => props.onRun(props.workflow)}
              >
                {props.running ? "Triggering..." : "Review"}
              </button>
              <span>{approved() ? "Confirmation required" : "Not ready"}</span>
            </>
          }
        >
          <>
            <button
              type="button"
              class="btn-primary"
              disabled={!approved() || props.running}
              title={approved() ? "Show latest and previous n8n executions without starting the workflow" : "Only approved workflows can be used"}
              onClick={() => props.onRun(props.workflow)}
            >
              {props.running ? "Checking..." : "View Executions"}
            </button>
            <button
              type="button"
              class="btn-secondary"
              disabled={!approved() || props.running || !props.onRunNow || lifecycleBlocked()}
              title={
                lifecycleBlocked()
                  ? "This workflow changed in n8n. Refresh/review before running."
                  : approved()
                    ? "Run this workflow now through the configured KRIA runner"
                    : "Only approved workflows can be used"
              }
              onClick={() => props.onRunNow?.(props.workflow)}
            >
              Run Now
            </button>
            <span>History is read-only; Run Now starts via runner</span>
          </>
        </Show>
      </div>
    </article>
  );
};

export default N8nWorkflowCard;
