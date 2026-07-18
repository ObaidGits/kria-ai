/**
 * Workflow HITL → Approval Center bridge tests (task 7.5, Req 6.5/11.1/11.6).
 *
 * Verifies the pure envelope builder maps a workflow HITL pause into a
 * `workflow-resume` approval routed back through `workflow_hitl_respond` /
 * `workflow_cancel`, and that enqueue/dismiss keep the unified Approval Center
 * as the single home for the decision.
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  buildWorkflowHitlEnvelope,
  enqueueWorkflowHitl,
  dismissWorkflowHitl,
  workflowApprovalId,
  pickApproveOption,
  pickDenyOption,
} from "./workflowApproval";
import { approvalStore } from "../stores/approvalStore";
import type { ActiveHitl, HitlOption } from "../types/workflowRuntime";

function hitl(over: Partial<ActiveHitl> = {}): ActiveHitl {
  const options: HitlOption[] = [
    { id: "opt-approve", label: "Approve", action_type: { type: "approve" } },
    { id: "opt-cancel", label: "Cancel", action_type: { type: "cancel" } },
  ];
  return {
    reason: { type: "approval_needed", action: "Send email", risk_level: "yellow", description: "Send the digest" },
    options,
    context: "About to send an email to the team",
    receivedAt: 1000,
    ...over,
  };
}

describe("workflowApproval — envelope builder (Req 11.1/11.6)", () => {
  it("maps a HITL pause to a workflow-resume envelope routed by workflow id", () => {
    const env = buildWorkflowHitlEnvelope("wf-1", hitl());
    expect(env.id).toBe(workflowApprovalId("wf-1"));
    expect(env.source).toBe("workflow-resume");
    expect(env.title).toBe("Send email");
    expect(env.description).toBe("Send the digest");
    expect(env.risk).toBe("yellow");
    expect(env.routing?.workflowId).toBe("wf-1");
    expect(env.routing?.approveOptionId).toBe("opt-approve");
    expect(env.routing?.denyOptionId).toBe("opt-cancel");
  });

  it("escalates risk to red for a high-risk approval and marks it irreversible", () => {
    const env = buildWorkflowHitlEnvelope(
      "wf-2",
      hitl({ reason: { type: "approval_needed", action: "Delete", risk_level: "high", description: "rm -rf" } }),
    );
    expect(env.risk).toBe("red");
    expect(env.irreversible).toBe(true);
  });

  it("picks proceed and deny options from the HITL choice set", () => {
    const options: HitlOption[] = [
      { id: "r", label: "Retry", action_type: { type: "retry" } },
      { id: "d", label: "Deny", action_type: { type: "deny" } },
    ];
    expect(pickApproveOption(options)?.id).toBe("r");
    expect(pickDenyOption(options)?.id).toBe("d");
  });
});

describe("workflowApproval — enqueue/dismiss into the unified Approval Center", () => {
  beforeEach(() => approvalStore.setQueue([]));

  it("enqueues a pending workflow-resume request", () => {
    enqueueWorkflowHitl("wf-1", hitl());
    const q = approvalStore.queue();
    expect(q).toHaveLength(1);
    expect(q[0].type).toBe("workflow-resume");
    expect(q[0].status).toBe("pending");
    expect(q[0].routing?.workflowId).toBe("wf-1");
  });

  it("dedupes a re-emitted pause by stable id (no duplicate cards)", () => {
    enqueueWorkflowHitl("wf-1", hitl());
    enqueueWorkflowHitl("wf-1", hitl({ context: "changed" }));
    expect(approvalStore.queue()).toHaveLength(1);
  });

  it("dismiss removes the card without staging a decision", () => {
    enqueueWorkflowHitl("wf-1", hitl());
    dismissWorkflowHitl("wf-1");
    expect(approvalStore.queue()).toHaveLength(0);
  });
});
