import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { RegistryPanel } from "./RegistryPanel";
import { n8nStore, type N8nRuntimeProfileDraft, type N8nWorkflow } from "../../../stores/n8n";

const profile = {
  schema_version: "v1", profile_id: "profile-1", workflow_id: "manual-report",
  n8n_workflow_id: "n8n-1", display_name: "Manual report", n8n_workflow_name: "Manual report",
  n8n_workflow_hash: "hash", status: "draft", trigger_strategy: "manual_api_execute",
  result_mode: "poll_execution", detected_triggers: [], input_candidates: [], output_strategy: "terminal_node",
  runner_backend: "remote_ssh", runner_target: "old-host", runner_container_name: "",
  credential_requirements: ["none"], credential_status: "present", category: "reporting",
  risk_estimate: "yellow", irreversibility_estimate: "read_only", data_scope: ["user_requested"],
  external_data_transfer: false, hitl_detected: false, hitl_strategy: "none", confidence: 0.9,
  warnings: [], created_at_ms: 1, updated_at_ms: 1,
} as N8nRuntimeProfileDraft;

const authoredWorkflow = {
  workflow_id: "chat-mail", workflow_version: "v1", display_name: "Mail", endpoint_path: "/mail",
  status: "draft", environment: "dev", risk_tier: "Yellow", adaptation_strategy: "chat_canvas_authored_draft",
  generated_copy_n8n_verified: true, credential_requirements: ["gmailOAuth2Api"],
} as N8nWorkflow;

beforeEach(() => {
  vi.spyOn(n8nStore, "initialize").mockResolvedValue(undefined);
  vi.spyOn(n8nStore, "runtimeProfileDrafts").mockReturnValue([profile]);
  vi.spyOn(n8nStore, "savedRuntimeProfiles").mockReturnValue([profile]);
  vi.spyOn(n8nStore, "configuredWorkflows").mockReturnValue([authoredWorkflow]);
  vi.spyOn(n8nStore, "managementBusyKey").mockReturnValue(null);
  vi.spyOn(n8nStore, "managementError").mockReturnValue(null);
  vi.spyOn(n8nStore, "saveProfileAsWorkflowDraft").mockResolvedValue({ status: "blocked", blockers: ["runner review"] });
  vi.spyOn(n8nStore, "syncRuntimeProfileDrafts").mockResolvedValue([profile]);
  vi.spyOn(n8nStore, "loadCredentialSummaries").mockResolvedValue([
    { credential_id: "cred-1", credential_name: "Work Gmail", credential_type: "gmailOAuth2Api", node_family: "gmail", redacted: true },
  ]);
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("RegistryPanel production onboarding", () => {
  it("syncs profiles and submits reviewed runner metadata with backend blockers", async () => {
    render(() => <RegistryPanel />);
    fireEvent.click(screen.getByRole("button", { name: /Sync workflows from n8n/i }));
    await waitFor(() => expect(n8nStore.syncRuntimeProfileDrafts).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByText("Review setup metadata"));
    const target = screen.getByRole("textbox", { name: "Runner target" });
    fireEvent.input(target, { target: { value: "prod-host" } });
    fireEvent.click(screen.getByRole("button", { name: "Save to KRIA" }));

    await waitFor(() => expect(n8nStore.saveProfileAsWorkflowDraft).toHaveBeenCalledWith(
      expect.objectContaining({ profileId: "profile-1", runnerBackend: "remote_ssh", runnerTarget: "prod-host" }),
    ));
    expect(screen.getByText(/runner review/i)).toBeInTheDocument();
  });

  it("loads only redacted credential summaries on explicit request", async () => {
    render(() => <RegistryPanel />);
    fireEvent.click(screen.getByText(/Credential mapping/));
    fireEvent.click(screen.getByRole("button", { name: "Load redacted credentials" }));
    await waitFor(() => expect(n8nStore.loadCredentialSummaries).toHaveBeenCalledOnce());
    expect(screen.getByRole("button", { name: /gmailOAuth2Api/i })).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("secret");
  });
});