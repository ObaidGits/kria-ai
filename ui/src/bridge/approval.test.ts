/**
 * Approval bridge tests (kria-ui-redesign task 4.2).
 *
 * Proves the two authorized backend-contract-change halves work end to end on
 * the frontend seam:
 *  - the unified `approval://request` envelope → single `approvalStore` queue,
 *    carrying the right source type + risk, calming the Core to `blocked`
 *    (Req 11.1 / 3.3);
 *  - a staged `approval:resolved` decision routes to the correct backend
 *    resolution command per source type, degrading gracefully (Req 11.6 / 20.4).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock the invoke layer so routing can be asserted without a Tauri runtime.
const invokeMock = vi.fn();
vi.mock("./invoke", () => ({
  bridgeInvoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

import {
  coerceApprovalEnvelope,
  routeApprovalDecision,
  initApprovalResolver,
  disposeApprovalResolver,
} from "./approval";
import { approvalStore, type ApprovalEnvelope } from "../stores/approvalStore";
import { coreStore } from "../stores/coreStore";
import { eventBus } from "../stores/eventBus";

function envelope(overrides: Partial<ApprovalEnvelope> = {}): ApprovalEnvelope {
  return {
    id: "req-1",
    source: "tool-hitl",
    title: "Run shell.run",
    description: "Needs to inspect the directory",
    risk: "yellow",
    routing: { requestId: "req-1" },
    payload: { cmd: "ls" },
    ...overrides,
  };
}

describe("coerceApprovalEnvelope (intake validation)", () => {
  it("accepts a well-formed envelope and normalizes routing", () => {
    const env = coerceApprovalEnvelope({
      id: "req-9",
      source: "workflow-resume",
      title: "Resume",
      description: "why",
      risk: "red",
      routing: { workflowId: "wf-9", approveOptionId: "approve" },
      payload: {},
      createdAtMs: 123,
    });
    expect(env).not.toBeNull();
    expect(env!.source).toBe("workflow-resume");
    expect(env!.risk).toBe("red");
    expect(env!.routing?.workflowId).toBe("wf-9");
  });

  it("rejects a payload with no id or an unknown source", () => {
    expect(coerceApprovalEnvelope(null)).toBeNull();
    expect(coerceApprovalEnvelope({ source: "tool-hitl" })).toBeNull();
    expect(coerceApprovalEnvelope({ id: "x", source: "not-a-source" })).toBeNull();
  });

  it("falls back to a conservative yellow risk for an unknown risk value", () => {
    const env = coerceApprovalEnvelope({ id: "x", source: "tool-hitl", risk: "wat" });
    expect(env!.risk).toBe("yellow");
  });
});

describe("addFromEnvelope → single queue + Core (Req 11.1 / 3.3)", () => {
  beforeEach(() => {
    approvalStore.setQueue([]);
    coreStore.reset();
    coreStore.initCoreStateMachine();
  });
  afterEach(() => {
    coreStore.disposeCoreStateMachine();
    coreStore.reset();
  });

  it("populates a card of the right source type and blocks the Core", () => {
    approvalStore.addFromEnvelope(envelope({ source: "gui-cognition", id: "g1", routing: { requestId: "g1" } }));

    const card = approvalStore.queue().find((r) => r.id === "g1");
    expect(card?.type).toBe("gui-cognition");
    expect(card?.status).toBe("pending");
    expect(coreStore.state()).toBe("blocked");
  });

  it("calms the Core back to idle once the only pending approval resolves", () => {
    approvalStore.addFromEnvelope(envelope({ id: "req-2", routing: { requestId: "req-2" } }));
    expect(coreStore.state()).toBe("blocked");

    approvalStore.keepPaused("req-2");
    expect(coreStore.state()).toBe("idle");
  });

  it("dedupes a re-emitted approval by id", () => {
    approvalStore.addFromEnvelope(envelope());
    approvalStore.addFromEnvelope(envelope({ title: "Updated title" }));
    const matches = approvalStore.queue().filter((r) => r.id === "req-1");
    expect(matches).toHaveLength(1);
    expect(matches[0].title).toBe("Updated title");
  });
});

describe("routeApprovalDecision (Req 11.6 — correct command per source)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ ok: true, data: {} });
  });

  it("tool-hitl approve → approve_action by requestId", async () => {
    const out = await routeApprovalDecision("tool-hitl", "approve", { requestId: "req-1" });
    expect(invokeMock).toHaveBeenCalledWith("approve_action", { requestId: "req-1" });
    expect(out.ok).toBe(true);
  });

  it("tool-hitl deny → deny_action with reason", async () => {
    await routeApprovalDecision("tool-hitl", "deny", { requestId: "req-1" }, { reason: "nope" });
    expect(invokeMock).toHaveBeenCalledWith("deny_action", { requestId: "req-1", reason: "nope" });
  });

  it("gui-cognition approve → approve_action (same request-id path)", async () => {
    await routeApprovalDecision("gui-cognition", "approve", { requestId: "g1" });
    expect(invokeMock).toHaveBeenCalledWith("approve_action", { requestId: "g1" });
  });

  it("interaction-decision approve → resolve_interaction_decision with the approve option", async () => {
    await routeApprovalDecision("interaction-decision", "approve", {
      decisionId: "d1",
      approveOptionId: "opt-a",
      denyOptionId: "opt-cancel",
    });
    expect(invokeMock).toHaveBeenCalledWith("resolve_interaction_decision", {
      decisionId: "d1",
      optionId: "opt-a",
    });
  });

  it("workflow-resume approve → workflow_hitl_respond; deny → workflow_cancel", async () => {
    await routeApprovalDecision("workflow-resume", "approve", {
      workflowId: "wf-1",
      approveOptionId: "approve",
    });
    expect(invokeMock).toHaveBeenCalledWith("workflow_hitl_respond", {
      workflowId: "wf-1",
      optionId: "approve",
      actionType: "approve",
      value: null,
    });

    invokeMock.mockClear();
    await routeApprovalDecision("workflow-resume", "deny", { workflowId: "wf-1" });
    expect(invokeMock).toHaveBeenCalledWith("workflow_cancel", { workflowId: "wf-1" });
  });

  it("capability-run approve → cpp_approve at scope then re-cpp_execute (Req 7.3)", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: { status: "ok" } });
    const out = await routeApprovalDecision(
      "capability-run",
      "approve",
      { providerId: "p", capabilityId: "c", capabilityArgs: { q: 1 } },
      { scope: "session" },
    );
    expect(invokeMock).toHaveBeenNthCalledWith(1, "cpp_approve", {
      providerId: "p",
      capabilityId: "c",
      sessionId: null,
      workspaceId: null,
      scope: "session",
      allow: true,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "cpp_execute", {
      providerId: "p",
      capabilityId: "c",
      args: { q: 1 },
      sessionId: null,
      workspaceId: null,
    });
    expect(out.ok).toBe(true);
  });

  it("capability-run deny → cpp_approve(allow=false), no execute", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: {} });
    await routeApprovalDecision("capability-run", "deny", { providerId: "p", capabilityId: "c" });
    expect(invokeMock).toHaveBeenCalledWith("cpp_approve", {
      providerId: "p",
      capabilityId: "c",
      sessionId: null,
      workspaceId: null,
      scope: "once",
      allow: false,
    });
    expect(invokeMock).not.toHaveBeenCalledWith("cpp_execute", expect.anything());
  });

  it("capability-run defaults to once scope for an invalid scope", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: { status: "ok" } });
    await routeApprovalDecision(
      "capability-run",
      "approve",
      { providerId: "p", capabilityId: "c" },
      { scope: "forever" },
    );
    expect(invokeMock).toHaveBeenNthCalledWith(1, "cpp_approve", expect.objectContaining({ scope: "once" }));
  });

  it("keep-paused never calls the backend (agent stays paused)", async () => {
    const out = await routeApprovalDecision("tool-hitl", "keep-paused", { requestId: "req-1" });
    expect(invokeMock).not.toHaveBeenCalled();
    expect(out.command).toBeNull();
    expect(out.ok).toBe(true);
  });

  it("reports missing routing keys instead of invoking", async () => {
    const out = await routeApprovalDecision("tool-hitl", "approve", {});
    expect(invokeMock).not.toHaveBeenCalled();
    expect(out.ok).toBe(false);
    expect(out.note).toMatch(/requestId/);
  });

  it("degrades gracefully when the resolution command is unavailable (Req 20.4)", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "unavailable", message: "no such command", command: "approve_action" });
    const out = await routeApprovalDecision("tool-hitl", "approve", { requestId: "req-1" });
    expect(out.ok).toBe(false);
    expect(out.note).toContain("unavailable");
  });
});

describe("initApprovalResolver (staged decision → runtime routing)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ ok: true, data: {} });
    approvalStore.setQueue([]);
    disposeApprovalResolver();
  });
  afterEach(() => disposeApprovalResolver());

  it("routes the resolved decision to the backend using the stored routing", async () => {
    initApprovalResolver();
    approvalStore.addFromEnvelope(envelope({ id: "req-7", routing: { requestId: "req-7" } }));

    // Staging an approve emits approval:resolved, which the resolver consumes.
    approvalStore.approve("req-7", "once");
    await Promise.resolve();
    await Promise.resolve();

    expect(invokeMock).toHaveBeenCalledWith("approve_action", { requestId: "req-7" });
  });

  it("does not route a request without runtime routing; only syncs presentation", async () => {
    initApprovalResolver();
    approvalStore.addRequest({
      id: "local-1",
      type: "tool-hitl",
      title: "t",
      description: "d",
      risk: "green",
      payload: null,
      createdAt: Date.now(),
      status: "pending",
    });
    eventBus.emit("approval:resolved", { id: "local-1", action: "approve" });
    await Promise.resolve();
    expect(invokeMock).not.toHaveBeenCalledWith("approve_action", expect.anything());
    expect(invokeMock).toHaveBeenCalledWith("sync_approval_presentation", {
      id: "local-1",
      status: "approved",
    });
  });
});
