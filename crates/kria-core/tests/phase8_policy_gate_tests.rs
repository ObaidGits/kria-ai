//! Tests for the Capability-Based PolicyGate.
//!
//! Test IDs: PG01-PG20

use kria_core::safety::policy_gate::{
    ArgPattern, ArgCapabilityOverride, BinaryProfile, CapabilityPolicyGate,
    CommandCapability, CustomRule, PolicyDecision, PolicyGate,
};
use kria_core::safety::RiskLevel;
use std::collections::HashSet;

fn gate() -> CapabilityPolicyGate {
    CapabilityPolicyGate::new()
}

fn args(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

// ─── Read-Only Auto-Approval ─────────────────────────────────────────────────

#[test]
fn pg01_readonly_commands_auto_approved() {
    let g = gate();
    for binary in &["ls", "cat", "top", "ps", "df", "free", "uptime", "uname"] {
        let decision = g.evaluate(binary, &[]);
        assert!(
            decision.is_auto_approved(),
            "Expected {} to be auto-approved, got {:?}",
            binary,
            decision
        );
    }
}

#[test]
fn pg02_systemctl_status_auto_approved() {
    let g = gate();
    let decision = g.evaluate("systemctl", &args(&["status", "nginx"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Green);
}

#[test]
fn pg03_git_status_auto_approved() {
    let g = gate();
    let decision = g.evaluate("git", &args(&["status"]));
    assert!(decision.is_auto_approved());
}

#[test]
fn pg04_git_log_auto_approved() {
    let g = gate();
    let decision = g.evaluate("git", &args(&["log", "--oneline"]));
    assert!(decision.is_auto_approved());
}

#[test]
fn pg05_git_diff_auto_approved() {
    let g = gate();
    let decision = g.evaluate("git", &args(&["diff"]));
    assert!(decision.is_auto_approved());
}

#[test]
fn pg06_docker_ps_auto_approved() {
    let g = gate();
    let decision = g.evaluate("docker", &args(&["ps"]));
    assert!(decision.is_auto_approved());
}

#[test]
fn pg07_docker_images_auto_approved() {
    let g = gate();
    let decision = g.evaluate("docker", &args(&["images"]));
    assert!(decision.is_auto_approved());
}

// ─── Yellow (Auto-Approved with higher risk) ────────────────────────────────

#[test]
fn pg08_mkdir_auto_approved_yellow() {
    let g = gate();
    let decision = g.evaluate("mkdir", &args(&["/tmp/test"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

#[test]
fn pg09_cp_auto_approved_yellow() {
    let g = gate();
    let decision = g.evaluate("cp", &args(&["file1", "file2"]));
    assert!(decision.is_auto_approved());
}

#[test]
fn pg10_systemctl_restart_auto_approved_yellow() {
    let g = gate();
    let decision = g.evaluate("systemctl", &args(&["restart", "nginx"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

// ─── Yellow (Requires Approval for destructive actions) ─────────────────────

#[test]
fn pg11_rm_auto_approved_yellow() {
    // rm with a normal file is WriteFilesystem (Yellow) → AutoApproved
    let g = gate();
    let decision = g.evaluate("rm", &args(&["file.txt"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

#[test]
fn pg12_systemctl_stop_auto_approved_yellow() {
    // systemctl stop is ProcessControl (Yellow) → AutoApproved
    let g = gate();
    let decision = g.evaluate("systemctl", &args(&["stop", "nginx"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

#[test]
fn pg13_git_push_auto_approved_yellow() {
    // git push is NetworkWrite (Yellow) → AutoApproved
    let g = gate();
    let decision = g.evaluate("git", &args(&["push"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

#[test]
fn pg14_apt_install_auto_approved_yellow() {
    // apt install is WriteFilesystem + ProcessControl + NetworkRead (Yellow) → AutoApproved
    let g = gate();
    let decision = g.evaluate("apt", &args(&["install", "nginx"]));
    assert!(decision.is_auto_approved());
    assert_eq!(decision.risk_level(), RiskLevel::Yellow);
}

// ─── Black (Always Blocked) ─────────────────────────────────────────────────

#[test]
fn pg15_dd_blocked() {
    let g = gate();
    let decision = g.evaluate("dd", &args(&["if=/dev/zero", "of=/dev/sda"]));
    assert!(decision.is_blocked());
}

#[test]
fn pg16_shutdown_blocked() {
    let g = gate();
    let decision = g.evaluate("shutdown", &args(&["-h", "now"]));
    assert!(decision.is_blocked());
}

#[test]
fn pg17_rm_rf_root_blocked() {
    let g = gate();
    let decision = g.evaluate("rm", &args(&["-rf", "/"]));
    assert!(decision.is_blocked());
}

#[test]
fn pg18_bash_blocked() {
    let g = gate();
    let decision = g.evaluate("bash", &args(&["-c", "echo hello"]));
    assert!(decision.is_blocked(), "Shell interpreters must be blocked");
}

#[test]
fn pg19_sh_blocked() {
    let g = gate();
    let decision = g.evaluate("sh", &args(&["-c", "echo hello"]));
    assert!(decision.is_blocked());
}

// ─── Unknown Binary ─────────────────────────────────────────────────────────

#[test]
fn pg20_unknown_binary_quarantined() {
    let g = gate();
    let decision = g.evaluate("some_random_tool", &[]);
    // Unknown binaries get CodeExecution capability → RequiresApproval
    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
}

// ─── Capability Resolution ──────────────────────────────────────────────────

#[test]
fn pg21_capabilities_resolved_correctly() {
    let g = gate();

    // systemctl status → ProcessInspect
    let caps = g.resolve_capabilities("systemctl", &args(&["status", "nginx"]));
    assert!(caps.contains(&CommandCapability::ProcessInspect));
    assert!(!caps.contains(&CommandCapability::ProcessControl));

    // systemctl restart → ProcessControl
    let caps = g.resolve_capabilities("systemctl", &args(&["restart", "nginx"]));
    assert!(caps.contains(&CommandCapability::ProcessControl));

    // git push → NetworkWrite
    let caps = g.resolve_capabilities("git", &args(&["push"]));
    assert!(caps.contains(&CommandCapability::NetworkWrite));

    // curl GET → NetworkRead
    let caps = g.resolve_capabilities("curl", &args(&["https://example.com"]));
    assert!(caps.contains(&CommandCapability::NetworkRead));
}

#[test]
fn pg22_shell_injection_impossible() {
    let g = gate();
    // Even if the LLM tries to inject shell metacharacters in args,
    // they are passed as literal arguments, not interpreted.
    let decision = g.evaluate("ls", &args(&["; rm -rf /"]));
    assert!(decision.is_auto_approved()); // ls is auto-approved
    // The "; rm -rf /" is just a filename argument to ls, not a shell command
}

#[test]
fn pg23_is_known_binary() {
    let g = gate();
    assert!(g.is_known_binary("ls"));
    assert!(g.is_known_binary("systemctl"));
    assert!(g.is_known_binary("dd")); // blocked = known
    assert!(!g.is_known_binary("some_random_tool_xyz"));
}

#[test]
fn pg24_custom_rule_overrides() {
    let g = CapabilityPolicyGate::new();

    // Add a custom rule that allows a specific binary
    g.add_custom_rule(CustomRule {
        binary: "my_custom_tool".into(),
        arg_pattern: ArgPattern::Any,
        decision: PolicyDecision::AutoApproved {
            risk_level: RiskLevel::Green,
            capabilities: [CommandCapability::ReadFilesystem].into_iter().collect(),
        },
        description: "User approved this tool".into(),
        expires_at: None,
    });

    let decision = g.evaluate("my_custom_tool", &[]);
    assert!(decision.is_auto_approved());
}

#[test]
fn pg25_risk_level_classification() {
    let g = gate();

    // Read-only = Green
    assert_eq!(g.classify_risk("ls", &[]), RiskLevel::Green);
    assert_eq!(g.classify_risk("cat", &[]), RiskLevel::Green);

    // Write = Yellow
    assert_eq!(g.classify_risk("mkdir", &[]), RiskLevel::Yellow);
    assert_eq!(g.classify_risk("cp", &[]), RiskLevel::Yellow);

    // Process control = Yellow
    assert_eq!(g.classify_risk("systemctl", &args(&["restart", "nginx"])), RiskLevel::Yellow);

    // rm file = Yellow (WriteFilesystem, not SystemDestructive)
    assert_eq!(g.classify_risk("rm", &args(&["file.txt"])), RiskLevel::Yellow);

    // Blocked = Black
    assert_eq!(g.classify_risk("dd", &[]), RiskLevel::Black);
}

#[test]
fn pg25b_red_risk_operations() {
    let g = gate();

    // systemctl kill → ProcessControl + SystemDestructive → Red
    let decision = g.evaluate("systemctl", &args(&["kill", "nginx"]));
    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
    assert_eq!(decision.risk_level(), RiskLevel::Red);
}

#[test]
fn pg26_code_interpreter_capability() {
    let g = gate();

    // python3 gets CodeExecution capability
    let caps = g.resolve_capabilities("python3", &[]);
    assert!(caps.contains(&CommandCapability::CodeExecution));

    // Code execution is Red risk → RequiresApproval
    let decision = g.evaluate("python3", &[]);
    assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
}
