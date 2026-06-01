import { Component, For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import N8nEvidenceViewer from "./N8nEvidenceViewer";
import N8nRunProgress from "./N8nRunProgress";
import N8nRunTimeline from "./N8nRunTimeline";
import N8nSettings from "./N8nSettings";
import N8nWorkflowCard from "./N8nWorkflowCard";
import N8nWorkflowManagementPanel from "./N8nWorkflowManagementPanel";
import WorkflowSuggestionCard from "./WorkflowSuggestionCard";
import {
  friendlyN8nError,
  n8nStore,
  type N8nPreparedWorkflowInput,
  type N8nRunState,
  type N8nWorkflow,
  type N8nWorkflowExecutionPage,
  type N8nWorkflowExecutionSummary,
  type WorkflowCandidate,
} from "../stores/n8n";

type N8nDashboardTab = "connect" | "workflows" | "add_workflow" | "runs";

function normalize(value?: string): string {
  return String(value ?? "").trim().toLowerCase();
}

function shortId(id?: string): string {
  if (!id) return "pending";
  return id.length > 14 ? `${id.slice(0, 8)}...${id.slice(-4)}` : id;
}

function shortTime(ms?: number | null): string {
  if (!ms) return "not synced";
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function executionTime(ms?: number | null): string {
  if (!ms) return "unknown time";
  return new Date(ms).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function durationLabel(ms?: number | null): string {
  if (!ms) return "";
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest ? `${minutes}m ${rest}s` : `${minutes}m`;
}

function compactValue(value: unknown): string {
  if (value === null || value === undefined) return "empty";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  try {
    const serialized = JSON.stringify(value);
    return serialized.length > 180 ? `${serialized.slice(0, 180)}...` : serialized;
  } catch {
    return String(value);
  }
}

function latestEvidence(run?: N8nRunState): any {
  const evidence = run?.evidence_log ?? [];
  return evidence.length > 0 ? evidence[evidence.length - 1] : {};
}

function runNeedsHitlResume(run?: N8nRunState): boolean {
  if (!run) return false;
  const status = normalize(run.status);
  const phase = normalize(latestEvidence(run)?.phase);
  return status === "waiting_for_approval" || phase.includes("waiting_for_approval");
}

function hitlResumeInfo(run?: N8nRunState): any {
  return latestEvidence(run)?.hitl_resume ?? {};
}

const N8nWorkflowHub: Component = () => {
  const [activeTab, setActiveTab] = createSignal<N8nDashboardTab>("workflows");
  const [selectedCorrelationId, setSelectedCorrelationId] = createSignal<string | undefined>();
  const [runError, setRunError] = createSignal("");
  const [routePrompt, setRoutePrompt] = createSignal("");
  const [preparedInput, setPreparedInput] = createSignal<N8nPreparedWorkflowInput | undefined>();
  const [preparedWorkflow, setPreparedWorkflow] = createSignal<N8nWorkflow | undefined>();
  const [preparedRunMode, setPreparedRunMode] = createSignal("");
  const [inputMappingBusy, setInputMappingBusy] = createSignal(false);
  const [confirmingRemoveSamples, setConfirmingRemoveSamples] = createSignal(false);
  const [sampleMessage, setSampleMessage] = createSignal("");
  const [historyWorkflowId, setHistoryWorkflowId] = createSignal<string | undefined>();
  const [historyPage, setHistoryPage] = createSignal<N8nWorkflowExecutionPage | undefined>();
  const [historyLoading, setHistoryLoading] = createSignal(false);
  const [historyError, setHistoryError] = createSignal("");
  const [hitlActionMessage, setHitlActionMessage] = createSignal("");
  const status = n8nStore.status;
  const sampleCount = createMemo(() => n8nStore.sampleWorkflows().length);
  const removingSamples = createMemo(() => n8nStore.managementBusyKey() === "samples:remove");

  const selectedRun = createMemo(() => {
    const id = selectedCorrelationId();
    if (!id) return undefined;
    return n8nStore.runs().find((run) => run.correlation_id === id);
  });
  const selectedGovernance = createMemo(() => n8nStore.governanceForRun(selectedRun()?.correlation_id));
  const selectedWorkflow = createMemo(() =>
    n8nStore.configuredWorkflows().find((workflow) => workflow.workflow_id === selectedRun()?.workflow_id)
  );
  const visibleRuns = createMemo(() => n8nStore.runs().slice(0, 12));
  const workflowCount = createMemo(() => n8nStore.configuredWorkflows().length);
  const approvedCount = createMemo(() => n8nStore.approvedWorkflows().length);
  const visibleWorkflowCards = createMemo(() =>
    n8nStore.filteredWorkflows().filter((workflow) => normalize(workflow.status) === "approved")
  );
  const runCount = createMemo(() => n8nStore.runs().length);
  const terminalCount = createMemo(() => n8nStore.terminalRuns().length);
  const activeCount = createMemo(() => n8nStore.runningRuns().length);
  const apiKeyPresent = createMemo(() => Boolean(n8nStore.runtimeStatus()?.secret_sources?.api_key?.present));
  const lastConnection = createMemo(() => n8nStore.runtimeStatus()?.runtime?.last_connection);
  const runnerLabel = createMemo(() => {
    const runtime = n8nStore.runtimeStatus();
    if (runtime?.mode === "managed_docker") {
      return runtime.runtime?.container?.running ? "Docker ready" : "Docker needs start";
    }
    return "External n8n";
  });
  const historyWorkflow = createMemo(() =>
    n8nStore.configuredWorkflows().find((workflow) => workflow.workflow_id === historyWorkflowId())
  );
  const preparedPayloadEntries = createMemo(() => {
    const payload = preparedInput()?.input_payload;
    if (!payload || typeof payload !== "object" || Array.isArray(payload)) return [];
    return Object.entries(payload)
      .filter(([key]) => key !== "confirmed_by_user")
      .slice(0, 10);
  });

  onMount(() => {
    void n8nStore.initialize();
    const timer = setInterval(() => void n8nStore.refresh(), 5000);
    onCleanup(() => clearInterval(timer));
  });

  async function executeWorkflow(workflow: N8nWorkflow, inputPayload?: any, runMode = "", inputMapped = false) {
    setRunError("");
    try {
      const hasInputPayload =
        inputPayload &&
        typeof inputPayload === "object" &&
        Object.keys(inputPayload).length > 0;
      const result = await n8nStore.runWorkflow(
        workflow,
        hasInputPayload
          ? inputPayload
          : {
              source: "kria_workflow_hub",
              workflow_id: workflow.workflow_id,
              requested_at_ms: Date.now(),
            },
        runMode,
        inputMapped
      );
      const optimistic = n8nStore.runs().find((item) => item.correlation_id === result.correlation_id);
      setSelectedCorrelationId(optimistic?.correlation_id ?? result.correlation_id);
    } catch (err) {
      setRunError(String(err));
    }
  }

  async function removeSamples() {
    setSampleMessage("");
    setConfirmingRemoveSamples(false);
    try {
      const result = await n8nStore.removeSampleWorkflows();
      setSampleMessage(result?.message || "Sample workflows removed.");
    } catch (err) {
      setSampleMessage(friendlyN8nError(err));
    }
  }

  async function routeWorkflowPrompt() {
    setRunError("");
    try {
      await n8nStore.suggestWorkflows(routePrompt());
    } catch (err) {
      setRunError(String(err));
    }
  }

  async function prepareWorkflowRunInput(
    workflow: N8nWorkflow,
    prompt: string,
    basePayload: any = {},
    runMode = "",
  ) {
    setRunError("");
    setInputMappingBusy(true);
    try {
      const prepared = await n8nStore.prepareWorkflowInput(workflow, prompt, basePayload, true);
      setPreparedWorkflow(workflow);
      setPreparedRunMode(runMode);
      setPreparedInput(prepared);
    } catch (err) {
      setRunError(friendlyN8nError(err));
    } finally {
      setInputMappingBusy(false);
    }
  }

  async function confirmCandidate(candidate: WorkflowCandidate) {
    const workflow = n8nStore
      .configuredWorkflows()
      .find((item) => item.workflow_id === candidate.workflow_id);
    if (!workflow) {
      setRunError(`Workflow ${candidate.workflow_id} is no longer configured.`);
      return;
    }
    const prompt = n8nStore.workflowSuggestion()?.prompt || routePrompt() || `Run ${candidate.workflow_id}`;
    await prepareWorkflowRunInput(workflow, prompt, candidate.suggested_input_payload ?? {});
  }

  async function runPreparedInput() {
    const workflow = preparedWorkflow();
    const prepared = preparedInput();
    if (!workflow || !prepared) return;
    const missing = prepared.missing_inputs ?? [];
    const validation = prepared.validation_issues ?? [];
    if (missing.length || validation.length || normalize(prepared.status) !== "ready") {
      setRunError("Workflow input still needs review before running.");
      return;
    }
    n8nStore.clearWorkflowSuggestion();
    setPreparedInput(undefined);
    setPreparedWorkflow(undefined);
    await executeWorkflow(workflow, prepared.input_payload, preparedRunMode(), true);
  }

  async function loadWorkflowExecutions(workflow: N8nWorkflow, offset = 0, append = false) {
    setHistoryWorkflowId(workflow.workflow_id);
    setHistoryError("");
    setHistoryLoading(true);
    try {
      const page = await n8nStore.listWorkflowExecutions(workflow, offset, 10);
      setHistoryPage((previous) => {
        if (!append || !previous || previous.workflow_id !== page.workflow_id) return page;
        return {
          ...page,
          executions: [...previous.executions, ...page.executions],
        };
      });
    } catch (err) {
      setHistoryError(friendlyN8nError(err));
    } finally {
      setHistoryLoading(false);
    }
  }

  async function openWorkflowExecution(workflow: N8nWorkflow, execution: N8nWorkflowExecutionSummary) {
    if (!execution.n8n_execution_id) return;
    setHistoryError("");
    try {
      const result = await n8nStore.viewWorkflowExecution(workflow, execution.n8n_execution_id);
      setSelectedCorrelationId(result?.correlation_id);
      setActiveTab("runs");
    } catch (err) {
      setHistoryError(friendlyN8nError(err));
    }
  }

  async function resumeSelectedRun(decision: "approve" | "reject") {
    const run = selectedRun();
    if (!run) return;
    setHitlActionMessage("");
    try {
      const result = await n8nStore.resumeWaitingExecution(run, decision, {
        source: "kria_n8n_dashboard",
      });
      setHitlActionMessage(result?.message || "Decision sent to n8n. KRIA is polling the resumed execution.");
    } catch (err) {
      setHitlActionMessage(friendlyN8nError(err));
    }
  }

  async function runNow(workflow: N8nWorkflow) {
    await executeWorkflow(
      workflow,
      {
        source: "kria_monitor_run_now",
        workflow_id: workflow.workflow_id,
        requested_at_ms: Date.now(),
      },
      "run_now"
    );
  }

  async function run(workflow: N8nWorkflow) {
    setRunError("");
    if (normalize(workflow.result_mode) === "monitor_only") {
      await loadWorkflowExecutions(workflow, 0, false);
      return;
    }
    try {
      const prompt = `Run ${workflow.workflow_id}`;
      const suggestion = await n8nStore.suggestWorkflows(prompt);
      if (suggestion.candidates.length === 1 && suggestion.candidates[0].workflow_id === workflow.workflow_id) {
        setRoutePrompt(prompt);
      }
    } catch (err) {
      setRunError(String(err));
    }
  }

  const tabs: { id: N8nDashboardTab; label: string; summary: () => string }[] = [
    { id: "connect", label: "Connect n8n", summary: () => apiKeyPresent() ? "API key saved" : "setup" },
    { id: "workflows", label: "Ready to Run", summary: () => `${approvedCount()} approved` },
    { id: "add_workflow", label: "Add from n8n", summary: () => `${n8nStore.savedRuntimeProfiles().length} saved` },
    { id: "runs", label: "Run History", summary: () => `${runCount()} runs` },
  ];

  return (
    <section class="n8n-hub">
      <div class="n8n-hub-header">
        <div>
          <span class="n8n-hub-eyebrow">n8n Setup</span>
          <h3>Automations from n8n</h3>
          <p>Bring workflows from n8n into KRIA, let AI fill the setup details, then run only approved automations.</p>
        </div>
        <div class="n8n-hub-actions">
          <button class="btn-secondary" disabled={n8nStore.loading()} onClick={() => void n8nStore.refresh()}>
            Refresh
          </button>
        </div>
      </div>

      <Show when={n8nStore.error() || runError()}>
        <div class="startup-warning-banner">
          <strong>n8n:</strong> {friendlyN8nError(runError() || n8nStore.error())}
        </div>
      </Show>

      <div class="n8n-health-strip">
        <div>
          <span>n8n</span>
          <strong>{lastConnection()?.status || (status()?.enabled ? "Enabled" : "Needs setup")}</strong>
          <small>{n8nStore.runtimeStatus()?.base_url || status()?.base_url || "base URL unknown"}</small>
        </div>
        <div>
          <span>API</span>
          <strong>{apiKeyPresent() ? "Key present" : "Key missing"}</strong>
          <small>{lastConnection()?.message || "Use Connect n8n to test API access"}</small>
        </div>
        <div>
          <span>Runner</span>
          <strong>{runnerLabel()}</strong>
          <small>{n8nStore.runtimeStatus()?.mode || "mode unknown"}</small>
        </div>
        <div>
          <span>Workflows</span>
          <strong>{workflowCount()}</strong>
          <small>{approvedCount()} approved</small>
        </div>
        <div>
          <span>Runs</span>
          <strong>{runCount()} total</strong>
          <small>{activeCount()} active · {terminalCount()} completed</small>
        </div>
        <div>
          <span>Last check</span>
          <strong>{shortTime(lastConnection()?.checked_at_ms)}</strong>
          <small>{n8nStore.savedRuntimeProfiles().length} profiles · synced {shortTime(n8nStore.lastProfileSyncAt())}</small>
        </div>
      </div>

      <nav class="n8n-tab-list" aria-label="n8n dashboard sections">
        <For each={tabs}>
          {(tab) => (
            <button
              type="button"
              class={`n8n-tab ${activeTab() === tab.id ? "active" : ""}`}
              aria-current={activeTab() === tab.id ? "page" : undefined}
              onClick={() => setActiveTab(tab.id)}
            >
              <strong>{tab.label}</strong>
              <span>{tab.summary()}</span>
            </button>
          )}
        </For>
      </nav>

      <Show when={activeTab() === "connect"}>
        <section class="n8n-tab-panel">
          <N8nSettings />
        </section>
      </Show>

      <Show when={activeTab() === "workflows"}>
        <section class="n8n-tab-panel">
          <Show when={sampleCount() > 0}>
            <div class="n8n-sample-banner" role="note">
              <div class="n8n-sample-banner-text">
                <strong>{sampleCount()} sample workflow{sampleCount() === 1 ? "" : "s"} are pre-loaded for testing.</strong>
                <small>KRIA added these demos (Gmail, Calendar, Slack, diagnostics). They aren't workflows you created. You can remove them anytime.</small>
              </div>
              <div class="n8n-sample-banner-actions">
                <Show
                  when={confirmingRemoveSamples()}
                  fallback={
                    <button type="button" class="btn-secondary" disabled={removingSamples()} onClick={() => setConfirmingRemoveSamples(true)}>
                      Remove samples
                    </button>
                  }
                >
                  <span class="n8n-sample-confirm-text">Remove all {sampleCount()} samples?</span>
                  <button type="button" class="btn-secondary danger" disabled={removingSamples()} onClick={() => void removeSamples()}>
                    {removingSamples() ? "Removing..." : "Yes, remove"}
                  </button>
                  <button type="button" class="btn-secondary" disabled={removingSamples()} onClick={() => setConfirmingRemoveSamples(false)}>
                    Cancel
                  </button>
                </Show>
              </div>
            </div>
          </Show>
          <Show when={sampleMessage()}>
            <div class="n8n-management-message ok" role="status">{sampleMessage()}</div>
          </Show>
          <section class="n8n-routing-panel">
            <div class="n8n-section-head">
              <h4>Ask KRIA to pick a workflow</h4>
              <span>Review before run</span>
            </div>
            <div class="n8n-route-form">
              <input
                type="text"
                value={routePrompt()}
                onInput={(event) => setRoutePrompt(event.currentTarget.value)}
                placeholder="Example: summarize my inbox"
              />
              <button type="button" class="btn-secondary" disabled={n8nStore.loading()} onClick={() => void routeWorkflowPrompt()}>
                Find match
              </button>
            </div>
            <Show when={n8nStore.workflowSuggestion()}>
              {(suggestion) => (
                <WorkflowSuggestionCard
                  suggestion={suggestion()}
                  busy={n8nStore.runningWorkflowId() != null || inputMappingBusy()}
                  onConfirm={(candidate) => void confirmCandidate(candidate)}
                  onCancel={() => n8nStore.clearWorkflowSuggestion()}
                />
              )}
            </Show>
            <Show when={inputMappingBusy()}>
              <div class="n8n-input-preview loading" role="status">
                <strong>Preparing workflow input...</strong>
                <span>KRIA is waking the configured LLM if needed and converting your prompt into safe JSON.</span>
              </div>
            </Show>
            <Show when={preparedInput()}>
              {(prepared) => (
                <div class="n8n-input-preview">
                  <div class="n8n-input-preview-head">
                    <div>
                      <strong>Review input for {prepared().display_name || prepared().workflow_id}</strong>
                      <span>{prepared().message || "KRIA prepared JSON input from your prompt."}</span>
                    </div>
                    <span class={`n8n-status-pill ${normalize(prepared().status) === "ready" ? "ok" : "warning"}`}>
                      {normalize(prepared().status) === "ready" ? "Ready" : "Needs input"}
                    </span>
                  </div>
                  <div class="n8n-input-meta">
                    <span>Source: {prepared().source === "llm_active_provider" ? `LLM${prepared().model ? ` (${prepared().model})` : ""}` : "Heuristic fallback"}</span>
                    <span>Confidence: {Math.round((prepared().confidence ?? 0) * 100)}%</span>
                  </div>
                  <Show when={(prepared().missing_inputs ?? []).length > 0 || (prepared().validation_issues ?? []).length > 0}>
                    <div class="n8n-run-warning">
                      <strong>Fix before running:</strong>{" "}
                      {[...(prepared().missing_inputs ?? []).map((item) => `Missing ${item}`), ...(prepared().validation_issues ?? [])].join("; ")}
                    </div>
                  </Show>
                  <div class="n8n-input-field-list">
                    <For each={preparedPayloadEntries()}>
                      {([key, value]) => (
                        <div>
                          <span>{key}</span>
                          <strong>{compactValue(value)}</strong>
                        </div>
                      )}
                    </For>
                  </div>
                  <details class="n8n-technical-details">
                    <summary>Show JSON payload</summary>
                    <pre>{JSON.stringify(prepared().input_payload, null, 2)}</pre>
                  </details>
                  <div class="n8n-input-actions">
                    <button
                      type="button"
                      class="btn-primary"
                      disabled={
                        n8nStore.runningWorkflowId() != null ||
                        normalize(prepared().status) !== "ready" ||
                        (prepared().missing_inputs ?? []).length > 0 ||
                        (prepared().validation_issues ?? []).length > 0
                      }
                      onClick={() => void runPreparedInput()}
                    >
                      Run with this input
                    </button>
                    <button
                      type="button"
                      class="btn-secondary"
                      disabled={n8nStore.runningWorkflowId() != null}
                      onClick={() => {
                        setPreparedInput(undefined);
                        setPreparedWorkflow(undefined);
                        n8nStore.clearPreparedWorkflowInput();
                      }}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}
            </Show>
          </section>

          <div class="n8n-filter-row">
            <label class="n8n-search-field">
              <span>Search</span>
              <input
                type="search"
                value={n8nStore.search()}
                onInput={(event) => n8nStore.setSearch(event.currentTarget.value)}
                placeholder="Workflow ID, name, action"
              />
            </label>
            <label>
              <span>Status</span>
              <select value={n8nStore.statusFilter()} onChange={(event) => n8nStore.setStatusFilter(event.currentTarget.value as any)}>
                <option value="all">All</option>
                <option value="approved">Approved</option>
                <option value="draft">Draft</option>
                <option value="test">Test</option>
                <option value="disabled">Disabled</option>
                <option value="deprecated">Deprecated</option>
              </select>
            </label>
            <label>
              <span>Risk</span>
              <select value={n8nStore.riskFilter()} onChange={(event) => n8nStore.setRiskFilter(event.currentTarget.value as any)}>
                <option value="all">All</option>
                <option value="green">Green</option>
                <option value="yellow">Yellow</option>
                <option value="red">Red</option>
              </select>
            </label>
            <label>
              <span>Environment</span>
              <select value={n8nStore.environmentFilter()} onChange={(event) => n8nStore.setEnvironmentFilter(event.currentTarget.value as any)}>
                <option value="all">All</option>
                <option value="dev">Dev</option>
                <option value="staging">Staging</option>
                <option value="production">Production</option>
                <option value="destructive_eval">Destructive eval</option>
              </select>
            </label>
          </div>

          <div class="n8n-workflow-grid">
            <Show
              when={visibleWorkflowCards().length > 0}
              fallback={
                <div class="n8n-empty n8n-empty-action">
                  <strong>No approved n8n workflows yet.</strong>
                  <Show
                    when={n8nStore.savedRuntimeProfiles().length > 0}
                    fallback={<span>Go to Add from n8n, click Sync, then let AI prepare the setup.</span>}
                  >
                    <span>You have saved workflow profiles, but none are approved yet. Go to Add from n8n to finish setup.</span>
                  </Show>
                  <button type="button" class="btn-secondary" onClick={() => setActiveTab("add_workflow")}>
                    Add from n8n
                  </button>
                </div>
              }
            >
              <For each={visibleWorkflowCards()}>
                {(workflow) => (
                  <N8nWorkflowCard
                    workflow={workflow}
                    latestRun={n8nStore.latestRunForWorkflow(workflow.workflow_id)}
                    deadLetterCount={n8nStore.deadLettersByWorkflowId().get(workflow.workflow_id)?.length ?? 0}
                    running={
                      n8nStore.runningWorkflowId() === workflow.workflow_id ||
                      (historyLoading() && historyWorkflowId() === workflow.workflow_id)
                    }
                    isSample={n8nStore.workflowIsSample(workflow.workflow_id)}
                    onRun={run}
                    onRunNow={runNow}
                  />
                )}
              </For>
            </Show>
          </div>

          <Show when={historyWorkflow()}>
            {(workflow) => (
              <section class="n8n-monitor-history">
                <div class="n8n-section-head">
                  <div>
                    <h4>{workflow().display_name || workflow().workflow_id} executions</h4>
                    <small>Latest n8n executions first. Viewing history does not start the workflow.</small>
                  </div>
                  <div class="n8n-history-actions">
                    <button
                      type="button"
                      class="btn-secondary"
                      disabled={historyLoading()}
                      onClick={() => void loadWorkflowExecutions(workflow(), 0, false)}
                    >
                      {historyLoading() ? "Refreshing..." : "Refresh"}
                    </button>
                    <button
                      type="button"
                      class="btn-secondary"
                      onClick={() => {
                        setHistoryWorkflowId(undefined);
                        setHistoryPage(undefined);
                        setHistoryError("");
                      }}
                    >
                      Close
                    </button>
                  </div>
                </div>
                <Show when={historyError()}>
                  <div class="n8n-run-warning">{historyError()}</div>
                </Show>
                <Show
                  when={(historyPage()?.executions?.length ?? 0) > 0}
                  fallback={<div class="n8n-empty">{historyLoading() ? "Loading executions..." : "No n8n executions found yet."}</div>}
                >
                  <div class="n8n-execution-history-list">
                    <For each={historyPage()?.executions ?? []}>
                      {(execution) => (
                        <div class="n8n-execution-history-row">
                          <span class={`n8n-status-dot ${normalize(execution.status) === "completed" ? "ok" : normalize(execution.status) === "failed" ? "danger" : "waiting"}`} />
                          <div class="n8n-run-main">
                            <strong>{execution.status || "unknown"} · {shortId(execution.n8n_execution_id)}</strong>
                            <small>
                              {executionTime(execution.started_at_ms)}
                              {durationLabel(execution.duration_ms) ? ` · ${durationLabel(execution.duration_ms)}` : ""}
                              {execution.output_source ? ` · ${execution.output_source}` : ""}
                            </small>
                            <small>{execution.result_preview || "No output preview yet."}</small>
                          </div>
                          <button
                            type="button"
                            class="btn-secondary"
                            disabled={!execution.n8n_execution_id || n8nStore.runningWorkflowId() === workflow().workflow_id}
                            onClick={() => void openWorkflowExecution(workflow(), execution)}
                          >
                            View result
                          </button>
                        </div>
                      )}
                    </For>
                  </div>
                  <Show when={historyPage()?.has_more}>
                    <button
                      type="button"
                      class="btn-secondary n8n-load-more"
                      disabled={historyLoading()}
                      onClick={() => void loadWorkflowExecutions(workflow(), historyPage()?.executions.length ?? 0, true)}
                    >
                      {historyLoading() ? "Loading..." : "Load previous 10"}
                    </button>
                  </Show>
                </Show>
              </section>
            )}
          </Show>
        </section>
      </Show>

      <Show when={activeTab() === "add_workflow"}>
        <section class="n8n-tab-panel">
          <div class="n8n-layman-flow" role="note">
            <div>
              <strong>1. Sync</strong>
              <small>KRIA reads workflow names and safe structure from n8n.</small>
            </div>
            <div>
              <strong>2. Prepare with AI</strong>
              <small>Your configured LLM wakes up if needed and suggests plain-English details.</small>
            </div>
            <div>
              <strong>3. Review</strong>
              <small>KRIA highlights anything risky or unclear before saving.</small>
            </div>
            <div>
              <strong>4. Save</strong>
              <small>Safe metadata is stored locally; n8n workflows are not modified.</small>
            </div>
          </div>
          <N8nWorkflowManagementPanel view="profiles" />
        </section>
      </Show>

      <Show when={activeTab() === "runs"}>
        <section class="n8n-tab-panel n8n-run-layout">
          <section class="n8n-run-panel">
            <div class="n8n-section-head">
              <h4>Recent Runs</h4>
              <span>{visibleRuns().length}</span>
            </div>
            <N8nRunTimeline
              runs={visibleRuns()}
              selectedCorrelationId={selectedCorrelationId()}
              busy={n8nStore.loading()}
              onSelect={(run) => setSelectedCorrelationId(run.correlation_id)}
              onReconcile={(correlationId) => void n8nStore.reconcileRun(correlationId)}
            />
          </section>

          <section class="n8n-run-panel">
            <div class="n8n-section-head">
              <h4>Selected Result</h4>
              <span>{selectedRun() ? shortId(selectedRun()?.correlation_id) : "none"}</span>
            </div>
            <Show when={selectedRun()} fallback={<div class="n8n-empty">Select a run to view evidence and governance.</div>}>
              {(run) => (
                <>
                  <div class="n8n-selected-run">
                    <strong>{run().workflow_id}</strong>
                    <span>
                      {run().terminal
                        ? run().status
                        : normalize(selectedWorkflow()?.result_mode) === "monitor_only"
                          ? "monitoring_execution"
                          : selectedWorkflow()?.requires_callback === false
                            ? "polling_execution"
                            : normalize(run().status) === "accepted"
                              ? "waiting_for_callback"
                              : run().status}
                    </span>
                    <small>{run().correlation_id}</small>
                  </div>
                  <N8nRunProgress run={run()} workflow={selectedWorkflow()} governance={selectedGovernance()} />
                  <Show when={runNeedsHitlResume(run())}>
                    <div class="n8n-hitl-resume-card">
                      <div>
                        <strong>Workflow is waiting for your decision</strong>
                        <small>
                          {hitlResumeInfo(run())?.available
                            ? `Resume link detected · ${hitlResumeInfo(run())?.method || "POST"}`
                            : "Resume link was not found in n8n execution details."}
                        </small>
                        <Show when={(hitlResumeInfo(run())?.warnings ?? []).length > 0}>
                          <ul>
                            <For each={hitlResumeInfo(run())?.warnings ?? []}>
                              {(warning: string) => <li>{warning}</li>}
                            </For>
                          </ul>
                        </Show>
                        <Show when={hitlActionMessage()}>
                          <small class="n8n-hitl-action-message">{hitlActionMessage()}</small>
                        </Show>
                      </div>
                      <div class="n8n-hitl-resume-actions">
                        <button
                          type="button"
                          class="btn-primary"
                          disabled={
                            !hitlResumeInfo(run())?.available ||
                            n8nStore.resumingHitlCorrelationId() === run().correlation_id
                          }
                          onClick={() => void resumeSelectedRun("approve")}
                        >
                          {n8nStore.resumingHitlCorrelationId() === run().correlation_id
                            ? "Sending..."
                            : "Approve and continue"}
                        </button>
                        <button
                          type="button"
                          class="btn-secondary danger"
                          disabled={
                            !hitlResumeInfo(run())?.available ||
                            n8nStore.resumingHitlCorrelationId() === run().correlation_id
                          }
                          onClick={() => void resumeSelectedRun("reject")}
                        >
                          Reject and continue
                        </button>
                      </div>
                    </div>
                  </Show>
                  <N8nEvidenceViewer run={run()} governance={selectedGovernance()} />
                </>
              )}
            </Show>
          </section>
        </section>
      </Show>

    </section>
  );
};

export default N8nWorkflowHub;
