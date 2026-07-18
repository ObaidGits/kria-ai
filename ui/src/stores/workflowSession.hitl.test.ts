/**
 * workflowSession → Approval Center wiring (task 7.5, Req 11.1/11.6).
 *
 * A `hitl_required` telemetry event must surface the pause in the unified
 * Approval Center (not an inline modal), and a subsequent completion/cancel
 * must clear the now-moot card.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";

const { bridgeInvoke } = vi.hoisted(() => ({ bridgeInvoke: vi.fn() }));
vi.mock("../bridge/invoke", () => ({ bridgeInvoke, bridgeInvokeOptional: vi.fn() }));

import { handleTelemetryEvent } from "./workflowSession";
import { approvalStore } from "./approvalStore";
import type { TelemetryEnvelope } from "../types/workflowRuntime";

function envelope(event: TelemetryEnvelope["event"], seq = 1): TelemetryEnvelope {
  return { version: 1, seq, event, timestamp_ms: seq * 10, source: "substrate_router" };
}

describe("workflowSession HITL → Approval Center", () => {
  beforeEach(() => {
    bridgeInvoke.mockReset();
    approvalStore.setQueue([]);
  });

  it("routes a workflow HITL pause into the unified Approval Center (Req 11.1)", () => {
    handleTelemetryEvent(
      envelope({
        type: "started",
        workflow_id: "wf-9",
        title: "Send report",
        steps: [],
        execution_mode: { type: "structural" },
      }),
    );
    handleTelemetryEvent(
      envelope(
        {
          type: "hitl_required",
          workflow_id: "wf-9",
          reason: { type: "approval_needed", action: "Send report", risk_level: "yellow", description: "email it" },
          options: [
            { id: "ok", label: "Approve", action_type: { type: "approve" } },
            { id: "no", label: "Cancel", action_type: { type: "cancel" } },
          ],
          context: "About to send",
        },
        2,
      ),
    );

    const pending = approvalStore.queue().filter((r) => r.status === "pending");
    expect(pending).toHaveLength(1);
    expect(pending[0].type).toBe("workflow-resume");
    expect(pending[0].routing?.workflowId).toBe("wf-9");
    expect(pending[0].routing?.approveOptionId).toBe("ok");
  });

  it("clears the approval card when the workflow completes (Req 11.1)", () => {
    handleTelemetryEvent(
      envelope({
        type: "started",
        workflow_id: "wf-9",
        title: "Send report",
        steps: [],
        execution_mode: { type: "structural" },
      }),
    );
    handleTelemetryEvent(
      envelope(
        {
          type: "hitl_required",
          workflow_id: "wf-9",
          reason: { type: "manual_step_needed", instruction: "Do it", context: "" },
          options: [{ id: "ok", label: "Continue", action_type: { type: "approve" } }],
          context: "",
        },
        2,
      ),
    );
    expect(approvalStore.queue()).toHaveLength(1);

    handleTelemetryEvent(
      envelope(
        {
          type: "completed",
          workflow_id: "wf-9",
          verdict: { type: "complete" },
          summary: "done",
          artifacts: [],
          continuation: [],
        },
        3,
      ),
    );
    expect(approvalStore.queue()).toHaveLength(0);
  });
});
