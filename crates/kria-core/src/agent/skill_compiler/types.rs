//! Skill Compiler types — compiled skills, variables, validation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::tools::subprocess_executor::StructuredCommand;

/// Status of a compiled skill in the quarantine pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SkillStatus {
    /// Accumulating successes before compilation (N < 3).
    Accumulating,
    /// Compiled and in quarantine (awaiting HITL approval for yellow/red).
    Quarantined,
    /// Promoted to active registry (auto for green, HITL for yellow/red).
    Active,
    /// Disabled by circuit breaker (3 consecutive failures).
    Disabled,
    /// Deprecated (confidence decayed below threshold).
    Deprecated,
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accumulating => write!(f, "accumulating"),
            Self::Quarantined => write!(f, "quarantined"),
            Self::Active => write!(f, "active"),
            Self::Disabled => write!(f, "disabled"),
            Self::Deprecated => write!(f, "deprecated"),
        }
    }
}

impl std::str::FromStr for SkillStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "accumulating" => Ok(Self::Accumulating),
            "quarantined" => Ok(Self::Quarantined),
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "deprecated" => Ok(Self::Deprecated),
            _ => Err(format!("unknown skill status: {}", s)),
        }
    }
}

/// Strict type for extracted variables — each type has validation rules.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VariableType {
    /// IPv4 or IPv6 address.
    IpAddress,
    /// Absolute file path (must start with /).
    FilePath,
    /// System service name (alphanumeric + hyphens only).
    ServiceName,
    /// TCP/UDP port number (1-65535).
    PortNumber,
    /// Hostname or domain name.
    Hostname,
    /// Numeric value (integer or float).
    Numeric,
    /// Generic string (most permissive — last resort).
    String,
}

impl VariableType {
    /// Validation regex for this variable type.
    pub fn validation_pattern(&self) -> &str {
        match self {
            Self::IpAddress => r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$|^[0-9a-fA-F:]+$",
            Self::FilePath => r"^/[\w./\-]+$",
            Self::ServiceName => r"^[a-zA-Z][\w\-\.]*$",
            Self::PortNumber => r"^[1-9]\d{0,4}$",
            Self::Hostname => r"^[a-zA-Z0-9][\w\-\.]+$",
            Self::Numeric => r"^\d+\.?\d*$",
            Self::String => r"^[^\s;|&`$(){}<>\\]+$",
        }
    }

    /// Whether this type allows shell metacharacters.
    pub fn allows_metacharacters(&self) -> bool {
        false // No type allows shell metacharacters
    }
}

/// A validated variable extracted from a successful plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillVariable {
    /// Parameter name (e.g., "target_host", "service_name").
    pub name: String,
    /// Strict type with validation.
    pub var_type: VariableType,
    /// Description of what this variable represents.
    pub description: String,
    /// Example values from the 3 successful executions.
    pub examples: Vec<String>,
    /// Whether this variable is required.
    pub required: bool,
}

/// A compiled skill — reusable tool schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledSkill {
    /// SQLite row id.
    pub id: Option<i64>,
    /// Unique skill name (e.g., "optimize_service_workers").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Goal patterns that trigger this skill.
    pub trigger_patterns: Vec<String>,
    /// Extracted variables with strict types.
    pub variables: Vec<SkillVariable>,
    /// Parameterized command sequence (uses {variable_name} placeholders).
    pub commands: Vec<ParameterizedCommand>,
    /// Number of successful executions observed.
    pub success_count: i64,
    /// Number of failed executions observed.
    pub failure_count: i64,
    /// Current status in the quarantine pipeline.
    pub status: SkillStatus,
    /// Confidence (Beta posterior mean).
    pub confidence: f64,
    /// Average execution duration.
    pub avg_duration_ms: i64,
    /// When this skill was first observed.
    pub first_seen: DateTime<Utc>,
    /// When this skill was last used.
    pub last_used: DateTime<Utc>,
}

/// A command with parameterized arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterizedCommand {
    /// The binary to execute.
    pub binary: String,
    /// Arguments with {variable} placeholders.
    pub args: Vec<String>,
    /// Target environment.
    pub target: String,
    /// Timeout in seconds.
    pub timeout_secs: u64,
}

impl ParameterizedCommand {
    /// Instantiate this command with concrete variable values.
    pub fn instantiate(
        &self,
        vars: &std::collections::HashMap<String, String>,
    ) -> Option<StructuredCommand> {
        let mut args = Vec::new();
        for arg in &self.args {
            let mut resolved = arg.clone();
            for (key, value) in vars {
                resolved = resolved.replace(&format!("{{{}}}", key), value);
            }
            // Safety: check that no unresolved placeholders remain
            if resolved.contains('{') && resolved.contains('}') {
                return None; // Unresolved variable
            }
            args.push(resolved);
        }

        Some(StructuredCommand {
            binary: self.binary.clone(),
            args,
            target: self.target.clone(),
            timeout_secs: self.timeout_secs,
            working_dir: None,
            env_vars: None,
        })
    }
}

/// Playbook — an uncompiled successful plan (before N=3 gating).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    /// SQLite row id.
    pub id: Option<i64>,
    /// The goal that was achieved.
    pub goal: String,
    /// The commands that were executed.
    pub commands: Vec<StructuredCommand>,
    /// The target environment.
    pub target: String,
    /// Duration of execution.
    pub duration_ms: i64,
    /// When this playbook was observed.
    pub observed_at: DateTime<Utc>,
}
