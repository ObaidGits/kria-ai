/**
 * approvalStore decision-staging regression tests (IU-12 task 12.4, Req 11.6).
 *
 * Locks the architecture invariant that the store is a PURE typed-decision
 * stage: `approve` / `deny` / `keepPaused` only mutate a request's `status` and
 * emit a typed `approval:resolved` decision on the bus. They NEVER call a
 * runtime command — routing back to the runtime is the approval resolver's job
 * (bridge/approval.ts). `dismiss` stages NO decision at all.
 *
 * This is the store-level companion to the component proof in
 * shell/approvals/ApprovalCenter.test.tsx and the routing proof in
 * bridge/approval.test.ts — together they prove the Approval Center never
 * executes an action directly.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { approvalStore, type ApprovalRequest } from "./approvalStore";
import { eventBus } from "./eventBus";
// Raw source of the store, imported type-safely via vite/client's `*?raw`
// module declaration (avoids a node:fs type dependency in the src tsconfig).
import approvalStoreSource from "./approvalStore.ts?raw";

function makeRequest(over: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Run shell.run",
    description: "Needs to inspect the directory",
    risk: "yellow",
    routing: { requestId: "req-1" },
    payload: { cmd: "ls" },
    createdAt: 1000,
    status: "pending",
    ...over,
  };
}

describe("approvalStore — typed decision staging (Req 11.6, task 12.4)", () => {
  beforeEach(() => {
    approvalStore.setQueue([]);
  });
  afterEach(() => {
    vi.restoreAllMocks();
    approvalStore.setQueue([]);
  });

  it("approve stages status='approved' and emits a typed approve decision with scope", () => {
    approvalStore.setQueue([makeRequest()]);
    const emit = vi.spyOn(eventBus, "emit");

    approvalStore.approve("req-1", "session");

    expect(approvalStore.get("req-1")?.status).toBe("approved");
    expect(emit).toHaveBeenCalledWith("approval:resolved", {
      id: "req-1",
      action: "approve",
      scope: "session",
    });
  });

  it("deny stages status='denied' and emits a typed deny decision with reason", () => {
    approvalStore.setQueue([makeRequest()]);
    const emit = vi.spyOn(eventBus, "emit");

    approvalStore.deny("req-1", "not now");

    expect(approvalStore.get("req-1")?.status).toBe("denied");
    expect(emit).toHaveBeenCalledWith("approval:resolved", {
      id: "req-1",
      action: "deny",
      reason: "not now",
    });
  });

  it("keepPaused stages status='kept-paused' and emits a typed keep-paused decision", () => {
    approvalStore.setQueue([makeRequest()]);
    const emit = vi.spyOn(eventBus, "emit");

    approvalStore.keepPaused("req-1");

    expect(approvalStore.get("req-1")?.status).toBe("kept-paused");
    expect(emit).toHaveBeenCalledWith("approval:resolved", {
      id: "req-1",
      action: "keep-paused",
    });
  });

  it("every decision emits exactly one approval:resolved event (no extra runtime signal)", () => {
    approvalStore.setQueue([makeRequest()]);
    const resolved: unknown[] = [];
    const unsub = eventBus.on("approval:resolved", (p) => resolved.push(p));

    approvalStore.approve("req-1", "once");
    unsub();

    // A single typed decision — the store does not additionally emit a
    // capability/tool/workflow execution signal of its own.
    expect(resolved).toHaveLength(1);
  });

  it("dismiss removes the card WITHOUT staging any decision (no approval:resolved)", () => {
    approvalStore.setQueue([makeRequest()]);
    const emit = vi.spyOn(eventBus, "emit");

    approvalStore.dismiss("req-1");

    expect(approvalStore.get("req-1")).toBeUndefined();
    expect(emit).not.toHaveBeenCalledWith("approval:resolved", expect.anything());
  });

  it("does NOT statically depend on the runtime invoke bridge (never executes directly)", async () => {
    // The store must not import bridge/invoke: staging a decision can never
    // reach a Tauri command from inside the store module itself.
    const src = approvalStoreSource;
    expect(src).not.toMatch(/from ["']\.\.\/bridge\//);
    expect(src).not.toMatch(/bridgeInvoke|["']@tauri-apps\/api/);
  });
});
