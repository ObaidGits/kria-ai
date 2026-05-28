/**
 * Canonical Workflow Runtime Types — Frontend Contract
 *
 * These types mirror the backend `workflow_types.rs` and define the
 * structured telemetry protocol between backend and frontend.
 *
 * The frontend MUST render workflow state from these types — never from
 * parsed natural-language strings.
 *
 * @module workflowRuntime
 */

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Telemetry Envelope (Transport Protocol)
// ═══════════════════════════════════════════════════════════════════════════════

/** Versioned telemetry envelope — the primary backend→frontend contract. */
export interface TelemetryEnvelope {
  /** Protocol version (frontend ignores unknown versions) */
  version: number;
  /** Monotonic sequence number */
  seq: number;
  /** The actual event */
  event: WorkflowTelemetry;
  /** Milliseconds since workflow start */
  timestamp_ms: number;
  /** Which runtime path produced this */
  source: WorkflowSource;
}

export type WorkflowSource = 'substrate_router' | 'legacy_shim' | 'react_loop';

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Workflow Telemetry Events
// ═══════════════════════════════════════════════════════════════════════════════

export type WorkflowTelemetry =
  | WorkflowStarted
  | WorkflowPlanPreview
  | WorkflowStepStarted
  | WorkflowStepCompleted
  | WorkflowHitlRequired
  | WorkflowCompleted
  | WorkflowCancelled;

export interface WorkflowStarted {
  type: 'started';
  workflow_id: string;
  title: string;
  steps: StepPreview[];
  execution_mode: ExecutionMode;
  estimated_duration_ms?: number;
}

export interface WorkflowPlanPreview {
  type: 'plan_preview';
  workflow_id: string;
  title: string;
  steps: StepPreview[];
  outcome_summary: string[];
  requires_approval: boolean;
}

export interface WorkflowStepStarted {
  type: 'step_started';
  workflow_id: string;
  step_index: number;
  description: string;
  step_type: StepType;
}

export interface WorkflowStepCompleted {
  type: 'step_completed';
  workflow_id: string;
  step_index: number;
  structural_success: boolean;
  visibility_confidence: VisibilityConfidence;
  artifacts: string[];
}

export interface WorkflowHitlRequired {
  type: 'hitl_required';
  workflow_id: string;
  reason: HitlReason;
  options: HitlOption[];
  context: string;
}

export interface WorkflowCompleted {
  type: 'completed';
  workflow_id: string;
  verdict: WorkflowVerdict;
  summary: string;
  artifacts: string[];
  continuation: ContinuationAction[];
}

export interface WorkflowCancelled {
  type: 'cancelled';
  workflow_id: string;
  reason: string;
  completed_steps: number;
  total_steps: number;
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Step & Execution Types
// ═══════════════════════════════════════════════════════════════════════════════

export interface StepPreview {
  index: number;
  description: string;
  step_type: StepType;
  execution_mode: StepExecutionMode;
}

export type StepType =
  | 'file_write'
  | 'app_launch'
  | 'command_execution'
  | 'browser_navigation'
  | 'interaction'
  | 'verification';

export type StepExecutionMode = 'backend' | 'visible' | 'hybrid_surface' | 'interactive';

export type ExecutionMode =
  | { type: 'structural' }
  | { type: 'hybrid'; visible_steps: number[] }
  | { type: 'visible' };

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Visibility & Verdict
// ═══════════════════════════════════════════════════════════════════════════════

export type VisibilityConfidence =
  | { level: 'confirmed'; confidence: number; evidence: string }
  | { level: 'structural_only'; reason: string }
  | { level: 'inconclusive'; reason: string; suggestion?: string }
  | { level: 'not_applicable' };

export type WorkflowVerdict =
  | { type: 'complete' }
  | { type: 'already_satisfied'; evidence: string }
  | { type: 'structurally_complete'; unverified_outcomes: string[] }
  | { type: 'partial'; completed: number; total: number; reason: string }
  | { type: 'blocked'; reason: string }
  | { type: 'failed'; step: number; reason: string; recovery?: RecoveryPath };

export interface RecoveryPath {
  description: string;
  actions: ContinuationAction[];
}

export interface ContinuationAction {
  id: string;
  label: string;
  action_type: ContinuationActionType;
}

export type ContinuationActionType =
  | { type: 'bring_to_front'; app: string }
  | { type: 'open_url'; url: string }
  | { type: 'retry_step'; step_index: number }
  | { type: 'open_file'; path: string }
  | { type: 'show_output'; content: string }
  | { type: 'retry_workflow' };

// ═══════════════════════════════════════════════════════════════════════════════
// §5 — HITL Types
// ═══════════════════════════════════════════════════════════════════════════════

export type HitlReason =
  | { type: 'install_required'; app: string; install_command?: string }
  | { type: 'login_required'; service: string; guidance: string }
  | { type: 'session_expired'; service: string }
  | { type: 'ambiguous_target'; options: string[]; question: string }
  | { type: 'execution_mode_choice'; task: string; backend_option: string; gui_option: string }
  | { type: 'approval_needed'; action: string; risk_level: string; description: string }
  | { type: 'visibility_uncertain'; step_description: string; suggestion: string }
  | { type: 'focus_lost'; step_description: string }
  | { type: 'manual_step_needed'; instruction: string; context: string }
  | { type: 'intent_unclear'; original_text: string; what_understood: string; suggestion: string }
  | { type: 'budget_exhausted'; elapsed_ms: number; remaining_steps: number }
  | { type: 'accessibility_setup'; current_state: string; impact: string }
  | { type: 'step_failed'; step_description: string; error: string };

export interface HitlOption {
  id: string;
  label: string;
  action_type: HitlActionType;
}

export type HitlActionType =
  | { type: 'approve' }
  | { type: 'deny' }
  | { type: 'retry' }
  | { type: 'skip' }
  | { type: 'choose_alternative'; value: string }
  | { type: 'open_url'; url: string }
  | { type: 'run_command'; command: string }
  | { type: 'manual_complete' }
  | { type: 'cancel' };

export interface HitlResponse {
  workflow_id: string;
  option_id: string;
  action_type: HitlActionType;
}

// ═══════════════════════════════════════════════════════════════════════════════
// §6 — Workflow Session (Frontend State Model)
// ═══════════════════════════════════════════════════════════════════════════════

/** Complete frontend state for a single workflow session. */
export interface WorkflowSession {
  /** Unique workflow identifier */
  workflowId: string;
  /** Current lifecycle state */
  lifecycle: WorkflowLifecycle;
  /** Execution mode (structural/hybrid/visible) */
  executionMode: ExecutionMode;
  /** Step previews from planning */
  steps: WorkflowStepState[];
  /** Accumulated telemetry events */
  telemetry: TelemetryEnvelope[];
  /** Final verdict (set when workflow completes) */
  verdict?: WorkflowVerdict;
  /** HITL state (set when workflow is paused) */
  hitlState?: ActiveHitl;
  /** Continuation actions (set after completion) */
  continuationActions: ContinuationAction[];
  /** Timing */
  startedAt: number;
  updatedAt: number;
  /** Source runtime */
  source: WorkflowSource;
}

export type WorkflowLifecycle =
  | 'created'
  | 'planned'
  | 'executing'
  | 'hitl_pending'
  | 'verifying'
  | 'finalized'
  | 'cancelled';

export interface WorkflowStepState {
  index: number;
  description: string;
  stepType: StepType;
  executionMode: StepExecutionMode;
  status: StepStatus;
  visibility?: VisibilityConfidence;
  artifacts: string[];
}

export type StepStatus = 'pending' | 'running' | 'completed' | 'failed' | 'skipped';

export interface ActiveHitl {
  reason: HitlReason;
  options: HitlOption[];
  context: string;
  receivedAt: number;
}

// ═══════════════════════════════════════════════════════════════════════════════
// §7 — Verdict Display Helpers
// ═══════════════════════════════════════════════════════════════════════════════

/** Get the display icon for a verdict. */
export function verdictIcon(verdict: WorkflowVerdict): string {
  switch (verdict.type) {
    case 'complete': return '✓';
    case 'already_satisfied': return '✓';
    case 'structurally_complete': return '⚙';
    case 'partial': return '⚠';
    case 'blocked': return '🔒';
    case 'failed': return '✗';
  }
}

/** Get the display color class for a verdict. */
export function verdictColorClass(verdict: WorkflowVerdict): string {
  switch (verdict.type) {
    case 'complete':
    case 'already_satisfied':
      return 'text-green-400';
    case 'structurally_complete':
      return 'text-blue-400';
    case 'partial':
      return 'text-yellow-400';
    case 'blocked':
      return 'text-orange-400';
    case 'failed':
      return 'text-red-400';
  }
}

/** Get a short human-readable label for a verdict. */
export function verdictLabel(verdict: WorkflowVerdict): string {
  switch (verdict.type) {
    case 'complete': return 'Complete';
    case 'already_satisfied': return 'Already Done';
    case 'structurally_complete': return 'Done (visibility unverified)';
    case 'partial': return `Partial (${verdict.completed}/${verdict.total})`;
    case 'blocked': return 'Action Needed';
    case 'failed': return `Failed at step ${verdict.step}`;
  }
}

/** Get the icon for a step execution mode. */
export function stepModeIcon(mode: StepExecutionMode): string {
  switch (mode) {
    case 'backend': return '🔧';
    case 'visible': return '🖥️';
    case 'hybrid_surface': return '🔧→🖥️';
    case 'interactive': return '👆';
  }
}

/** Get the icon for a step type. */
export function stepTypeIcon(type: StepType): string {
  switch (type) {
    case 'file_write': return '📄';
    case 'app_launch': return '🚀';
    case 'command_execution': return '⚡';
    case 'browser_navigation': return '🌐';
    case 'interaction': return '👆';
    case 'verification': return '🔍';
  }
}
