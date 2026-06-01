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

import { createSignal, createMemo } from "solid-js";
import { createStore, produce } from "solid-js/store";
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

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Derived State (Memos)
// ═══════════════════════════════════════════════════════════════════════════════

/** The currently active workflow session (if any). */
export const activeSession = createMemo<WorkflowSession | null>(() => {
  const id = state.activeWorkflowId;
  if (!id) return null;
  return state.sessions[id] ?? null;
});

/** Whether any workflow is currently executing. */
export const hasActiveWorkflow = createMemo(() => {
  return Object.values(state.sessions).some(
    (s) => s.lifecycle === 'executing' || s.lifecycle === 'hitl_pending'
  );
});

/** All sessions sorted by most recent first. */
export const recentSessions = createMemo(() => {
  return Object.values(state.sessions).sort((a, b) => b.updatedAt - a.updatedAt);
});

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

/** Send a HITL response back to the backend. */
export async function respondToHitl(response: HitlResponse): Promise<void> {
  // This will be wired to a Tauri invoke command
  workflowDebug('[WorkflowStore] HITL response:', response);
  // TODO: invoke("workflow_hitl_respond", { response })

  // Optimistically update state
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

/** Cancel the active workflow. */
export async function cancelActiveWorkflow(): Promise<void> {
  const id = state.activeWorkflowId;
  if (!id) return;
  workflowDebug('[WorkflowStore] Cancelling workflow:', id);
  // TODO: invoke("workflow_cancel", { workflow_id: id })
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

/** Execute a continuation action. */
export async function executeContinuation(
  workflowId: string,
  action: ContinuationAction,
): Promise<void> {
  workflowDebug('[WorkflowStore] Continuation action:', workflowId, action);
  // TODO: invoke("workflow_continuation", { workflow_id: workflowId, action })
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
  dismissWorkflow,
  executeContinuation,
};
