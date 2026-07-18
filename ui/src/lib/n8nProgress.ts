import type { N8nGovernanceDecision, N8nRunState, N8nWorkflow } from "../stores/n8n";

export type N8nLifecycleStatus =
  | "idle"
  | "triggering"
  | "accepted"
  | "calling_n8n"
  | "finding_execution"
  | "polling_execution"
  | "monitoring_execution"
  | "extracting_output"
  | "waiting_for_callback"
  | "completed"
  | "partial"
  | "failed"
  | "cancelled"
  | "timed_out"
  | "rejected"
  | "needs_review";

export interface N8nProgressView {
  lifecycle: N8nLifecycleStatus;
  label: string;
  tone: "neutral" | "waiting" | "ok" | "warn" | "danger";
  correlationLabel: string;
  elapsedLabel: string;
  lastEvidenceLabel: string;
  warning?: string;
  recoveryHint?: string;
  finalSummary?: string;
}

const TERMINAL_STATUSES = new Set(["completed", "partial", "failed", "cancelled", "timed_out", "rejected"]);

export function normalizeN8nValue(value?: string): string {
  return String(value ?? "").trim().toLowerCase();
}

export function shortN8nId(id?: string): string {
  if (!id) return "pending";
  return id.length > 14 ? `${id.slice(0, 8)}...${id.slice(-4)}` : id;
}

export function latestN8nEvidence(run?: N8nRunState): any | undefined {
  return run?.evidence_log?.[run.evidence_log.length - 1];
}

export function latestN8nEvidenceAtMs(run?: N8nRunState): number {
  const latest = latestN8nEvidence(run);
  return Number(latest?.occurred_at_ms ?? latest?.timestamp_ms ?? latest?.issued_at_ms ?? 0) || 0;
}

export function n8nRunStartedAtMs(run?: N8nRunState): number {
  if (!run) return 0;
  return Number(run.triggered_at_ms ?? 0) || latestN8nEvidenceAtMs(run);
}

export function n8nTimeoutMs(workflow?: N8nWorkflow): number {
  switch (normalizeN8nValue(workflow?.timeout_class)) {
    case "interactive":
      return 60_000;
    case "long_running":
      return 3_600_000;
    case "background":
    default:
      return 300_000;
  }
}

export function formatN8nElapsed(startedAtMs: number, nowMs = Date.now()): string {
  if (!startedAtMs) return "time pending";
  const totalSeconds = Math.max(0, Math.floor((nowMs - startedAtMs) / 1000));
  if (totalSeconds < 1) return "<1s";
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m ${String(seconds).padStart(2, "0")}s`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}h ${String(remainingMinutes).padStart(2, "0")}m`;
}

export function formatN8nTimestamp(timestampMs: number): string {
  if (!timestampMs) return "No callback evidence yet";
  return new Date(timestampMs).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function summarizeN8nEvidence(run?: N8nRunState): string {
  const latest = latestN8nEvidence(run);
  if (!latest) return "No callback evidence yet.";
  if (typeof latest === "string") return latest;
  if (latest.summary) return String(latest.summary);
  if (latest.message) return String(latest.message);
  if (latest.result) return typeof latest.result === "string" ? latest.result : "Result evidence received.";
  if (latest.error) return String(latest.error);
  const keys = Object.keys(latest);
  return keys.length > 0 ? `Evidence keys: ${keys.slice(0, 6).join(", ")}` : "Evidence received.";
}

export function n8nGovernanceNeedsReview(governance?: N8nGovernanceDecision): boolean {
  const verification = normalizeN8nValue(governance?.verification_status);
  const action = normalizeN8nValue(governance?.continuation_action);
  return (
    verification === "failed" ||
    verification === "needs_more_evidence" ||
    action === "recover_workflow" ||
    action === "pause_for_hitl"
  );
}

export function n8nGovernanceLabel(governance?: N8nGovernanceDecision): string {
  if (!governance) return "Governance pending";
  const verification = normalizeN8nValue(governance.verification_status);
  const action = normalizeN8nValue(governance.continuation_action);
  if (verification === "verified" && action === "continue_workflow") return "Verified";
  if (action === "pause_for_hitl") return "Needs review";
  if (action === "recover_workflow" || verification === "failed") return "Failed";
  if (verification === "needs_more_evidence") return "Waiting for evidence";
  return governance.verification_status || "Governance pending";
}

export function deriveN8nLifecycle(
  run?: N8nRunState,
  workflow?: N8nWorkflow,
  governance?: N8nGovernanceDecision,
  nowMs = Date.now(),
): N8nProgressView {
  if (!run) {
    return {
      lifecycle: "idle",
      label: "No runs yet",
      tone: "neutral",
      correlationLabel: "pending",
      elapsedLabel: "time pending",
      lastEvidenceLabel: "No callback evidence yet",
    };
  }

  const status = normalizeN8nValue(run.status);
  const phase = normalizeN8nValue(latestN8nEvidence(run)?.phase);
  const startedAt = n8nRunStartedAtMs(run);
  const elapsedMs = startedAt ? Math.max(0, nowMs - startedAt) : 0;
  const timeoutMs = n8nTimeoutMs(workflow);
  const evidenceAt = latestN8nEvidenceAtMs(run);
  const correlationLabel = shortN8nId(run.correlation_id);
  const elapsedLabel = formatN8nElapsed(startedAt, nowMs);
  const lastEvidenceLabel = formatN8nTimestamp(evidenceAt);
  const finalSummary = run.terminal ? summarizeN8nEvidence(run) : undefined;

  if (status === "triggering") {
    return {
      lifecycle: "triggering",
      label: "Triggering workflow",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (run.terminal || TERMINAL_STATUSES.has(status)) {
    if (status === "completed") {
      return {
        lifecycle: "completed",
        label: "Completed",
        tone: "ok",
        correlationLabel,
        elapsedLabel,
        lastEvidenceLabel,
        finalSummary,
      };
    }
    if (status === "partial") {
      return {
        lifecycle: "partial",
        label: "Partial result",
        tone: "warn",
        correlationLabel,
        elapsedLabel,
        lastEvidenceLabel,
        finalSummary,
        recoveryHint: "Review the evidence before using the partial result.",
      };
    }
    if (status === "timed_out") {
      return {
        lifecycle: "timed_out",
        label: "Timed out",
        tone: "danger",
        correlationLabel,
        elapsedLabel,
        lastEvidenceLabel,
        finalSummary,
        recoveryHint: "No terminal callback arrived before the deadline. Check n8n execution logs, then retry only if safe.",
      };
    }

    return {
      lifecycle: status === "cancelled" ? "cancelled" : status === "rejected" ? "rejected" : "failed",
      label: status === "cancelled" ? "Cancelled" : status === "rejected" ? "Rejected" : "Failed",
      tone: "danger",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      finalSummary,
      recoveryHint: "Review workflow logs and governance before retrying.",
    };
  }

  if (n8nGovernanceNeedsReview(governance)) {
    return {
      lifecycle: "needs_review",
      label: n8nGovernanceLabel(governance),
      tone: "warn",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      warning: governance?.explanation || "KRIA needs more evidence before continuing.",
      recoveryHint: "Review governance details before taking action.",
    };
  }

  if (status === "accepted" && elapsedMs < 2_000) {
    return {
      lifecycle: "accepted",
      label: phase === "polling_started" ? "Polling started" : "Accepted by n8n",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (phase === "calling_webhook") {
    return {
      lifecycle: "calling_n8n",
      label: "Calling n8n",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (phase === "finding_execution" || phase === "polling_started") {
    return {
      lifecycle: "finding_execution",
      label: "Finding n8n execution",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (phase === "monitor_lookup") {
    return {
      lifecycle: "monitoring_execution",
      label: "Checking latest n8n run",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (phase === "run_now_preparing") {
    return {
      lifecycle: "triggering",
      label: "Preparing Run Now",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      recoveryHint: "KRIA is creating a temporary runner workflow. Your original n8n workflow is not changed.",
    };
  }

  if (phase === "run_now_clone_created" || phase === "runner_starting" || phase === "runner_completed") {
    return {
      lifecycle: "polling_execution",
      label: phase === "run_now_clone_created" ? "Running temporary workflow" : "Running through n8n runner",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      recoveryHint: phase === "runner_completed" ? "KRIA is extracting the runner output." : undefined,
    };
  }

  if (phase === "run_now_output_extracted") {
    return {
      lifecycle: "extracting_output",
      label: "Run Now output ready",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  if (phase === "monitor_execution_running") {
    return {
      lifecycle: "monitoring_execution",
      label: "Latest n8n run is still running",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      recoveryHint: "Refresh Run History after n8n finishes.",
    };
  }

  if (phase === "polling_execution") {
    return {
      lifecycle: "polling_execution",
      label: "Polling n8n execution",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
      warning: elapsedMs > Math.min(30_000, Math.floor(timeoutMs / 4))
        ? "Still polling n8n execution output."
        : undefined,
      recoveryHint: elapsedMs > Math.min(30_000, Math.floor(timeoutMs / 4))
        ? "Open n8n executions or wait for KRIA to finish polling."
        : undefined,
    };
  }

  if (phase === "output_extracted") {
    return {
      lifecycle: "extracting_output",
      label: "Extracting output",
      tone: "waiting",
      correlationLabel,
      elapsedLabel,
      lastEvidenceLabel,
    };
  }

  const warning =
    elapsedMs > Math.min(30_000, Math.floor(timeoutMs / 4))
      ? (workflow?.requires_callback === false ? "Still polling n8n execution output." : "Still waiting for a terminal callback from n8n.")
      : undefined;

  return {
    lifecycle: "waiting_for_callback",
    label: status === "waiting_for_approval" ? "Waiting for approval" : workflow?.requires_callback === false ? "Polling n8n execution" : "Waiting for callback",
    tone: status === "waiting_for_approval" ? "warn" : "waiting",
    correlationLabel,
    elapsedLabel,
    lastEvidenceLabel,
    warning,
    recoveryHint: warning ? (workflow?.requires_callback === false ? "Check the n8n execution or API key if polling stays pending." : "Use Reconcile or check the n8n execution if this stays pending.") : undefined,
  };
}
