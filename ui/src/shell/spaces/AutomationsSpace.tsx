/**
 * Automations Space — segments + top-level workflow surfacing (task 7.1,
 * Req 6.1 / 6.2).
 *
 * Provides the four Automations segments as a real tablist (kit `Tabs`,
 * Kobalte-backed → correct tablist/tab/tabpanel roles + arrow-key nav,
 * Req 17.1/17.2), driven by the typed router `space=automations,
 * segment=<id>` (Req 1.3/1.5):
 *   • Run ....... the default; ask-KRIA-to-pick + WorkflowCards surfacing
 *                 WORKFLOWS AT THE TOP LEVEL (Req 6.2 — never buried behind a
 *                 dashboard sub-tab), with run progress + evidence + HITL →
 *                 Approval Center (task 7.2, Req 6.3/6.5 — see ./automations).
 *   • Build ..... 2D node builder canvas ............ task 7.3
 *   • Schedule .. scheduled tasks + routines + reminders + to-dos, merged into
 *                 one grouped view with real create/complete/snooze/delete
 *                 dispatch (task 7.4, Req 6.6 — see ./automations/ScheduleRegion).
 *   • History ... past run history + evidence
 *
 * Here each segment is a labelled region with a heading and either a basic list
 * (where `automationStore` already holds data) or an honest empty/loading
 * placeholder (filled by tasks 7.2–7.5).
 *
 * Pure presentation / read-model (KRIA runtime-authority invariant): reads
 * `automationStore` only — fed by the Tauri bridge from the existing
 * n8n / task / reminder commands + events. Running workflows are execution
 * substrate (n8n); this Space surfaces them, it does not orchestrate them
 * (no prompt→tool shortcut, no workflow mutation here — HITL/cancel wiring is
 * task 7.5). Workflow/task/reminder text is rendered as text (Solid
 * auto-escapes), never as HTML, so untrusted content cannot inject markup.
 *
 * Requirements: 6.1, 6.2
 */
import { createEffect, createMemo, createSignal, For, Show } from "solid-js";
import { automationStore, shellStore, type AutomationSegment } from "../../stores";
import { n8nStore, type N8nWorkflow, type N8nWorkflowExecutionPage } from "../../stores/n8n";
import { currentRoute, navigate } from "../router";
import { Button, EmptyState, StatusDot, Tabs } from "../../kit";
import { Icon } from "../../components/Icon";
import type { StatusTone } from "../../kit";
import type { WorkflowStatus } from "../../stores";
import { RunRegion, NodeBuilder, ScheduleRegion, HealthPanel, RegistryPanel } from "./automations";
import "./AutomationsSpace.css";

// ─── Segment model ───────────────────────────────────────────────────────────

interface SegmentDef {
  value: AutomationSegment;
  label: string;
}

/** The four Automations segments (Req 6.1). Run is first → the default. */
const SEGMENTS: readonly SegmentDef[] = [
  { value: "run", label: "Run" },
  { value: "build", label: "Build" },
  { value: "schedule", label: "Schedule" },
  { value: "history", label: "History" },
] as const;

function isAutomationSegment(value: string | undefined): value is AutomationSegment {
  return !!value && SEGMENTS.some((s) => s.value === value);
}

/** Resolve the routed segment, defaulting to Run (Req 1.5 / 6.2). */
function routedSegment(): AutomationSegment {
  const seg = currentRoute().segment;
  return isAutomationSegment(seg) ? seg : "run";
}

// ─── Space ─────────────────────────────────────────────────────────────────────

export default function AutomationsSpace() {
  // Seed the tablist from the route at mount so a deep link (e.g.
  // `automations/schedule`) opens the right segment (Req 1.5). The tablist then
  // owns selection + arrow-key nav (Kobalte); each switch re-routes below,
  // keeping the route the single address for the active segment. (Uncontrolled-
  // seeded-from-route — the Kobalte controlled-tabs limitation noted in 6.1.)
  const isCompact = createMemo(() => shellStore.windowMode() === "compact");

  // Mirror the routed segment into the store so downstream Run/Build/Schedule
  // tasks (7.2–7.4) read a single source of truth.
  createEffect(() => automationStore.setActiveSegment(routedSegment()));

  function selectSegment(value: string) {
    if (value === "run") navigate("automations");
    else navigate("automations", value);
  }

  const items = SEGMENTS.map((seg) => ({
    value: seg.value,
    label: seg.label,
    content: () => <SegmentRegion segment={seg.value} label={seg.label} />,
  }));

  return (
    <section class="kria-automations" data-space="automations" aria-label="Automations">
      <header class="kria-automations__header">
        <h1 class="kria-automations__title">Automations</h1>
        <p class="kria-automations__subtitle">
          Everything KRIA does on command or schedule.
        </p>
      </header>

      <Show
        when={isCompact()}
        fallback={
          <Tabs
            class="kria-automations__segments"
            items={items}
            value={routedSegment()}
            onChange={selectSegment}
          />
        }
      >
        <div class="kria-automations__compact" data-curated-primary="run">
          <RunRegion />
        </div>
      </Show>
    </section>
  );
}

// ─── Regions ─────────────────────────────────────────────────────────────────

function SegmentRegion(props: { segment: AutomationSegment; label: string }) {
  return (
    <div
      class="kria-automations__region"
      data-segment={props.segment}
      aria-label={props.label}
    >
      <Show when={props.segment === "run"}>
        <RunRegion />
      </Show>
      <Show when={props.segment === "build"}>
        <BuildRegion />
      </Show>
      <Show when={props.segment === "schedule"}>
        <ScheduleRegion />
      </Show>
      <Show when={props.segment === "history"}>
        <HistoryRegion />
      </Show>
    </div>
  );
}

/** Loading helper shared by data-backed regions (honest states, Req 6.1). */
function LoadingRow(props: { label: string }) {
  return (
    <div class="kria-automations__status" role="status" aria-live="polite">
      {props.label}
    </div>
  );
}

/** Map a workflow status to a StatusDot tone + human label (Req 17.3: not
 *  color-alone — the label always carries meaning). */
function statusPresentation(status: WorkflowStatus): { tone: StatusTone; label: string } {
  switch (status) {
    case "running":
      return { tone: "busy", label: "Running" };
    case "completed":
      return { tone: "online", label: "Completed" };
    case "failed":
      return { tone: "error", label: "Failed" };
    case "paused":
      return { tone: "info", label: "Paused" };
    case "idle":
    default:
      return { tone: "offline", label: "Idle" };
  }
}

/** Build — the 2D node builder canvas + palette + node Inspector (task 7.3,
 *  Req 6.3 / 6.4). Authoring dispatches through existing n8n commands. */
function BuildRegion() {
  return (
    <div class="kria-automations__build">
      <h2 class="kria-automations__region-title">Build</h2>
      <NodeBuilder />
      {/* Health/diagnostics (folds the orphaned N8nDiagnosticsPanel, Req 20.2)
          and the reachable advanced registry (Req 20.3) — task 7.5. */}
      <HealthPanel />
      <RegistryPanel />
    </div>
  );
}

/**
 * History — authoritative n8n execution pages plus KRIA's latest run read-model.
 * Viewing an execution dispatches through `view_n8n_workflow_execution`, which
 * imports redacted evidence/governance into KRIA without starting a workflow.
 */
function HistoryRegion() {
  const [selectedWorkflowId, setSelectedWorkflowId] = createSignal<string | null>(null);
  const [page, setPage] = createSignal<N8nWorkflowExecutionPage | null>(null);
  const [historyLoading, setHistoryLoading] = createSignal(false);
  const [historyError, setHistoryError] = createSignal("");
  const [resultMessage, setResultMessage] = createSignal("");
  const configured = createMemo(() => n8nStore.configuredWorkflows());
  const selectedWorkflow = createMemo(() =>
    configured().find((workflow) => workflow.workflow_id === selectedWorkflowId()),
  );
  const recent = createMemo(() =>
    automationStore
      .workflows()
      .filter((workflow) => workflow.lastRunAt !== null)
      .sort((a, b) => (b.lastRunAt ?? 0) - (a.lastRunAt ?? 0)),
  );

  async function loadExecutions(workflow: N8nWorkflow, offset = 0, append = false) {
    setSelectedWorkflowId(workflow.workflow_id);
    setHistoryError("");
    setResultMessage("");
    setHistoryLoading(true);
    try {
      const next = await n8nStore.listWorkflowExecutions(workflow, offset, 10);
      setPage((previous) =>
        append && previous?.workflow_id === next.workflow_id
          ? { ...next, executions: [...previous.executions, ...next.executions] }
          : next,
      );
    } catch (error) {
      setHistoryError(error instanceof Error ? error.message : String(error));
    } finally {
      setHistoryLoading(false);
    }
  }

  async function viewExecution(executionId: string) {
    const workflow = selectedWorkflow();
    if (!workflow || !executionId) return;
    setHistoryError("");
    setResultMessage("");
    try {
      const result = await n8nStore.viewWorkflowExecution(workflow, executionId);
      setResultMessage(
        result?.message
          || `Execution ${executionId} evidence and governance loaded into KRIA.`,
      );
    } catch (error) {
      setHistoryError(error instanceof Error ? error.message : String(error));
    }
  }

  const hasAnyHistory = createMemo(() => configured().length > 0 || recent().length > 0);

  return (
    <div class="kria-automations__history">
      <div class="kria-automations__history-head">
        <div>
          <h2 class="kria-automations__region-title">History</h2>
          <p class="kria-automations__muted">Read execution output and evidence without rerunning workflows.</p>
        </div>
      </div>

      <Show when={automationStore.loading()}>
        <LoadingRow label="Loading run history…" />
      </Show>

      <Show
        when={!automationStore.loading() && hasAnyHistory()}
        fallback={
          <Show when={!automationStore.loading()}>
            <EmptyState
              icon="history"
              title="No runs yet"
              description="Once a workflow runs, its history and evidence will appear here."
            />
          </Show>
        }
      >
        <Show when={configured().length > 0}>
          <section class="kria-automations__history-panel" aria-label="n8n execution history">
            <h3 class="kria-automations__panel-title">Workflow executions</h3>
            <div class="kria-automations__history-workflows" role="group" aria-label="Choose workflow history">
              <For each={configured()}>
                {(workflow) => (
                  <Button
                    variant={selectedWorkflowId() === workflow.workflow_id ? "primary" : "secondary"}
                    size="sm"
                    disabled={historyLoading()}
                    onClick={() => void loadExecutions(workflow)}
                  >
                    <Icon name="history" size={14} />
                    {workflow.display_name || workflow.workflow_id}
                  </Button>
                )}
              </For>
            </div>

            <Show when={historyError()}>
              <p class="kria-automations__history-error" role="alert">{historyError()}</p>
            </Show>
            <Show when={resultMessage()}>
              <p class="kria-automations__history-message" role="status">{resultMessage()}</p>
            </Show>
            <Show when={historyLoading()}>
              <LoadingRow label="Loading n8n executions…" />
            </Show>

            <Show when={!historyLoading() && selectedWorkflow() && page()}>
              <Show
                when={(page()?.executions.length ?? 0) > 0}
                fallback={<p class="kria-automations__muted">No n8n executions found for this workflow.</p>}
              >
                <ol class="kria-automations__history-list">
                  <For each={page()?.executions ?? []}>
                    {(execution) => {
                      const normalized = execution.status.toLowerCase();
                      const tone: StatusTone = normalized.includes("complete")
                        || normalized.includes("success")
                        ? "online"
                        : normalized.includes("fail") || normalized.includes("error")
                          ? "error"
                          : "info";
                      return (
                        <li class="kria-automations__execution">
                          <div class="kria-automations__execution-main">
                            <span class="kria-automations__history-when">
                              {execution.started_at_ms
                                ? new Date(execution.started_at_ms).toLocaleString()
                                : "Start time unavailable"}
                            </span>
                            <span class="kria-automations__history-what">
                              {execution.result_preview || `Execution ${execution.n8n_execution_id}`}
                            </span>
                            <span class="kria-automations__history-detail">
                              {execution.n8n_execution_id}
                              {execution.duration_ms != null ? ` · ${execution.duration_ms} ms` : ""}
                              {execution.output_source ? ` · ${execution.output_source}` : ""}
                            </span>
                          </div>
                          <StatusDot tone={tone} label={execution.status || "unknown"} />
                          <Button
                            variant="secondary"
                            size="sm"
                            disabled={n8nStore.runningWorkflowId() === selectedWorkflowId()}
                            onClick={() => void viewExecution(execution.n8n_execution_id)}
                          >
                            View evidence
                          </Button>
                        </li>
                      );
                    }}
                  </For>
                </ol>
                <Show when={page()?.has_more}>
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={historyLoading()}
                    onClick={() => {
                      const workflow = selectedWorkflow();
                      if (workflow) void loadExecutions(workflow, page()?.executions.length ?? 0, true);
                    }}
                  >
                    Load previous 10
                  </Button>
                </Show>
              </Show>
            </Show>
          </section>
        </Show>

        <Show when={recent().length > 0}>
          <section class="kria-automations__history-panel" aria-label="Recent KRIA workflow runs">
            <h3 class="kria-automations__panel-title">Recent KRIA runs</h3>
            <ol class="kria-automations__history-list">
              <For each={recent()}>
                {(workflow) => {
                  const presentation = statusPresentation(workflow.status);
                  return (
                    <li class="kria-automations__history-item" data-workflow-id={workflow.id}>
                      <span class="kria-automations__history-when">
                        {new Date(workflow.lastRunAt!).toLocaleString()}
                      </span>
                      <span class="kria-automations__history-what">{workflow.name}</span>
                      <StatusDot tone={presentation.tone} label={presentation.label} />
                      <span class="kria-automations__history-status-label">{presentation.label}</span>
                    </li>
                  );
                }}
              </For>
            </ol>
          </section>
        </Show>
      </Show>
    </div>
  );
}
