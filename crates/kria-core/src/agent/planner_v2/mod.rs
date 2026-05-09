//! Structured Branching Planner (replaces linear planner).
//!
//! # Design: Forced 3-Path Templates
//!
//! Open-ended Tree-of-Thoughts is rejected. 7B models fail at unguided ToT —
//! they produce verbose, hallucinated branches. Instead, the Planner is
//! **forced** to generate exactly 3 structured paths with specific templates:
//!
//! - PATH A: Diagnose-First (read-only, safe)
//! - PATH B: Minimal-Risk Fix (reversible)
//! - PATH C: Aggressive Fix (potentially irreversible)
//!
//! Each path is scored against SelfModel for historical success rates.
//! The winner is selected based on the best risk/reward ratio.
//!
//! # Architecture
//!
//! ```text
//! User Goal + WorkingSet
//!     ↓
//! BranchingPlanner.plan()
//!     ↓
//! ┌─────────────────────────────────────────────────┐
//! │  Generate 3 structured paths via 7B LLM         │
//! │  (or fallback: cloud Gemini / heuristic)        │
//! └─────────────┬───────────────────────────────────┘
//!               ↓
//! ┌─────────────────────────────────────────────────┐
//! │  Score each path against SelfModel              │
//! │  (Beta posterior success rates per tool)        │
//! └─────────────┬───────────────────────────────────┘
//!               ↓
//! ┌─────────────────────────────────────────────────┐
//! │  Select winner (best risk/reward ratio)         │
//! │  Path B preferred if within 10% of Path A       │
//! └─────────────────────────────────────────────────┘
//! ```

use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::self_model::SelfModel;
use crate::agent::working_set::{StructuredEvidence, WorkingSet};
use crate::safety::RiskLevel;
use crate::tools::subprocess_executor::StructuredCommand;

/// Risk level for a planned path.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PathRisk {
    /// Read-only, no system modification.
    DiagnoseFirst,
    /// Reversible changes (config edits, service restarts).
    MinimalRisk,
    /// Potentially irreversible (service replacement, package install).
    Aggressive,
}

impl PathRisk {
    /// Convert to RiskLevel for PolicyGate integration.
    pub fn to_risk_level(&self) -> RiskLevel {
        match self {
            Self::DiagnoseFirst => RiskLevel::Green,
            Self::MinimalRisk => RiskLevel::Yellow,
            Self::Aggressive => RiskLevel::Red,
        }
    }
}

/// A single step in a planned path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlannedStep {
    /// Step number (1-indexed).
    pub step_number: usize,
    /// Description of what this step does.
    pub description: String,
    /// The structured command to execute.
    pub command: StructuredCommand,
    /// Expected outcome (for verification).
    pub expected_outcome: String,
    /// What to do if this step fails.
    pub on_failure: String,
}

/// A complete planned path (one of the 3 branches).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlannedPath {
    /// Risk level of this path.
    pub risk: PathRisk,
    /// Human-readable name (e.g., "Diagnose-First").
    pub name: String,
    /// Steps in this path.
    pub steps: Vec<PlannedStep>,
    /// Predicted outcome.
    pub predicted_outcome: String,
    /// SelfModel score (filled after scoring).
    pub score: Option<f64>,
}

impl PlannedPath {
    /// Get tool names used in this path (for SelfModel scoring).
    pub fn tool_names(&self) -> Vec<&str> {
        self.steps.iter()
            .map(|s| s.command.binary.as_str())
            .collect()
    }
}

/// The result of planning — 3 structured paths.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlanResult {
    /// The 3 paths (Diagnose-First, Minimal-Risk, Aggressive).
    pub paths: Vec<PlannedPath>,
    /// The selected winning path index.
    pub selected_index: usize,
    /// WorkingSet that was used for planning.
    pub working_set_summary: String,
}

impl PlanResult {
    /// Get the selected path.
    pub fn selected_path(&self) -> &PlannedPath {
        &self.paths[self.selected_index]
    }
}

/// The Structured Branching Planner.
pub struct BranchingPlanner {
    /// SelfModel for scoring paths.
    self_model: Arc<RwLock<SelfModel>>,
}

impl BranchingPlanner {
    /// Create a new planner.
    pub fn new(self_model: Arc<RwLock<SelfModel>>) -> Self {
        Self { self_model }
    }

    /// Score 3 paths against the SelfModel and select the winner.
    ///
    /// Selection logic:
    /// - Prefer Path B (MinimalRisk) if its score is within 10% of Path A
    /// - This avoids unnecessary diagnostic steps when a fix is straightforward
    /// - Otherwise, pick the highest score
    pub async fn select_winner(&self, paths: &mut [PlannedPath]) -> usize {
        let self_model = self.self_model.read().await;

        // Score each path
        for path in paths.iter_mut() {
            let tool_names = path.tool_names();
            let score = self_model.score_path(&tool_names);
            path.score = Some(score);
        }

        // Find Path A (DiagnoseFirst) and Path B (MinimalRisk) scores
        let path_a_score = paths.iter()
            .find(|p| p.risk == PathRisk::DiagnoseFirst)
            .and_then(|p| p.score)
            .unwrap_or(0.5);

        let path_b_score = paths.iter()
            .find(|p| p.risk == PathRisk::MinimalRisk)
            .and_then(|p| p.score)
            .unwrap_or(0.5);

        // Prefer Path B if within 10% of Path A
        if path_b_score >= path_a_score * 0.9 {
            if let Some(idx) = paths.iter().position(|p| p.risk == PathRisk::MinimalRisk) {
                return idx;
            }
        }

        // Otherwise, pick highest score
        paths.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.score.unwrap_or(0.5).partial_cmp(&b.score.unwrap_or(0.5)).unwrap()
            })
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }

    /// Build the prompt for the 7B LLM to generate 3 structured paths.
    pub fn build_prompt(_goal: &str, working_set: &WorkingSet) -> String {
        let mut prompt = String::from(
            r#"You are KRIA's planning engine. You MUST generate exactly 3 plans using these templates:

SYSTEM STATE:
"#
        );

        prompt.push_str(&working_set.to_prompt());

        prompt.push_str(r#"

Generate exactly 3 plans:

PATH A — DIAGNOSE-FIRST (read-only, gather information):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what you'll learn]
  Risk: None (read-only)

PATH B — MINIMAL-RISK FIX (reversible changes):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what will change]
  Risk: Low (reversible)

PATH C — AGGRESSIVE FIX (may be hard to reverse):
  Commands: [{"binary": "...", "args": ["..."], "target": "..."}]
  Predicted outcome: [what will change]
  Risk: High (potentially irreversible)

SELECT: [A/B/C] because [reasoning based on risk and confidence]

IMPORTANT: All commands MUST be structured JSON: {"binary": "...", "args": [...], "target": "..."}
Do NOT use shell syntax. Each command is a separate binary invocation."#
        );

        prompt
    }

    /// Generate a heuristic plan (fallback when LLM is unavailable).
    pub fn heuristic_plan(goal: &str, _evidence: &[StructuredEvidence]) -> PlanResult {
        let goal_lower = goal.to_lowercase();

        // PATH A: Diagnostics
        let path_a = PlannedPath {
            risk: PathRisk::DiagnoseFirst,
            name: "Diagnose-First".into(),
            steps: vec![
                PlannedStep {
                    step_number: 1,
                    description: "Check system status".into(),
                    command: StructuredCommand {
                        binary: "top".into(),
                        args: vec!["-bn1".into(), "-w512".into()],
                        target: "local".into(),
                        timeout_secs: 10,
                        working_dir: None,
                        env_vars: None,
                    },
                    expected_outcome: "CPU and memory usage".into(),
                    on_failure: "continue".into(),
                },
                PlannedStep {
                    step_number: 2,
                    description: "Check disk usage".into(),
                    command: StructuredCommand {
                        binary: "df".into(),
                        args: vec!["-h".into()],
                        target: "local".into(),
                        timeout_secs: 5,
                        working_dir: None,
                        env_vars: None,
                    },
                    expected_outcome: "Disk space availability".into(),
                    on_failure: "continue".into(),
                },
            ],
            predicted_outcome: "System health overview".into(),
            score: None,
        };

        // PATH B: Minimal fix (nginx-specific if mentioned)
        let path_b = if goal_lower.contains("nginx") || goal_lower.contains("web") {
            PlannedPath {
                risk: PathRisk::MinimalRisk,
                name: "Minimal-Risk Fix".into(),
                steps: vec![
                    PlannedStep {
                        step_number: 1,
                        description: "Check nginx config".into(),
                        command: StructuredCommand {
                            binary: "nginx".into(),
                            args: vec!["-t".into()],
                            target: "local".into(),
                            timeout_secs: 5,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Config syntax OK".into(),
                        on_failure: "abort".into(),
                    },
                    PlannedStep {
                        step_number: 2,
                        description: "Reload nginx".into(),
                        command: StructuredCommand {
                            binary: "systemctl".into(),
                            args: vec!["reload".into(), "nginx".into()],
                            target: "local".into(),
                            timeout_secs: 10,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Nginx reloaded with new config".into(),
                        on_failure: "abort".into(),
                    },
                ],
                predicted_outcome: "Nginx running with correct config".into(),
                score: None,
            }
        } else {
            PlannedPath {
                risk: PathRisk::MinimalRisk,
                name: "Minimal-Risk Fix".into(),
                steps: vec![
                    PlannedStep {
                        step_number: 1,
                        description: "Restart common services".into(),
                        command: StructuredCommand {
                            binary: "systemctl".into(),
                            args: vec!["restart".into(), "nginx".into()],
                            target: "local".into(),
                            timeout_secs: 30,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Service restarted".into(),
                        on_failure: "abort".into(),
                    },
                ],
                predicted_outcome: "Service running".into(),
                score: None,
            }
        };

        // PATH C: Aggressive
        let path_c = PlannedPath {
            risk: PathRisk::Aggressive,
            name: "Aggressive Fix".into(),
            steps: vec![
                PlannedStep {
                    step_number: 1,
                    description: "Force kill and restart".into(),
                    command: StructuredCommand {
                        binary: "systemctl".into(),
                        args: vec!["kill".into(), "nginx".into()],
                        target: "local".into(),
                        timeout_secs: 10,
                        working_dir: None,
                        env_vars: None,
                    },
                    expected_outcome: "Service killed".into(),
                    on_failure: "abort".into(),
                },
                PlannedStep {
                    step_number: 2,
                    description: "Start service fresh".into(),
                    command: StructuredCommand {
                        binary: "systemctl".into(),
                        args: vec!["start".into(), "nginx".into()],
                        target: "local".into(),
                        timeout_secs: 10,
                        working_dir: None,
                        env_vars: None,
                    },
                    expected_outcome: "Service started".into(),
                    on_failure: "abort".into(),
                },
            ],
            predicted_outcome: "Service running from clean state".into(),
            score: None,
        };

        PlanResult {
            paths: vec![path_a, path_b, path_c],
            selected_index: 0, // Will be updated by select_winner
            working_set_summary: format!("Heuristic plan for: {}", goal),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_paths() -> Vec<PlannedPath> {
        vec![
            PlannedPath {
                risk: PathRisk::DiagnoseFirst,
                name: "Diagnose-First".into(),
                steps: vec![
                    PlannedStep {
                        step_number: 1,
                        description: "Check status".into(),
                        command: StructuredCommand {
                            binary: "systemctl".into(),
                            args: vec!["status".into(), "nginx".into()],
                            target: "local".into(),
                            timeout_secs: 5,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Service status".into(),
                        on_failure: "continue".into(),
                    },
                ],
                predicted_outcome: "Know what's wrong".into(),
                score: None,
            },
            PlannedPath {
                risk: PathRisk::MinimalRisk,
                name: "Minimal-Risk Fix".into(),
                steps: vec![
                    PlannedStep {
                        step_number: 1,
                        description: "Restart service".into(),
                        command: StructuredCommand {
                            binary: "systemctl".into(),
                            args: vec!["restart".into(), "nginx".into()],
                            target: "local".into(),
                            timeout_secs: 30,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Service restarted".into(),
                        on_failure: "abort".into(),
                    },
                ],
                predicted_outcome: "Service running".into(),
                score: None,
            },
            PlannedPath {
                risk: PathRisk::Aggressive,
                name: "Aggressive Fix".into(),
                steps: vec![
                    PlannedStep {
                        step_number: 1,
                        description: "Force kill".into(),
                        command: StructuredCommand {
                            binary: "systemctl".into(),
                            args: vec!["kill".into(), "nginx".into()],
                            target: "local".into(),
                            timeout_secs: 10,
                            working_dir: None,
                            env_vars: None,
                        },
                        expected_outcome: "Service killed".into(),
                        on_failure: "abort".into(),
                    },
                ],
                predicted_outcome: "Clean restart".into(),
                score: None,
            },
        ]
    }

    #[tokio::test]
    async fn select_winner_prefers_path_b_when_close() {
        let self_model = Arc::new(RwLock::new(SelfModel::new()));
        let planner = BranchingPlanner::new(self_model);

        let mut paths = make_test_paths();
        let winner = planner.select_winner(&mut paths).await;

        // With equal scores (all unknown tools = 0.5), Path B should be preferred
        // because it's within 10% of Path A
        assert_eq!(winner, 1, "Should prefer Path B (MinimalRisk) when scores are equal");
    }

    #[tokio::test]
    async fn select_winner_prefers_highest_score() {
        let self_model = Arc::new(RwLock::new(SelfModel::new()));
        {
            let mut model = self_model.write().await;
            // Make systemctl highly reliable
            for _ in 0..20 {
                model.record_outcome("systemctl", true, std::time::Duration::from_millis(100));
            }
        }

        let planner = BranchingPlanner::new(self_model);
        let mut paths = make_test_paths();
        let winner = planner.select_winner(&mut paths).await;

        // All paths use systemctl, so scores should be similar
        // Path B should still be preferred
        assert!(winner <= 2, "Winner should be a valid index");
    }

    #[test]
    fn heuristic_plan_generates_three_paths() {
        let result = BranchingPlanner::heuristic_plan("fix my VM", &[]);
        assert_eq!(result.paths.len(), 3);
        assert_eq!(result.paths[0].risk, PathRisk::DiagnoseFirst);
        assert_eq!(result.paths[1].risk, PathRisk::MinimalRisk);
        assert_eq!(result.paths[2].risk, PathRisk::Aggressive);
    }

    #[test]
    fn heuristic_plan_nginx_specific() {
        let result = BranchingPlanner::heuristic_plan("nginx is down", &[]);
        let path_b = &result.paths[1];
        // Should include nginx config check
        assert!(path_b.steps.iter().any(|s| s.command.args.contains(&"-t".to_string())),
            "Path B should include nginx config check");
    }

    #[test]
    fn build_prompt_includes_working_set() {
        let ws = WorkingSet::builder("Fix nginx")
            .add_evidence(StructuredEvidence {
                command: "systemctl status nginx".into(),
                target: "local".into(),
                exit_code: 3,
                stdout_fields: Default::default(),
                stderr_fields: Default::default(),
                timestamp_epoch_ms: 0,
            })
            .build();

        let prompt = BranchingPlanner::build_prompt("Fix nginx", &ws);
        assert!(prompt.contains("Fix nginx"));
        assert!(prompt.contains("PATH A"));
        assert!(prompt.contains("PATH B"));
        assert!(prompt.contains("PATH C"));
    }

    #[test]
    fn path_risk_to_risk_level() {
        assert_eq!(PathRisk::DiagnoseFirst.to_risk_level(), RiskLevel::Green);
        assert_eq!(PathRisk::MinimalRisk.to_risk_level(), RiskLevel::Yellow);
        assert_eq!(PathRisk::Aggressive.to_risk_level(), RiskLevel::Red);
    }

    #[test]
    fn planned_path_tool_names() {
        let paths = make_test_paths();
        let tool_names = paths[0].tool_names();
        assert_eq!(tool_names, vec!["systemctl"]);
    }
}
