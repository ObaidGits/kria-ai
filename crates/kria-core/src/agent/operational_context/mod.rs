//! Batch 3 — Operational Context Tracker.
//!
//! # Core Mission
//!
//! Maintain a bounded, prunable operational context that represents "what has
//! KRIA been doing recently." This is the _coworker memory_ — the context a
//! team member carries in their head, not a vector database.
//!
//! # What Is Tracked
//!
//! | Subject prefix        | Fact                                      |
//! |-----------------------|-------------------------------------------|
//! | `op.context.current`  | Active workflow session ID                |
//! | `op.context.chain`    | Last N workflow IDs (JSON array)          |
//! | `op.context.project`  | Active project root path                  |
//! | `op.context.browser`  | Last known browser URL                    |
//! | `op.context.ide`      | Last known IDE workspace root             |
//! | `op.interruption.*`   | Interruption lineage (class + timestamp)  |
//! | `op.recovery.*`       | Recovery lineage (action + outcome)       |
//!
//! # Invariants
//!
//! - Writes are fire-and-forget via `PsdgHandle`.
//! - Workflow chain is bounded to `MAX_CHAIN_LENGTH` (5).
//! - All context facts decay at the PSDG default rate (0.05/h).
//! - This module NEVER reads from the LLM.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::agent::psdg::PsdgHandle;
use crate::agent::world_model::FactSource;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum workflow IDs kept in the recent-chain fact.
pub const MAX_CHAIN_LENGTH: usize = 5;

/// Maximum interruption events tracked in lineage.
pub const MAX_INTERRUPTION_LINEAGE: usize = 10;

/// Maximum recovery events tracked in lineage.
pub const MAX_RECOVERY_LINEAGE: usize = 10;

// ─── Lineage Records ─────────────────────────────────────────────────────────

/// A single entry in the interruption lineage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterruptionEntry {
    /// Session ID this interruption belongs to.
    pub session_id: String,
    /// Human-readable interruption class name.
    pub class: String,
    /// When the interruption occurred (Unix seconds).
    pub timestamp: u64,
    /// Whether it required human intervention.
    pub required_human: bool,
}

/// A single entry in the recovery lineage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEntry {
    /// Session ID this recovery belongs to.
    pub session_id: String,
    /// Human-readable recovery action.
    pub action: String,
    /// Whether the recovery succeeded.
    pub succeeded: bool,
    /// When the recovery occurred (Unix seconds).
    pub timestamp: u64,
}

// ─── Operational Context ─────────────────────────────────────────────────────

/// In-memory snapshot of the current operational context.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperationalContextSnapshot {
    /// Currently active workflow session ID, if any.
    pub current_session_id: Option<String>,
    /// Recent workflow session IDs (bounded to MAX_CHAIN_LENGTH).
    pub recent_session_chain: Vec<String>,
    /// Active project root path.
    pub active_project: Option<String>,
    /// Last known browser URL.
    pub browser_url: Option<String>,
    /// Last known IDE workspace root.
    pub ide_workspace: Option<String>,
    /// Recent interruptions (bounded).
    pub interruption_lineage: Vec<InterruptionEntry>,
    /// Recent recoveries (bounded).
    pub recovery_lineage: Vec<RecoveryEntry>,
}

// ─── Tracker ─────────────────────────────────────────────────────────────────

/// Tracks operational context and persists it to PSDG.
///
/// All writes are fire-and-forget. The in-memory state is always consistent.
/// Use `snapshot()` to get a bounded read of current context.
pub struct OperationalContextTracker {
    psdg: Option<PsdgHandle>,
    state: Mutex<OperationalContextSnapshot>,
}

impl OperationalContextTracker {
    /// Create a new tracker, optionally backed by PSDG persistence.
    pub fn new(psdg: Option<PsdgHandle>) -> Self {
        Self {
            psdg,
            state: Mutex::new(OperationalContextSnapshot::default()),
        }
    }

    /// Record that a workflow started.
    ///
    /// Updates the current session and prepends to the recent chain.
    pub fn record_workflow_started(&self, session_id: &str, description: &str) {
        let mut state = self.state.lock().unwrap();

        // Push to chain (bounded)
        if let Some(prev) = state.current_session_id.take() {
            if !state.recent_session_chain.contains(&prev) {
                state.recent_session_chain.insert(0, prev);
                state.recent_session_chain.truncate(MAX_CHAIN_LENGTH);
            }
        }
        state.current_session_id = Some(session_id.to_string());

        debug!(
            target: "operational_context",
            session_id, description,
            "workflow started recorded"
        );

        // Persist to PSDG
        self.write_psdg("op.context.current", session_id, FactSource::Detected, 0.95);
        let chain_json = serde_json::to_string(&state.recent_session_chain).unwrap_or_default();
        drop(state);
        self.write_psdg("op.context.chain", &chain_json, FactSource::Detected, 0.90);
    }

    /// Record that a workflow completed or failed.
    pub fn record_workflow_ended(&self, session_id: &str, succeeded: bool) {
        let mut state = self.state.lock().unwrap();
        if state.current_session_id.as_deref() == Some(session_id) {
            state.current_session_id = None;
        }
        if !state.recent_session_chain.contains(&session_id.to_string()) {
            state.recent_session_chain.insert(0, session_id.to_string());
            state.recent_session_chain.truncate(MAX_CHAIN_LENGTH);
        }
        let outcome = if succeeded { "completed" } else { "failed" };
        debug!(target: "operational_context", session_id, outcome, "workflow ended recorded");
        drop(state);
        self.write_psdg("op.context.current", "", FactSource::Detected, 0.5);
    }

    /// Update the active project path.
    pub fn set_active_project(&self, root: &str) {
        {
            let mut state = self.state.lock().unwrap();
            state.active_project = Some(root.to_string());
        }
        self.write_psdg("op.context.project", root, FactSource::Detected, 0.95);
    }

    /// Update the last known browser URL.
    pub fn set_browser_url(&self, url: &str) {
        {
            let mut state = self.state.lock().unwrap();
            state.browser_url = Some(url.to_string());
        }
        self.write_psdg("op.context.browser", url, FactSource::Detected, 0.90);
    }

    /// Update the last known IDE workspace root.
    pub fn set_ide_workspace(&self, workspace: &str) {
        {
            let mut state = self.state.lock().unwrap();
            state.ide_workspace = Some(workspace.to_string());
        }
        self.write_psdg("op.context.ide", workspace, FactSource::Detected, 0.90);
    }

    /// Record an interruption.
    pub fn record_interruption(&self, session_id: &str, class: &str, required_human: bool) {
        let entry = InterruptionEntry {
            session_id: session_id.to_string(),
            class: class.to_string(),
            timestamp: now_epoch(),
            required_human,
        };
        {
            let mut state = self.state.lock().unwrap();
            state.interruption_lineage.insert(0, entry.clone());
            state
                .interruption_lineage
                .truncate(MAX_INTERRUPTION_LINEAGE);
        }
        let subject = format!("op.interruption.{}", session_id);
        let obj = format!("class={} required_human={}", entry.class, required_human);
        self.write_psdg(&subject, &obj, FactSource::Detected, 0.90);
    }

    /// Record a recovery attempt.
    pub fn record_recovery(&self, session_id: &str, action: &str, succeeded: bool) {
        let entry = RecoveryEntry {
            session_id: session_id.to_string(),
            action: action.to_string(),
            succeeded,
            timestamp: now_epoch(),
        };
        {
            let mut state = self.state.lock().unwrap();
            state.recovery_lineage.insert(0, entry.clone());
            state.recovery_lineage.truncate(MAX_RECOVERY_LINEAGE);
        }
        let subject = format!("op.recovery.{}", session_id);
        let obj = format!("action={} succeeded={}", action, succeeded);
        self.write_psdg(&subject, &obj, FactSource::Detected, 0.85);
    }

    /// Return a snapshot of the current operational context.
    pub fn snapshot(&self) -> OperationalContextSnapshot {
        self.state.lock().unwrap().clone()
    }

    /// Whether there is currently an active workflow session.
    pub fn is_workflow_active(&self) -> bool {
        self.state.lock().unwrap().current_session_id.is_some()
    }

    // ── Internal ─────────────────────────────────────────────────────────────

    fn write_psdg(&self, subject: &str, object: &str, source: FactSource, confidence: f64) {
        if let Some(ref psdg) = self.psdg {
            psdg.record_fact(subject, "op_context", object, confidence, source, "");
        }
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> OperationalContextTracker {
        OperationalContextTracker::new(None)
    }

    #[test]
    fn record_workflow_started_sets_current_session() {
        let t = tracker();
        t.record_workflow_started("s1", "do stuff");
        assert_eq!(t.snapshot().current_session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn record_workflow_ended_clears_current_session() {
        let t = tracker();
        t.record_workflow_started("s1", "do stuff");
        t.record_workflow_ended("s1", true);
        assert!(t.snapshot().current_session_id.is_none());
    }

    #[test]
    fn chain_bounded_to_max_chain_length() {
        let t = tracker();
        for i in 0..=(MAX_CHAIN_LENGTH + 3) {
            t.record_workflow_started(&format!("s{}", i), "wf");
            t.record_workflow_ended(&format!("s{}", i), true);
        }
        let snap = t.snapshot();
        assert!(
            snap.recent_session_chain.len() <= MAX_CHAIN_LENGTH,
            "chain overflowed: {}",
            snap.recent_session_chain.len()
        );
    }

    #[test]
    fn set_browser_url_updates_snapshot() {
        let t = tracker();
        t.set_browser_url("https://example.com");
        assert_eq!(
            t.snapshot().browser_url.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn record_interruption_bounded() {
        let t = tracker();
        for _ in 0..(MAX_INTERRUPTION_LINEAGE + 5) {
            t.record_interruption("s1", "Timeout", false);
        }
        assert!(t.snapshot().interruption_lineage.len() <= MAX_INTERRUPTION_LINEAGE);
    }

    #[test]
    fn record_recovery_bounded() {
        let t = tracker();
        for _ in 0..(MAX_RECOVERY_LINEAGE + 5) {
            t.record_recovery("s1", "retry", true);
        }
        assert!(t.snapshot().recovery_lineage.len() <= MAX_RECOVERY_LINEAGE);
    }

    #[test]
    fn is_workflow_active_reflects_state() {
        let t = tracker();
        assert!(!t.is_workflow_active());
        t.record_workflow_started("s1", "wf");
        assert!(t.is_workflow_active());
        t.record_workflow_ended("s1", true);
        assert!(!t.is_workflow_active());
    }

    #[test]
    fn set_active_project_updates_snapshot() {
        let t = tracker();
        t.set_active_project("/home/user/project");
        assert_eq!(
            t.snapshot().active_project.as_deref(),
            Some("/home/user/project")
        );
    }
}
