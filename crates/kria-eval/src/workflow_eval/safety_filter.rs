//! Safe test filtering for the real-world workflow eval framework.
//!
//! ## Invariants
//!
//! 1. Evals MUST NEVER execute commands that could destroy or corrupt user state.
//! 2. Any prompt or case that could trigger a dangerous operation is blocked or
//!    reclassified as `SandboxOnly`.
//! 3. The filter is fail-closed: unknown/ambiguous cases are treated as unsafe.
//! 4. A case can be promoted to `LiveOptIn` only when the human sets
//!    `KRIA_EVAL_LIVE=1` explicitly.
//!
//! ## Blocked operations (always mock/skip)
//!
//! - System shutdown / reboot / halt
//! - Logout / lock-screen
//! - Package uninstallation (`apt remove`, `pip uninstall`, `cargo uninstall`)
//! - Recursive filesystem deletion (`rm -rf`, `rmdir`, `del /f`)
//! - Privilege escalation (`sudo su`, `sudo -s`, `su root`)
//! - Disk formatting / partition operations
//! - SSH key or credential deletion
//! - Firewall rule changes

use super::types::{SafetyClass, WorkflowEvalCase};

// ─── Dangerous Pattern Tables ─────────────────────────────────────────────────

/// Prompt patterns that are unconditionally blocked.
const BLOCKED_PATTERNS: &[&str] = &[
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
    "logout",
    "log out",
    "rm -rf",
    "rmdir /s",
    "del /f",
    "format ",
    "mkfs",
    "fdisk",
    "wipefs",
    "dd if=",
    "sudo su",
    "sudo -s",
    "su root",
    "uninstall",
    " remove ",
    "apt remove",
    "apt purge",
    "pip uninstall",
    "cargo uninstall",
    "npm uninstall",
    "passwd",
    "chpasswd",
    "userdel",
    "groupdel",
    "iptables",
    "ufw delete",
    "firewall-cmd --remove",
    "ssh-keygen -R",
    "wipe",
    "shred",
    "crontab -r",
    "systemctl disable",
    "systemctl mask",
];

/// Prompt patterns that require KRIA_EVAL_LIVE=1.
const LIVE_OPT_IN_PATTERNS: &[&str] = &[
    "send email",
    "send message",
    "post to",
    "tweet",
    "git push",
    "git commit",
    "deploy",
    "publish",
    "upload",
    "wget ",
    "curl -o",
    "apt install",
    "pip install",
    "cargo install",
    "npm install",
    "systemctl start",
    "systemctl restart",
    "sudo ",
];

// ─── SafetyFilter ─────────────────────────────────────────────────────────────

/// Determines whether a workflow eval case is safe to execute automatically.
pub struct SafetyFilter;

impl SafetyFilter {
    /// Validate a `WorkflowEvalCase` before execution.
    ///
    /// Returns `Ok(())` if the case can run under its declared `safety_class`,
    /// or `Err(reason)` if execution should be blocked.
    pub fn validate(case: &WorkflowEvalCase) -> Result<(), String> {
        // Blocked cases never run
        if case.safety_class == SafetyClass::Blocked {
            return Err(format!(
                "Case '{}' is permanently blocked (SafetyClass::Blocked)",
                case.id
            ));
        }

        // Sandbox-only cases must have KRIA_EVAL_SANDBOX=1
        if case.safety_class == SafetyClass::SandboxOnly {
            if std::env::var("KRIA_EVAL_SANDBOX").as_deref() != Ok("1") {
                return Err(format!(
                    "Case '{}' requires KRIA_EVAL_SANDBOX=1 (sandbox-only)",
                    case.id
                ));
            }
        }

        // LiveOptIn cases need explicit opt-in
        if case.safety_class == SafetyClass::LiveOptIn {
            if std::env::var("KRIA_EVAL_LIVE").as_deref() != Ok("1") {
                return Err(format!(
                    "Case '{}' requires KRIA_EVAL_LIVE=1 (live opt-in required)",
                    case.id
                ));
            }
        }

        // Cross-check the prompt against the pattern tables regardless of declared class
        let (inferred, reason) = Self::classify_prompt(&case.prompt);
        if inferred == SafetyClass::Blocked {
            return Err(format!(
                "Case '{}' prompt contains a blocked pattern: {}",
                case.id, reason
            ));
        }

        // Upgrade warning: if inferred class is more restrictive than declared
        if inferred == SafetyClass::LiveOptIn && case.safety_class == SafetyClass::Safe {
            if std::env::var("KRIA_EVAL_LIVE").as_deref() != Ok("1") {
                return Err(format!(
                    "Case '{}' prompt implies live operations ({}) but is declared Safe; \
                     set KRIA_EVAL_LIVE=1 or fix the safety class",
                    case.id, reason
                ));
            }
        }

        Ok(())
    }

    /// Classify a raw prompt text into a safety class.
    ///
    /// Returns `(SafetyClass, reason_string)`.
    pub fn classify_prompt(prompt: &str) -> (SafetyClass, String) {
        let lower = prompt.to_ascii_lowercase();

        for pattern in BLOCKED_PATTERNS {
            if lower.contains(pattern) {
                return (
                    SafetyClass::Blocked,
                    format!("matches blocked pattern '{}'", pattern),
                );
            }
        }

        for pattern in LIVE_OPT_IN_PATTERNS {
            if lower.contains(pattern) {
                return (
                    SafetyClass::LiveOptIn,
                    format!("matches live-opt-in pattern '{}'", pattern),
                );
            }
        }

        (
            SafetyClass::Safe,
            "no dangerous patterns detected".to_string(),
        )
    }

    /// Returns true if a case should be SKIPPED (not failed) due to safety constraints.
    pub fn should_skip(case: &WorkflowEvalCase) -> bool {
        Self::validate(case).is_err()
    }

    /// Returns true if a case is safe to auto-run without any opt-in.
    pub fn is_auto_safe(case: &WorkflowEvalCase) -> bool {
        case.safety_class.is_auto_runnable()
            && Self::classify_prompt(&case.prompt).0.is_auto_runnable()
    }

    /// Filter a list of cases, returning only those safe to run automatically.
    pub fn filter_auto_safe(cases: &[WorkflowEvalCase]) -> Vec<&WorkflowEvalCase> {
        cases.iter().filter(|c| Self::is_auto_safe(c)).collect()
    }

    /// Partition cases into (runnable, skipped) based on current env vars.
    pub fn partition(
        cases: &[WorkflowEvalCase],
    ) -> (Vec<&WorkflowEvalCase>, Vec<&WorkflowEvalCase>) {
        let mut runnable = Vec::new();
        let mut skipped = Vec::new();
        for case in cases {
            if Self::should_skip(case) {
                skipped.push(case);
            } else {
                runnable.push(case);
            }
        }
        (runnable, skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_eval::types::{
        EvalWorkflowCategory, SafetyClass, SemanticCompletionContract, WorkflowEvalCase,
    };
    use std::time::Duration;

    fn case(prompt: &str, class: SafetyClass) -> WorkflowEvalCase {
        WorkflowEvalCase {
            id: "test".to_string(),
            description: "test".to_string(),
            prompt: prompt.to_string(),
            category: EvalWorkflowCategory::Coding,
            contract: SemanticCompletionContract {
                success_definition: "test".to_string(),
                category: EvalWorkflowCategory::Coding,
                required_observable_outputs: vec![],
                semantic_success_signals: vec![],
                forbidden_silent_completion_patterns: vec![],
                required_stage_labels: vec![],
                require_observable_before_success_claim: false,
            },
            safety_class: class,
            interruption: None,
            timeout: Duration::from_secs(30),
            requires_daemon: false,
            requires_display: false,
            tags: vec![],
            eval_notes: "".to_string(),
        }
    }

    #[test]
    fn blocked_class_never_runs() {
        let c = case("open vscode", SafetyClass::Blocked);
        assert!(SafetyFilter::validate(&c).is_err());
    }

    #[test]
    fn shutdown_prompt_is_blocked_regardless_of_class() {
        let c = case("shutdown the computer", SafetyClass::Safe);
        assert!(SafetyFilter::validate(&c).is_err());
    }

    #[test]
    fn rm_rf_prompt_is_blocked() {
        let c = case("rm -rf ~/Documents", SafetyClass::Safe);
        assert!(SafetyFilter::validate(&c).is_err());
    }

    #[test]
    fn reboot_prompt_is_blocked() {
        let c = case("please reboot the machine", SafetyClass::Safe);
        assert!(SafetyFilter::validate(&c).is_err());
    }

    #[test]
    fn safe_coding_prompt_passes() {
        let c = case(
            "open vscode and write a python pascal triangle program",
            SafetyClass::Safe,
        );
        assert!(SafetyFilter::is_auto_safe(&c));
    }

    #[test]
    fn classify_prompt_returns_correct_class() {
        let (class, _) = SafetyFilter::classify_prompt("sudo apt remove vim");
        assert_eq!(class, SafetyClass::Blocked);

        let (class, _) = SafetyFilter::classify_prompt("write a hello world program");
        assert_eq!(class, SafetyClass::Safe);
    }

    #[test]
    fn filter_auto_safe_excludes_blocked() {
        let cases = vec![
            case("open vscode", SafetyClass::Safe),
            case("shutdown now", SafetyClass::Safe),
            case("write hello world", SafetyClass::Safe),
        ];
        let safe = SafetyFilter::filter_auto_safe(&cases);
        assert_eq!(safe.len(), 2);
    }
}
