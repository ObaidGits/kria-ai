/**
 * 2D node builder tests (task 7.3, Req 6.3 / 6.4).
 *
 * Verifies the NodeBuilder / NodePalette / NodeCanvas / NodeInspector set:
 *   • the canvas renders nodes + connections (edges)
 *   • the palette adds a node (click-to-add) and selects it
 *   • selecting a node opens the shared node Inspector (Req 1.6 / 5.2)
 *   • editing params in the Inspector updates the draft (Req 6.3)
 *   • Test dispatches the authoritative backend test command; Approve dispatches
 *     the EXISTING approve command (authoring lifecycle → existing n8n commands)
 *   • keyboard add/select/move (Req 17.1) + a11y (labelled canvas + nodes)
 *   • honest "not persisted" state + client-side dry-run fallback (Req 20.4)
 *
 * The Tauri bridge is mocked so we assert the exact command routing.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@solidjs/testing-library";

const { bridgeInvoke } = vi.hoisted(() => ({ bridgeInvoke: vi.fn() }));
vi.mock("../../../bridge/invoke", () => ({
  bridgeInvoke,
  bridgeInvokeOptional: vi.fn(),
}));

import { NodeBuilder } from "./NodeBuilder";
import { NodeCanvas } from "./NodeCanvas";
import { NodeInspector } from "./NodeInspector";
import { registerAutomationNodeInspector } from "./registerAutomationNodeInspector";
import { InspectorHost } from "../../InspectorHost";
import { resetInspectorRegistry } from "../../inspectorRegistry";
import { automationStore, shellStore } from "../../../stores";

function ok<T>(data: T) {
  return { ok: true as const, data };
}

function savedDraft(status: "created_as_draft" | "updated_as_draft" = "created_as_draft") {
  return ok({
    status,
    workflow: { n8n_workflow_id: "n8n-draft-1" },
    message: "Draft persisted in n8n.",
  });
}

beforeEach(() => {
  bridgeInvoke.mockReset();
  automationStore.newDraft();
  shellStore.setInspectorTarget(null);
  resetInspectorRegistry();
});
afterEach(() => cleanup());

describe("NodePalette / NodeCanvas — add + render (task 7.3, Req 6.4)", () => {
  it("adds a node from the palette (click-to-add) and shows it on the canvas", () => {
    render(() => <NodeBuilder />);
    expect(screen.getByRole("group", { name: "Node palette" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Add Manual Trigger node" }));

    expect(automationStore.builderNodes()).toHaveLength(1);
    expect(
      screen.getByRole("button", { name: /Manual Trigger node — select to configure/ }),
    ).toBeInTheDocument();
  });

  it("renders connections between nodes as edges (Req 6.4)", () => {
    const a = automationStore.addNode("manual-trigger");
    const b = automationStore.addNode("http-request");
    automationStore.connectNodes(a, b);

    const { container } = render(() => <NodeCanvas />);
    expect(automationStore.builderEdges()).toHaveLength(1);
    // The SVG edge layer draws one line per connection.
    expect(container.querySelectorAll("line.kria-nb-canvas__edge")).toHaveLength(1);
  });

  it("exposes a labelled canvas group + labelled nodes (a11y, Req 17.1/17.2)", () => {
    automationStore.addNode("code");
    render(() => <NodeCanvas />);
    expect(screen.getByRole("group", { name: "Workflow node canvas" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Code node — select to configure/ })).toBeInTheDocument();
  });

  it("moves a node with the arrow keys while its button is focused (Req 17.1)", () => {
    const id = automationStore.addNode("set");
    const startX = automationStore.builderNodes()[0].x;
    render(() => <NodeCanvas />);
    const nodeButton = screen.getByRole("button", { name: /Edit Fields node — select to configure/ });
    fireEvent.keyDown(nodeButton, { key: "ArrowRight" });
    const moved = automationStore.builderNodes().find((n) => n.id === id)!;
    expect(moved.x).toBe(startX + 20);
  });
});

describe("Node selection → shared Inspector (task 7.3, Req 1.6 / 5.2)", () => {
  it("selecting a node opens the shared node Inspector", () => {
    render(() => <NodeBuilder />);
    fireEvent.click(screen.getByRole("button", { name: "Add HTTP Request node" }));
    // addNode selects the node → the builder's effect targets the shared Inspector.
    const target = shellStore.inspectorTarget();
    expect(target?.type).toBe("automation-node");
    expect(target?.id).toBe(automationStore.builderNodes()[0].id);
  });

  it("renders the node Inspector body through the registry for automation-node targets", () => {
    const id = automationStore.addNode("http-request");
    registerAutomationNodeInspector();
    shellStore.setInspectorTarget({ type: "automation-node", id });
    render(() => <InspectorHost />);
    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();
    expect(screen.getByText("HTTP Request")).toBeInTheDocument();
  });
});

describe("NodeInspector — edit params updates the draft (task 7.3, Req 6.3)", () => {
  it("adds a parameter that lands on the node's draft params", () => {
    const id = automationStore.addNode("http-request");
    render(() => <NodeInspector target={{ type: "automation-node", id }} />);

    fireEvent.input(screen.getByLabelText("New parameter"), { target: { value: "url" } });
    fireEvent.input(screen.getByLabelText("Value"), { target: { value: "https://api.test" } });
    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));

    const node = automationStore.builderNodes().find((n) => n.id === id)!;
    expect(node.params.url).toBe("https://api.test");
  });

  it("editing params resets the lifecycle to editing (no stale save presented)", () => {
    const id = automationStore.addNode("set");
    // Pretend a prior save succeeded.
    render(() => <NodeInspector target={{ type: "automation-node", id }} />);
    fireEvent.input(screen.getByLabelText("New parameter"), { target: { value: "field" } });
    fireEvent.input(screen.getByLabelText("Value"), { target: { value: "1" } });
    fireEvent.click(screen.getByRole("button", { name: /^Add$/ }));
    expect(automationStore.draftPersisted()).toBe(false);
  });
});

describe("Authoring lifecycle → authoritative n8n commands (task 7.3, Req 6.3)", () => {
  it("Save draft dispatches real create/update with runnable metadata", async () => {
    bridgeInvoke.mockResolvedValue(savedDraft());
    automationStore.addNode("manual-trigger");
    render(() => <NodeBuilder />);
    fireEvent.click(screen.getByRole("button", { name: /Save draft/ }));

    await waitFor(() => {
      expect(bridgeInvoke).toHaveBeenCalledWith(
        "create_or_update_n8n_workflow_draft",
        expect.objectContaining({
          request: expect.objectContaining({
            workflowId: automationStore.draftId(),
            updateExisting: false,
            requiresCallback: false,
          }),
        }),
      );
    });
    expect(automationStore.draftPersisted()).toBe(true);
    expect(automationStore.draftLifecycle()).toBe("saved");
  });

  it("keeps persisted identity across edits and updates the existing n8n draft", async () => {
    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    const trigger = automationStore.addNode("manual-trigger");
    await automationStore.saveDraft();

    automationStore.renameNode(trigger, "Start now");
    expect(automationStore.draftPersisted()).toBe(true);
    expect(automationStore.draftLifecycle()).toBe("editing");

    bridgeInvoke.mockResolvedValueOnce(savedDraft("updated_as_draft"));
    await automationStore.saveDraft();
    expect(bridgeInvoke).toHaveBeenLastCalledWith(
      "create_or_update_n8n_workflow_draft",
      expect.objectContaining({ request: expect.objectContaining({ updateExisting: true }) }),
    );
  });

  it("does not treat a rejected backend payload as a successful save", async () => {
    bridgeInvoke.mockResolvedValue(ok({ status: "rejected", message: "Workflow JSON failed validation." }));
    automationStore.addNode("manual-trigger");
    const res = await automationStore.saveDraft();
    expect(res.ok).toBe(false);
    expect(automationStore.draftPersisted()).toBe(false);
    expect(automationStore.builderStatus()).toMatch(/failed validation/i);
  });

  it("Test dispatches authoritative backend execution after persistence", async () => {
    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    automationStore.addNode("manual-trigger");
    await automationStore.saveDraft();
    bridgeInvoke.mockResolvedValueOnce(ok({
      status: "test_started",
      correlation_id: "test-run-1",
      message: "Draft test started.",
    }));

    const res = await automationStore.testDraft();
    expect(res.ok).toBe(true);
    expect(bridgeInvoke).toHaveBeenLastCalledWith(
      "test_n8n_workflow_draft",
      expect.objectContaining({
        request: expect.objectContaining({ workflowId: automationStore.draftId(), confirmed: true }),
      }),
    );
    expect(automationStore.draftLifecycle()).toBe("tested");
    expect(automationStore.draftTestResult()?.clientSideOnly).toBe(false);
  });

  it("uses local validation only for unavailable backend and never marks tested", async () => {
    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    automationStore.addNode("manual-trigger");
    await automationStore.saveDraft();
    bridgeInvoke.mockResolvedValueOnce({
      ok: false,
      code: "unavailable",
      message: "n8n offline",
      command: "test_n8n_workflow_draft",
    });

    const res = await automationStore.testDraft();
    expect(res.ok).toBe(false);
    expect(automationStore.draftTestResult()?.clientSideOnly).toBe(true);
    expect(automationStore.draftLifecycle()).toBe("saved");
    expect(automationStore.builderStatus()).toMatch(/does not count as a backend test/i);
  });

  it("does not hide backend test failures behind local validation", async () => {
    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    automationStore.addNode("manual-trigger");
    await automationStore.saveDraft();
    bridgeInvoke.mockResolvedValueOnce({ ok: false, code: "error", message: "runner rejected" });

    const res = await automationStore.testDraft();
    expect(res.ok).toBe(false);
    expect(automationStore.draftTestResult()).toBeNull();
    expect(automationStore.builderStatus()).toBe("runner rejected");
  });

  it("approveDraft dispatches only after authoritative backend test", async () => {
    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    automationStore.addNode("manual-trigger");
    await automationStore.saveDraft();
    bridgeInvoke.mockResolvedValueOnce(ok({ status: "test_started", correlation_id: "test-run-1" }));
    await automationStore.testDraft();
    bridgeInvoke.mockResolvedValueOnce(ok({ status: "approved", message: "Approved." }));

    const res = await automationStore.approveDraft();
    expect(res.ok).toBe(true);
    expect(bridgeInvoke).toHaveBeenLastCalledWith(
      "approve_n8n_workflow_draft",
      { request: { workflowId: automationStore.draftId(), confirmed: true } },
    );
    expect(automationStore.draftLifecycle()).toBe("approved");
  });

  it("blocks approve until persistence and authoritative testing complete", async () => {
    automationStore.addNode("manual-trigger");
    expect((await automationStore.approveDraft()).ok).toBe(false);
    expect(bridgeInvoke).not.toHaveBeenCalled();

    bridgeInvoke.mockResolvedValueOnce(savedDraft());
    await automationStore.saveDraft();
    bridgeInvoke.mockClear();
    expect((await automationStore.approveDraft()).ok).toBe(false);
    expect(bridgeInvoke).not.toHaveBeenCalled();
  });

  it("shows an honest 'not yet persisted' state before a save (Req 6.3)", () => {
    render(() => <NodeBuilder />);
    expect(screen.getByText(/Not yet persisted/)).toBeInTheDocument();
  });
});

describe("builderToWorkflowJson — n8n serialization (task 7.3, Req 6.4)", () => {
  it("serializes nodes + a connections map keyed by unique node name", () => {
    const a = automationStore.addNode("manual-trigger");
    const b = automationStore.addNode("http-request");
    automationStore.connectNodes(a, b);

    const wf = automationStore.builderToWorkflowJson();
    expect(wf.nodes).toHaveLength(2);
    const trigger = wf.nodes.find((n) => n.type === "n8n-nodes-base.manualTrigger")!;
    const http = wf.nodes.find((n) => n.type === "n8n-nodes-base.httpRequest")!;
    expect(wf.connections[trigger.name].main[0][0].node).toBe(http.name);
  });
});
