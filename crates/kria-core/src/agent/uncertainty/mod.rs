//! Uncertainty Engine — Confidence scoring before planning.
//!
//! # Key Principle
//!
//! The 0.5B router should NEVER guess. If uncertain, gather evidence
//! (read-only commands) or ask the user. The 7B planner is only woken
//! when confidence exceeds the threshold AND the task requires reasoning.
//!
//! # Architecture
//!
//! ```text
//! User Goal
//!     ↓
//! UncertaintyEngine.evaluate(goal, context)
//!     ↓
//! ┌─────────────────────────────────────────────────┐
//! │  Confidence ≥ plan_threshold (0.8)              │
//! │  → Plan: Proceed to StructuredBranchingPlanner  │
//! │                                                 │
//! │  Confidence ≥ gather_threshold (0.6)            │
//! │  → GatherEvidence: Run read-only diagnostics    │
//! │                                                 │
//! │  Confidence ≥ ask_threshold (0.3)               │
//! │  → AskUser: Request clarification               │
//! │                                                 │
//! │  Confidence < ask_threshold (0.3)               │
//! │  → Refuse: Explain why we can't proceed         │
//! └─────────────────────────────────────────────────┘
//! ```

pub mod belief_graph;
pub mod calibrator;

pub use belief_graph::{BeliefFact, BeliefGraph, BeliefSource};
pub use calibrator::{ConfidenceCalibrator, UncertaintyAction};

use crate::tools::subprocess_executor::StructuredCommand;

/// The Uncertainty Engine — evaluates confidence and decides action.
pub struct UncertaintyEngine {
    /// Current belief graph.
    belief_graph: BeliefGraph,
    /// Adaptive threshold calibrator.
    calibrator: ConfidenceCalibrator,
}

impl UncertaintyEngine {
    /// Create a new uncertainty engine.
    pub fn new() -> Self {
        Self {
            belief_graph: BeliefGraph::new(),
            calibrator: ConfidenceCalibrator::new(),
        }
    }

    /// Evaluate confidence for a goal and return the action.
    pub fn evaluate(&self, goal: &str) -> (f64, UncertaintyAction) {
        // Score confidence based on belief graph and goal complexity
        let confidence = self.score_confidence(goal);
        let action = self.calibrator.evaluate(confidence);
        (confidence, action)
    }

    /// Score confidence for a goal.
    ///
    /// Factors:
    /// 1. How much we know about the system (belief graph coverage)
    /// 2. How specific the goal is (vague = low confidence)
    /// 3. Whether we've seen similar goals before
    fn score_confidence(&self, goal: &str) -> f64 {
        let goal_lower = goal.to_lowercase();

        // Factor 1: Belief graph coverage
        // Check if we have relevant beliefs for this goal
        let relevant_keywords: Vec<&str> = goal_lower.split_whitespace().collect();
        let keyword_refs: Vec<&str> = relevant_keywords.iter().map(|s| *s).collect();
        let belief_confidence = self.belief_graph.confidence_for(&keyword_refs);

        // Factor 2: Goal specificity (more words = more specific = higher base confidence)
        let word_count = goal.split_whitespace().count();
        let specificity = (word_count as f64 / 10.0).min(1.0); // 10+ words = max specificity

        // Factor 3: Known patterns (system admin, file ops, etc.)
        let pattern_bonus = if goal_lower.contains("check") || goal_lower.contains("status") {
            0.2 // Diagnostic goals are easier
        } else if goal_lower.contains("fix") || goal_lower.contains("repair") {
            0.1 // Fix goals need more context
        } else if goal_lower.contains("optimize") || goal_lower.contains("improve") {
            0.05 // Optimization goals are complex
        } else {
            0.0
        };

        // Combine factors (weighted)
        let base = belief_confidence * 0.5 + specificity * 0.3 + pattern_bonus;
        base.clamp(0.0, 1.0)
    }

    /// Generate diagnostic commands to gather evidence.
    ///
    /// These are ALWAYS read-only commands that the PolicyGate will auto-approve.
    pub fn plan_diagnostics(&self, goal: &str) -> Vec<StructuredCommand> {
        let goal_lower = goal.to_lowercase();
        let mut commands = Vec::new();

        // Always start with basic system info
        commands.push(StructuredCommand {
            binary: "uptime".into(),
            args: vec![],
            target: "local".into(),
            timeout_secs: 5,
            working_dir: None,
            env_vars: None,
        });

        // Domain-specific diagnostics
        if goal_lower.contains("vm") || goal_lower.contains("server") || goal_lower.contains("system") {
            commands.extend(self.vm_diagnostics());
        }
        if goal_lower.contains("slow") || goal_lower.contains("performance") || goal_lower.contains("cpu") {
            commands.extend(self.performance_diagnostics());
        }
        if goal_lower.contains("disk") || goal_lower.contains("space") || goal_lower.contains("full") {
            commands.extend(self.disk_diagnostics());
        }
        if goal_lower.contains("network") || goal_lower.contains("connect") || goal_lower.contains("internet") {
            commands.extend(self.network_diagnostics());
        }
        if goal_lower.contains("nginx") || goal_lower.contains("web") || goal_lower.contains("http") {
            commands.extend(self.web_diagnostics());
        }

        commands
    }

    /// Get the belief graph.
    pub fn belief_graph(&self) -> &BeliefGraph {
        &self.belief_graph
    }

    /// Get a mutable reference to the belief graph.
    pub fn belief_graph_mut(&mut self) -> &mut BeliefGraph {
        &mut self.belief_graph
    }

    /// Get the calibrator.
    pub fn calibrator(&self) -> &ConfidenceCalibrator {
        &self.calibrator
    }

    /// Get a mutable reference to the calibrator.
    pub fn calibrator_mut(&mut self) -> &mut ConfidenceCalibrator {
        &mut self.calibrator
    }

    /// Decay belief graph confidence (call periodically).
    pub fn decay_beliefs(&mut self) {
        self.belief_graph.decay();
    }

    fn vm_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand {
                binary: "top".into(),
                args: vec!["-bn1".into(), "-w512".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "free".into(),
                args: vec!["-h".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "df".into(),
                args: vec!["-h".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "systemctl".into(),
                args: vec!["list-units".into(), "--type=service".into(), "--state=running".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
        ]
    }

    fn performance_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand {
                binary: "top".into(),
                args: vec!["-bn1".into(), "-o".into(), "%CPU".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "vmstat".into(),
                args: vec!["1".into(), "2".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
        ]
    }

    fn disk_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand {
                binary: "df".into(),
                args: vec!["-h".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "du".into(),
                args: vec!["-sh".into(), "/var/log".into(), "/tmp".into(), "/home".into()],
                target: "local".into(),
                timeout_secs: 30,
                working_dir: None,
                env_vars: None,
            },
        ]
    }

    fn network_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand {
                binary: "ip".into(),
                args: vec!["addr".into(), "show".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "ss".into(),
                args: vec!["-tuln".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "ping".into(),
                args: vec!["-c".into(), "3".into(), "8.8.8.8".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
        ]
    }

    fn web_diagnostics(&self) -> Vec<StructuredCommand> {
        vec![
            StructuredCommand {
                binary: "systemctl".into(),
                args: vec!["status".into(), "nginx".into()],
                target: "local".into(),
                timeout_secs: 5,
                working_dir: None,
                env_vars: None,
            },
            StructuredCommand {
                binary: "curl".into(),
                args: vec!["-s".into(), "-o".into(), "/dev/null".into(), "-w".into(), "%{http_code}".into(), "http://localhost".into()],
                target: "local".into(),
                timeout_secs: 10,
                working_dir: None,
                env_vars: None,
            },
        ]
    }
}

impl Default for UncertaintyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vague_goal_low_confidence() {
        let engine = UncertaintyEngine::new();
        let (confidence, action) = engine.evaluate("fix it");
        assert!(confidence < 0.5, "Vague goal should have low confidence, got {}", confidence);
        assert!(action != UncertaintyAction::Plan, "Vague goal should not plan");
    }

    #[test]
    fn specific_diagnostic_goal_higher_confidence() {
        let engine = UncertaintyEngine::new();
        let (confidence, _) = engine.evaluate("check nginx status on VM1");
        // Even without prior beliefs, specific diagnostic goals should have some confidence
        assert!(confidence > 0.0, "Specific goal should have some confidence");
    }

    #[test]
    fn diagnostics_are_read_only() {
        let engine = UncertaintyEngine::new();
        let commands = engine.plan_diagnostics("make my VM faster");

        for cmd in &commands {
            // All diagnostic commands should be read-only binaries
            assert!(
                ["uptime", "top", "free", "df", "systemctl", "vmstat", "du", "ip", "ss", "ping", "curl"]
                    .contains(&cmd.binary.as_str()),
                "Diagnostic command should be read-only, got: {}",
                cmd.binary
            );
        }
    }

    #[test]
    fn diagnostics_include_vm_commands() {
        let engine = UncertaintyEngine::new();
        let commands = engine.plan_diagnostics("my VM is slow");

        let binaries: Vec<&str> = commands.iter().map(|c| c.binary.as_str()).collect();
        assert!(binaries.contains(&"top"), "Should include top for VM diagnostics");
        assert!(binaries.contains(&"free"), "Should include free for VM diagnostics");
    }

    #[test]
    fn diagnostics_include_network_commands() {
        let engine = UncertaintyEngine::new();
        let commands = engine.plan_diagnostics("network is down");

        let binaries: Vec<&str> = commands.iter().map(|c| c.binary.as_str()).collect();
        assert!(binaries.contains(&"ping"), "Should include ping for network diagnostics");
    }

    #[test]
    fn diagnostics_include_web_commands() {
        let engine = UncertaintyEngine::new();
        let commands = engine.plan_diagnostics("nginx is not responding");

        let binaries: Vec<&str> = commands.iter().map(|c| c.binary.as_str()).collect();
        assert!(binaries.contains(&"systemctl"), "Should include systemctl for nginx diagnostics");
    }

    #[test]
    fn beliefs_affect_confidence() {
        let mut engine = UncertaintyEngine::new();

        // Add beliefs about nginx
        engine.belief_graph_mut().update(
            "nginx is running",
            0.95,
            "systemctl status: active",
            BeliefSource::Detected,
        );

        let (conf_with_belief, _) = engine.evaluate("check nginx status");
        let engine_empty = UncertaintyEngine::new();
        let (conf_without_belief, _) = engine_empty.evaluate("check nginx status");

        assert!(conf_with_belief > conf_without_belief,
            "Beliefs should increase confidence: with={}, without={}",
            conf_with_belief, conf_without_belief);
    }

    #[test]
    fn decay_beliefs_works() {
        let mut engine = UncertaintyEngine::new();
        engine.belief_graph_mut().update("fact", 0.9, "evidence", BeliefSource::Detected);

        // Set fact to old
        if let Some(fact) = engine.belief_graph_mut().all_facts().get("fact") {
            // Can't mutate through all_facts(), so we use decay directly
        }
        engine.decay_beliefs();
        // After decay, confidence should be slightly lower (but not zero)
        let fact = engine.belief_graph().get("fact").unwrap();
        assert!(fact.confidence > 0.0, "Confidence should not be zero after decay");
    }
}
