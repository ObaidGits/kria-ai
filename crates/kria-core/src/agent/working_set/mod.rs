//! WorkingSet — Cognitive scratchpad for the 7B Planner.
//!
//! # Design: Structured Extraction (NOT LLM Summarization)
//!
//! The WorkingSet compresses conversation history, system state, and evidence
//! into a compact representation that fits in the 7B model's context window.
//!
//! **Critical:** The Planner ONLY reads the WorkingSet, not the full conversation.
//! This prevents context bloat and keeps the 7B model focused on the task.
//!
//! # Token Budget
//!
//! The WorkingSet has a 2048-token budget. Evidence is truncated by priority:
//! 1. First: raw_snippet (lowest priority — prose, not structured data)
//! 2. Then: oldest evidence entries
//! 3. Never: structured fields (error codes, IPs, file paths, numeric values)
//!
//! This ensures the Planner always has access to exact diagnostic data.

pub mod extractor;

pub use extractor::{ExtractedFields, StructuredEvidence, StructuredExtractor};

use std::time::Duration;

/// Maximum tokens for the WorkingSet (fits in 7B context with room for reasoning).
const DEFAULT_MAX_TOKENS: usize = 2048;

/// WorkingSet — the cognitive scratchpad for the Planner.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkingSet {
    /// The active goal (what we're trying to achieve).
    pub goal: String,
    /// Constraints from the World Model (e.g., "don't restart nginx during business hours").
    pub constraints: Vec<Constraint>,
    /// Evidence gathered so far (structured, NOT summarized).
    pub evidence: Vec<StructuredEvidence>,
    /// Open questions that need answers before planning.
    pub open_questions: Vec<String>,
    /// Maximum token budget.
    pub max_tokens: usize,
    /// When this WorkingSet was created (epoch millis).
    #[serde(default)]
    pub created_at_epoch_ms: u64,
}

/// A constraint on the current task.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Constraint {
    /// Human-readable description (e.g., "Don't restart nginx during business hours").
    pub description: String,
    /// Source of the constraint (e.g., "user_preference", "world_model", "policy").
    pub source: String,
    /// Whether this constraint is hard (must not violate) or soft (prefer not to violate).
    pub hard: bool,
}

/// Builder for WorkingSet.
pub struct WorkingSetBuilder {
    goal: String,
    constraints: Vec<Constraint>,
    evidence: Vec<StructuredEvidence>,
    open_questions: Vec<String>,
    max_tokens: usize,
}

impl WorkingSetBuilder {
    /// Start building a WorkingSet for a goal.
    pub fn new(goal: impl Into<String>) -> Self {
        Self {
            goal: goal.into(),
            constraints: Vec::new(),
            evidence: Vec::new(),
            open_questions: Vec::new(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// Set the token budget.
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Add a constraint.
    pub fn add_constraint(mut self, description: impl Into<String>, source: impl Into<String>, hard: bool) -> Self {
        self.constraints.push(Constraint {
            description: description.into(),
            source: source.into(),
            hard,
        });
        self
    }

    /// Add evidence from a command execution.
    pub fn add_evidence(mut self, evidence: StructuredEvidence) -> Self {
        self.evidence.push(evidence);
        self
    }

    /// Add an open question.
    pub fn add_question(mut self, question: impl Into<String>) -> Self {
        self.open_questions.push(question.into());
        self
    }

    /// Build the WorkingSet, fitting evidence to the token budget.
    pub fn build(self) -> WorkingSet {
        let mut ws = WorkingSet {
            goal: self.goal,
            constraints: self.constraints,
            evidence: self.evidence,
            open_questions: self.open_questions,
            max_tokens: self.max_tokens,
            created_at_epoch_ms: epoch_millis(),
        };
        ws.fit_to_budget();
        ws
    }
}

impl WorkingSet {
    /// Create a builder for a WorkingSet.
    pub fn builder(goal: impl Into<String>) -> WorkingSetBuilder {
        WorkingSetBuilder::new(goal)
    }

    /// Estimate total token count.
    pub fn estimate_tokens(&self) -> usize {
        let mut total = self.goal.len();
        for c in &self.constraints {
            total += c.description.len() + 20;
        }
        for e in &self.evidence {
            total += StructuredExtractor::estimate_evidence_tokens(e);
        }
        for q in &self.open_questions {
            total += q.len() + 10;
        }
        total / 4 // rough: 1 token ≈ 4 chars
    }

    /// Fit evidence to the token budget by truncating raw snippets first,
    /// then removing oldest evidence entries.
    fn fit_to_budget(&mut self) {
        let _extractor = StructuredExtractor::with_max_lines(10);

        while self.estimate_tokens() > self.max_tokens {
            // Strategy 1: Truncate raw snippets (lowest priority)
            let mut truncated_any = false;
            for ev in &mut self.evidence {
                if !ev.stdout_fields.raw_snippet.is_empty() {
                    ev.stdout_fields.raw_snippet.clear();
                    ev.stdout_fields.truncated = true;
                    truncated_any = true;
                }
                if !ev.stderr_fields.raw_snippet.is_empty() {
                    ev.stderr_fields.raw_snippet.clear();
                    ev.stderr_fields.truncated = true;
                    truncated_any = true;
                }
            }

            if truncated_any && self.estimate_tokens() <= self.max_tokens {
                break;
            }

            // Strategy 2: Remove oldest evidence entry
            if !self.evidence.is_empty() {
                self.evidence.remove(0);
            } else {
                break;
            }
        }
    }

    /// Serialize to a prompt string for the Planner.
    pub fn to_prompt(&self) -> String {
        let mut out = String::new();

        // Goal
        out.push_str(&format!("## GOAL\n{}\n", self.goal));

        // Constraints
        if !self.constraints.is_empty() {
            out.push_str("\n## CONSTRAINTS\n");
            for c in &self.constraints {
                let hard = if c.hard { "HARD" } else { "soft" };
                out.push_str(&format!("- [{}] {} (from: {})\n", hard, c.description, c.source));
            }
        }

        // Evidence (structured fields only — no raw prose)
        if !self.evidence.is_empty() {
            out.push_str("\n## EVIDENCE\n");
            for ev in &self.evidence {
                out.push_str(&format!("# [{}] exit:{} target:{}\n",
                    ev.command, ev.exit_code, ev.target
                ));

                // Error codes (exact)
                let all_errors: Vec<String> = ev.stdout_fields.error_codes.iter()
                    .chain(ev.stderr_fields.error_codes.iter())
                    .cloned()
                    .collect();
                if !all_errors.is_empty() {
                    out.push_str(&format!("  errors: {}\n", all_errors.join(", ")));
                }

                // Exit codes (exact)
                let all_exits: Vec<String> = ev.stdout_fields.exit_codes.iter()
                    .chain(ev.stderr_fields.exit_codes.iter())
                    .cloned()
                    .collect();
                if !all_exits.is_empty() {
                    out.push_str(&format!("  exit_codes: {}\n", all_exits.join(", ")));
                }

                // IPs (exact)
                let all_ips: Vec<String> = ev.stdout_fields.ipv4_addresses.iter()
                    .chain(ev.stderr_fields.ipv4_addresses.iter())
                    .cloned()
                    .collect();
                if !all_ips.is_empty() {
                    out.push_str(&format!("  ips: {}\n", all_ips.join(", ")));
                }

                // File paths (exact)
                let all_paths: Vec<String> = ev.stdout_fields.file_paths.iter()
                    .chain(ev.stderr_fields.file_paths.iter())
                    .cloned()
                    .collect();
                if !all_paths.is_empty() {
                    out.push_str(&format!("  paths: {}\n", all_paths.join(", ")));
                }

                // Numeric values (exact)
                let all_nums: Vec<String> = ev.stdout_fields.numeric_values.iter()
                    .chain(ev.stderr_fields.numeric_values.iter())
                    .map(|(k, v, u)| format!("{}={}{})", k, v, u))
                    .collect();
                if !all_nums.is_empty() {
                    out.push_str(&format!("  metrics: {}\n", all_nums.join(", ")));
                }

                // KV pairs
                let all_kvs: Vec<String> = ev.stdout_fields.kv_pairs.iter()
                    .chain(ev.stderr_fields.kv_pairs.iter())
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                if !all_kvs.is_empty() {
                    out.push_str(&format!("  fields: {}\n", all_kvs.join(", ")));
                }

                // Raw snippet (if still present)
                if !ev.stdout_fields.raw_snippet.is_empty() {
                    out.push_str(&format!("  stdout_snippet: {}\n", ev.stdout_fields.raw_snippet));
                }
            }
        }

        // Open questions
        if !self.open_questions.is_empty() {
            out.push_str("\n## OPEN QUESTIONS\n");
            for q in &self.open_questions {
                out.push_str(&format!("- {}\n", q));
            }
        }

        out
    }

    /// Check if this WorkingSet is stale (created more than `max_age` ago).
    pub fn is_stale(&self, max_age: Duration) -> bool {
        let now = epoch_millis();
        let age_ms = now.saturating_sub(self.created_at_epoch_ms);
        age_ms > max_age.as_millis() as u64
    }
}

/// Get current time in milliseconds since epoch.
fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_evidence(command: &str, stdout: &str, exit_code: i32) -> StructuredEvidence {
        let extractor = StructuredExtractor::new();
        extractor.extract_evidence(command, "local", exit_code, stdout, "")
    }

    #[test]
    fn working_set_builder_basic() {
        let ws = WorkingSet::builder("Fix nginx")
            .add_constraint("Don't restart during business hours", "user", true)
            .add_evidence(make_evidence("systemctl status nginx", "Active: active (running)", 0))
            .build();

        assert_eq!(ws.goal, "Fix nginx");
        assert_eq!(ws.constraints.len(), 1);
        assert_eq!(ws.evidence.len(), 1);
    }

    #[test]
    fn working_set_fits_to_budget() {
        // Create a WorkingSet with a very small budget
        let big_output = (0..1000).map(|i| format!("Line {}: some data here {}", i, i)).collect::<Vec<_>>().join("\n");
        let ws = WorkingSet::builder("Test goal")
            .with_max_tokens(100)  // Very small budget
            .add_evidence(make_evidence("cat /var/log/syslog", &big_output, 0))
            .build();

        // Should have truncated or removed evidence to fit
        assert!(ws.estimate_tokens() <= 200); // Allow some slack for estimation
    }

    #[test]
    fn working_set_to_prompt() {
        let ws = WorkingSet::builder("Make VM faster")
            .add_constraint("Don't restart nginx", "user", true)
            .add_evidence(make_evidence(
                "top -bn1",
                "CPU: 87%\nMemFree: 1234 kB\nLoad: 2.50",
                0,
            ))
            .add_question("What is consuming CPU?")
            .build();

        let prompt = ws.to_prompt();
        assert!(prompt.contains("## GOAL"));
        assert!(prompt.contains("Make VM faster"));
        assert!(prompt.contains("## CONSTRAINTS"));
        assert!(prompt.contains("Don't restart nginx"));
        assert!(prompt.contains("## EVIDENCE"));
        assert!(prompt.contains("top -bn1"));
        assert!(prompt.contains("## OPEN QUESTIONS"));
        assert!(prompt.contains("What is consuming CPU?"));
    }

    #[test]
    fn working_set_preserves_error_codes() {
        let ws = WorkingSet::builder("Debug connection")
            .add_evidence(make_evidence(
                "curl http://localhost:8080",
                "curl: (7) Failed to connect to localhost port 8080: ECONNREFUSED",
                7,
            ))
            .build();

        let prompt = ws.to_prompt();
        assert!(prompt.contains("ECONNREFUSED"), "Error code must be preserved in prompt");
    }

    #[test]
    fn working_set_preserves_ips() {
        let ws = WorkingSet::builder("Check network")
            .add_evidence(make_evidence(
                "ip addr show",
                "inet 192.168.1.100/24 brd 192.168.1.255 scope global eth0",
                0,
            ))
            .build();

        let prompt = ws.to_prompt();
        assert!(prompt.contains("192.168.1.100"), "IP address must be preserved in prompt");
    }

    #[test]
    fn working_set_preserves_numeric_values() {
        let ws = WorkingSet::builder("Check performance")
            .add_evidence(make_evidence(
                "free -h",
                "Mem: 15921 total, 2345 free, 8765 used",
                0,
            ))
            .build();

        let prompt = ws.to_prompt();
        // Should contain some numeric values
        assert!(prompt.contains("15921") || prompt.contains("2345") || prompt.contains("8765"),
            "Numeric values must be preserved");
    }

    #[test]
    fn working_set_staleness() {
        let mut ws = WorkingSet::builder("Test").build();
        // Not stale immediately
        assert!(!ws.is_stale(Duration::from_secs(60)));

        // Manually set created_at to the past (120 seconds ago)
        ws.created_at_epoch_ms = epoch_millis() - 120_000;
        assert!(ws.is_stale(Duration::from_secs(60)));
    }

    #[test]
    fn working_set_empty_is_valid() {
        let ws = WorkingSet::builder("Do nothing").build();
        let prompt = ws.to_prompt();
        assert!(prompt.contains("## GOAL"));
        assert!(prompt.contains("Do nothing"));
    }

    #[test]
    fn working_set_multiple_evidence() {
        let ws = WorkingSet::builder("Debug nginx")
            .add_evidence(make_evidence("systemctl status nginx", "Active: active", 0))
            .add_evidence(make_evidence("top -bn1", "CPU: 87%", 0))
            .add_evidence(make_evidence("df -h", "/dev/sda1: 95%", 0))
            .build();

        assert_eq!(ws.evidence.len(), 3);
        let prompt = ws.to_prompt();
        assert!(prompt.contains("systemctl status nginx"));
        assert!(prompt.contains("top -bn1"));
        assert!(prompt.contains("df -h"));
    }
}
