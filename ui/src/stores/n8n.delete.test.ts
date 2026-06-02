import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  configuredWorkflows,
  deleteWorkflow,
  isDeletingWorkflow,
  refreshN8nStatus,
} from "./n8n";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

const workflow = (workflowId: string) => ({
  workflow_id: workflowId,
  workflow_version: "v1",
  display_name: workflowId,
  endpoint_path: `/webhook/${workflowId}`,
  status: "approved",
  environment: "dev",
  risk_tier: "Green",
});

function statusPayload(workflows: any[]) {
  return {
    enabled: true,
    mode: "external",
    base_url: "http://127.0.0.1:5678",
    callback_url: "http://127.0.0.1:3001/api/n8n/callback",
    configured_workflows: workflows,
    catalog_workflows: workflows,
    workflow_registry: {
      workflow_count: workflows.length,
      records: workflows,
      workflows,
    },
    runs: [],
    dead_letters: [],
    governance_log: [],
    hitl_responses: {},
    inbox_path: "",
    audit_path: "",
  };
}

describe("n8n store workflow deletion UX", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockClear();
  });

  it("hides deleted workflow immediately and ignores stale refresh payloads", async () => {
    const workflows = [workflow("gmail_inbox_digest"), workflow("slack_post_update")];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_n8n_status") return statusPayload(workflows);
      if (command === "get_n8n_runtime_status") return null;
      if (command === "list_n8n_executions") return null;
      if (command === "remove_n8n_workflow_from_kria") return { status: "removed_from_kria" };
      return null;
    });

    await refreshN8nStatus();
    expect(configuredWorkflows().map((item) => item.workflow_id)).toEqual([
      "gmail_inbox_digest",
      "slack_post_update",
    ]);

    const deletion = deleteWorkflow("gmail_inbox_digest");
    expect(isDeletingWorkflow("gmail_inbox_digest")).toBe(true);
    expect(configuredWorkflows().map((item) => item.workflow_id)).toEqual([
      "slack_post_update",
    ]);

    await deletion;
    expect(isDeletingWorkflow("gmail_inbox_digest")).toBe(false);
    expect(configuredWorkflows().map((item) => item.workflow_id)).toEqual([
      "slack_post_update",
    ]);
  });

  it("restores workflow when delete command fails", async () => {
    const workflows = [workflow("calendar_create_meeting")];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_n8n_status") return statusPayload(workflows);
      if (command === "get_n8n_runtime_status") return null;
      if (command === "list_n8n_executions") return null;
      if (command === "remove_n8n_workflow_from_kria") throw new Error("delete failed");
      return null;
    });

    await refreshN8nStatus();
    await expect(deleteWorkflow("calendar_create_meeting")).rejects.toThrow("delete failed");
    expect(isDeletingWorkflow("calendar_create_meeting")).toBe(false);
    expect(configuredWorkflows().map((item) => item.workflow_id)).toEqual([
      "calendar_create_meeting",
    ]);
  });

  it("treats already-absent registry entries as deleted to avoid stale reappearance", async () => {
    const workflows = [workflow("old_toml_workflow")];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_n8n_status") return statusPayload(workflows);
      if (command === "get_n8n_runtime_status") return null;
      if (command === "list_n8n_executions") return null;
      if (command === "remove_n8n_workflow_from_kria") {
        throw "workflow 'old_toml_workflow' not found in KRIA workflow registry";
      }
      return null;
    });

    await refreshN8nStatus();
    const result = await deleteWorkflow("old_toml_workflow");
    expect(result.status).toBe("deleted");
    expect(isDeletingWorkflow("old_toml_workflow")).toBe(false);
    expect(configuredWorkflows().map((item) => item.workflow_id)).toEqual([]);
  });
});
