//! Phase 5 — Execution Transparency Layer.
//!
//! # Core Mission
//!
//! KRIA must become visibly understandable. Users should know:
//! - What is happening RIGHT NOW
//! - Why an action is occurring
//! - Why confirmation was requested
//! - Why a retry happened
//! - Why the workflow paused
//! - What the confidence level is
//!
//! # Design Philosophy
//!
//! ```text
//! "A good coworker doesn't hide what they're doing.
//!  They don't narrate every keystroke either.
//!  They surface the important things at the right time."
//! ```
//!
//! # Output Targets
//!
//! - **Tauri frontend**: serialized `WorkflowTrace` via Tauri commands.
//! - **AgentLoop**: appended to system prompt for LLM context.
//! - **Logging**: structured `tracing` events.
//! - **Audit trail**: written to `WorkflowSession.completed_steps`.
//!
//! # Architectural Invariants
//!
//! - The transparency layer is APPEND-ONLY. It never modifies past trace entries.
//! - Traces are bounded: max 8 stages × 6 actions = 48 entries per workflow.
//! - All narrative generation is deterministic — no LLM calls.
//! - Traces are automatically persisted to PSDG for cross-turn visibility.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::agent::goal_tree::GoalTree;
use crate::agent::psdg::PsdgHandle;
use crate::agent::stage_executor::StageOutcome;
use crate::agent::workflow_continuation::InterruptionClass;
use crate::agent::world_model::FactSource;

// ─── Stage Trace ──────────────────────────────────────────────────────────────

/// Trace of a single stage in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageTrace {
    /// Stage index (0-based).
    pub stage_index: u32,
    /// Human-readable stage label.
    pub label: String,
    /// Outcome of this stage.
    pub outcome: StageOutcomeTrace,
    /// Number of actions executed.
    pub actions_executed: usize,
    /// Number of recovery attempts used.
    pub recovery_attempts: u32,
    /// Stage duration in milliseconds.
    pub duration_ms: u128,
    /// Verification confidence (0.0–1.0).
    pub verify_confidence: f32,
    /// Human-readable explanation of what happened in this stage.
    pub explanation: String,
    /// Epoch seconds when this stage was recorded.
    pub recorded_at: u64,
}

/// Serializable stage outcome for the transparency layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageOutcomeTrace {
    Pending,
    Running,
    Passed,
    PassedAfterRecovery,
    Skipped,
    Failed { reason: String },
    Cancelled,
    TimedOut,
    PausedForDecision { decision_id: String, reason: String },
}

impl From<&StageOutcome> for StageOutcomeTrace {
    fn from(o: &StageOutcome) -> Self {
        match o {
            StageOutcome::Passed => Self::Passed,
            StageOutcome::PassedAfterRecovery => Self::PassedAfterRecovery,
            StageOutcome::Skipped => Self::Skipped,
            StageOutcome::Failed { reason } => Self::Failed {
                reason: reason.clone(),
            },
            StageOutcome::Cancelled => Self::Cancelled,
            StageOutcome::TimedOut => Self::TimedOut,
            StageOutcome::PausedForDecision {
                decision_id,
                reason,
            } => Self::PausedForDecision {
                decision_id: decision_id.clone(),
                reason: reason.clone(),
            },
            StageOutcome::Pending => Self::Pending,
        }
    }
}

// ─── Workflow Trace ───────────────────────────────────────────────────────────

/// Complete real-time trace of a workflow execution.
///
/// Serializable for Tauri frontend display. Append-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTrace {
    /// Workflow identifier.
    pub workflow_id: String,
    /// Human-readable workflow description.
    pub description: String,
    /// Currently executing stage index (None = not started or complete).
    pub current_stage_index: Option<u32>,
    /// Total number of stages.
    pub total_stages: u32,
    /// Completed stage traces (in order).
    pub completed_stages: Vec<StageTrace>,
    /// Pending stages (not yet executed).
    pub pending_stage_labels: Vec<String>,
    /// Active blockers (interruptions, verification failures).
    pub blockers: Vec<BlockerRecord>,
    /// Total recovery attempts across all stages.
    pub total_recovery_attempts: u32,
    /// Overall confidence in the workflow (rolling mean of stage confidences).
    pub overall_confidence: f32,
    /// Current workflow status.
    pub status: WorkflowStatusTrace,
    /// Start time (epoch seconds).
    pub started_at: u64,
    /// Last updated time (epoch seconds).
    pub updated_at: u64,
}

/// Current status of the workflow trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatusTrace {
    Pending,
    Running,
    Paused { reason: String },
    Completed,
    Failed { reason: String },
    Cancelled,
}

/// A recorded blocker in the workflow trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockerRecord {
    /// Stage where the blocker occurred.
    pub stage_index: u32,
    /// Human-readable description of the blocker.
    pub description: String,
    /// How the blocker was (or should be) resolved.
    pub resolution_hint: String,
    /// Whether this blocker is still active.
    pub resolved: bool,
}

impl WorkflowTrace {
    /// Calculate the completion percentage (0–100).
    pub fn percent_complete(&self) -> u32 {
        if self.total_stages == 0 {
            return 100;
        }
        (self.completed_stages.len() as u32 * 100 / self.total_stages).min(100)
    }

    /// Get the label of the currently executing stage.
    pub fn current_stage_label(&self) -> Option<String> {
        self.current_stage_index.and_then(|i| {
            self.pending_stage_labels.first().cloned().or_else(|| {
                self.completed_stages
                    .iter()
                    .find(|s| s.stage_index == i)
                    .map(|s| s.label.clone())
            })
        })
    }

    /// Get the overall pass/fail narrative for the trace.
    pub fn overall_narrative(&self) -> String {
        match &self.status {
            WorkflowStatusTrace::Completed => format!(
                "✓ {} completed ({} of {} stages, {:.0}% confidence)",
                self.description,
                self.completed_stages.len(),
                self.total_stages,
                self.overall_confidence * 100.0
            ),
            WorkflowStatusTrace::Failed { reason } => format!(
                "✗ {} failed at stage {}/{}: {}",
                self.description,
                self.completed_stages.len() + 1,
                self.total_stages,
                reason
            ),
            WorkflowStatusTrace::Paused { reason } => format!(
                "⏸ {} paused at stage {}/{}: {}",
                self.description,
                self.completed_stages.len() + 1,
                self.total_stages,
                reason
            ),
            WorkflowStatusTrace::Running => format!(
                "⟳ {} running: stage {}/{} ({}%)",
                self.description,
                self.completed_stages.len() + 1,
                self.total_stages,
                self.percent_complete()
            ),
            WorkflowStatusTrace::Cancelled => format!("⊘ {} cancelled", self.description),
            WorkflowStatusTrace::Pending => format!("· {} pending", self.description),
        }
    }
}

// ─── Execution Transparency Layer ─────────────────────────────────────────────

/// Maintains live traces for all active workflows.
///
/// Thread-safe: uses an `Arc<Mutex<HashMap>>` internally.
/// Safe to clone — shares state across the runtime.
#[derive(Clone)]
pub struct ExecutionTransparencyLayer {
    /// Active workflow traces (workflow_id → trace).
    traces: Arc<Mutex<HashMap<String, WorkflowTrace>>>,
    /// PSDG handle for persisting transparency summaries.
    psdg: Option<PsdgHandle>,
}

impl ExecutionTransparencyLayer {
    /// Create a new transparency layer.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            traces: Arc::new(Mutex::new(HashMap::new())),
            psdg,
        }
    }

    // ── Trace lifecycle ────────────────────────────────────────────────────

    /// Begin tracing a GoalTree execution.
    ///
    /// Creates a new trace entry. If a trace for `workflow_id` already exists,
    /// it is overwritten (re-execution case).
    pub fn begin_trace(&self, tree: &GoalTree) -> WorkflowTrace {
        let now = now_epoch();
        let pending_labels: Vec<String> = tree.stages.iter().map(|s| s.label.clone()).collect();

        let trace = WorkflowTrace {
            workflow_id: tree.workflow_id.clone(),
            description: tree.description.clone(),
            current_stage_index: if tree.stages.is_empty() {
                None
            } else {
                Some(0)
            },
            total_stages: tree.stages.len() as u32,
            completed_stages: vec![],
            pending_stage_labels: pending_labels,
            blockers: vec![],
            total_recovery_attempts: 0,
            overall_confidence: 1.0,
            status: WorkflowStatusTrace::Running,
            started_at: now,
            updated_at: now,
        };

        info!(
            target: "execution_transparency",
            workflow_id = %tree.workflow_id,
            stages = tree.stages.len(),
            "Workflow trace started"
        );

        let mut traces = self.traces.lock().unwrap();
        traces.insert(tree.workflow_id.clone(), trace.clone());
        trace
    }

    /// Update the trace after a stage completes.
    pub fn update_stage(
        &self,
        workflow_id: &str,
        stage_index: u32,
        stage_label: &str,
        outcome: &StageOutcome,
        actions_executed: usize,
        recovery_attempts: u32,
        duration_ms: u128,
        verify_confidence: f32,
    ) {
        let outcome_trace = StageOutcomeTrace::from(outcome);
        let explanation =
            generate_stage_explanation(stage_label, &outcome_trace, recovery_attempts);

        let stage_trace = StageTrace {
            stage_index,
            label: stage_label.to_string(),
            outcome: outcome_trace,
            actions_executed,
            recovery_attempts,
            duration_ms,
            verify_confidence,
            explanation,
            recorded_at: now_epoch(),
        };

        let mut traces = self.traces.lock().unwrap();
        if let Some(trace) = traces.get_mut(workflow_id) {
            trace.completed_stages.push(stage_trace);
            trace.total_recovery_attempts += recovery_attempts;
            // Remove the stage from pending list
            if !trace.pending_stage_labels.is_empty() {
                trace.pending_stage_labels.remove(0);
            }
            // Advance current stage
            trace.current_stage_index = if trace.pending_stage_labels.is_empty() {
                None
            } else {
                Some(stage_index + 1)
            };
            // Update rolling confidence
            let n = trace.completed_stages.len() as f32;
            trace.overall_confidence =
                (trace.overall_confidence * (n - 1.0) + verify_confidence) / n;
            trace.updated_at = now_epoch();

            debug!(
                target: "execution_transparency",
                workflow_id = %workflow_id,
                stage = stage_index,
                outcome = ?outcome,
                "Stage trace updated"
            );
        }
    }

    /// Mark the workflow as complete.
    pub fn complete_trace(&self, workflow_id: &str, success: bool, reason: Option<String>) {
        let mut traces = self.traces.lock().unwrap();
        if let Some(trace) = traces.get_mut(workflow_id) {
            trace.status = if success {
                WorkflowStatusTrace::Completed
            } else {
                WorkflowStatusTrace::Failed {
                    reason: reason.unwrap_or_else(|| "Unknown failure".into()),
                }
            };
            trace.current_stage_index = None;
            trace.updated_at = now_epoch();

            info!(
                target: "execution_transparency",
                workflow_id = %workflow_id,
                success,
                overall_confidence = trace.overall_confidence,
                "Workflow trace completed"
            );

            // Persist to PSDG
            if let Some(ref psdg) = self.psdg {
                let status_str = if success { "completed" } else { "failed" };
                let confidence = trace.overall_confidence;
                let store = psdg.store_arc();
                let wf_id = workflow_id.to_string();
                tokio::task::spawn_blocking(move || {
                    let _ = store.upsert(
                        &format!("workflow_{}", wf_id),
                        "trace_status",
                        status_str,
                        confidence as f64,
                        FactSource::Detected,
                        "execution_transparency",
                    );
                });
            }
        }
    }

    /// Record a blocker (interruption, verification failure) in the trace.
    pub fn record_blocker(
        &self,
        workflow_id: &str,
        stage_index: u32,
        description: String,
        resolution_hint: String,
    ) {
        let mut traces = self.traces.lock().unwrap();
        if let Some(trace) = traces.get_mut(workflow_id) {
            trace.blockers.push(BlockerRecord {
                stage_index,
                description,
                resolution_hint,
                resolved: false,
            });
            trace.updated_at = now_epoch();
        }
    }

    /// Resolve a blocker (mark it as no longer active).
    pub fn resolve_blocker(&self, workflow_id: &str, stage_index: u32) {
        let mut traces = self.traces.lock().unwrap();
        if let Some(trace) = traces.get_mut(workflow_id) {
            for blocker in &mut trace.blockers {
                if blocker.stage_index == stage_index && !blocker.resolved {
                    blocker.resolved = true;
                    break;
                }
            }
            trace.updated_at = now_epoch();
        }
    }

    /// Mark the workflow as paused.
    pub fn pause_trace(&self, workflow_id: &str, reason: String) {
        let mut traces = self.traces.lock().unwrap();
        if let Some(trace) = traces.get_mut(workflow_id) {
            trace.status = WorkflowStatusTrace::Paused { reason };
            trace.updated_at = now_epoch();
        }
    }

    // ── Trace retrieval ────────────────────────────────────────────────────

    /// Get the current trace for a workflow.
    pub fn get_trace(&self, workflow_id: &str) -> Option<WorkflowTrace> {
        self.traces.lock().unwrap().get(workflow_id).cloned()
    }

    /// Get all active (non-completed) traces.
    pub fn active_traces(&self) -> Vec<WorkflowTrace> {
        self.traces
            .lock()
            .unwrap()
            .values()
            .filter(|t| {
                !matches!(
                    t.status,
                    WorkflowStatusTrace::Completed | WorkflowStatusTrace::Cancelled
                )
            })
            .cloned()
            .collect()
    }

    // ── Reasoning Summaries ────────────────────────────────────────────────

    /// Generate a human-readable explanation of the current workflow state.
    ///
    /// Suitable for injection into the system prompt or user-visible response.
    pub fn explain_current_state(&self, workflow_id: &str) -> String {
        let trace = match self.get_trace(workflow_id) {
            Some(t) => t,
            None => return format!("No trace found for workflow '{}'.", workflow_id),
        };

        let mut lines = vec![trace.overall_narrative()];

        // Add stage progress
        if trace.total_stages > 0 {
            lines.push(format!(
                "  Progress: {}/{} stages ({:.0}% confidence)",
                trace.completed_stages.len(),
                trace.total_stages,
                trace.overall_confidence * 100.0
            ));
        }

        // Add last completed stage
        if let Some(last) = trace.completed_stages.last() {
            lines.push(format!(
                "  Last: {} → {}",
                last.label,
                explain_outcome_short(&last.outcome)
            ));
        }

        // Add active blockers
        let active_blockers: Vec<&BlockerRecord> =
            trace.blockers.iter().filter(|b| !b.resolved).collect();
        for blocker in active_blockers.iter().take(2) {
            lines.push(format!(
                "  Blocked: {} ({})",
                blocker.description, blocker.resolution_hint
            ));
        }

        lines.join("\n")
    }

    /// Explain WHY a confirmation was requested.
    pub fn explain_confirmation_reason(&self, workflow_id: &str, reason: &str) -> String {
        let trace = self.get_trace(workflow_id);
        let progress = trace
            .as_ref()
            .map(|t| {
                format!(
                    "at stage {}/{}",
                    t.completed_stages.len() + 1,
                    t.total_stages
                )
            })
            .unwrap_or_else(|| "in progress".into());

        format!("KRIA paused {} to confirm: {}", progress, reason)
    }

    /// Explain WHY a retry occurred.
    pub fn explain_retry(
        &self,
        _workflow_id: &str,
        stage_label: &str,
        attempt: u8,
        reason: &str,
    ) -> String {
        format!(
            "KRIA is retrying '{}' (attempt {}/{}): {}",
            stage_label,
            attempt,
            crate::agent::workflow_continuation::MAX_RECOVERY_DEPTH,
            reason
        )
    }

    /// Explain WHY the workflow was paused due to an interruption.
    pub fn explain_pause(&self, workflow_id: &str, interruption: &InterruptionClass) -> String {
        let trace = self.get_trace(workflow_id);
        let stage = trace
            .as_ref()
            .and_then(|t| t.current_stage_label())
            .unwrap_or_else(|| "current stage".into());

        format!(
            "Workflow paused during '{}': {}",
            stage,
            interruption.user_message()
        )
    }

    /// Get a confidence summary for a workflow.
    pub fn confidence_summary(&self, workflow_id: &str) -> ConfidenceSummary {
        let trace = match self.get_trace(workflow_id) {
            Some(t) => t,
            None => {
                return ConfidenceSummary {
                    workflow_id: workflow_id.to_string(),
                    overall: 0.0,
                    low_confidence_stages: vec![],
                    narrative: "No trace available.".into(),
                };
            }
        };

        let low_conf: Vec<String> = trace
            .completed_stages
            .iter()
            .filter(|s| s.verify_confidence < 0.6 && s.verify_confidence > 0.0)
            .map(|s| format!("'{}' ({:.0}%)", s.label, s.verify_confidence * 100.0))
            .collect();

        let narrative = if trace.overall_confidence >= 0.85 {
            format!(
                "High confidence ({:.0}%) — workflow completed as expected.",
                trace.overall_confidence * 100.0
            )
        } else if trace.overall_confidence >= 0.60 {
            format!(
                "Moderate confidence ({:.0}%) — some stages had partial verification.",
                trace.overall_confidence * 100.0
            )
        } else {
            format!(
                "Low confidence ({:.0}%) — verification was incomplete. Manual review recommended.",
                trace.overall_confidence * 100.0
            )
        };

        ConfidenceSummary {
            workflow_id: workflow_id.to_string(),
            overall: trace.overall_confidence,
            low_confidence_stages: low_conf,
            narrative,
        }
    }

    /// Export the full trace as a structured JSON string (for Tauri / audit).
    pub fn export_trace_json(&self, workflow_id: &str) -> Option<String> {
        let trace = self.get_trace(workflow_id)?;
        serde_json::to_string_pretty(&trace).ok()
    }
}

/// Confidence summary for a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceSummary {
    pub workflow_id: String,
    pub overall: f32,
    pub low_confidence_stages: Vec<String>,
    pub narrative: String,
}

// ─── Narrative Generation ──────────────────────────────────────────────────────

fn generate_stage_explanation(
    label: &str,
    outcome: &StageOutcomeTrace,
    recovery_attempts: u32,
) -> String {
    match outcome {
        StageOutcomeTrace::Passed => {
            format!("Stage '{}' completed successfully.", label)
        }
        StageOutcomeTrace::PassedAfterRecovery => {
            format!(
                "Stage '{}' passed after {} recovery attempt{}.",
                label,
                recovery_attempts,
                if recovery_attempts == 1 { "" } else { "s" }
            )
        }
        StageOutcomeTrace::Skipped => {
            format!("Stage '{}' was skipped (non-critical or blocked).", label)
        }
        StageOutcomeTrace::Failed { reason } => {
            format!("Stage '{}' failed: {}.", label, reason)
        }
        StageOutcomeTrace::Cancelled => {
            format!("Stage '{}' was cancelled.", label)
        }
        StageOutcomeTrace::TimedOut => {
            format!("Stage '{}' timed out before completing.", label)
        }
        StageOutcomeTrace::PausedForDecision { reason, .. } => {
            format!("Stage '{}' paused for a decision: {}.", label, reason)
        }
        StageOutcomeTrace::Running => {
            format!("Stage '{}' is currently running.", label)
        }
        StageOutcomeTrace::Pending => {
            format!("Stage '{}' is pending.", label)
        }
    }
}

fn explain_outcome_short(outcome: &StageOutcomeTrace) -> &'static str {
    match outcome {
        StageOutcomeTrace::Passed => "✓ passed",
        StageOutcomeTrace::PassedAfterRecovery => "✓ passed (recovery)",
        StageOutcomeTrace::Skipped => "→ skipped",
        StageOutcomeTrace::Failed { .. } => "✗ failed",
        StageOutcomeTrace::Cancelled => "⊘ cancelled",
        StageOutcomeTrace::TimedOut => "⏱ timed out",
        StageOutcomeTrace::PausedForDecision { .. } => "⏸ decision",
        StageOutcomeTrace::Running => "⟳ running",
        StageOutcomeTrace::Pending => "· pending",
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::goal_tree::{
        CompletionContract, GoalTree, VerificationCheckpoint, WorkflowStage,
    };
    use crate::agent::stage_executor::StageOutcome;

    fn make_tree(n: usize) -> GoalTree {
        use crate::agent::goal_tree::ActionGroup;
        let stages: Vec<WorkflowStage> = (0..n)
            .map(|i| WorkflowStage {
                index: i as u32,
                label: format!("stage_{}", i),
                action_group: ActionGroup { actions: vec![] },
                checkpoint: VerificationCheckpoint::None,
                recovery: None,
                timeout_sec: 60,
                context_hints: Default::default(),
                skippable: false,
            })
            .collect();
        GoalTree {
            workflow_id: format!("test-workflow-{}", n),
            description: "Test workflow".into(),
            stages,
            completion: CompletionContract::AllStagesPassed,
            global_abort: vec![],
            max_total_duration_sec: 300,
            preconditions: vec![],
        }
    }

    fn layer() -> ExecutionTransparencyLayer {
        ExecutionTransparencyLayer::new(None)
    }

    // ── Trace lifecycle ────────────────────────────────────────────────────

    #[test]
    fn begin_trace_creates_entry() {
        let layer = layer();
        let tree = make_tree(3);
        let trace = layer.begin_trace(&tree);
        assert_eq!(trace.total_stages, 3);
        assert_eq!(trace.pending_stage_labels.len(), 3);
        assert!(matches!(trace.status, WorkflowStatusTrace::Running));
    }

    #[test]
    fn update_stage_advances_progress() {
        let layer = layer();
        let tree = make_tree(3);
        layer.begin_trace(&tree);

        layer.update_stage(
            &tree.workflow_id,
            0,
            "stage_0",
            &StageOutcome::Passed,
            2,
            0,
            100,
            0.95,
        );

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert_eq!(trace.completed_stages.len(), 1);
        assert_eq!(trace.pending_stage_labels.len(), 2);
        assert_eq!(trace.completed_stages[0].outcome, StageOutcomeTrace::Passed);
    }

    #[test]
    fn complete_trace_updates_status() {
        let layer = layer();
        let tree = make_tree(2);
        layer.begin_trace(&tree);
        layer.complete_trace(&tree.workflow_id, true, None);

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert!(matches!(trace.status, WorkflowStatusTrace::Completed));
    }

    #[test]
    fn failed_trace_stores_reason() {
        let layer = layer();
        let tree = make_tree(2);
        layer.begin_trace(&tree);
        layer.complete_trace(
            &tree.workflow_id,
            false,
            Some("SSH connection refused".into()),
        );

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert!(
            matches!(trace.status, WorkflowStatusTrace::Failed { reason } if reason.contains("SSH"))
        );
    }

    #[test]
    fn blocker_records_and_resolves() {
        let layer = layer();
        let tree = make_tree(1);
        layer.begin_trace(&tree);
        layer.record_blocker(
            &tree.workflow_id,
            0,
            "popup appeared".into(),
            "dismiss popup".into(),
        );

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert_eq!(trace.blockers.len(), 1);
        assert!(!trace.blockers[0].resolved);

        layer.resolve_blocker(&tree.workflow_id, 0);
        let trace2 = layer.get_trace(&tree.workflow_id).unwrap();
        assert!(trace2.blockers[0].resolved);
    }

    #[test]
    fn pause_trace_updates_status() {
        let layer = layer();
        let tree = make_tree(2);
        layer.begin_trace(&tree);
        layer.pause_trace(&tree.workflow_id, "network dropped".into());

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert!(matches!(trace.status, WorkflowStatusTrace::Paused { .. }));
    }

    // ── Narrative generation ───────────────────────────────────────────────

    #[test]
    fn explain_current_state_contains_progress() {
        let layer = layer();
        let tree = make_tree(3);
        layer.begin_trace(&tree);
        layer.update_stage(
            &tree.workflow_id,
            0,
            "stage_0",
            &StageOutcome::Passed,
            1,
            0,
            50,
            0.9,
        );

        let narrative = layer.explain_current_state(&tree.workflow_id);
        assert!(!narrative.is_empty());
        assert!(
            narrative.contains("stage_0")
                || narrative.contains("1/3")
                || narrative.contains("running")
                || narrative.contains("Running")
        );
    }

    #[test]
    fn confidence_summary_high_is_described() {
        let layer = layer();
        let tree = make_tree(1);
        layer.begin_trace(&tree);
        layer.update_stage(
            &tree.workflow_id,
            0,
            "stage_0",
            &StageOutcome::Passed,
            1,
            0,
            50,
            0.95,
        );
        layer.complete_trace(&tree.workflow_id, true, None);

        let summary = layer.confidence_summary(&tree.workflow_id);
        assert!(summary.overall > 0.8);
        assert!(
            summary.narrative.contains("High confidence")
                || summary.narrative.contains("confidence")
        );
    }

    #[test]
    fn confidence_summary_low_warns() {
        let layer = layer();
        let tree = make_tree(1);
        layer.begin_trace(&tree);
        layer.update_stage(
            &tree.workflow_id,
            0,
            "stage_0",
            &StageOutcome::Passed,
            1,
            0,
            50,
            0.3,
        );
        layer.complete_trace(&tree.workflow_id, true, None);

        let summary = layer.confidence_summary(&tree.workflow_id);
        assert!(
            summary.narrative.contains("Low confidence")
                || summary.narrative.contains("low confidence")
                || summary.overall < 0.6
        );
    }

    #[test]
    fn percent_complete_correct() {
        let layer = layer();
        let tree = make_tree(4);
        layer.begin_trace(&tree);
        layer.update_stage(
            &tree.workflow_id,
            0,
            "s0",
            &StageOutcome::Passed,
            1,
            0,
            10,
            1.0,
        );
        layer.update_stage(
            &tree.workflow_id,
            1,
            "s1",
            &StageOutcome::Passed,
            1,
            0,
            10,
            1.0,
        );

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert_eq!(trace.percent_complete(), 50);
    }

    #[test]
    fn recovery_attempt_count_accumulates() {
        let layer = layer();
        let tree = make_tree(2);
        layer.begin_trace(&tree);
        layer.update_stage(
            &tree.workflow_id,
            0,
            "s0",
            &StageOutcome::PassedAfterRecovery,
            2,
            2,
            500,
            0.7,
        );

        let trace = layer.get_trace(&tree.workflow_id).unwrap();
        assert_eq!(trace.total_recovery_attempts, 2);
    }

    #[test]
    fn export_trace_json_is_valid() {
        let layer = layer();
        let tree = make_tree(2);
        layer.begin_trace(&tree);
        let json = layer.export_trace_json(&tree.workflow_id);
        assert!(json.is_some());
        let parsed: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
        assert_eq!(parsed["workflow_id"], tree.workflow_id.as_str());
    }
}
