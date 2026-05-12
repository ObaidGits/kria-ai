/**
 * RFC 007 Phase 4 - GUI Automation HTN Types
 * 
 * TypeScript interfaces for the Hierarchical Task Network (HTN) workflow
 * system that replaces ReAct loops for GUI automation tasks.
 */

/**
 * Bounding box for UI elements [x1, y1, x2, y2]
 */
export type BoundingBox = [number, number, number, number];

/**
 * OmniParser element as returned from Python sidecar
 */
export interface OmniElement {
  id: string;
  element_type: string;
  label: string;
  label_wrapped: string;  // Cognitive poisoning defense: truncated + <evidence> tags
  bbox: BoundingBox;
  confidence: number;
  monitor_id: number;
  dpi_scale: number;
  visual_hash: string;  // pHash for verification
}

/**
 * OmniParser output schema
 */
export interface OmniParserOutput {
  elements: OmniElement[];
  screen_dimensions: [number, number];
  monitor_dimensions: [number, number][];
  timestamp: number;
  visual_hash: string;
}

/**
 * Verification strategy types per RFC 007
 */
export type VerificationType =
  | { type: 'screen_changed'; element_id?: string; threshold: number }
  | { type: 'elements_found'; element_ids: string[]; min_count: number }
  | { type: 'text_present'; text: string; case_insensitive: boolean }
  | { type: 'window_state'; title_contains?: string; class?: string }
  | { type: 'none' };

/**
 * Individual sub-goal in HTN workflow
 */
export interface SubGoal {
  step: number;
  action: string;
  params: Record<string, unknown>;
  verify: VerificationType;
  timeout_ms?: number;
}

/**
 * Safe abort step for graceful failure recovery
 */
export interface SafeAbortStep {
  action: string;
  params: Record<string, unknown>;
}

/**
 * HTN GUI Workflow - The core data structure for GUI automation
 * 
 * Per RFC 007:
 * - Generated once by TurnGate, never modified during execution
 * - Maximum 5 minute duration
 * - Must include safe_abort_steps
 */
export interface GuiWorkflow {
  task_id: string;
  max_duration_sec: number;
  sub_goals: SubGoal[];
  safe_abort_steps: SafeAbortStep[];
}

/**
 * Workflow execution result
 */
export interface WorkflowResult {
  task_id: string;
  success: boolean;
  completed_steps: number;
  total_steps: number;
  error?: string;
  aborted: boolean;
  duration_ms: number;
}

/**
 * Extended TurnGate output that can include HTN workflow
 */
export type TurnGateOutput =
  | {
      type: 'standard';
      intent: {
        modality: string;
        operation: string;
        hazard_hint: string;
        confidence: number;
      };
      direct_tool_hint?: string;
      fallback_tool_hints: string[];
    }
  | {
      type: 'htn_workflow';
      intent: {
        modality: string;
        operation: string;
        hazard_hint: string;
        confidence: number;
      };
      workflow: GuiWorkflow;
    };

/**
 * Kill Switch state for UI display
 */
export interface KillSwitchState {
  is_active: boolean;
  triggered_at?: number;
  reason?: string;
}

/**
 * GUI execution progress for real-time UI updates
 */
export interface GuiExecutionProgress {
  task_id: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'aborted';
  current_step: number;
  total_steps: number;
  current_action: string;
  sub_goals: SubGoal[];
  safe_abort_steps: SafeAbortStep[];
  kill_switch: KillSwitchState;
  started_at: number;
  completed_at?: number;
  result?: WorkflowResult;
}

/**
 * Tauri event payloads for GUI automation
 */
export interface GuiWorkflowStartedEvent {
  task_id: string;
  workflow: GuiWorkflow;
  timestamp: number;
}

export interface GuiWorkflowStepEvent {
  task_id: string;
  step: number;
  action: string;
  status: 'started' | 'completed' | 'failed';
  timestamp: number;
}

export interface GuiWorkflowCompletedEvent {
  task_id: string;
  result: WorkflowResult;
  timestamp: number;
}

export interface KillSwitchTriggeredEvent {
  task_id: string;
  reason: string;
  timestamp: number;
}

/**
 * Props for GUI Workflow visualization component
 */
export interface GuiWorkflowVisualizerProps {
  progress: GuiExecutionProgress;
  onCancel?: () => void;
  showKillSwitch?: boolean;
  className?: string;
}

/**
 * Props for SubGoal list component
 */
export interface SubGoalListProps {
  sub_goals: SubGoal[];
  current_step: number;
  completed_steps: number[];
  failed_steps?: number[];
}

/**
 * Props for Safe Abort Steps display
 */
export interface SafeAbortStepsProps {
  steps: SafeAbortStep[];
  is_active: boolean;  // True if workflow was aborted and these ran
}
