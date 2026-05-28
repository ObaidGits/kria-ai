//! Workflow Lifecycle State Machine — Deterministic State Ownership.
//!
//! Every workflow instance follows this FSM. State transitions are:
//! - Explicit (no implicit mutation)
//! - Validated (invalid transitions are compile-time or runtime errors)
//! - Traceable (every transition emits telemetry)
//! - Monotonic (timestamps always increase)
//!
//! # Authority
//!
//! This module OWNS workflow state. No other module may mutate workflow state
//! directly. All state changes flow through `WorkflowInstance::transition()`.

use std::time::Instant;

use crate::agent::workflow_types::{
    HitlReason, TelemetryEnvelope, WorkflowSource, WorkflowState, WorkflowTelemetry,
    WorkflowVerdict, TELEMETRY_VERSION,
};

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Workflow Instance (Runtime State Container)
// ═══════════════════════════════════════════════════════════════════════════════

/// A live workflow instance with deterministic lifecycle management.
#[derive(Debug)]
pub struct WorkflowInstance {
    /// Unique workflow identifier
    pub id: String,
    /// Human-readable title
    pub title: String,
    /// Current lifecycle state
    state: WorkflowState,
    /// When this workflow was created
    created_at: Instant,
    /// Monotonic sequence counter for telemetry
    seq: u64,
    /// Total steps in the workflow
    pub total_steps: u32,
    /// Which system produced this workflow
    pub source: WorkflowSource,
    /// Telemetry events emitted during this workflow's lifetime
    trace: Vec<TelemetryEnvelope>,
}

impl WorkflowInstance {
    /// Create a new workflow instance in the `Created` state.
    pub fn new(id: String, title: String, total_steps: u32, source: WorkflowSource) -> Self {
        Self {
            id,
            title,
            state: WorkflowState::Created,
            created_at: Instant::now(),
            seq: 0,
            total_steps,
            source,
            trace: Vec::new(),
        }
    }

    /// Get the current state (read-only).
    pub fn state(&self) -> &WorkflowState {
        &self.state
    }

    /// Get elapsed time since creation.
    pub fn elapsed_ms(&self) -> u64 {
        self.created_at.elapsed().as_millis() as u64
    }

    /// Get the telemetry trace for debugging/persistence.
    pub fn trace(&self) -> &[TelemetryEnvelope] {
        &self.trace
    }

    // ─── State Transitions ────────────────────────────────────────────────

    /// Transition to Planned state.
    pub fn mark_planned(&mut self) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Created => {
                self.state = WorkflowState::Planned;
                self.emit_trace("planned");
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: "Planned".into(),
            }),
        }
    }

    /// Transition to Executing state.
    pub fn mark_executing(&mut self, step: u32) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Planned
            | WorkflowState::Executing { .. }
            | WorkflowState::HitlPending { .. } => {
                let completed = match &self.state {
                    WorkflowState::Executing { completed_steps, .. } => *completed_steps,
                    WorkflowState::HitlPending { suspended_at_step, .. } => *suspended_at_step,
                    _ => 0,
                };
                self.state = WorkflowState::Executing {
                    current_step: step,
                    completed_steps: completed,
                };
                self.emit_trace(&format!("executing_step_{}", step));
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: "Executing".into(),
            }),
        }
    }

    /// Record a step completion (stays in Executing state).
    pub fn mark_step_completed(&mut self, step: u32) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Executing { current_step, .. } if *current_step == step => {
                self.state = WorkflowState::Executing {
                    current_step: step,
                    completed_steps: step,
                };
                self.emit_trace(&format!("step_{}_completed", step));
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: format!("StepCompleted({})", step),
            }),
        }
    }

    /// Transition to HitlPending state.
    pub fn mark_hitl_pending(
        &mut self,
        reason: HitlReason,
        at_step: u32,
    ) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Executing { .. } => {
                self.state = WorkflowState::HitlPending {
                    reason,
                    suspended_at_step: at_step,
                };
                self.emit_trace(&format!("hitl_pending_at_step_{}", at_step));
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: "HitlPending".into(),
            }),
        }
    }

    /// Transition to Verifying state.
    pub fn mark_verifying(&mut self) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Executing { .. } => {
                self.state = WorkflowState::Verifying;
                self.emit_trace("verifying");
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: "Verifying".into(),
            }),
        }
    }

    /// Transition to Finalized state with a verdict.
    pub fn mark_finalized(&mut self, verdict: WorkflowVerdict) -> Result<(), LifecycleError> {
        match &self.state {
            WorkflowState::Created  // plan failed
            | WorkflowState::Executing { .. }  // step failed fatally
            | WorkflowState::Verifying => {
                self.state = WorkflowState::Finalized {
                    verdict: verdict.clone(),
                };
                self.emit_trace(&format!("finalized_{:?}", verdict));
                Ok(())
            }
            other => Err(LifecycleError::InvalidTransition {
                from: format!("{:?}", other),
                to: "Finalized".into(),
            }),
        }
    }

    /// Transition to Cancelled state.
    pub fn mark_cancelled(&mut self, reason: String) -> Result<(), LifecycleError> {
        let at_step = match &self.state {
            WorkflowState::Executing { current_step, .. } => *current_step,
            WorkflowState::HitlPending { suspended_at_step, .. } => *suspended_at_step,
            _ => 0,
        };
        // Cancellation is allowed from any non-terminal state
        match &self.state {
            WorkflowState::Finalized { .. } | WorkflowState::Cancelled { .. } => {
                Err(LifecycleError::InvalidTransition {
                    from: format!("{:?}", self.state),
                    to: "Cancelled".into(),
                })
            }
            _ => {
                self.state = WorkflowState::Cancelled {
                    reason: reason.clone(),
                    at_step,
                };
                self.emit_trace(&format!("cancelled: {}", reason));
                Ok(())
            }
        }
    }

    /// Check if the workflow is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            WorkflowState::Finalized { .. } | WorkflowState::Cancelled { .. }
        )
    }

    // ─── Telemetry ────────────────────────────────────────────────────────

    fn emit_trace(&mut self, label: &str) {
        self.seq += 1;
        self.trace.push(TelemetryEnvelope {
            version: TELEMETRY_VERSION,
            seq: self.seq,
            event: WorkflowTelemetry::StepStarted {
                workflow_id: self.id.clone(),
                step_index: self.seq as u32,
                description: label.to_string(),
                step_type: crate::agent::workflow_types::StepType::Verification,
            },
            timestamp_ms: self.elapsed_ms(),
            source: self.source,
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Lifecycle Errors
// ═══════════════════════════════════════════════════════════════════════════════

/// Error when an invalid state transition is attempted.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LifecycleError {
    #[error("Invalid transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::workflow_types::*;

    fn make_instance() -> WorkflowInstance {
        WorkflowInstance::new(
            "test-wf-1".into(),
            "Test Workflow".into(),
            3,
            WorkflowSource::SubstrateRouter,
        )
    }

    #[test]
    fn lifecycle_happy_path() {
        let mut wf = make_instance();
        assert!(matches!(wf.state(), WorkflowState::Created));

        wf.mark_planned().unwrap();
        assert!(matches!(wf.state(), WorkflowState::Planned));

        wf.mark_executing(1).unwrap();
        assert!(matches!(wf.state(), WorkflowState::Executing { current_step: 1, .. }));

        wf.mark_step_completed(1).unwrap();
        wf.mark_executing(2).unwrap();
        wf.mark_step_completed(2).unwrap();
        wf.mark_executing(3).unwrap();
        wf.mark_step_completed(3).unwrap();

        wf.mark_verifying().unwrap();
        assert!(matches!(wf.state(), WorkflowState::Verifying));

        wf.mark_finalized(WorkflowVerdict::Complete).unwrap();
        assert!(matches!(wf.state(), WorkflowState::Finalized { .. }));
        assert!(wf.is_terminal());
    }

    #[test]
    fn lifecycle_with_hitl_pause_and_resume() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();
        wf.mark_step_completed(1).unwrap();
        wf.mark_executing(2).unwrap();

        // HITL pause
        wf.mark_hitl_pending(
            HitlReason::InstallRequired { app: "code".into(), install_command: None },
            2,
        ).unwrap();
        assert!(matches!(wf.state(), WorkflowState::HitlPending { .. }));

        // Resume after user responds
        wf.mark_executing(2).unwrap();
        assert!(matches!(wf.state(), WorkflowState::Executing { current_step: 2, .. }));
    }

    #[test]
    fn lifecycle_cancellation_from_executing() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();

        wf.mark_cancelled("User cancelled".into()).unwrap();
        assert!(matches!(wf.state(), WorkflowState::Cancelled { .. }));
        assert!(wf.is_terminal());
    }

    #[test]
    fn lifecycle_cancellation_from_hitl() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();
        wf.mark_hitl_pending(
            HitlReason::LoginRequired { service: "youtube".into(), guidance: "".into() },
            1,
        ).unwrap();

        wf.mark_cancelled("User cancelled during HITL".into()).unwrap();
        assert!(wf.is_terminal());
    }

    #[test]
    fn cannot_transition_from_terminal_state() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();
        wf.mark_verifying().unwrap();
        wf.mark_finalized(WorkflowVerdict::Complete).unwrap();

        // Cannot go back to executing
        assert!(wf.mark_executing(2).is_err());
        // Cannot cancel a finalized workflow
        assert!(wf.mark_cancelled("too late".into()).is_err());
    }

    #[test]
    fn cannot_skip_planned_state() {
        let mut wf = make_instance();
        // Cannot go directly from Created to Executing
        assert!(wf.mark_executing(1).is_err());
    }

    #[test]
    fn cannot_verify_from_planned() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        // Cannot verify without executing
        assert!(wf.mark_verifying().is_err());
    }

    #[test]
    fn trace_records_all_transitions() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();
        wf.mark_step_completed(1).unwrap();
        wf.mark_verifying().unwrap();
        wf.mark_finalized(WorkflowVerdict::Complete).unwrap();

        // Should have 5 trace entries
        assert_eq!(wf.trace().len(), 5);
        // Sequence numbers should be monotonic
        for (i, entry) in wf.trace().iter().enumerate() {
            assert_eq!(entry.seq, (i + 1) as u64);
        }
        // Timestamps should be monotonically non-decreasing
        for window in wf.trace().windows(2) {
            assert!(window[1].timestamp_ms >= window[0].timestamp_ms);
        }
    }

    #[test]
    fn failed_workflow_can_finalize_from_executing() {
        let mut wf = make_instance();
        wf.mark_planned().unwrap();
        wf.mark_executing(1).unwrap();

        // Step fails fatally — finalize directly from Executing
        wf.mark_finalized(WorkflowVerdict::Failed {
            step: 1,
            reason: "app not found".into(),
            recovery: None,
        }).unwrap();
        assert!(wf.is_terminal());
    }

    #[test]
    fn elapsed_ms_increases() {
        let wf = make_instance();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(wf.elapsed_ms() >= 5);
    }
}
