/**
 * Capability management action tests (task 8.2, Req 7.4).
 *
 * Each action is a dispatch-only call to an EXISTING backend command; these
 * tests assert the correct command + args and honest success/failure results.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("./invoke", () => ({
  bridgeInvoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args),
}));

// The store loaders are dispatch-only too; stub them so refreshes don't invoke.
vi.mock("../stores/capabilityStore", () => ({
  capabilityStore: {
    loadSkills: vi.fn().mockResolvedValue({ ok: true, data: [] }),
    loadModels: vi.fn().mockResolvedValue({ ok: true, data: [] }),
    loadIntegrations: vi.fn().mockResolvedValue({ ok: true, data: [] }),
    loadQuarantine: vi.fn().mockResolvedValue({ ok: true, data: [] }),
    loadScopedGrants: vi.fn().mockResolvedValue({ ok: true, data: [] }),
  },
}));

import {
  installSkill,
  toggleSkill,
  switchProvider,
  testProvider,
  connectMcpServer,
  connectGoogleWorkspace,
  connectColabTier,
  connectTelegram,
  approveQuarantinedTool,
  rejectQuarantinedTool,
  revokeGrant,
} from "./capabilityActions";

describe("capabilityActions (Req 7.4)", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({ ok: true, data: null });
  });

  it("installSkill dispatches clawhub_install_skill with the approved capabilities", async () => {
    const res = await installSkill({
      slug: "s",
      manifestUrl: "https://hub/x",
      approvedCapabilities: { capabilities: ["net"] },
    });
    expect(res.ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("clawhub_install_skill", {
      request: {
        manifest_url: "https://hub/x",
        slug: "s",
        approved_capabilities: { capabilities: ["net"] },
      },
    });
  });

  it("toggleSkill dispatches clawhub_toggle_skill", async () => {
    await toggleSkill("s", false);
    expect(invokeMock).toHaveBeenCalledWith("clawhub_toggle_skill", { skillId: "s", enabled: false });
  });

  it("switchProvider dispatches switch_provider", async () => {
    await switchProvider("openai");
    expect(invokeMock).toHaveBeenCalledWith("switch_provider", { providerId: "openai" });
  });

  it("testProvider dispatches test_provider_connection_cmd and normalizes data", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      data: {
        status: "success",
        message: "Connected",
        latency_ms: 42,
        discovered_models: ["model-a"],
        diagnostics: { reachable: true },
      },
    });
    const res = await testProvider("openai");
    expect(invokeMock).toHaveBeenCalledWith("test_provider_connection_cmd", { providerId: "openai" });
    expect(res).toEqual({
      ok: true,
      data: {
        status: "success",
        message: "Connected",
        latencyMs: 42,
        discoveredModels: ["model-a"],
        diagnostics: { reachable: true },
      },
    });
  });

  it("connectMcpServer dispatches add_mcp_server with args", async () => {
    await connectMcpServer({ name: "fs", command: "npx server", args: ["--root", "/tmp"] });
    expect(invokeMock).toHaveBeenCalledWith("add_mcp_server", {
      name: "fs",
      command: "npx server",
      args: ["--root", "/tmp"],
      trustLevel: null,
    });
  });

  it("connectGoogleWorkspace + connectColabTier dispatch their connect commands", async () => {
    await connectGoogleWorkspace();
    expect(invokeMock).toHaveBeenCalledWith("connect_google_workspace", { account: null });
    invokeMock.mockClear();
    await connectColabTier("gpu");
    expect(invokeMock).toHaveBeenCalledWith("connect_colab_tier", { serverName: "gpu" });
  });

  it("connectTelegram dispatches update_telegram_config", async () => {
    await connectTelegram({ enabled: true, botToken: "tok", allowedChatIds: "1,2" });
    expect(invokeMock).toHaveBeenCalledWith("update_telegram_config", {
      enabled: true,
      botToken: "tok",
      allowedChatIds: "1,2",
      autoStart: false,
    });
  });

  it("returns an honest failure when the command fails", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "error", message: "boom" });
    const res = await switchProvider("x");
    expect(res).toEqual({ ok: false, message: "boom" });
  });

  // ── Governance (task 8.4) — dispatch-only relays to existing commands ──────

  it("approveQuarantinedTool dispatches approve_quarantined_tool", async () => {
    const res = await approveQuarantinedTool("tool-1");
    expect(res.ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("approve_quarantined_tool", { toolId: "tool-1" });
  });

  it("rejectQuarantinedTool dispatches reject_quarantined_tool", async () => {
    const res = await rejectQuarantinedTool("tool-2");
    expect(res.ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("reject_quarantined_tool", { toolId: "tool-2" });
  });

  it("revokeGrant dispatches openclaw_revoke_grant", async () => {
    const res = await revokeGrant("grant-9");
    expect(res.ok).toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("openclaw_revoke_grant", { grantId: "grant-9" });
  });

  it("quarantine actions surface an honest failure when the command fails", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "error", message: "nope" });
    expect(await approveQuarantinedTool("t")).toEqual({ ok: false, message: "nope" });
    expect(await rejectQuarantinedTool("t")).toEqual({ ok: false, message: "nope" });
    expect(await revokeGrant("g")).toEqual({ ok: false, message: "nope" });
  });
});
