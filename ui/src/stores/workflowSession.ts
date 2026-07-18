/**
 * Workflow Session Store — Canonical Frontend Workflow State.
 *
 * This store manages all active workflow sessions. It consumes
 * WorkflowTelemetry events and maintains typed workflow state.
 *
 * The frontend renders workflow UI from this store — never from
 * parsed chat messages.
 *
 * @module workflowSession
 */

import { createSignal } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { bridgeInvoke } from "../bridge/invoke";
import {
  enqueueWorkflowHitl,
  dismissWorkflowHitl,
} from "../bridge/workflowApproval";
import type {
  TelemetryEnvelope,
  WorkflowTelemetry,
  WorkflowSession,
  WorkflowLifecycle,
  WorkflowStepState,
  WorkflowVerdict,
  ActiveHitl,
  HitlResponse,
  ContinuationAction,
  WorkflowSource,
} from "../types/workflowRuntime";

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Store State
// ═══════════════════════════════════════════════════════════════════════════════

interface WorkflowStoreState {
  /** All active/recent workflow sessions indexed by ID */
  sessions: Record<string, WorkflowSession>;
  /** Currently focused workflow (for UI rendering) */
  activeWorkflowId: string | null;
  /** Maximum sessions to keep in memory */
  maxSessions: number;
}

const [state, setState] = createStore<WorkflowStoreState>({
  sessions: {},
  activeWorkflowId: null,
  maxSessions: 20,
});
const [continuationNotices, setContinuationNotices] = createSignal<Record<string, string>>({});

export function continuationNotice(workflowId: string): string | null {
  return continuationNotices()[workflowId] ?? null;
}

function setContinuationNotice(workflowId: string, message: string | null): void {
  setContinuationNotices((previous) => {
    const next = { ...previous };
    if (message) next[workflowId] = message;
    else delete next[workflowId];
    return next;
  });
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Derived State
// ═══════════════════════════════════════════════════════════════════════════════

/** The currently active workflow session (if any). */
export function activeSession(): WorkflowSession | null {
  const id = state.activeWorkflowId;
  if (!id) return null;
  return state.sessions[id] ?? null;
}

/** Whether any workflow is currently executing. */
export function hasActiveWorkflow(): boolean {
  return Object.values(state.sessions).some(
    (s) => s.lifecycle === 'executing' || s.lifecycle === 'hitl_pending'
  );
}

/** All sessions sorted by most recent first. */
export function recentSessions(): WorkflowSession[] {
  return Object.values(state.sessions).sort((a, b) => b.updatedAt - a.updatedAt);
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Telemetry Event Handler (Primary Input)
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Process a telemetry envelope from the backend.
 * This is the ONLY way workflow state changes in the frontend.
 */
export function handleTelemetryEvent(envelope: TelemetryEnvelope): void {
  // Version check — ignore unknown protocol versions
  if (envelope.version > 1) {
    console.warn(`[WorkflowStore] Unknown telemetry version ${envelope.version}, ignoring`);
    return;
  }

  const event = envelope.event;
  const now = Date.now();

  switch (event.type) {
    case 'started':
      createSession(event.workflow_id, event, envelope.source, now);
      break;

    case 'step_started':
      updateStepStatus(event.workflow_id, event.step_index, 'running', now);
      break;

    case 'step_completed':
      updateStepCompleted(event.workflow_id, event.step_index, event, now);
      break;

    case 'hitl_required':
      setHitlPending(event.workflow_id, event, now);
      break;

    case 'completed':
      finalizeWorkflow(event.workflow_id, event, now);
      break;

    case 'cancelled':
      cancelWorkflow(event.workflow_id, event.reason, now);
      break;

    case 'plan_preview':
      // Plan preview doesn't change lifecycle — just stores preview data
      break;
  }

  // Store the raw telemetry event in the session trace
  appendTelemetry(event.type === 'started' ? (event as any).workflow_id : getWorkflowIdFromEvent(event), envelope);
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — State Mutations (Internal)
// ═══════════════════════════════════════════════════════════════════════════════

function createSession(
  workflowId: string,
  event: Extract<WorkflowTelemetry, { type: 'started' }>,
  source: WorkflowSource,
  now: number,
): void {
  const steps: WorkflowStepState[] = event.steps.map((s) => ({
    index: s.index,
    description: s.description,
    stepType: s.step_type,
    executionMode: s.execution_mode,
    status: 'pending' as const,
    artifacts: [],
  }));

  setState(
    produce((s) => {
      s.sessions[workflowId] = {
        workflowId,
        lifecycle: 'executing',
        executionMode: event.execution_mode,
        steps,
        telemetry: [],
        continuationActions: [],
        startedAt: now,
        updatedAt: now,
        source,
      };
      s.activeWorkflowId = workflowId;

      // Prune old sessions if over limit
      const ids = Object.keys(s.sessions);
      if (ids.length > s.maxSessions) {
        const sorted = ids
          .map((id) => ({ id, updated: s.sessions[id].updatedAt }))
          .sort((a, b) => a.updated - b.updated);
        const toRemove = sorted.slice(0, ids.length - s.maxSessions);
        for (const { id } of toRemove) {
          delete s.sessions[id];
        }
      }
    })
  );
}

function updateStepStatus(
  workflowId: string,
  stepIndex: number,
  status: 'running' | 'failed' | 'skipped',
  now: number,
): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      const step = session.steps.find((st) => st.index === stepIndex);
      if (step) {
        step.status = status;
      }
      session.updatedAt = now;
    })
  );
}

function updateStepCompleted(
  workflowId: string,
  stepIndex: number,
  event: Extract<WorkflowTelemetry, { type: 'step_completed' }>,
  now: number,
): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      const step = session.steps.find((st) => st.index === stepIndex);
      if (step) {
        step.status = event.structural_success ? 'completed' : 'failed';
        step.visibility = event.visibility_confidence;
        step.artifacts = event.artifacts;
      }
      session.updatedAt = now;
    })
  );
}

function setHitlPending(
  workflowId: string,
  event: Extract<WorkflowTelemetry, { type: 'hitl_required' }>,
  now: number,
): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      session.lifecycle = 'hitl_pending';
      session.hitlState = {
        reason: event.reason,
        options: event.options,
        context: event.context,
        receivedAt: now,
      };
      session.updatedAt = now;
    })
  );

  // Route the pause into the ONE unified Approval Center (Req 11.1 / 11.6).
  // The Approval Center owns the decision; the resolver routes approve/deny
  // back through `workflow_hitl_respond` / `workflow_cancel`. No inline modal.
  enqueueWorkflowHitl(workflowId, {
    reason: event.reason,
    options: event.options,
    context: event.context,
    receivedAt: now,
  });
}

function finalizeWorkflow(
  workflowId: string,
  event: Extract<WorkflowTelemetry, { type: 'completed' }>,
  now: number,
): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      session.lifecycle = 'finalized';
      session.verdict = event.verdict;
      session.continuationActions = event.continuation;
      session.hitlState = undefined;
      session.updatedAt = now;

      // Clear active if this was the active workflow
      if (s.activeWorkflowId === workflowId) {
        // Keep it active briefly for the user to see the verdict
      }
    })
  );

  // The run resolved on its own — drop any lingering approval card (Req 11.1).
  dismissWorkflowHitl(workflowId);
}

function cancelWorkflow(workflowId: string, reason: string, now: number): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      session.lifecycle = 'cancelled';
      session.hitlState = undefined;
      session.updatedAt = now;
    })
  );

  // Cancelled — drop any pending approval card for this workflow (Req 11.1).
  dismissWorkflowHitl(workflowId);
}

function appendTelemetry(workflowId: string, envelope: TelemetryEnvelope): void {
  setState(
    produce((s) => {
      const session = s.sessions[workflowId];
      if (!session) return;
      session.telemetry.push(envelope);
      // Keep telemetry bounded
      if (session.telemetry.length > 100) {
        session.telemetry = session.telemetry.slice(-50);
      }
    })
  );
}

function getWorkflowIdFromEvent(event: WorkflowTelemetry): string {
  return (event as any).workflow_id ?? '';
}

function workflowDebug(...args: unknown[]): void {
  if (import.meta.env.DEV) {
    console.debug(...args);
  }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — Public Actions
// ═══════════════════════════════════════════════════════════════════════════════

/**
 * Send a HITL response back to the backend via the now-registered
 * `workflow_hitl_respond` command (kria-ui-redesign task 4.2 / design.md §3.3
 * contract change b, Req 11.6). Graceful degradation: an unavailable command
 * is logged, not thrown (Req 20.4). Optimistic UI update only applies once the
 * decision has been handed to the runtime.
 */
export async function respondToHitl(response: HitlResponse): Promise<void> {
  workflowDebug('[WorkflowStore] HITL response:', response);

  const result = await bridgeInvoke('workflow_hitl_respond', {
    workflowId: response.workflow_id,
    optionId: response.option_id,
    actionType: response.action_type.type,
    value: 'value' in response.action_type ? response.action_type.value : null,
  });

  if (!result.ok) {
    workflowDebug('[WorkflowStore] HITL respond not completed:', result);
    return;
  }

  // Decision handed to the runtime — clear the unified approval card too, so a
  // response made from the legacy inline path doesn't leave a stale card.
  dismissWorkflowHitl(response.workflow_id);

  // Optimistically resume once the runtime has accepted the decision.
  setState(
    produce((s) => {
      const session = s.sessions[response.workflow_id];
      if (!session) return;
      session.lifecycle = 'executing';
      session.hitlState = undefined;
      session.updatedAt = Date.now();
    })
  );
}

/**
 * Cancel the active workflow via the now-registered `workflow_cancel` command
 * (Req 11.6). Cancellation propagates to the runtime, which owns teardown.
 */
export async function cancelActiveWorkflow(): Promise<void> {
  const id = state.activeWorkflowId;
  if (!id) return;
  workflowDebug('[WorkflowStore] Cancelling workflow:', id);
  await bridgeInvoke('workflow_cancel', { workflowId: id });
}

/**
 * Cancel a specific workflow by id via the now-registered `workflow_cancel`
 * command (Req 11.6). Used by the Automations Space run list, where several
 * runs may be visible. Cancellation propagates to the runtime, which owns
 * teardown; the pending approval card (if any) clears via telemetry.
 */
export async function cancelWorkflowById(workflowId: string): Promise<void> {
  if (!workflowId) return;
  workflowDebug('[WorkflowStore] Cancelling workflow by id:', workflowId);
  await bridgeInvoke('workflow_cancel', { workflowId });
}

/** Clear a completed/cancelled workflow from the active slot. */
export function dismissWorkflow(workflowId: string): void {
  setState(
    produce((s) => {
      if (s.activeWorkflowId === workflowId) {
        s.activeWorkflowId = null;
      }
    })
  );
}

/** Execute a continuation action and surface its authoritative outcome. */
export async function executeContinuation(
  workflowId: string,
  action: ContinuationAction,
): Promise<void> {
  workflowDebug('[WorkflowStore] Continuation action:', workflowId, action);
  const at = action.action_type;
  let payload: string | null = null;
  switch (at.type) {
    case 'bring_to_front':
      payload = at.app;
      break;
    case 'open_url':
      payload = at.url;
      break;
    case 'open_file':
      payload = at.path;
      break;
    case 'show_output':
      payload = at.content;
      break;
    case 'retry_step':
      payload = String(at.step_index);
      break;
    case 'retry_workflow':
      payload = null;
      break;
  }
  setContinuationNotice(workflowId, null);
  const result = await bridgeInvoke<{
    status: string;
    action?: string;
    content?: string;
    resume?: { summary?: string };
  }>('workflow_continuation', {
    workflowId,
    actionId: action.id,
    actionType: at.type,
    payload,
  });

  if (!result.ok) {
    setContinuationNotice(workflowId, result.message);
    return;
  }
  const message = result.data.content
    ?? result.data.resume?.summary
    ?? (result.data.status === 'started' ? `${action.label} started.` : `${action.label} completed.`);
  setContinuationNotice(workflowId, message);
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — Export Store
// ═══════════════════════════════════════════════════════════════════════════════

export const workflowStore = {
  state,
  activeSession,
  hasActiveWorkflow,
  recentSessions,
  handleTelemetryEvent,
  respondToHitl,
  cancelActiveWorkflow,
  cancelWorkflowById,
  dismissWorkflow,
  executeContinuation,
};
