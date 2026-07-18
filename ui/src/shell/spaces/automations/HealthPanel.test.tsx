import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { HealthPanel } from "./HealthPanel";
import { n8nStore, type N8nCopyLifecycleOperation, type N8nStatusPayload } from "../../../stores/n8n";

const status = {
  enabled: true, base_url: "http://127.0.0.1:5678", callback_url: "http://127.0.0.1/callback",
  configured_workflows: [], runs: [], dead_letters: [], governance_log: [], hitl_responses: {},
  inbox_path: "/tmp/inbox", audit_path: "/tmp/audit",
} as N8nStatusPayload;

const pending = {
  operation_id: "op-1", status: "pending_recovery", stage: "registry_save_failed",
  source_profile_id: "source-profile", source_workflow_id: "source-workflow",
  source_n8n_workflow_id: "source-n8n", copy_workflow_id: "generated-copy",
  copy_n8n_workflow_id: "copy-n8n", adaptation_strategy: "input_aware_copy",
  last_error: "Registry write failed", recovery_actions: ["continue_setup", "delete_n8n_copy"],
  created_at_ms: 1, updated_at_ms: 2,
} as N8nCopyLifecycleOperation;

beforeEach(() => {
  vi.spyOn(n8nStore, "initialize").mockResolvedValue(undefined);
  vi.spyOn(n8nStore, "status").mockReturnValue(status);
  vi.spyOn(n8nStore, "runtimeStatus").mockReturnValue(null);
  vi.spyOn(n8nStore, "productionAudit").mockReturnValue(null);
  vi.spyOn(n8nStore, "workflowLifecycleReports").mockReturnValue([]);
  vi.spyOn(n8nStore, "copyLifecycleOperations").mockReturnValue([pending]);
  vi.spyOn(n8nStore, "managementBusyKey").mockReturnValue(null);
  vi.spyOn(n8nStore, "continuePendingCopyOperation").mockResolvedValue({ message: "Recovered safely." });
  vi.spyOn(n8nStore, "runProductionAudit").mockResolvedValue({
    schema_version: "v1", generated_at_ms: 1, expires_at_ms: 2, overall_status: "ready",
    security_status: "ready", reliability_status: "ready", adapter_readiness: [], summary_counts: {},
    findings: [], recommended_actions: [],
  });
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("HealthPanel production operations", () => {
  it("surfaces pending copy recovery and dispatches explicit continuation", async () => {
    render(() => <HealthPanel />);
    expect(screen.getByText("Pending generated-copy recovery")).toBeInTheDocument();
    expect(screen.getByText("Registry write failed")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Continue setup" }));
    await waitFor(() => expect(n8nStore.continuePendingCopyOperation).toHaveBeenCalledWith("op-1"));
    expect(await screen.findByText("Recovered safely.")).toBeInTheDocument();
  });

  it("runs and renders the authoritative production audit", async () => {
    render(() => <HealthPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Run audit" }));
    await waitFor(() => expect(n8nStore.runProductionAudit).toHaveBeenCalledOnce());
    expect(await screen.findByText(/Production audit complete: ready/)).toBeInTheDocument();
  });
});