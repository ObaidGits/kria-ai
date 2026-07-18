import { describe, expect, it } from "vitest";
import {
  deriveN8nLifecycle,
  formatN8nElapsed,
  n8nGovernanceLabel,
  shortN8nId,
} from "./n8nProgress";
import type { N8nGovernanceDecision, N8nRunState, N8nWorkflow } from "../stores/n8n";

const workflow: N8nWorkflow = {
  workflow_id: "test_workflow",
  workflow_version: "v1",
  display_name: "Test Workflow",
  endpoint_path: "/webhook/test",
  status: "approved",
  environment: "dev",
  risk_tier: "Green",
  timeout_class: "interactive",
};

function run(overrides: Partial<N8nRunState>): N8nRunState {
  return {
    correlation_id: "corr_1234567890abcdef",
    workflow_id: "test_workflow",
    workflow_version: "v1",
    n8n_run_id: "run_1234567890abcdef",
    last_sequence_number: 0,
    status: "accepted",
    evidence_log: [],
    side_effects: [],
    terminal: false,
    triggered_at_ms: 1_000,
    ...overrides,
  };
}

describe("n8n progress model", () => {
  it("shows accepted briefly before waiting for callback", () => {
    expect(deriveN8nLifecycle(run({}), workflow, undefined, 2_000).lifecycle).toBe("accepted");
    expect(deriveN8nLifecycle(run({}), workflow, undefined, 4_000).lifecycle).toBe("waiting_for_callback");
  });

  it("renders terminal timeout as a recovery state", () => {
    const progress = deriveN8nLifecycle(
      run({ status: "timed_out", terminal: true, evidence_log: [{ summary: "late", occurred_at_ms: 1_500 }] }),
      workflow,
      undefined,
      62_000,
    );

    expect(progress.lifecycle).toBe("timed_out");
    expect(progress.tone).toBe("danger");
    expect(progress.recoveryHint).toContain("No terminal callback");
  });

  it("labels non-callback polling phases without saying callback", () => {
    const pollingWorkflow = { ...workflow, requires_callback: false };
    const progress = deriveN8nLifecycle(
      run({
        status: "running",
        evidence_log: [{ phase: "polling_execution", result: "Polling", occurred_at_ms: 1_500 }],
      }),
      pollingWorkflow,
      undefined,
      12_000,
    );

    expect(progress.lifecycle).toBe("polling_execution");
    expect(progress.label).toBe("Polling n8n execution");
    expect(progress.recoveryHint || "").not.toContain("callback");
  });

  it("surfaces governance review states without raw JSON", () => {
    const governance: N8nGovernanceDecision = {
      workflow_id: "test_workflow",
      correlation_id: "corr_1234567890abcdef",
      run_status: "running",
      verification_status: "needs_more_evidence",
      continuation_action: "await_more_events",
      explanation: "evidence pending",
    };

    const progress = deriveN8nLifecycle(run({ status: "running" }), workflow, governance, 12_000);
    expect(progress.lifecycle).toBe("needs_review");
    expect(progress.warning).toBe("evidence pending");
    expect(n8nGovernanceLabel(governance)).toBe("Waiting for evidence");
  });

  it("formats compact IDs and elapsed time", () => {
    expect(shortN8nId("corr_1234567890abcdef")).toBe("corr_123...cdef");
    expect(formatN8nElapsed(1_000, 62_000)).toBe("1m 01s");
  });
});
