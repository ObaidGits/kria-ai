/**
 * WorkflowCard — an interactive workflow surfaced in Run (task 7.2, Req 6.5).
 *
 * Shows workflow status plus deliberate Run control and authoritative live
 * progress/evidence. n8n's public API has no supported execution-stop endpoint,
 * so running cards show clear guidance instead of a fake Cancel button.
 * step, the card does NOT open an inline modal — it surfaces a calm pointer to
 * the unified Approval Center (Req 6.5 / 11.1), where the decision actually
 * lives (routed there by the unified approval event, task 4.2).
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Run is DISPATCH-ONLY through `invoke_n8n_workflow_from_ui` via
 * `automationStore` → bridge. KRIA-native workflow continuation cancellation
 * remains separate in WorkflowRuns; it is never misrepresented as n8n stop.
 *
 * Requirements: 6.5, 11.1, 17.3
 */
import { createMemo, createSignal, Show } from "solid-js";
import { Button, Card, StatusDot } from "../../../kit";
import type { StatusTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { automationStore, approvalStore, shellStore } from "../../../stores";
import type { Workflow, WorkflowStatus } from "../../../stores";
import { RunProgress } from "./RunProgress";
import { EvidenceViewer } from "./EvidenceViewer";
import "./run.css";

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

export interface WorkflowCardProps {
  workflow: Workflow;
  /** Run handler — defaults to `automationStore.startRun`. Override for tests. */
  onRun?: (workflow: Workflow) => void | Promise<void>;
  /** Open the Approval Center — defaults to `shellStore.setApprovalsOpen`. */
  onOpenApprovals?: () => void;
}

export function WorkflowCard(props: WorkflowCardProps) {
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const wf = () => props.workflow;
  const pres = createMemo(() => statusPresentation(wf().status));
  const progress = createMemo(() => automationStore.runProgress()[wf().id]);
  const evidence = createMemo(() => automationStore.runEvidence()[wf().id] ?? []);
  const isRunning = createMemo(() => automationStore.runningWorkflowIds().has(wf().id));

  // HITL routes to the Approval Center — surface a pointer, never an inline
  // modal (Req 6.5 / 11.1). Match a pending workflow-resume approval for this
  // workflow (unified approval event, task 4.2).
  const hitlPending = createMemo(() =>
    approvalStore.queue().some(
      (r) =>
        r.status === "pending" &&
        r.type === "workflow-resume" &&
        r.routing?.workflowId === wf().id,
    ),
  );

  const openApprovals = () =>
    props.onOpenApprovals ? props.onOpenApprovals() : shellStore.setApprovalsOpen(true);

  async function run() {
    setError(null);
    setBusy(true);
    try {
      if (props.onRun) {
        await props.onRun(wf());
        return;
      }
      const res = await automationStore.startRun({
        workflowId: wf().id,
        workflowVersion: wf().version ?? "",
      });
      if (!res.ok) setError(res.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card class="kria-wfcard" aria-label={wf().name}>
      <div class="kria-wfcard__head">
        <div class="kria-wfcard__main">
          <span class="kria-wfcard__name" data-workflow-id={wf().id}>
            {wf().name}
          </span>
          <Show when={wf().description}>
            <span class="kria-wfcard__desc">{wf().description}</span>
          </Show>
        </div>
        <div class="kria-wfcard__status">
          <StatusDot tone={pres().tone} label={pres().label} pulse={pres().tone === "busy"} />
          <span class="kria-wfcard__status-label">{pres().label}</span>
        </div>
      </div>

      <Show when={progress()}>
        <RunProgress progress={progress()!} />
      </Show>

      {/* HITL → Approval Center pointer (calm, non-blocking). */}
      <Show when={hitlPending()}>
        <div class="kria-wfcard__hitl" role="status">
          <Icon name="shield" size={14} aria-hidden />
          <span>This run needs your approval.</span>
          <Button
            variant="secondary"
            size="sm"
            aria-label="Open the Approval Center to respond"
            onClick={openApprovals}
          >
            Open Approval Center
          </Button>
        </div>
      </Show>

      <Show when={error()}>
        <p class="kria-wfcard__error" role="alert">
          <Icon name="alert-triangle" size={13} aria-hidden /> {error()}
        </p>
      </Show>

      <div class="kria-wfcard__actions">
        <Show
          when={isRunning()}
          fallback={
            <Button
              variant="primary"
              size="sm"
              disabled={busy()}
              aria-label={`Run ${wf().name}`}
              onClick={() => void run()}
            >
              <Icon name={busy() ? "loader" : "play"} size={14} />
              {busy() ? "Starting…" : "Run"}
            </Button>
          }
        >
          <p class="kria-run__muted" role="status">
            Running in n8n. Stop it from n8n Executions if needed.
          </p>
        </Show>
      </div>

      <Show when={evidence().length > 0}>
        <EvidenceViewer evidence={evidence()} />
      </Show>
    </Card>
  );
}

export default WorkflowCard;
