//! Structured subprocess executor — the single execution path for all external commands.
//!
//! # Design Principles
//!
//! 1. **No raw shell strings.** The LLM outputs structured `StructuredCommand`
//!    (binary + args). There is NO shell parsing — the binary and args are
//!    passed directly to `execvp`-style execution.
//!
//! 2. **PolicyGate integration.** Every command passes through the PolicyGate
//!    BEFORE execution. Blocked commands are rejected. Unknown commands
//!    require HITL approval.
//!
//! 3. **Code Interpreter sandboxing.** LLM-generated code (Python, JS, Bash)
//!    NEVER executes on the local host. It is strictly confined to QEMU VMs
//!    or Docker containers. The `CodeInterpreterTarget` type has no `Local`
//!    variant — this is enforced at compile time.
//!
//! 4. **Audit logging.** Every command execution (success, failure, blocked)
//!    is logged to the audit trail.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::infra::environment::traits::{CommandRequest, EnvironmentProvider};
use crate::safety::policy_gate::{PolicyDecision, PolicyGate};
use crate::safety::RiskLevel;

// ─── Structured Command ──────────────────────────────────────────────────────

/// A structured command submitted by the LLM.
///
/// This is the ONLY way to execute external commands in KRIA.
/// There is no raw shell string passthrough.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StructuredCommand {
    /// The binary to execute (e.g., "systemctl", "ls", "grep").
    /// Must be a valid binary name — no shell metacharacters.
    pub binary: String,
    /// Arguments. Each element is one argv entry.
    /// NO shell parsing, NO metacharacter interpretation.
    pub args: Vec<String>,
    /// Target environment: "local", VM name, or Docker container.
    #[serde(default = "default_target")]
    pub target: String,
    /// Timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Working directory (optional).
    pub working_dir: Option<String>,
    /// Environment variables to set (optional).
    pub env_vars: Option<HashMap<String, String>>,
}

fn default_target() -> String {
    "local".into()
}
fn default_timeout() -> u64 {
    30
}

// ─── Execution Result ────────────────────────────────────────────────────────

/// Result of a command execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
    pub execution_time_ms: u64,
    pub risk_level: RiskLevel,
    pub policy_decision: PolicyDecisionSummary,
    pub target: String,
    pub command: String,
}

/// Summary of the policy decision (for audit logging).
#[derive(Debug, Clone, serde::Serialize)]
pub enum PolicyDecisionSummary {
    AutoApproved,
    HitlApproved,
    Blocked { reason: String },
    Quarantined { reason: String },
}

// ─── Code Interpreter Target ─────────────────────────────────────────────────

/// Where LLM-generated code can execute.
///
/// **CRITICAL SAFETY CONSTRAINT:** There is NO `Local` variant.
/// LLM-generated code NEVER executes on the local host.
/// It is strictly confined to VMs or Docker containers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CodeInterpreterTarget {
    /// Execute on a named QEMU VM via SSH.
    Vm(String),
    /// Execute in a named Docker container.
    Docker(String),
}

impl CodeInterpreterTarget {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Vm(name) => name,
            Self::Docker(name) => name,
        }
    }
}

// ─── Subprocess Executor ─────────────────────────────────────────────────────

/// The single execution path for all external commands.
///
/// Every command passes through:
/// 1. PolicyGate evaluation
/// 2. HITL approval (if required)
/// 3. EnvironmentProvider execution
/// 4. Audit logging
pub struct SubprocessExecutor {
    policy_gate: Arc<dyn PolicyGate>,
    environments: Arc<dyn EnvironmentProvider>,
    audit_tx: tokio::sync::mpsc::UnboundedSender<AuditEntry>,
    #[allow(dead_code)]
    default_timeout: Duration,
    max_output_bytes: usize,
    max_output_lines: usize,
}

/// An audit log entry for command execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub command: String,
    pub target: String,
    pub exit_code: Option<i32>,
    pub risk_level: RiskLevel,
    pub policy_decision: PolicyDecisionSummary,
    pub execution_time_ms: u64,
    pub truncated: bool,
}

impl SubprocessExecutor {
    pub fn new(
        policy_gate: Arc<dyn PolicyGate>,
        environments: Arc<dyn EnvironmentProvider>,
        audit_tx: tokio::sync::mpsc::UnboundedSender<AuditEntry>,
    ) -> Self {
        Self {
            policy_gate,
            environments,
            audit_tx,
            default_timeout: Duration::from_secs(30),
            max_output_bytes: 64 * 1024, // 64KB
            max_output_lines: 500,
        }
    }

    /// Execute a structured command through the full policy pipeline.
    pub async fn execute(&self, cmd: &StructuredCommand) -> ExecutionResult {
        let start = Instant::now();
        let command_str = format!("{} {}", cmd.binary, cmd.args.join(" "));

        // 1. PolicyGate evaluation
        let policy_decision = self.policy_gate.evaluate(&cmd.binary, &cmd.args);
        let risk_level = self.policy_gate.classify_risk(&cmd.binary, &cmd.args);

        match &policy_decision {
            PolicyDecision::Blocked { reason } => {
                let result = ExecutionResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Blocked by policy: {}", reason),
                    truncated: false,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    risk_level: RiskLevel::Black,
                    policy_decision: PolicyDecisionSummary::Blocked {
                        reason: reason.clone(),
                    },
                    target: cmd.target.clone(),
                    command: command_str,
                };
                self.log_audit(&result);
                return result;
            }
            PolicyDecision::RequiresApproval { reason, .. } => {
                // In a real implementation, this would call HitlGateway.
                // For now, we log it and return a rejection.
                let result = ExecutionResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("Requires HITL approval: {}", reason),
                    truncated: false,
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    risk_level,
                    policy_decision: PolicyDecisionSummary::Quarantined {
                        reason: reason.clone(),
                    },
                    target: cmd.target.clone(),
                    command: command_str,
                };
                self.log_audit(&result);
                return result;
            }
            PolicyDecision::AutoApproved { .. } => {
                // Proceed to execution
            }
        }

        // 2. Execute via EnvironmentProvider
        let request = CommandRequest {
            program: cmd.binary.clone(),
            args: cmd.args.clone(),
            timeout_ms: cmd.timeout_secs * 1000,
            max_bytes: self.max_output_bytes,
            max_lines: self.max_output_lines,
        };

        // TODO: Resolve the correct EnvironmentProvider based on cmd.target
        // For now, we use the default environment.
        let result = self
            .environments
            .execute_command(request, crate::infra::environment::ShellState::default())
            .await;

        let execution_time_ms = start.elapsed().as_millis() as u64;

        let execution_result = match result {
            Ok(cmd_result) => ExecutionResult {
                exit_code: cmd_result.exit_code,
                stdout: cmd_result.stdout,
                stderr: cmd_result.stderr,
                truncated: cmd_result.truncated,
                execution_time_ms,
                risk_level,
                policy_decision: PolicyDecisionSummary::AutoApproved,
                target: cmd.target.clone(),
                command: command_str,
            },
            Err(e) => ExecutionResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("Execution error: {}", e),
                truncated: false,
                execution_time_ms,
                risk_level,
                policy_decision: PolicyDecisionSummary::AutoApproved,
                target: cmd.target.clone(),
                command: command_str,
            },
        };

        self.log_audit(&execution_result);
        execution_result
    }

    /// Log an execution result to the audit trail.
    fn log_audit(&self, result: &ExecutionResult) {
        let entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            command: result.command.clone(),
            target: result.target.clone(),
            exit_code: Some(result.exit_code),
            risk_level: result.risk_level,
            policy_decision: result.policy_decision.clone(),
            execution_time_ms: result.execution_time_ms,
            truncated: result.truncated,
        };

        // Best-effort audit logging (non-blocking)
        let _ = self.audit_tx.send(entry);
    }
}
