//! GUI Cognition V2 — cross-substrate execution BRIDGE (spec Task 10, Req 16).
//!
//! Some sub-goals are not GUI actions at all: running a shell command, writing a
//! file, or reading captured output. Driving those through keystrokes is fragile
//! ("type the command and hope it ran"). The bridge instead routes a
//! [`SubGoalKind::is_bridged`] sub-goal to the EXISTING shell/file tool
//! executors, captures the result into a per-turn [`WorkingContext`], and lets
//! the SAME external-signal verifier confirm it (command output / file exists).
//!
//! Core defines the seam ([`GuiBridge`]) + the working context; the desktop wires
//! the concrete bridge over the real tool registry (with the unchanged safety
//! gate — Requirement 16.5). The loop calls the bridge ONLY for bridged
//! sub-goals; GUI sub-goals stay on the Sight/Brain/Hands path.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::types::SubGoal;

/// The captured result of one bridged sub-goal.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeOutcome {
    /// Whether the underlying tool reported success.
    pub ok: bool,
    /// Captured textual output (stdout / created path / surfaced text). May be
    /// empty when the tool produced none.
    pub output: String,
    /// Human-readable detail for events/logs.
    pub detail: String,
}

impl BridgeOutcome {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
            detail: String::new(),
        }
    }
    pub fn failed(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: String::new(),
            detail: detail.into(),
        }
    }
}

/// Per-turn working context: results of bridged sub-goals, available to LATER
/// sub-goals and to the final reply (Requirement 16.2). Cloneable handle around
/// shared state so the bridge (writer) and the verification probe (reader, via
/// `command_output`) observe the same captured output.
#[derive(Clone, Default)]
pub struct WorkingContext {
    inner: Arc<Mutex<WorkingContextState>>,
}

#[derive(Default)]
struct WorkingContextState {
    /// Most-recent bridged command/output (what `command_output` returns).
    last_output: Option<String>,
    /// All bridged results in order (intent → output), for the final reply.
    history: Vec<(String, String)>,
}

impl WorkingContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a bridged result (most-recent wins for `last_output`).
    pub fn record(&self, intent: &str, output: &str) {
        let mut s = self.inner.lock().unwrap();
        if !output.trim().is_empty() {
            s.last_output = Some(output.to_string());
        }
        s.history.push((intent.to_string(), output.to_string()));
    }

    /// The most-recent non-empty captured output (for the RunCommand/ReadOutput
    /// verifier signal). `None` when nothing has been captured yet.
    pub fn last_output(&self) -> Option<String> {
        self.inner.lock().unwrap().last_output.clone()
    }

    /// A compact human summary of all captured outputs (for the final reply).
    pub fn summary(&self) -> String {
        let s = self.inner.lock().unwrap();
        s.history
            .iter()
            .filter(|(_, o)| !o.trim().is_empty())
            .map(|(i, o)| format!("{i}: {}", o.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The cross-substrate bridge seam. The desktop implements this over the real
/// shell/file tools; the loop calls it for bridged sub-goals only.
#[async_trait]
pub trait GuiBridge: Send + Sync {
    /// Execute one bridged sub-goal (run-command / write-file / read-output) via
    /// the existing tool substrate, returning the captured outcome. MUST pass
    /// through the existing safety gate unchanged (Requirement 16.5).
    async fn execute(&self, sub_goal: &SubGoal) -> BridgeOutcome;

    /// Whether THIS bridge wants to handle the given bridged `kind`. The loop
    /// only routes a sub-goal to the bridge when both `kind.is_bridged()` AND
    /// this returns true; otherwise the sub-goal stays on the GUI path. Lets a
    /// desktop keep, e.g., `RunCommand` as visible-terminal typing while still
    /// bridging file writes. Default: handle every bridged kind.
    fn handles(&self, kind: super::types::SubGoalKind) -> bool {
        kind.is_bridged()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_context_records_and_summarizes() {
        let ctx = WorkingContext::new();
        assert_eq!(ctx.last_output(), None);
        ctx.record("run ls", "file1\nfile2");
        assert_eq!(ctx.last_output().as_deref(), Some("file1\nfile2"));
        // Empty output does not clobber the last non-empty output.
        ctx.record("write file", "");
        assert_eq!(ctx.last_output().as_deref(), Some("file1\nfile2"));
        let sum = ctx.summary();
        assert!(sum.contains("run ls: file1"));
    }

    #[test]
    fn bridge_outcome_constructors() {
        assert!(BridgeOutcome::ok("x").ok);
        assert!(!BridgeOutcome::failed("nope").ok);
    }
}
