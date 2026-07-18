/**
 * Capability run → permission-gate → Approval Center bridge tests
 * (kria-ui-redesign task 8.2, Req 7.3).
 *
 * Verifies the pure envelope builder offers the full scope ladder, and that
 * `runCapability` dispatches through the gated `cpp_execute` and — ONLY when the
 * runtime asks — enqueues a unified `capability-run` approval. It never executes
 * the capability itself.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("./invoke", () => ({
  bridgeInvoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

import {
  buildCapabilityRunEnvelope,
  capabilityRunApprovalId,
  riskFromDecision,
  runCapability,
  CAPABILITY_RUN_SCOPES,
} from "./capabilityRun";
import { approvalStore } from "../stores/approvalStore";
import { eventBus } from "../stores/eventBus";

describe("buildCapabilityRunEnvelope (Req 7.3)", () => {
  it("builds a capability-run envelope with the full scope ladder", () => {
    const env = buildCapabilityRunEnvelope({
      providerId: "prov",
      capabilityId: "cap",
      name: "Search web",
      description: "why",
      effects: ["network.read"],
      risk: "medium",
      args: { q: "x" },
    });
    expect(env.id).toBe(capabilityRunApprovalId("prov", "cap"));
    expect(env.source).toBe("capability-run");
    expect(env.title).toBe("Run Search web");
    expect(env.risk).toBe("yellow");
    expect(env.effects).toEqual(["network.read"]);
    expect(env.scopeOptions).toEqual([...CAPABILITY_RUN_SCOPES]);
    expect(env.routing?.providerId).toBe("prov");
    expect(env.routing?.capabilityId).toBe("cap");
    expect(env.routing?.capabilityArgs).toEqual({ q: "x" });
  });

  it("marks high-risk runs irreversible", () => {
    const env = buildCapabilityRunEnvelope({
      providerId: "p",
      capabilityId: "c",
      risk: "high",
    });
    expect(env.risk).toBe("red");
    expect(env.irreversible).toBe(true);
  });

  it("elevated low-risk capabilities are at least yellow", () => {
    expect(riskFromDecision("low", true)).toBe("yellow");
    expect(riskFromDecision("low", false)).toBe("green");
    expect(riskFromDecision(null, false)).toBe("green");
    expect(riskFromDecision("critical")).toBe("black");
  });
});

describe("runCapability (Req 7.3 — gate then route)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    approvalStore.setQueue([]);
  });

  it("enqueues a capability-run approval when the gate returns needs_approval", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      data: { status: "needs_approval", decision: { effects: ["fs.write"], risk: "medium" } },
    });

    const out = await runCapability({ providerId: "p", capabilityId: "c", name: "Write file" });
    expect(out.status).toBe("needs_approval");
    expect(invokeMock).toHaveBeenCalledWith(
      "cpp_execute",
      expect.objectContaining({ providerId: "p", capabilityId: "c" }),
    );

    const q = approvalStore.queue();
    expect(q).toHaveLength(1);
    expect(q[0].type).toBe("capability-run");
    expect(q[0].scopeOptions).toEqual([...CAPABILITY_RUN_SCOPES]);
    expect(q[0].effects).toEqual(["fs.write"]);
  });

  it("returns ok without enqueuing when the gate allows the run", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: { status: "ok", value: { done: true } } });
    const out = await runCapability({ providerId: "p", capabilityId: "c" });
    expect(out).toEqual({ status: "ok", value: { done: true } });
    expect(approvalStore.queue()).toHaveLength(0);
  });

  it("surfaces a denied run honestly (no approval card)", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: { status: "denied", reason: "policy" } });
    const out = await runCapability({ providerId: "p", capabilityId: "c" });
    expect(out).toEqual({ status: "denied", reason: "policy" });
    expect(approvalStore.queue()).toHaveLength(0);
  });

  it("degrades gracefully when cpp_execute is unavailable (Req 20.4)", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "unavailable", message: "no command" });
    const out = await runCapability({ providerId: "p", capabilityId: "c" });
    expect(out.status).toBe("error");
    expect(approvalStore.queue()).toHaveLength(0);
  });

  it("emits a run-result event with the honest status", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: { status: "ok", value: null } });
    const seen: string[] = [];
    const unsub = eventBus.on("capability:run-result", (p) => seen.push(p.status));
    await runCapability({ providerId: "p", capabilityId: "c" });
    unsub();
    expect(seen).toContain("ok");
  });
});
