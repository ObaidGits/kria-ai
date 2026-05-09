/**
 * TypeScript interfaces for KRIA Intelligence Enhancement backend payloads.
 *
 * These types mirror the Rust structs emitted by the ExecutiveController,
 * PolicyGate, QuarantineRegistry, and StructuredBranchingPlanner.
 */

// ─── Executive Controller ───────────────────────────────────────────────────

export type TaskPriority = "Voice" | "Interactive" | "HitlResponse" | "Background" | "Maintenance";

export type TaskSource =
  | "VoicePipeline"
  | "TextChat"
  | "HitlGateway"
  | "CuriosityLoop"
  | "ProactiveScheduler"
  | "SkillCompiler"
  | "Maintenance"
  | { CompiledSkill: string };

export type TaskState = "Queued" | "Running" | "Completed" | "Failed" | "Cancelled" | "Preempted";

export interface ExecutiveTask {
  id: string;
  priority: TaskPriority;
  source: TaskSource;
  state: TaskState;
  description: string;
  submitted_at: string; // ISO 8601
  started_at: string | null;
  completed_at: string | null;
  /** Duration in milliseconds, null if still running. */
  duration_ms: number | null;
  /** Error message if failed. */
  error: string | null;
  /** Whether this task requires GPU lease. */
  requires_gpu: boolean;
}

export interface ExecutiveSnapshot {
  active_foreground: ExecutiveTask | null;
  active_background: ExecutiveTask[];
  queued: ExecutiveTask[];
  /** GPU lease holder task ID, null if free. */
  gpu_lease_holder: string | null;
  /** Time remaining on GPU lease in ms, null if free. */
  gpu_lease_remaining_ms: number | null;
  /** Total tasks completed since startup. */
  total_completed: number;
  /** Total tasks failed since startup. */
  total_failed: number;
}

export interface ExecutiveTaskStarted {
  task_id: string;
  priority: TaskPriority;
  source: TaskSource;
  description: string;
  ts: string;
}

export interface ExecutiveTaskCompleted {
  task_id: string;
  success: boolean;
  duration_ms: number;
  output_summary: string | null;
  error: string | null;
  ts: string;
}

export interface ExecutivePreemption {
  /** The task that was preempted. */
  victim_id: string;
  victim_priority: TaskPriority;
  /** The task that caused the preemption. */
  replacement_id: string;
  replacement_priority: TaskPriority;
  ts: string;
}

export interface GpuLeaseEvent {
  task_id: string;
  action: "acquired" | "released" | "expired";
  ts: string;
}

// ─── Policy Gate ────────────────────────────────────────────────────────────

export type RiskLevel = "Green" | "Yellow" | "Red" | "Black";

export type PolicyDecisionKind = "AutoApproved" | "RequiresApproval" | "Blocked";

export interface PolicyGateEvaluation {
  /** The command that was evaluated. */
  command: string;
  binary: string;
  args: string[];
  decision: PolicyDecisionKind;
  risk_level: RiskLevel;
  /** Capabilities resolved for this command. */
  capabilities: string[];
  /** Reason if blocked or requires approval. */
  reason: string | null;
  ts: string;
}

// ─── Quarantine Registry ────────────────────────────────────────────────────

export type QuarantineStatus = "Testing" | "PendingApproval" | "Active" | "Disabled" | "Rejected";

export type ToolSourceKind = "SkillCompiler" | "DynamicDiscovery" | "McpServer";

export interface QuarantinedTool {
  id: string;
  name: string;
  description: string;
  risk_level: RiskLevel;
  status: QuarantineStatus;
  source: ToolSourceKind;
  success_count: number;
  consecutive_failures: number;
  total_executions: number;
  created_at: string;
  last_tested: string;
  review_notes: string | null;
  /** Parameter schema (JSON Schema). */
  parameters_schema: Record<string, unknown> | null;
}

export interface QuarantineApprovalRequest {
  tool_id: string;
  tool_name: string;
  risk_level: RiskLevel;
  source: ToolSourceKind;
  success_count: number;
  description: string;
  ts: string;
}

export interface QuarantinePromotionEvent {
  tool_id: string;
  tool_name: string;
  risk_level: RiskLevel;
  ts: string;
}

export interface QuarantineDisabledEvent {
  tool_id: string;
  tool_name: string;
  reason: string;
  consecutive_failures: number;
  ts: string;
}

// ─── Intelligence: Uncertainty Engine ───────────────────────────────────────

export type UncertaintyAction = "Plan" | "GatherEvidence" | "AskUser" | "Refuse";

export interface UncertaintyEvaluation {
  goal: string;
  confidence: number;
  action: UncertaintyAction;
  /** Belief graph facts relevant to this evaluation. */
  relevant_facts: string[];
  ts: string;
}

// ─── Intelligence: Structured Branching Planner ─────────────────────────────

export type PathRisk = "DiagnoseFirst" | "MinimalRisk" | "Aggressive";

export interface PlannedStep {
  step_number: number;
  tool_name: string;
  description: string;
  /** The structured command to execute. */
  command: {
    binary: string;
    args: string[];
    target: string;
    timeout_secs: number;
  };
  /** Error handling strategy. */
  error_handling: "continue" | "abort" | "retry";
}

export interface StructuredPath {
  risk_level: PathRisk;
  /** Human-readable label for this path. */
  label: string;
  steps: PlannedStep[];
  /** SelfModel Beta posterior score for this path. */
  self_model_score: number;
  /** Estimated success probability. */
  confidence: number;
  /** Whether this path was selected as the winner. */
  is_winner: boolean;
}

export interface PlanGenerated {
  goal: string;
  /** Always exactly 3 paths. */
  paths: [StructuredPath, StructuredPath, StructuredPath];
  /** Index of the winning path (0, 1, or 2). */
  winner_index: number;
  /** Reason the winner was selected. */
  selection_reason: string;
  ts: string;
}

export interface PlanStepResult {
  goal: string;
  step_number: number;
  tool_name: string;
  success: boolean;
  exit_code: number | null;
  stdout_summary: string;
  stderr_summary: string;
  duration_ms: number;
  ts: string;
}

export interface GoalVerification {
  goal: string;
  outcome: "Achieved" | "Failed" | "Continue";
  reason: string | null;
  ts: string;
}

// ─── Intelligence: Self Model ───────────────────────────────────────────────

export interface ToolStatsSnapshot {
  tool_name: string;
  alpha: number;
  beta: number;
  success_rate: number;
  total_calls: number;
  avg_latency_ms: number;
  confidence_width: number;
  last_used: string;
}

export interface SelfModelSnapshot {
  tools: ToolStatsSnapshot[];
  total_outcomes: number;
}

// ─── Composite Intelligence State ───────────────────────────────────────────

export interface IntelligenceState {
  uncertainty_confidence: number;
  working_set_tokens: number;
  self_model_tool_count: number;
  compiled_skill_count: number;
  quarantined_skill_count: number;
  curiosity_findings: number;
}
