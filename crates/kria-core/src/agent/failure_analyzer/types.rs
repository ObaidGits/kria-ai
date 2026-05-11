//! Failure Analyzer types — patterns, contexts, root causes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::tools::subprocess_executor::StructuredCommand;

/// A deterministic root cause extracted from exit code + stderr.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RootCause {
    /// Known exit code (e.g., 127 = command not found, 126 = permission denied).
    ExitCode { code: i32, meaning: String },
    /// Known stderr pattern (e.g., "ECONNREFUSED", "No such file or directory").
    StderrPattern { pattern: String, category: String },
    /// Command timed out.
    Timeout { seconds: u64 },
    /// Permission denied (detected from exit code 126 or stderr).
    PermissionDenied { path: Option<String> },
    /// Resource exhausted (OOM, disk full, etc.).
    ResourceExhausted { resource: String },
    /// Service not running.
    ServiceNotRunning { service: String },
    /// Network unreachable.
    NetworkUnreachable { target: String },
    /// Configuration error.
    ConfigError {
        file: Option<String>,
        detail: String,
    },
    /// Unknown — could not determine root cause deterministically.
    Unknown { stderr_snippet: String },
}

impl RootCause {
    /// Human-readable description of the root cause.
    pub fn description(&self) -> String {
        match self {
            Self::ExitCode { code, meaning } => format!("Exit code {}: {}", code, meaning),
            Self::StderrPattern { pattern, category } => format!("{}: {}", category, pattern),
            Self::Timeout { seconds } => format!("Timed out after {}s", seconds),
            Self::PermissionDenied { path } => {
                format!(
                    "Permission denied{}",
                    path.as_ref()
                        .map(|p| format!(": {}", p))
                        .unwrap_or_default()
                )
            }
            Self::ResourceExhausted { resource } => format!("Resource exhausted: {}", resource),
            Self::ServiceNotRunning { service } => format!("Service not running: {}", service),
            Self::NetworkUnreachable { target } => format!("Network unreachable: {}", target),
            Self::ConfigError { file, detail } => {
                format!(
                    "Config error{}: {}",
                    file.as_ref()
                        .map(|f| format!(" in {}", f))
                        .unwrap_or_default(),
                    detail
                )
            }
            Self::Unknown { stderr_snippet } => format!("Unknown error: {}", stderr_snippet),
        }
    }

    /// Whether this root cause is likely to recur without intervention.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::NetworkUnreachable { .. })
    }
}

/// Context of a failed plan execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureContext {
    /// The goal that was being pursued.
    pub goal: String,
    /// The command that failed.
    pub failed_command: StructuredCommand,
    /// Exit code of the failed command.
    pub exit_code: i32,
    /// Stderr output from the failed command.
    pub stderr: String,
    /// Stdout output from the failed command (may contain useful context).
    pub stdout: String,
    /// The extracted root cause.
    pub root_cause: RootCause,
    /// When the failure occurred.
    pub timestamp: DateTime<Utc>,
}

/// A persisted failure pattern for matching against future plans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePattern {
    /// SQLite row id.
    pub id: Option<i64>,
    /// The goal that was being pursued.
    pub goal: String,
    /// The binary that failed (e.g., "systemctl", "apt").
    pub failed_binary: String,
    /// The first argument (e.g., "restart", "install").
    pub failed_arg: Option<String>,
    /// The root cause category.
    pub root_cause_category: String,
    /// The stderr snippet (for matching).
    pub stderr_signature: String,
    /// How many times this pattern has been observed.
    pub occurrences: i64,
    /// Confidence that this pattern is accurate (Beta posterior).
    pub confidence: f64,
    /// Suggested alternative command (if known).
    pub suggested_alternative: Option<String>,
    /// When this pattern was first observed.
    pub first_seen: DateTime<Utc>,
    /// When this pattern was last observed.
    pub last_seen: DateTime<Utc>,
}

impl std::fmt::Display for RootCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description())
    }
}
