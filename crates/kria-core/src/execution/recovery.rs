//! A7.8 Failure Recovery — configurable recovery policies.
//!
//! Supports retry, alternate node, alternate executor, checkpoint restore,
//! rollback, partial completion, cancellation and abort. Backend-agnostic:
//! recovery decisions are made on the abstract graph/executor level.

use serde::{Deserialize, Serialize};

/// What to do when a node fails (A7.8). Evaluated in order by the scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryAction {
    /// Retry the same node up to `max_attempts` with backoff.
    Retry { max_attempts: u32, backoff_ms: u64 },
    /// Try a different node id instead.
    AlternateNode { node_id: String },
    /// Re-run the same action on a different provider (open-vocabulary id).
    AlternateExecutor { provider_id: String },
    /// Restore context from a checkpoint label.
    CheckpointRestore { label: String },
    /// Roll back to a checkpoint label.
    Rollback { to_label: String },
    /// Accept partial completion and continue.
    PartialCompletion,
    /// Cancel the whole graph gracefully.
    Cancel,
    /// Abort immediately (hard stop).
    Abort,
}

/// A recovery policy: an ordered list of actions to try on failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    pub actions: Vec<RecoveryAction>,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        // Sensible default: retry twice, then abort.
        Self {
            actions: vec![
                RecoveryAction::Retry {
                    max_attempts: 2,
                    backoff_ms: 100,
                },
                RecoveryAction::Abort,
            ],
        }
    }
}

impl RecoveryPolicy {
    pub fn new(actions: Vec<RecoveryAction>) -> Self {
        Self { actions }
    }

    /// A policy that never recovers (fail-fast).
    pub fn abort_only() -> Self {
        Self {
            actions: vec![RecoveryAction::Abort],
        }
    }

    /// A retry-only policy.
    pub fn retry(max_attempts: u32, backoff_ms: u64) -> Self {
        Self {
            actions: vec![
                RecoveryAction::Retry {
                    max_attempts,
                    backoff_ms,
                },
                RecoveryAction::Abort,
            ],
        }
    }
}

/// Outcome of applying recovery to a failed node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Retry the node now.
    RetryNow,
    /// Switch to an alternate node.
    UseAlternateNode(String),
    /// Switch to an alternate provider (open-vocabulary id).
    UseAlternateExecutor(String),
    /// Restore/rollback to a checkpoint label.
    RestoreCheckpoint(String),
    /// Continue despite failure.
    Continue,
    /// Cancel the whole graph.
    Cancel,
    /// Abort immediately.
    Abort,
}

/// The single recovery manager (A7.8). Deterministic decision engine.
#[derive(Default)]
pub struct RecoveryManager;

impl RecoveryManager {
    /// Decide the recovery outcome given a policy and the current attempt count.
    pub fn decide(policy: &RecoveryPolicy, attempts_so_far: u32) -> RecoveryOutcome {
        for action in &policy.actions {
            match action {
                RecoveryAction::Retry { max_attempts, .. } => {
                    if attempts_so_far < *max_attempts {
                        return RecoveryOutcome::RetryNow;
                    }
                    // exhausted retries → fall through to next action
                }
                RecoveryAction::AlternateNode { node_id } => {
                    return RecoveryOutcome::UseAlternateNode(node_id.clone());
                }
                RecoveryAction::AlternateExecutor { provider_id } => {
                    return RecoveryOutcome::UseAlternateExecutor(provider_id.clone());
                }
                RecoveryAction::CheckpointRestore { label }
                | RecoveryAction::Rollback { to_label: label } => {
                    return RecoveryOutcome::RestoreCheckpoint(label.clone());
                }
                RecoveryAction::PartialCompletion => return RecoveryOutcome::Continue,
                RecoveryAction::Cancel => return RecoveryOutcome::Cancel,
                RecoveryAction::Abort => return RecoveryOutcome::Abort,
            }
        }
        RecoveryOutcome::Abort
    }

    /// Backoff duration for a given retry policy and attempt.
    pub fn backoff_ms(policy: &RecoveryPolicy, attempt: u32) -> u64 {
        for action in &policy.actions {
            if let RecoveryAction::Retry { backoff_ms, .. } = action {
                return backoff_ms.saturating_mul(attempt.max(1) as u64);
            }
        }
        0
    }
}
