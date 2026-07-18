/**
 * Workflow HITL → Approval Center bridge (kria-ui-redesign task 7.5,
 * Req 6.5 / 11.1 / 11.6).
 *
 * The canonical workflow runtime (`workflowSession`) pauses on a
 * human-in-the-loop step and emits `hitl_required` telemetry. Historically the
 * frontend surfaced that inline (a dead/duplicated approval UI). This bridge
 * routes every workflow HITL pause into the ONE unified Approval Center
 * (`approvalStore`) as a `workflow-resume` request, so the decision lives where
 * every other approval does (Req 11.1) and is routed back through the
 * runtime's `workflow_hitl_respond` / `workflow_cancel` commands by the
 * existing approval resolver (bridge/approval.ts, Req 11.6).
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * This module only TRANSLATES a pause into an approval request and enqueues it.
 * It never executes the action, never resumes the workflow, and never bypasses
 * policy. Approve/deny are staged by the Approval Center and handed to the
 * runtime, which owns re-grounding + teardown.
 *
 * Pure builder (`buildWorkflowHitlEnvelope`) is exported for testing.
 */

import type { ActiveHitl, HitlOption, HitlReason } from "../types/workflowRuntime";
import {
  approvalStore,
  type ApprovalEnvelope,
  type RiskLevel,
} from "../stores/approvalStore";
import { mirrorApprovalPresentation } from "./approval";

/** Stable approval id for a workflow HITL pause (dedupe on re-emit). */
export function workflowApprovalId(workflowId: string): string {
  return `wf-hitl:${workflowId}`;
}

/** Map a HITL reason's risk to the unified risk ramp (Req 11.2). */
function riskFromReason(reason: HitlReason): RiskLevel {
  if (reason.type === "approval_needed") {
    const lvl = (reason.risk_level ?? "").toLowerCase();
    if (lvl.includes("black")) return "black";
    if (lvl.includes("red") || lvl.includes("high") || lvl.includes("critical")) return "red";
    // Check yellow BEFORE green — "yellow" literally contains "low".
    if (lvl.includes("yellow") || lvl.includes("medium") || lvl.includes("moderate")) return "yellow";
    if (lvl === "low" || lvl.includes("green")) return "green";
    return "yellow";
  }
  // A failed/blocked step is more consequential to resume than a benign prompt.
  if (reason.type === "step_failed" || reason.type === "session_expired") return "yellow";
  return "yellow";
}

/** Human-readable title/description/effects for a HITL reason (Req 11.2). */
function describeReason(reason: HitlReason): {
  title: string;
  description: string;
  effects?: string[];
} {
  switch (reason.type) {
    case "install_required":
      return {
        title: `Install ${reason.app}`,
        description: `The workflow needs ${reason.app} installed before it can continue.`,
        effects: reason.install_command ? [`Run: ${reason.install_command}`] : undefined,
      };
    case "login_required":
      return {
        title: `Sign in to ${reason.service}`,
        description: reason.guidance || `The workflow needs you to sign in to ${reason.service}.`,
      };
    case "session_expired":
      return {
        title: `Session expired — ${reason.service}`,
        description: `Your ${reason.service} session expired. Re-authenticate to continue.`,
      };
    case "ambiguous_target":
      return {
        title: "Choose a target",
        description: reason.question,
        effects: reason.options,
      };
    case "execution_mode_choice":
      return {
        title: "Choose how to run this step",
        description: reason.task,
        effects: [`Backend: ${reason.backend_option}`, `Visible: ${reason.gui_option}`],
      };
    case "approval_needed":
      return {
        title: reason.action || "Approval needed",
        description: reason.description,
      };
    case "visibility_uncertain":
      return {
        title: "Confirm the result",
        description: `${reason.step_description}. ${reason.suggestion}`,
      };
    case "focus_lost":
      return {
        title: "Window focus lost",
        description: reason.step_description,
      };
    case "manual_step_needed":
      return {
        title: "Manual step needed",
        description: reason.instruction,
        effects: reason.context ? [reason.context] : undefined,
      };
    case "intent_unclear":
      return {
        title: "Clarify what you meant",
        description: `Understood: ${reason.what_understood}. ${reason.suggestion}`,
      };
    case "budget_exhausted":
      return {
        title: "Time budget reached",
        description: `Ran for ${Math.round(reason.elapsed_ms / 1000)}s with ${reason.remaining_steps} step(s) left. Continue?`,
      };
    case "accessibility_setup":
      return {
        title: "Accessibility setup needed",
        description: `${reason.current_state}. ${reason.impact}`,
      };
    case "step_failed":
      return {
        title: "A step failed",
        description: `${reason.step_description}: ${reason.error}`,
      };
  }
}

/** Whether a HITL option represents a proceed/approve-style choice. */
function isProceedOption(o: HitlOption): boolean {
  return (
    o.action_type.type === "approve" ||
    o.action_type.type === "retry" ||
    o.action_type.type === "manual_complete" ||
    o.action_type.type === "choose_alternative" ||
    o.action_type.type === "skip" ||
    o.action_type.type === "open_url" ||
    o.action_type.type === "run_command"
  );
}

/** Whether a HITL option represents a deny/cancel-style choice. */
function isDenyOption(o: HitlOption): boolean {
  return o.action_type.type === "deny" || o.action_type.type === "cancel";
}

/** Pick the option submitted on APPROVE (first proceed option, else first). */
export function pickApproveOption(options: HitlOption[]): HitlOption | undefined {
  return options.find(isProceedOption) ?? options.find((o) => !isDenyOption(o)) ?? options[0];
}

/** Pick the option submitted on DENY (an explicit deny/cancel option). */
export function pickDenyOption(options: HitlOption[]): HitlOption | undefined {
  return options.find(isDenyOption);
}

/**
 * Build a unified `workflow-resume` approval envelope from a workflow HITL
 * pause. Pure — no side effects — so it is independently testable.
 */
export function buildWorkflowHitlEnvelope(
  workflowId: string,
  hitl: ActiveHitl,
): ApprovalEnvelope {
  const approve = pickApproveOption(hitl.options);
  const deny = pickDenyOption(hitl.options);
  const d = describeReason(hitl.reason);
  const risk = riskFromReason(hitl.reason);

  return {
    id: workflowApprovalId(workflowId),
    source: "workflow-resume",
    title: d.title,
    description: d.description || hitl.context || "",
    risk,
    effects: d.effects,
    evidence: hitl.context || undefined,
    // Resuming a workflow into a side-effecting step is not casually
    // reversible; high risk already forces an explicit confirm (Req 11.3).
    irreversible: risk === "red" || risk === "black",
    scopeOptions: ["once"],
    routing: {
      workflowId,
      approveOptionId: approve?.id,
      denyOptionId: deny?.id,
    },
    payload: { reason: hitl.reason, options: hitl.options },
    createdAtMs: hitl.receivedAt,
  };
}

/**
 * Enqueue a workflow HITL pause into the unified Approval Center. Idempotent:
 * the stable id dedupes re-emitted pauses (approvalStore replaces in place).
 */
export function enqueueWorkflowHitl(workflowId: string, hitl: ActiveHitl): void {
  const envelope = buildWorkflowHitlEnvelope(workflowId, hitl);
  approvalStore.addFromEnvelope(envelope);
  mirrorApprovalPresentation(envelope);
}

/**
 * Drop a workflow's pending approval card when the run ends on its own
 * (finalized/cancelled) so a now-moot decision does not linger. Stages no
 * decision and routes nothing to the runtime.
 */
export function dismissWorkflowHitl(workflowId: string): void {
  approvalStore.dismiss(workflowApprovalId(workflowId));
}
