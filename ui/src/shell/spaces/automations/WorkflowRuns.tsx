/**
 * WorkflowRuns — active/recent canonical workflow runs in the Automations Run
 * segment (kria-ui-redesign task 7.5, Req 6.5 / 11.6 / 20.3).
 *
 * Surfaces the canonical `workflowSession` runtime (the agent/GUI workflow
 * continuation engine — distinct from the n8n workflows shown by
 * {@link WorkflowCard}) so its previously-INERT controls are reachable in the
 * redesign:
 *   • live step progress (Req 6.5)
 *   • Cancel a run in progress → `workflow_cancel` (Req 11.6, cancellation
 *     propagates to the runtime, which owns teardown)
 *   • post-run continuation actions → `workflow_continuation` (Req 11.6)
 *   • a HITL pause shows a CALM pointer to the unified Approval Center — never
 *     an inline modal (Req 11.1); the pause was routed there by the workflow
 *     HITL → Approval bridge.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Cancel / continuation are DISPATCH-ONLY through the registered `workflow_*`
 * commands via `workflowStore`. No orchestration, no run loop, no inline HITL
 * resolution. Untrusted text (summaries, labels) is rendered as escaped text.
 *
 * Hidden entirely when there are no runs (honest empty — the n8n empty state
 * below it already covers "nothing to run").
 *
 * Requirements: 6.5, 11.1, 11.6, 20.3
 */
import { createMemo, For, Show } from "solid-js";
import { Button, Card, StatusDot } from "../../../kit";
import type { StatusTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { approvalStore, shellStore } from "../../../stores";
import {
  workflowStore,
  cancelWorkflowById,
  continuationNotice,
  executeContinuation,
  dismissWorkflow,
} from "../../../stores/workflowSession";
import type {
  ContinuationAction,
  WorkflowLifecycle,
  WorkflowSession,
} from "../../../types/workflowRuntime";
import "./run.css";

function lifecyclePresentation(lc: WorkflowLifecycle): { tone: StatusTone; label: string } {
  switch (lc) {
    case "executing":
      return { tone: "busy", label: "Running" };
    case "hitl_pending":
      return { tone: "info", label: "Needs approval" };
    case "verifying":
      return { tone: "busy", label: "Verifying" };
    case "finalized":
      return { tone: "online", label: "Finished" };
    case "cancelled":
      return { tone: "offline", label: "Cancelled" };
    case "created":
    case "planned":
    default:
      return { tone: "offline", label: "Planned" };
  }
}

/** A run is active (cancellable) while it executes, verifies, or awaits HITL. */
function isActive(lc: WorkflowLifecycle): boolean {
  return lc === "executing" || lc === "verifying" || lc === "hitl_pending";
}

export interface WorkflowRunsProps {
  /** Open the Approval Center — defaults to `shellStore.setApprovalsOpen`. */
  onOpenApprovals?: () => void;
}

export function WorkflowRuns(props: WorkflowRunsProps) {
  const sessions = createMemo(() => workflowStore.recentSessions());

  const openApprovals = () =>
    props.onOpenApprovals ? props.onOpenApprovals() : shellStore.setApprovalsOpen(true);

  return (
    <Show when={sessions().length > 0}>
      <section class="kria-run__section" aria-label="Active workflow runs">
        <h2 class="kria-run__section-title">Workflow runs</h2>
        <ul class="kria-run__list">
          <For each={sessions()}>
            {(session) => <RunRow session={session} onOpenApprovals={openApprovals} />}
          </For>
        </ul>
      </section>
    </Show>
  );
}

function RunRow(props: { session: WorkflowSession; onOpenApprovals: () => void }) {
  const s = () => props.session;
  const pres = createMemo(() => lifecyclePresentation(s().lifecycle));
  const completed = createMemo(() => s().steps.filter((st) => st.status === "completed").length);
  const total = createMemo(() => s().steps.length);

  // A HITL pause is surfaced in the unified Approval Center (routed there by the
  // workflow HITL → Approval bridge). Show a calm pointer, not an inline modal.
  const hitlPending = createMemo(
    () =>
      s().lifecycle === "hitl_pending" &&
      approvalStore.queue().some(
        (r) =>
          r.status === "pending" &&
          r.type === "workflow-resume" &&
          r.routing?.workflowId === s().workflowId,
      ),
  );

  const continuations = createMemo<ContinuationAction[]>(() =>
    s().lifecycle === "finalized" ? s().continuationActions : [],
  );

  return (
    <li>
      <Card class="kria-wfcard" aria-label={`Workflow ${s().workflowId}`}>
        <div class="kria-wfcard__head">
          <div class="kria-wfcard__main">
            <span class="kria-wfcard__name" data-workflow-id={s().workflowId}>
              {s().workflowId}
            </span>
            <Show when={total() > 0}>
              <span class="kria-wfcard__desc">
                {completed()} of {total()} steps
              </span>
            </Show>
          </div>
          <div class="kria-wfcard__status">
            <StatusDot tone={pres().tone} label={pres().label} pulse={pres().tone === "busy"} />
            <span class="kria-wfcard__status-label">{pres().label}</span>
          </div>
        </div>

        {/* HITL → Approval Center pointer (calm, non-blocking). */}
        <Show when={hitlPending()}>
          <div class="kria-wfcard__hitl" role="status">
            <Icon name="shield" size={14} aria-hidden />
            <span>This run needs your approval.</span>
            <Button
              variant="secondary"
              size="sm"
              aria-label="Open the Approval Center to respond"
              onClick={() => props.onOpenApprovals()}
            >
              Open Approval Center
            </Button>
          </div>
        </Show>

          <Show when={continuationNotice(s().workflowId)}>
            {(notice) => <p class="kria-wfcard__continuation-notice" role="status">{notice()}</p>}
          </Show>

          <div class="kria-wfcard__actions">
          {/* Cancel a run in progress → workflow_cancel (Req 11.6). */}
          <Show when={isActive(s().lifecycle)}>
            <Button
              variant="danger"
              size="sm"
              aria-label={`Cancel workflow ${s().workflowId}`}
              onClick={() => void cancelWorkflowById(s().workflowId)}
            >
              <Icon name="square" size={14} />
              Cancel
            </Button>
          </Show>

          {/* Continuation actions after completion → workflow_continuation. */}
          <For each={continuations()}>
            {(action) => (
              <Button
                variant="secondary"
                size="sm"
                aria-label={action.label}
                onClick={() => void executeContinuation(s().workflowId, action)}
              >
                {action.label}
              </Button>
            )}
          </For>

          {/* Dismiss a finished/cancelled run from the active slot. */}
          <Show when={!isActive(s().lifecycle)}>
            <Button
              variant="ghost"
              size="sm"
              aria-label={`Dismiss workflow ${s().workflowId}`}
              onClick={() => dismissWorkflow(s().workflowId)}
            >
              Dismiss
            </Button>
          </Show>
        </div>
      </Card>
    </li>
  );
}

export default WorkflowRuns;
