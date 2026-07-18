/**
 * Run segment tests (task 7.2, Req 6.3 / 6.5).
 *
 * Verifies the ask-KRIA-to-pick / WorkflowCard / SuggestionCard /
 * PreparedInputPreview / RunProgress / EvidenceViewer set:
 *   • ask-KRIA-to-pick dispatches the EXISTING `suggest_n8n_workflows` command
 *   • WorkflowCard Run dispatches `invoke_n8n_workflow_from_ui`
 *   • running n8n cards never fake cancellation through KRIA workflow commands
 *   • PreparedInputPreview shows prepared inputs + confirm gate
 *   • RunProgress reflects run events (determinate progressbar)
 *   • EvidenceViewer renders + SANITIZES untrusted output
 *   • a run's HITL step surfaces via the Approval Center (approvalStore), not inline
 *   • honest loading / empty / failure states + a11y
 *
 * The Tauri bridge is mocked so we assert the exact command routing.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";

// Hoisted bridge spy so the store's `bridgeInvoke` import is the mock.
const { bridgeInvoke } = vi.hoisted(() => ({ bridgeInvoke: vi.fn() }));
vi.mock("../../../bridge/invoke", () => ({
  bridgeInvoke,
  bridgeInvokeOptional: vi.fn(),
}));

import { RunRegion } from "./RunRegion";
import { WorkflowCard } from "./WorkflowCard";
import { PreparedInputPreview } from "./PreparedInputPreview";
import { RunProgress } from "./RunProgress";
import { EvidenceViewer } from "./EvidenceViewer";
import { AskKriaToPick } from "./AskKriaToPick";
import { automationStore, approvalStore } from "../../../stores";
import type { Workflow, PreparedRunInput, ApprovalRequest } from "../../../stores";

function ok<T>(data: T) {
  return { ok: true as const, data };
}

function makeWorkflow(over: Partial<Workflow> = {}): Workflow {
  return {
    id: "w1",
    name: "Nightly backup",
    description: "Back up the DB",
    status: "idle",
    lastRunAt: null,
    createdAt: Date.now(),
    version: "1",
    ...over,
  };
}

function resetStore() {
  automationStore.setWorkflows([]);
  automationStore.setSearchQuery("");
  automationStore.setLoading(false);
  automationStore.clearRunState();
  approvalStore.setQueue([]);
  // Clear running set by completing any leftover ids.
  for (const id of automationStore.runningWorkflowIds()) {
    automationStore.markWorkflowCompleted(id, true);
  }
}

describe("Run segment — ask-KRIA-to-pick + WorkflowCard (task 7.2, Req 6.3/6.5)", () => {
  beforeEach(() => {
    bridgeInvoke.mockReset();
    resetStore();
  });
  afterEach(() => cleanup());

  // ── ask-KRIA-to-pick ───────────────────────────────────────────────────────
  it("dispatches the pick/suggest command with the described intent (Req 6.3)", async () => {
    bridgeInvoke.mockResolvedValueOnce(
      ok({
        candidates: [
          {
            workflow_id: "wf-digest",
            workflow_version: "2",
            display_name: "Email digest",
            reason: "Matches summarizing unread email",
            confidence: 0.91,
            confidence_label: "High",
            risk_tier: "green",
            requires_confirmation: false,
            missing_inputs: [],
          },
        ],
        message: "",
      }),
    );

    const { container } = render(() => <AskKriaToPick />);
    const box = screen.getByPlaceholderText(/save a briefing/i) as HTMLTextAreaElement;
    fireEvent.input(box, { target: { value: "summarize my unread email" } });
    fireEvent.submit(container.querySelector("form")!);

    await waitFor(() => {
      expect(bridgeInvoke).toHaveBeenCalledWith("suggest_n8n_workflows", {
        request: { prompt: "summarize my unread email" },
      });
    });
    // The suggestion surfaces as a KRIA-authored card.
    expect(await screen.findByText("Email digest")).toBeInTheDocument();
    expect(screen.getByText("Suggested by KRIA")).toBeInTheDocument();
  });

  it("shows an honest failure when the pick command is unavailable (Req 6.5/20.4)", async () => {
    bridgeInvoke.mockResolvedValueOnce({ ok: false, code: "unavailable", message: "n8n offline", command: "suggest_n8n_workflows" });
    const { container } = render(() => <AskKriaToPick />);
    fireEvent.input(screen.getByPlaceholderText(/save a briefing/i), {
      target: { value: "do a thing" },
    });
    fireEvent.submit(container.querySelector("form")!);
    expect(await screen.findByRole("alert")).toHaveTextContent("n8n offline");
  });

  // ── WorkflowCard run / cancel ────────────────────────────────────────────────
  it("Run dispatches the existing run command via the bridge (Req 6.5)", async () => {
    bridgeInvoke.mockResolvedValue(ok({}));
    render(() => <WorkflowCard workflow={makeWorkflow()} />);
    fireEvent.click(screen.getByRole("button", { name: "Run Nightly backup" }));

    await waitFor(() => {
      expect(bridgeInvoke).toHaveBeenCalledWith(
        "invoke_n8n_workflow_from_ui",
        expect.objectContaining({
          request: expect.objectContaining({ workflowId: "w1", workflowVersion: "1", confirmed: true }),
        }),
      );
    });
  });

  it("does not misroute n8n cancellation through KRIA workflow cancellation", () => {
    automationStore.markWorkflowStarted("w1");
    render(() => <WorkflowCard workflow={makeWorkflow({ status: "running" })} />);

    expect(screen.queryByRole("button", { name: "Cancel Nightly backup" })).toBeNull();
    expect(screen.getByText("Running in n8n. Stop it from n8n Executions if needed.")).toBeInTheDocument();
    expect(bridgeInvoke).not.toHaveBeenCalledWith("workflow_cancel", expect.anything());
  });

  it("surfaces an honest failure when the run command fails", async () => {
    bridgeInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "runner rejected" });
    render(() => <WorkflowCard workflow={makeWorkflow()} />);
    fireEvent.click(screen.getByRole("button", { name: "Run Nightly backup" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("runner rejected");
  });

  // ── HITL → Approval Center (not inline) ──────────────────────────────────────
  it("routes a run's HITL step to the Approval Center, not an inline modal (Req 6.5/11.1)", () => {
    const req: ApprovalRequest = {
      id: "ap1",
      type: "workflow-resume",
      title: "Approve send",
      description: "Send the digest email",
      risk: "yellow",
      routing: { workflowId: "w1" },
      payload: null,
      createdAt: Date.now(),
      status: "pending",
    };
    approvalStore.setQueue([req]);
    const onOpenApprovals = vi.fn();
    render(() => <WorkflowCard workflow={makeWorkflow({ status: "running" })} onOpenApprovals={onOpenApprovals} />);

    // A calm pointer — no dialog is rendered inline.
    expect(screen.getByText("This run needs your approval.")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Open the Approval Center/i }));
    expect(onOpenApprovals).toHaveBeenCalled();
  });

  // ── PreparedInputPreview ─────────────────────────────────────────────────────
  it("PreparedInputPreview shows the prepared inputs before confirming (Req 6.3)", () => {
    const prepared: PreparedRunInput = {
      workflowId: "w1",
      workflowVersion: "1",
      displayName: "Email digest",
      prompt: "summarize email",
      payload: { hours: 24, label: "unread" },
      fields: [{ name: "hours", type: "number", required: true, description: "Lookback window" }],
      missingInputs: [],
      validationIssues: [],
      explanation: "Derived a 24h window from your request.",
      inputMapped: true,
    };
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(() => (
      <PreparedInputPreview prepared={prepared} onConfirm={onConfirm} onCancel={onCancel} />
    ));

    expect(screen.getByText("Prepared inputs — Email digest")).toBeInTheDocument();
    expect(screen.getByText("hours")).toBeInTheDocument();
    expect(screen.getByText(/"label": "unread"/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Confirm and run/i }));
    expect(onConfirm).toHaveBeenCalled();
  });

  it("blocks confirm when inputs are missing (Req 6.3 confirm gate)", () => {
    const prepared: PreparedRunInput = {
      workflowId: "w1",
      workflowVersion: "1",
      displayName: "Email digest",
      prompt: "x",
      payload: {},
      fields: [],
      missingInputs: ["recipient"],
      validationIssues: [],
      inputMapped: true,
    };
    render(() => <PreparedInputPreview prepared={prepared} onConfirm={vi.fn()} onCancel={vi.fn()} />);
    expect(screen.getByRole("button", { name: /Confirm and run/i })).toBeDisabled();
    expect(screen.getByText(/Missing inputs: recipient/)).toBeInTheDocument();
  });

  // ── RunProgress ──────────────────────────────────────────────────────────────
  it("RunProgress reflects run events with a determinate progressbar (Req 6.5/17.3)", () => {
    render(() => (
      <RunProgress
        progress={{
          workflowId: "w1",
          phase: "running",
          completedSteps: 2,
          totalSteps: 4,
          message: "Step 2 running",
          updatedAt: Date.now(),
        }}
      />
    ));
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "50");
    // Phase conveyed by text, not color alone (Req 17.3).
    expect(screen.getByText("Running")).toBeInTheDocument();
  });

  it("startRun seeds live progress fed onward by run events (Req 6.5)", async () => {
    bridgeInvoke.mockResolvedValue(ok({}));
    await automationStore.startRun({ workflowId: "w1", workflowVersion: "1" });
    expect(automationStore.runProgress()["w1"]?.phase).toBe("running");
  });

  // ── EvidenceViewer ───────────────────────────────────────────────────────────
  it("EvidenceViewer renders evidence and SANITIZES untrusted output (Req 6.5)", () => {
    const { container } = render(() => (
      <EvidenceViewer
        evidence={[
          { label: "Run output", detail: "<img src=x onerror=alert(1)>Done <b>ok</b>" },
          { label: "n8n execution", href: "https://example.test/exec/1" },
        ]}
      />
    ));
    expect(screen.getByText("Run output")).toBeInTheDocument();
    // Sanitized: the onerror handler must be stripped.
    expect(container.innerHTML).not.toContain("onerror");
    // Benign markup survives.
    expect(container.querySelector("b")).not.toBeNull();
    // Link evidence renders as a safe external link.
    const link = screen.getByRole("link", { name: "n8n execution" });
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  // ── RunRegion honest states + top-level surfacing ────────────────────────────
  it("RunRegion shows an honest empty state and the ask bar (Req 6.2/6.3)", () => {
    render(() => <RunRegion />);
    expect(screen.getByRole("heading", { name: "No workflows yet" })).toBeInTheDocument();
    expect(screen.getByLabelText("Describe what you want to automate")).toBeInTheDocument();
  });

  it("RunRegion surfaces workflows at the top level as interactive cards (Req 6.2)", () => {
    automationStore.setWorkflows([makeWorkflow(), makeWorkflow({ id: "w2", name: "Digest email" })]);
    render(() => <RunRegion />);
    expect(screen.getByText("Showing 2 of 2")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run Nightly backup" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Run Digest email" })).toBeInTheDocument();
  });
});
