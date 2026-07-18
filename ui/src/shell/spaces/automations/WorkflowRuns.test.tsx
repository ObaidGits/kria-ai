/**
 * WorkflowRuns tests (task 7.5, Req 6.5/11.1/11.6).
 *
 * The canonical workflow run list must:
 *   • dispatch `workflow_cancel` for a run in progress (no dead control)
 *   • dispatch `workflow_continuation` for a post-run action
 *   • surface a HITL pause as a pointer to the Approval Center, never inline
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";

const { bridgeInvoke } = vi.hoisted(() => ({ bridgeInvoke: vi.fn() }));
vi.mock("../../../bridge/invoke", () => ({ bridgeInvoke, bridgeInvokeOptional: vi.fn() }));

import { WorkflowRuns } from "./WorkflowRuns";
import { handleTelemetryEvent } from "../../../stores/workflowSession";
import { approvalStore } from "../../../stores";
import type { TelemetryEnvelope } from "../../../types/workflowRuntime";

function envelope(event: TelemetryEnvelope["event"], seq = 1): TelemetryEnvelope {
  return { version: 1, seq, event, timestamp_ms: seq * 10, source: "substrate_router" };
}

function start(workflowId: string) {
  handleTelemetryEvent(
    envelope({
      type: "started",
      workflow_id: workflowId,
      title: workflowId,
      steps: [{ index: 0, description: "step", step_type: "verification", execution_mode: "backend" }],
      execution_mode: { type: "structural" },
    }),
  );
}

describe("WorkflowRuns — reachable cancel/continuation (Req 11.6)", () => {
  beforeEach(() => {
    bridgeInvoke.mockReset();
    bridgeInvoke.mockResolvedValue({ ok: true, data: {} });
    approvalStore.setQueue([]);
  });
  afterEach(() => cleanup());

  it("Cancel dispatches workflow_cancel for a running workflow", async () => {
    start("wf-run-1");
    render(() => <WorkflowRuns />);
    fireEvent.click(screen.getByRole("button", { name: "Cancel workflow wf-run-1" }));
    await waitFor(() => {
      expect(bridgeInvoke).toHaveBeenCalledWith("workflow_cancel", { workflowId: "wf-run-1" });
    });
  });

  it("a continuation action dispatches workflow_continuation after completion", async () => {
    start("wf-run-2");
    handleTelemetryEvent(
      envelope(
        {
          type: "completed",
          workflow_id: "wf-run-2",
          verdict: { type: "complete" },
          summary: "done",
          artifacts: [],
          continuation: [
            { id: "open", label: "Open result", action_type: { type: "open_url", url: "https://example.test" } },
          ],
        },
        2,
      ),
    );
    render(() => <WorkflowRuns />);
    fireEvent.click(screen.getByRole("button", { name: "Open result" }));
    await waitFor(() => {
      expect(bridgeInvoke).toHaveBeenCalledWith(
        "workflow_continuation",
        expect.objectContaining({ workflowId: "wf-run-2", actionType: "open_url" }),
      );
    });
  });

  it("surfaces a HITL pause as an Approval Center pointer, not an inline dialog", () => {
    start("wf-run-3");
    handleTelemetryEvent(
      envelope(
        {
          type: "hitl_required",
          workflow_id: "wf-run-3",
          reason: { type: "manual_step_needed", instruction: "Sign in", context: "" },
          options: [{ id: "ok", label: "Continue", action_type: { type: "approve" } }],
          context: "",
        },
        2,
      ),
    );
    const onOpenApprovals = vi.fn();
    render(() => <WorkflowRuns onOpenApprovals={onOpenApprovals} />);

    expect(screen.getByText("This run needs your approval.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Open the Approval Center/i }));
    expect(onOpenApprovals).toHaveBeenCalled();
  });
});
