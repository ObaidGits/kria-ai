//! Capability-Based PolicyGate — Deterministic command safety evaluation.
//!
//! # Design: Capability Profiles (NOT Binary Allowlists)
//!
//! A static allowlist of binaries (rsync, tar, curl, etc.) is a closed-world
//! assumption that becomes a maintenance nightmare. Instead, we define
//! **capabilities** that describe what a command DOES, then map binaries
//! to capabilities.
//!
//! ## Capability Hierarchy
//!
//! ```text
//! ReadFilesystem     — read files, list dirs, stat, search
//! WriteFilesystem    — create, modify, delete files
//! NetworkRead        — HTTP GET, DNS lookup, ping
//! NetworkWrite       — HTTP POST, send mail, push to remote
//! ProcessInspect     — list processes, check status
//! ProcessControl     — start, stop, restart, kill processes
//! SystemDestructive  — shutdown, reboot, format disk, rm -rf /
//! ```
//!
//! ## Decision Flow
//!
//! ```text
//! StructuredCommand { binary, args }
//!        ↓
//! resolve_capabilities(binary, args) → Set<CommandCapability>
//!        ↓
//! evaluate_capabilities(caps) → PolicyDecision
//! ```
//!
//! The capability resolver is a simple lookup table (binary → default caps)
//! with argument-based overrides (e.g., `systemctl status` → ProcessInspect,
//! `systemctl restart` → ProcessControl).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::RwLock;

use crate::safety::RiskLevel;

// ─── Command Capabilities ────────────────────────────────────────────────────

/// Discrete capabilities that a command can possess.
/// A single command may have MULTIPLE capabilities (e.g., `docker run` has
/// both ProcessControl and NetworkRead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CommandCapability {
    /// Read files, list directories, stat, search. No modification.
    ReadFilesystem,
    /// Create, modify, or delete files.
    WriteFilesystem,
    /// Read from network: HTTP GET, DNS lookup, ping, curl GET.
    NetworkRead,
    /// Write to network: HTTP POST, send mail, git push, curl POST.
    NetworkWrite,
    /// Inspect processes: ps, top, htop, systemctl status.
    ProcessInspect,
    /// Control processes: start, stop, restart, kill, systemctl restart.
    ProcessControl,
    /// Destructive system operations: shutdown, reboot, format, rm -rf /.
    SystemDestructive,
    /// Execute arbitrary code: python3, node, bash -c.
    /// This is the most dangerous capability — it bypasses all other checks.
    CodeExecution,
}

impl CommandCapability {
    /// Returns `true` if this capability is read-only (no system modification).
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::ReadFilesystem | Self::NetworkRead | Self::ProcessInspect
        )
    }

    /// Returns `true` if this capability is destructive.
    pub fn is_destructive(&self) -> bool {
        matches!(self, Self::SystemDestructive | Self::CodeExecution)
    }

    /// Returns the risk level associated with this capability.
    pub fn risk_level(&self) -> RiskLevel {
        match self {
            Self::ReadFilesystem | Self::NetworkRead | Self::ProcessInspect => RiskLevel::Green,
            Self::WriteFilesystem | Self::NetworkWrite => RiskLevel::Yellow,
            Self::ProcessControl => RiskLevel::Yellow,
            Self::SystemDestructive | Self::CodeExecution => RiskLevel::Red,
        }
    }
}

impl fmt::Display for CommandCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadFilesystem => write!(f, "read_filesystem"),
            Self::WriteFilesystem => write!(f, "write_filesystem"),
            Self::NetworkRead => write!(f, "network_read"),
            Self::NetworkWrite => write!(f, "network_write"),
            Self::ProcessInspect => write!(f, "process_inspect"),
            Self::ProcessControl => write!(f, "process_control"),
            Self::SystemDestructive => write!(f, "system_destructive"),
            Self::CodeExecution => write!(f, "code_execution"),
        }
    }
}

// ─── Policy Decision ─────────────────────────────────────────────────────────

/// What the PolicyGate decided about a command.
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    /// Auto-approved. No user interaction needed.
    AutoApproved {
        risk_level: RiskLevel,
        capabilities: HashSet<CommandCapability>,
    },
    /// Requires HITL approval before execution.
    RequiresApproval {
        risk_level: RiskLevel,
        capabilities: HashSet<CommandCapability>,
        reason: String,
    },
    /// Blocked. Cannot be executed under any circumstances.
    Blocked { reason: String },
}

impl PolicyDecision {
    pub fn is_auto_approved(&self) -> bool {
        matches!(self, Self::AutoApproved { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    pub fn risk_level(&self) -> RiskLevel {
        match self {
            Self::AutoApproved { risk_level, .. } => *risk_level,
            Self::RequiresApproval { risk_level, .. } => *risk_level,
            Self::Blocked { .. } => RiskLevel::Black,
        }
    }
}

// ─── Binary Profile ──────────────────────────────────────────────────────────

/// Maps a binary to its default capabilities and argument-based overrides.
#[derive(Debug, Clone)]
pub struct BinaryProfile {
    /// The binary name (e.g., "systemctl", "git", "curl").
    pub binary: String,
    /// Default capabilities when no argument pattern matches.
    pub default_capabilities: HashSet<CommandCapability>,
    /// Argument-based overrides. First match wins.
    pub arg_overrides: Vec<ArgCapabilityOverride>,
}

/// An argument-based capability override.
#[derive(Debug, Clone)]
pub struct ArgCapabilityOverride {
    /// Match pattern for the first argument.
    pub pattern: ArgPattern,
    /// Capabilities when this pattern matches.
    pub capabilities: HashSet<CommandCapability>,
}

/// Pattern matching for arguments.
#[derive(Debug, Clone)]
pub enum ArgPattern {
    /// Exact match on the first argument.
    Exact(String),
    /// First argument starts with this prefix.
    Prefix(String),
    /// First argument contains this substring.
    Contains(String),
    /// Any arguments (matches everything).
    Any,
}

impl ArgPattern {
    pub fn matches(&self, arg: &str) -> bool {
        match self {
            Self::Exact(expected) => arg == expected,
            Self::Prefix(prefix) => arg.starts_with(prefix),
            Self::Contains(sub) => arg.contains(sub),
            Self::Any => true,
        }
    }
}

// ─── Policy Gate Trait ───────────────────────────────────────────────────────

/// The PolicyGate trait. Evaluates commands against safety rules.
///
/// Implementations must be `Send + Sync` (safe for跨-task sharing via Arc).
pub trait PolicyGate: Send + Sync {
    /// Evaluate a command against the policy rules.
    fn evaluate(&self, binary: &str, args: &[String]) -> PolicyDecision;

    /// Resolve the capabilities of a command.
    fn resolve_capabilities(&self, binary: &str, args: &[String]) -> HashSet<CommandCapability>;

    /// Check if a binary is known (has a profile).
    fn is_known_binary(&self, binary: &str) -> bool;

    /// Get the risk level for a command.
    fn classify_risk(&self, binary: &str, args: &[String]) -> RiskLevel;
}

// ─── Capability-Based Policy Gate ────────────────────────────────────────────

/// A PolicyGate implementation that uses capability profiles.
///
/// Rules are evaluated in this order:
/// 1. Blocked binaries (always blocked, regardless of args)
/// 2. Binary profile with argument overrides
/// 3. Unknown binary → CodeExecution capability → RequiresApproval
pub struct CapabilityPolicyGate {
    /// Binary profiles. Key = binary name.
    profiles: HashMap<String, BinaryProfile>,
    /// Permanently blocked binaries.
    blocked_binaries: HashSet<String>,
    /// Blocked argument patterns: (binary, args_prefix).
    blocked_arg_patterns: Vec<(String, Vec<String>)>,
    /// Custom rules added at runtime.
    custom_rules: RwLock<Vec<CustomRule>>,
}

/// A custom rule added at runtime (e.g., user approved a specific binary).
#[derive(Debug, Clone)]
pub struct CustomRule {
    pub binary: String,
    pub arg_pattern: ArgPattern,
    pub decision: PolicyDecision,
    pub description: String,
    /// Expiry time (None = permanent).
    pub expires_at: Option<std::time::Instant>,
}

impl Default for CapabilityPolicyGate {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityPolicyGate {
    pub fn new() -> Self {
        let mut gate = Self {
            profiles: HashMap::new(),
            blocked_binaries: HashSet::new(),
            blocked_arg_patterns: Vec::new(),
            custom_rules: RwLock::new(Vec::new()),
        };
        gate.load_default_profiles();
        gate
    }

    /// Add a custom rule (e.g., user approved a specific command).
    pub fn add_custom_rule(&self, rule: CustomRule) {
        let mut rules = self.custom_rules.write().unwrap();
        rules.push(rule);
    }

    fn load_default_profiles(&mut self) {
        use CommandCapability::*;

        // ─── Read-Only Binaries ───────────────────────────────────────────
        let readonly_binaries = [
            ("ls", vec![ReadFilesystem]),
            ("cat", vec![ReadFilesystem]),
            ("head", vec![ReadFilesystem]),
            ("tail", vec![ReadFilesystem]),
            ("less", vec![ReadFilesystem]),
            ("more", vec![ReadFilesystem]),
            ("wc", vec![ReadFilesystem]),
            ("find", vec![ReadFilesystem]),
            ("which", vec![ReadFilesystem]),
            ("file", vec![ReadFilesystem]),
            ("stat", vec![ReadFilesystem]),
            ("realpath", vec![ReadFilesystem]),
            ("readlink", vec![ReadFilesystem]),
            ("md5sum", vec![ReadFilesystem]),
            ("sha256sum", vec![ReadFilesystem]),
            ("diff", vec![ReadFilesystem]),
            ("tree", vec![ReadFilesystem]),
            ("du", vec![ReadFilesystem]),
            ("df", vec![ReadFilesystem]),
            ("top", vec![ProcessInspect]),
            ("htop", vec![ProcessInspect]),
            ("ps", vec![ProcessInspect]),
            ("pgrep", vec![ProcessInspect]),
            ("pidof", vec![ProcessInspect]),
            ("free", vec![ProcessInspect]),
            ("uptime", vec![ProcessInspect]),
            ("uname", vec![ReadFilesystem]),
            ("hostname", vec![ReadFilesystem]),
            ("id", vec![ReadFilesystem]),
            ("whoami", vec![ReadFilesystem]),
            ("pwd", vec![ReadFilesystem]),
            ("env", vec![ReadFilesystem]),
            ("printenv", vec![ReadFilesystem]),
            ("lscpu", vec![ProcessInspect]),
            ("lspci", vec![ProcessInspect]),
            ("lsusb", vec![ProcessInspect]),
            ("lsblk", vec![ProcessInspect]),
            ("lsmod", vec![ProcessInspect]),
            ("ip", vec![NetworkRead, ProcessInspect]),
            ("ss", vec![NetworkRead, ProcessInspect]),
            ("ping", vec![NetworkRead]),
            ("traceroute", vec![NetworkRead]),
            ("dig", vec![NetworkRead]),
            ("nslookup", vec![NetworkRead]),
            ("host", vec![NetworkRead]),
            ("jq", vec![ReadFilesystem]),
            ("awk", vec![ReadFilesystem]),
            ("sed", vec![ReadFilesystem]),
            ("tr", vec![ReadFilesystem]),
            ("sort", vec![ReadFilesystem]),
            ("uniq", vec![ReadFilesystem]),
            ("cut", vec![ReadFilesystem]),
            ("grep", vec![ReadFilesystem]),
            ("rg", vec![ReadFilesystem]),
            ("fd", vec![ReadFilesystem]),
            ("dmesg", vec![ProcessInspect]),
            ("journalctl", vec![ProcessInspect]),
            ("iostat", vec![ProcessInspect]),
            ("vmstat", vec![ProcessInspect]),
            ("sar", vec![ProcessInspect]),
            ("mpstat", vec![ProcessInspect]),
        ];

        for (binary, caps) in &readonly_binaries {
            self.profiles.insert(
                binary.to_string(),
                BinaryProfile {
                    binary: binary.to_string(),
                    default_capabilities: caps.iter().copied().collect(),
                    arg_overrides: Vec::new(),
                },
            );
        }

        // ─── Multi-Capability Binaries with Arg Overrides ─────────────────

        // systemctl: status → ProcessInspect, restart/stop/start → ProcessControl
        self.profiles.insert(
            "systemctl".into(),
            BinaryProfile {
                binary: "systemctl".into(),
                default_capabilities: [ProcessInspect].into_iter().collect(),
                arg_overrides: vec![
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("status".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("list-units".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("is-active".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("restart".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("reload".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("start".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("stop".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("enable".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("disable".into()),
                        capabilities: [ProcessControl, WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("kill".into()),
                        capabilities: [ProcessControl, SystemDestructive].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("mask".into()),
                        capabilities: [ProcessControl, WriteFilesystem].into_iter().collect(),
                    },
                ],
            },
        );

        // git: status/log/diff/show → ReadFilesystem, commit → WriteFilesystem, push → NetworkWrite
        self.profiles.insert(
            "git".into(),
            BinaryProfile {
                binary: "git".into(),
                default_capabilities: [ReadFilesystem].into_iter().collect(),
                arg_overrides: vec![
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("status".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("log".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("diff".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("show".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("branch".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("remote".into()),
                        capabilities: [ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("commit".into()),
                        capabilities: [WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("add".into()),
                        capabilities: [WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("checkout".into()),
                        capabilities: [WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("merge".into()),
                        capabilities: [WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("rebase".into()),
                        capabilities: [WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("push".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("pull".into()),
                        capabilities: [NetworkRead, WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("clone".into()),
                        capabilities: [NetworkRead, WriteFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("fetch".into()),
                        capabilities: [NetworkRead].into_iter().collect(),
                    },
                ],
            },
        );

        // docker: ps/images → ProcessInspect, run/exec → ProcessControl + NetworkRead
        self.profiles.insert(
            "docker".into(),
            BinaryProfile {
                binary: "docker".into(),
                default_capabilities: [ProcessInspect].into_iter().collect(),
                arg_overrides: vec![
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("ps".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("images".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("inspect".into()),
                        capabilities: [ProcessInspect].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("logs".into()),
                        capabilities: [ProcessInspect, ReadFilesystem].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("run".into()),
                        capabilities: [ProcessControl, NetworkRead, WriteFilesystem]
                            .into_iter()
                            .collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("exec".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("start".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("stop".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("restart".into()),
                        capabilities: [ProcessControl].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("kill".into()),
                        capabilities: [ProcessControl, SystemDestructive].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Exact("rm".into()),
                        capabilities: [SystemDestructive].into_iter().collect(),
                    },
                ],
            },
        );

        // curl/wget: GET → NetworkRead, POST/PUT → NetworkWrite
        self.profiles.insert(
            "curl".into(),
            BinaryProfile {
                binary: "curl".into(),
                default_capabilities: [NetworkRead].into_iter().collect(),
                arg_overrides: vec![
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-X POST".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-X PUT".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-X DELETE".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("--data".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-d".into()),
                        capabilities: [NetworkWrite].into_iter().collect(),
                    },
                ],
            },
        );

        self.profiles.insert(
            "wget".into(),
            BinaryProfile {
                binary: "wget".into(),
                default_capabilities: [NetworkRead].into_iter().collect(),
                arg_overrides: Vec::new(),
            },
        );

        // File operations: mkdir/cp/mv/chmod/chown → WriteFilesystem
        for binary in &[
            "mkdir", "cp", "mv", "ln", "chmod", "chown", "chgrp", "touch",
        ] {
            self.profiles.insert(
                binary.to_string(),
                BinaryProfile {
                    binary: binary.to_string(),
                    default_capabilities: [WriteFilesystem].into_iter().collect(),
                    arg_overrides: Vec::new(),
                },
            );
        }

        // rm: WriteFilesystem (destructive only with -rf /)
        self.profiles.insert(
            "rm".into(),
            BinaryProfile {
                binary: "rm".into(),
                default_capabilities: [WriteFilesystem].into_iter().collect(),
                arg_overrides: vec![
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-rf /".into()),
                        capabilities: [SystemDestructive].into_iter().collect(),
                    },
                    ArgCapabilityOverride {
                        pattern: ArgPattern::Prefix("-rf --no-preserve-root /".into()),
                        capabilities: [SystemDestructive].into_iter().collect(),
                    },
                ],
            },
        );

        // Package managers: install/remove → WriteFilesystem + ProcessControl
        for binary in &["apt", "apt-get", "dnf", "yum", "pacman", "zypper"] {
            self.profiles.insert(
                binary.to_string(),
                BinaryProfile {
                    binary: binary.to_string(),
                    default_capabilities: [ProcessInspect].into_iter().collect(),
                    arg_overrides: vec![
                        ArgCapabilityOverride {
                            pattern: ArgPattern::Prefix("install".into()),
                            capabilities: [WriteFilesystem, ProcessControl, NetworkRead]
                                .into_iter()
                                .collect(),
                        },
                        ArgCapabilityOverride {
                            pattern: ArgPattern::Prefix("remove".into()),
                            capabilities: [WriteFilesystem, ProcessControl].into_iter().collect(),
                        },
                        ArgCapabilityOverride {
                            pattern: ArgPattern::Prefix("update".into()),
                            capabilities: [NetworkRead].into_iter().collect(),
                        },
                        ArgCapabilityOverride {
                            pattern: ArgPattern::Prefix("search".into()),
                            capabilities: [ProcessInspect].into_iter().collect(),
                        },
                        ArgCapabilityOverride {
                            pattern: ArgPattern::Prefix("list".into()),
                            capabilities: [ProcessInspect].into_iter().collect(),
                        },
                    ],
                },
            );
        }

        // Code interpreters: always CodeExecution
        for binary in &["python3", "python", "node", "ruby", "perl", "php", "lua"] {
            self.profiles.insert(
                binary.to_string(),
                BinaryProfile {
                    binary: binary.to_string(),
                    default_capabilities: [CodeExecution].into_iter().collect(),
                    arg_overrides: Vec::new(),
                },
            );
        }

        // ─── Blocked Binaries (Always Blocked) ───────────────────────────
        self.blocked_binaries.insert("dd".into());
        self.blocked_binaries.insert("mkfs".into());
        self.blocked_binaries.insert("fdisk".into());
        self.blocked_binaries.insert("parted".into());
        self.blocked_binaries.insert("shutdown".into());
        self.blocked_binaries.insert("reboot".into());
        self.blocked_binaries.insert("poweroff".into());
        self.blocked_binaries.insert("halt".into());
        self.blocked_binaries.insert("init".into());
        self.blocked_binaries.insert("telinit".into());

        // Shell interpreters — the LLM MUST NOT invoke these directly.
        // Commands go through SubprocessExecutor which passes binary + args
        // directly to the OS (no shell parsing).
        self.blocked_binaries.insert("bash".into());
        self.blocked_binaries.insert("sh".into());
        self.blocked_binaries.insert("zsh".into());
        self.blocked_binaries.insert("fish".into());
        self.blocked_binaries.insert("csh".into());
        self.blocked_binaries.insert("tcsh".into());

        // ─── Blocked Argument Patterns ────────────────────────────────────
        self.blocked_arg_patterns
            .push(("rm".into(), vec!["-rf".into(), "/".into()]));
        self.blocked_arg_patterns.push((
            "rm".into(),
            vec!["-rf".into(), "--no-preserve-root".into(), "/".into()],
        ));
        self.blocked_arg_patterns
            .push(("rm".into(), vec!["-rf".into(), "~".into()]));
        self.blocked_arg_patterns
            .push(("rm".into(), vec!["-rf".into(), "/home".into()]));
        self.blocked_arg_patterns
            .push(("rm".into(), vec!["-rf".into(), "/etc".into()]));
    }
}

/// Extract the inner command string from shell interpreter args like `["-c", "systemctl status nginx"]`.
/// Returns `None` if the args don't contain a `-c` flag with a subsequent argument.
fn extract_shell_c_command(args: &[String]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == "-c" {
            // The next argument is the command string
            return args.get(i + 1).cloned();
        }
        // Handle combined form like `-ceval` — not standard, skip
    }
    None
}

/// How many nested shell invocations are followed before the command is refused.
///
/// `bash -c "sh -c '…'"` is legitimate but rare; nesting deeper than this is far
/// more likely to be an attempt to bury a prohibited command under enough layers
/// that the inspector gives up. Refusing is the safe direction: a false refusal
/// is visible and recoverable, a missed prohibition is neither.
const MAX_SHELL_NESTING: u8 = 4;

impl CapabilityPolicyGate {
    fn evaluate_at_depth(&self, binary: &str, args: &[String], depth: u8) -> PolicyDecision {
        // Command policy is DEFENCE-IN-DEPTH, subordinate to `ExecutionGate`
        // (design §2.1 / §6, OSC-001/OSC-002/OSC-004). Its hard prohibitions —
        // BLACK-scope administration, blocked binaries, and blocked argument
        // patterns — are evaluated BEFORE any runtime custom rule, so a custom
        // rule can never un-block a prohibited command, substitute for action
        // approval, or otherwise broaden authority. Custom rules apply only to
        // otherwise-allowable commands (step 5, below).

        // 1. BLACK-scope containment (OSC-030/OSC-004): a structured command that
        //    reaches prohibited administration is permanently blocked before
        //    capability resolution, custom rules, or approval. This mirrors the
        //    PolicyEngine block so both admission paths agree.
        if let Some(prohibited) = crate::safety::black_scope::classify_structured(binary, args) {
            return PolicyDecision::Blocked {
                reason: format!(
                    "prohibited scope [{}]: {}",
                    prohibited.id(),
                    prohibited.boundary_explanation()
                ),
            };
        }

        // 2. Command-level granularity for shell interpreters.
        //    When the binary is bash/sh/zsh with `-c <command>`, extract the
        //    inner command string and classify it via command_classifier.
        //    This ensures the SubprocessExecutor path agrees with PolicyEngine
        //    on tiering for the same command string (defense-in-depth).
        if matches!(binary, "bash" | "sh" | "zsh" | "fish" | "csh" | "tcsh") {
            if let Some(inner_cmd) = extract_shell_c_command(args) {
                // Prohibited administration smuggled through `sh -c "…"` is
                // blocked before any tiering decision.
                if let Some(prohibited) = crate::safety::black_scope::classify_command(&inner_cmd) {
                    return PolicyDecision::Blocked {
                        reason: format!(
                            "prohibited scope [{}]: {}",
                            prohibited.id(),
                            prohibited.boundary_explanation()
                        ),
                    };
                }
                let classification = crate::safety::command_classifier::classify(&inner_cmd);
                // The classifier's tier is only ONE opinion about the inner command.
                // Take the strictest of it and the verdict the inner command would
                // have received had it been invoked directly — see
                // `strictest_inner_verdict` for why that is required for soundness.
                let classifier_decision = match classification.tier {
                    RiskLevel::Green => PolicyDecision::AutoApproved {
                        risk_level: RiskLevel::Green,
                        capabilities: [CommandCapability::ProcessInspect].into_iter().collect(),
                    },
                    RiskLevel::Yellow => PolicyDecision::AutoApproved {
                        risk_level: RiskLevel::Yellow,
                        capabilities: [CommandCapability::ProcessControl].into_iter().collect(),
                    },
                    RiskLevel::Red => PolicyDecision::RequiresApproval {
                        risk_level: RiskLevel::Red,
                        capabilities: [CommandCapability::CodeExecution].into_iter().collect(),
                        reason: classification.reason,
                    },
                    RiskLevel::Black => PolicyDecision::Blocked {
                        reason: classification.reason,
                    },
                };
                return self.strictest_inner_verdict(&inner_cmd, classifier_decision, depth);
            }
        }

        // 3. Check blocked binaries
        if self.blocked_binaries.contains(binary) {
            return PolicyDecision::Blocked {
                reason: format!("Binary '{}' is permanently blocked", binary),
            };
        }

        // 4. Check blocked argument patterns
        for (blocked_binary, blocked_args) in &self.blocked_arg_patterns {
            if binary == blocked_binary && args.starts_with(blocked_args) {
                return PolicyDecision::Blocked {
                    reason: format!(
                        "Command '{} {}' matches blocked pattern",
                        binary,
                        blocked_args.join(" ")
                    ),
                };
            }
        }

        // 4b. Runtime custom rules (subordinate). These can refine the decision
        //     for an otherwise-allowable command, but they run only AFTER the
        //     BLACK-scope / blocked-binary / blocked-pattern hard denials above,
        //     so they cannot un-block a prohibited command or broaden authority.
        {
            let rules = self.custom_rules.read().unwrap();
            for rule in rules.iter() {
                if rule.binary == binary {
                    let matches = match &rule.arg_pattern {
                        ArgPattern::Any => true,
                        pattern => args.first().map(|a| pattern.matches(a)).unwrap_or(false),
                    };
                    if matches {
                        // Check expiry
                        if let Some(expires) = rule.expires_at {
                            if std::time::Instant::now() > expires {
                                continue;
                            }
                        }
                        return rule.decision.clone();
                    }
                }
            }
        }

        // 5. Resolve capabilities
        let caps = self.resolve_capabilities(binary, args);

        // 6. Determine risk level from capabilities
        let max_risk = caps
            .iter()
            .map(|c| c.risk_level())
            .max()
            .unwrap_or(RiskLevel::Yellow);

        // 7. Decision based on risk level
        match max_risk {
            RiskLevel::Green => PolicyDecision::AutoApproved {
                risk_level: max_risk,
                capabilities: caps,
            },
            RiskLevel::Yellow => PolicyDecision::AutoApproved {
                risk_level: max_risk,
                capabilities: caps,
            },
            RiskLevel::Red => PolicyDecision::RequiresApproval {
                risk_level: max_risk,
                capabilities: caps.clone(),
                reason: format!(
                    "Command requires approval: capabilities include {:?}",
                    caps.iter()
                        .filter(|c| !c.is_read_only())
                        .collect::<Vec<_>>()
                ),
            },
            RiskLevel::Black => PolicyDecision::Blocked {
                reason: format!("Command '{}' is blocked (Black risk)", binary),
            },
        }
    }

    /// Combine the classifier's opinion of a shell's inner command with the verdict
    /// that command would have received had it been invoked directly, and return
    /// whichever is stricter.
    ///
    /// # Why this is necessary
    ///
    /// Two independent judgements exist in this gate, and they do not agree:
    ///
    /// * `command_classifier` / `black_scope` read a command STRING.
    /// * capability resolution (step 5) reads a structured `(binary, args)` pair.
    ///
    /// Destructive filesystem removal is recognised by the second and not the first.
    /// So returning the classifier's tier alone for `bash -c "<destructive command>"`
    /// let a command through the shell wrapper that was blocked when run directly —
    /// the wrapper became a bypass. Replacing the old blanket "all shells are
    /// blocked" rule with command inspection is right, but only if the inspection is
    /// at least as strict as the direct path.
    ///
    /// The invariant this restores: **wrapping a command in a shell can never make it
    /// more permissible than running it directly.**
    ///
    /// Each `;`, `&&`, `||` or `|` separated segment is judged on its own and the
    /// strictest verdict wins, so a prohibited command cannot hide behind a harmless
    /// first segment such as `echo hi; <destructive command>`.
    fn strictest_inner_verdict(
        &self,
        inner_cmd: &str,
        classifier_decision: PolicyDecision,
        depth: u8,
    ) -> PolicyDecision {
        if depth >= MAX_SHELL_NESTING {
            return PolicyDecision::Blocked {
                reason: format!(
                    "shell nesting deeper than {MAX_SHELL_NESTING} levels is refused: the \
                     inner command cannot be inspected with confidence"
                ),
            };
        }

        let mut strictest = classifier_decision;
        for segment in split_shell_segments(inner_cmd) {
            let Some((segment_binary, segment_args)) = segment.split_first() else {
                continue;
            };
            let verdict = self.evaluate_at_depth(segment_binary, segment_args, depth + 1);
            // `risk_level()` reports Black for a Blocked decision, so a single
            // comparison orders every variant correctly.
            if verdict.risk_level() > strictest.risk_level() {
                strictest = verdict;
            }
        }
        strictest
    }
}

/// Split a shell command string into independently-judged argv segments.
///
/// A shell command can carry several commands at once (`a; b`, `a && b`, `a | b`),
/// and each one executes, so each must be judged. Returning them separately lets the
/// caller take the strictest verdict rather than reading only the first command.
///
/// # Why this respects quotes
///
/// A whitespace-only split gets nested shells wrong in the dangerous direction.
/// `sh -c 'rmdir /some/path'` would tokenise to `["sh", "-c", "'rmdir", "/some/path'"]`,
/// so the inner command seen by the next layer is the fragment `'rmdir` — which looks
/// harmless. The quoted run has to survive as ONE token for the recursion to inspect
/// the real command, otherwise every prohibition can be hidden one quote deep.
///
/// This is still not a full shell parser — it does not do expansion, substitution or
/// here-documents. It handles quoting and the command separators, which is what
/// deciding "how many commands are here, and what is each one" requires.
fn split_shell_segments(command: &str) -> Vec<Vec<String>> {
    // Close off the token being built, if any.
    fn end_token(token: &mut String, current: &mut Vec<String>) {
        if !token.is_empty() {
            current.push(std::mem::take(token));
        }
    }
    // Close off the segment being built, if any.
    fn end_segment(
        token: &mut String,
        current: &mut Vec<String>,
        segments: &mut Vec<Vec<String>>,
    ) {
        end_token(token, current);
        if !current.is_empty() {
            segments.push(std::mem::take(current));
        }
    }

    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut token = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();

    while let Some(c) = chars.next() {
        match quote {
            // Inside quotes only the matching close quote is special, so separators
            // and whitespace stay part of the command string.
            Some(q) => {
                if c == q {
                    quote = None;
                    // A quoted run is a token even when empty, so `sh -c ''` does not
                    // silently vanish.
                    current.push(std::mem::take(&mut token));
                } else {
                    token.push(c);
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                // A backslash escapes the next character outside quotes.
                '\\' => {
                    if let Some(next) = chars.next() {
                        token.push(next);
                    }
                }
                ';' | '\n' => end_segment(&mut token, &mut current, &mut segments),
                '&' | '|' => {
                    // `&&` and `||` are one separator, `&` and `|` are also separators,
                    // so either way the segment ends here.
                    if chars.peek() == Some(&c) {
                        chars.next();
                    }
                    end_segment(&mut token, &mut current, &mut segments);
                }
                c if c.is_whitespace() => end_token(&mut token, &mut current),
                _ => token.push(c),
            },
        }
    }
    // An unterminated quote means the string cannot be read with confidence. Keeping
    // what was collected is the strict choice: it yields MORE to inspect, never less.
    end_segment(&mut token, &mut current, &mut segments);
    segments
}

impl PolicyGate for CapabilityPolicyGate {
    fn evaluate(&self, binary: &str, args: &[String]) -> PolicyDecision {
        self.evaluate_at_depth(binary, args, 0)
    }

    fn resolve_capabilities(&self, binary: &str, args: &[String]) -> HashSet<CommandCapability> {
        // Look up binary profile
        if let Some(profile) = self.profiles.get(binary) {
            // Check argument overrides (first match wins)
            if let Some(first_arg) = args.first() {
                for arg_override in &profile.arg_overrides {
                    if arg_override.pattern.matches(first_arg) {
                        return arg_override.capabilities.clone();
                    }
                }
            }
            // No override matched — use default
            return profile.default_capabilities.clone();
        }

        // Unknown binary → CodeExecution (conservative default)
        [CommandCapability::CodeExecution].into_iter().collect()
    }

    fn is_known_binary(&self, binary: &str) -> bool {
        self.profiles.contains_key(binary) || self.blocked_binaries.contains(binary)
    }

    fn classify_risk(&self, binary: &str, args: &[String]) -> RiskLevel {
        // Check blocked binaries first
        if self.blocked_binaries.contains(binary) {
            return RiskLevel::Black;
        }

        // Check blocked arg patterns
        for (blocked_binary, blocked_args) in &self.blocked_arg_patterns {
            if binary == blocked_binary && args.starts_with(blocked_args) {
                return RiskLevel::Black;
            }
        }

        let caps = self.resolve_capabilities(binary, args);
        if caps.is_empty() {
            RiskLevel::Yellow
        } else {
            caps.iter()
                .map(|c| c.risk_level())
                .max()
                .unwrap_or(RiskLevel::Yellow)
        }
    }
}
