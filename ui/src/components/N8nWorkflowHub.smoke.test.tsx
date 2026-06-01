import { fireEvent, render, screen } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  runWorkflowMock,
  prepareWorkflowInputMock,
  refreshMock,
  initializeMock,
  suggestWorkflowsMock,
  listWorkflowExecutionsMock,
  viewWorkflowExecutionMock,
  resumeWaitingExecutionMock,
  n8nMockState,
} = vi.hoisted(() => {
  const n8nMockState = { workflowSuggestion: null as any, runs: [] as any[] };
  return {
    n8nMockState,
    runWorkflowMock: vi.fn(async () => ({ correlation_id: "corr-smoke-1" })),
    prepareWorkflowInputMock: vi.fn(async () => ({
      status: "ready",
      workflow_id: "test_workflow",
      workflow_version: "v1",
      display_name: "Test Workflow",
      prompt: "Run test_workflow",
      input_payload: { source_prompt: "Run test_workflow", genre: "action" },
      missing_inputs: [],
      validation_issues: [],
      field_summaries: [],
      schema_allows_additional: true,
      source: "llm_active_provider",
      model: "test-model",
      confidence: 0.92,
      explanation: "Mapped prompt into workflow input.",
      message: "KRIA prepared workflow input from your prompt. Review before running.",
    })),
    refreshMock: vi.fn(async () => undefined),
    initializeMock: vi.fn(async () => undefined),
    listWorkflowExecutionsMock: vi.fn(async () => ({
    source: "n8n_api",
    workflow_id: "daily_digest",
    workflow_version: "v1",
    limit: 10,
    offset: 0,
    has_more: false,
    executions: [
      {
        n8n_execution_id: "42",
        status: "completed",
        started_at_ms: 1780000000000,
        duration_ms: 1200,
        output_source: "Set",
        result_preview: "Latest digest output",
      },
    ],
    })),
    viewWorkflowExecutionMock: vi.fn(async () => ({ correlation_id: "corr-history-1" })),
    resumeWaitingExecutionMock: vi.fn(async () => ({
      status: "accepted",
      phase: "hitl_resume_sent",
      message: "Decision sent to n8n.",
    })),
    suggestWorkflowsMock: vi.fn(async () => {
    const suggestion = {
      schema_version: "kria.n8n.workflow_suggestion.v1",
      prompt: "Run test_workflow",
      reference: "test_workflow",
      status: "needs_confirmation",
      candidates: [
        {
          workflow_id: "test_workflow",
          workflow_version: "v1",
          display_name: "Test Workflow",
          category: "diagnostic",
          risk_tier: "Green",
          status: "approved",
          hitl_policy: "none",
          score: 100,
          confidence: 1,
          confidence_label: "high",
          matched_on: ["workflow_id"],
          requires_confirmation: true,
          reason: "Exact workflow_id match",
        },
      ],
      requires_confirmation: true,
      can_auto_run: false,
      ambiguous: false,
      hard_prompt: false,
      message: "I found Test Workflow. Confirm before I run it.",
      confirmation_hint: "Confirm with: Confirm workflow test_workflow",
    };
    n8nMockState.workflowSuggestion = suggestion;
    return suggestion;
  }),
  };
});

vi.mock("./N8nDiagnosticsPanel", () => ({
  default: () => <div data-testid="n8n-diagnostics-smoke">Diagnostics</div>,
}));

vi.mock("./N8nWorkflowManagementPanel", () => ({
  default: (props: { view?: string }) => (
    <div data-testid={`n8n-management-${props.view ?? "profiles"}-smoke`}>
      Workflow Management {props.view ?? "profiles"}
    </div>
  ),
}));

vi.mock("../stores/n8n", () => {
  const workflow = {
    workflow_id: "test_workflow",
    workflow_version: "v1",
    display_name: "Test Workflow",
    endpoint_path: "/webhook/test",
    status: "approved",
    environment: "dev",
    risk_tier: "Green",
    timeout_class: "interactive",
    category: "diagnostic",
    description: "Safe diagnostic workflow",
    example_prompts: ["Run test_workflow"],
    tags: ["diagnostic"],
    aliases: ["test n8n workflow"],
  };
  const monitorWorkflow = {
    ...workflow,
    workflow_id: "daily_digest",
    display_name: "Daily Digest",
    trigger_strategy: "scheduled_monitor",
    result_mode: "monitor_only",
  };
  return {
    friendlyN8nError: (raw: unknown) => String(raw ?? ""),
    n8nStore: {
      status: () => ({
        enabled: true,
        base_url: "http://127.0.0.1:5678",
        callback_url: "http://127.0.0.1:3001/api/n8n/callback",
        configured_workflows: [workflow, monitorWorkflow],
        runs: n8nMockState.runs,
        dead_letters: [],
        governance_log: [],
        hitl_responses: {},
        inbox_path: "",
        audit_path: "",
      }),
      runtimeStatus: () => ({
        mode: "external",
        enabled: true,
        base_url: "http://127.0.0.1:5678",
        secret_sources: { api_key: { present: true } },
      }),
      configuredWorkflows: () => [workflow, monitorWorkflow],
      approvedWorkflows: () => [workflow, monitorWorkflow],
      sampleWorkflows: () => [],
      sampleWorkflowIds: () => [],
      workflowIsSample: () => false,
      managementBusyKey: () => null,
      removeSampleWorkflows: vi.fn(async () => ({ status: "noop", removed_count: 0 })),
      terminalRuns: () => n8nMockState.runs.filter((run) => run.terminal),
      runningRuns: () => n8nMockState.runs.filter((run) => !run.terminal),
      runs: () => n8nMockState.runs,
      savedRuntimeProfiles: () => [],
      runtimeProfileDrafts: () => [],
      lastProfileSyncAt: () => 1780000000000,
      filteredWorkflows: () => [workflow, monitorWorkflow],
      loading: () => false,
      error: () => "",
      search: () => "",
      statusFilter: () => "all",
      riskFilter: () => "all",
      environmentFilter: () => "all",
      setSearch: vi.fn(),
      setStatusFilter: vi.fn(),
      setRiskFilter: vi.fn(),
      setEnvironmentFilter: vi.fn(),
      latestRunForWorkflow: () => undefined,
      governanceForRun: () => undefined,
      governanceForWorkflow: () => undefined,
      deadLettersByWorkflowId: () => new Map(),
      runningWorkflowId: () => null,
      resumingHitlCorrelationId: () => null,
      workflowSuggestion: () => n8nMockState.workflowSuggestion,
      initialize: initializeMock,
      refresh: refreshMock,
      suggestWorkflows: suggestWorkflowsMock,
      clearWorkflowSuggestion: vi.fn(() => {
        n8nMockState.workflowSuggestion = null;
      }),
      preparedWorkflowInput: () => null,
      prepareWorkflowInput: prepareWorkflowInputMock,
      clearPreparedWorkflowInput: vi.fn(),
      runWorkflow: runWorkflowMock,
      listWorkflowExecutions: listWorkflowExecutionsMock,
      viewWorkflowExecution: viewWorkflowExecutionMock,
      resumeWaitingExecution: resumeWaitingExecutionMock,
      reconcileRun: vi.fn(async () => undefined),
    },
  };
});

import N8nWorkflowHub from "./N8nWorkflowHub";

describe("N8nWorkflowHub smoke", () => {
  beforeEach(() => {
    runWorkflowMock.mockClear();
    refreshMock.mockClear();
    initializeMock.mockClear();
    suggestWorkflowsMock.mockClear();
    prepareWorkflowInputMock.mockClear();
    listWorkflowExecutionsMock.mockClear();
    viewWorkflowExecutionMock.mockClear();
    resumeWaitingExecutionMock.mockClear();
    n8nMockState.workflowSuggestion = null;
    n8nMockState.runs = [];
  });

  it("renders the native workflow hub and asks for confirmation before running", async () => {
    render(() => <N8nWorkflowHub />);

    expect(screen.getByText("Automations from n8n")).toBeInTheDocument();
    expect(screen.getByText("Test Workflow")).toBeInTheDocument();
    expect(screen.queryByTestId("n8n-diagnostics-smoke")).not.toBeInTheDocument();
    expect(screen.queryByTestId("n8n-management-profiles-smoke")).not.toBeInTheDocument();
    expect(screen.queryByTestId("n8n-management-advanced-smoke")).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Review" }));

    expect(suggestWorkflowsMock).toHaveBeenCalledWith("Run test_workflow");
    expect(runWorkflowMock).not.toHaveBeenCalled();
  });

  it("prepares structured workflow input before final execution", async () => {
    n8nMockState.workflowSuggestion = {
      schema_version: "kria.n8n.workflow_suggestion.v1",
      prompt: "Run test_workflow",
      reference: "test_workflow",
      status: "needs_confirmation",
      candidates: [
        {
          workflow_id: "test_workflow",
          workflow_version: "v1",
          display_name: "Test Workflow",
          category: "diagnostic",
          risk_tier: "Green",
          status: "approved",
          hitl_policy: "none",
          score: 100,
          confidence: 1,
          confidence_label: "high",
          matched_on: ["workflow_id"],
          requires_confirmation: true,
          reason: "Exact workflow_id match",
        },
      ],
      requires_confirmation: true,
      can_auto_run: false,
      ambiguous: false,
      hard_prompt: false,
      message: "I found Test Workflow. Confirm before I run it.",
      confirmation_hint: "Confirm with: Confirm workflow test_workflow",
    };
    render(() => <N8nWorkflowHub />);

    await fireEvent.click(screen.getByRole("button", { name: /Confirm/i }));

    expect(prepareWorkflowInputMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflow_id: "test_workflow" }),
      "Run test_workflow",
      expect.any(Object),
      true,
    );
    expect(await screen.findByText(/Review input for Test Workflow/i)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Run with this input" }));

    expect(runWorkflowMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflow_id: "test_workflow" }),
      expect.objectContaining({ genre: "action" }),
      "",
      true,
    );
  });

  it("switches dashboard tabs and keeps advanced data hidden from the normal dashboard", async () => {
    render(() => <N8nWorkflowHub />);

    expect(screen.getByRole("button", { name: /Ready to Run/i })).toHaveClass("active");
    expect(screen.queryByTestId("n8n-diagnostics-smoke")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Advanced/i })).not.toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: /Add from n8n/i }));
    expect(screen.getByTestId("n8n-management-profiles-smoke")).toBeInTheDocument();
    expect(screen.queryByTestId("n8n-diagnostics-smoke")).not.toBeInTheDocument();
  });

  it("opens monitor execution history instead of workflow suggestion confirmation", async () => {
    render(() => <N8nWorkflowHub />);

    await fireEvent.click(screen.getByRole("button", { name: "View Executions" }));

    expect(listWorkflowExecutionsMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflow_id: "daily_digest", result_mode: "monitor_only" }),
      0,
      10,
    );
    expect(screen.getByText("Latest digest output")).toBeInTheDocument();
    expect(runWorkflowMock).not.toHaveBeenCalled();
    expect(suggestWorkflowsMock).not.toHaveBeenCalled();
  });

  it("runs monitor-only workflows now through the runner when requested", async () => {
    render(() => <N8nWorkflowHub />);

    await fireEvent.click(screen.getByRole("button", { name: "Run Now" }));

    expect(runWorkflowMock).toHaveBeenCalledWith(
      expect.objectContaining({ workflow_id: "daily_digest", result_mode: "monitor_only" }),
      expect.objectContaining({ source: "kria_monitor_run_now" }),
      "run_now",
      false,
    );
  });

  it("shows HITL resume actions for waiting runs and sends approval", async () => {
    n8nMockState.runs = [
      {
        correlation_id: "corr-waiting-1",
        workflow_id: "test_workflow",
        workflow_version: "v1",
        n8n_run_id: "123",
        status: "waiting_for_approval",
        evidence_log: [
          {
            result: "n8n execution is waiting for approval or resume.",
            phase: "waiting_for_approval",
            hitl_resume: {
              available: true,
              method: "POST",
              warnings: [],
            },
          },
        ],
        side_effects: [],
        terminal: false,
      },
    ];

    render(() => <N8nWorkflowHub />);

    await fireEvent.click(screen.getByRole("button", { name: /Run History/i }));
    await fireEvent.click(screen.getByText("test_workflow"));

    expect(screen.getByText("Workflow is waiting for your decision")).toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Approve and continue" }));

    expect(resumeWaitingExecutionMock).toHaveBeenCalledWith(
      expect.objectContaining({ correlation_id: "corr-waiting-1" }),
      "approve",
      expect.objectContaining({ source: "kria_n8n_dashboard" }),
    );
  });
});
