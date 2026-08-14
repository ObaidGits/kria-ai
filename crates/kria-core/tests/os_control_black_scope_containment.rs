//! Task 0.2 — Freeze BLACK scope and raw-shell containment.
//!
//! Table-driven policy + routing tests proving that prohibited administration
//! (OSC-030) cannot be reached through generic shell execution or the
//! structured-command policy gate. Every prohibited fixture must be BLACK-
//! blocked *before* any tool call, provider acquisition, or resource lease.
//!
//! Categories exercised (OSC-030 acceptance criterion 1 + Task 0.2 validation):
//! partitioning, formatting, GRUB/bootloader, kernel, users, raw firewall,
//! firmware flash, fan control, PKI, SELinux/AppArmor, systemd-unit creation,
//! and privilege bypass.
//!
//! Invariant guarded: structured/benign commands are NOT blocked merely because
//! a system operation is dangerous — a curated allow-list of reversible or
//! read-only commands must remain admissible.
//!
//! This test performs NO OS mutation, launches NO process, and reads NO files.
//! It is gated behind `os-control-test` so it runs under the spec-mandated
//! `--no-default-features --features os-control-test` invocation.
#![cfg(feature = "os-control-test")]

use kria_core::safety::black_scope::{self, ProhibitedCapability};
use kria_core::safety::policy_gate::{CapabilityPolicyGate, PolicyGate};
use kria_core::safety::{PolicyEngine, RiskLevel};
use kria_core::tools::shell::{evaluate_raw_shell_admission, RawShellAdmission};
use kria_core::tools::TriggerProvenance;

/// One prohibited fixture: a category label, the raw command, its expected
/// prohibited capability, and the structured `(binary, args)` decomposition.
struct ProhibitedFixture {
    label: &'static str,
    command: &'static str,
    expected: ProhibitedCapability,
    binary: &'static str,
    args: &'static [&'static str],
}

fn fixtures() -> Vec<ProhibitedFixture> {
    use ProhibitedCapability::*;
    vec![
        ProhibitedFixture {
            label: "partitioning",
            command: "fdisk /dev/sda",
            expected: Partitioning,
            binary: "fdisk",
            args: &["/dev/sda"],
        },
        ProhibitedFixture {
            label: "partitioning/parted",
            command: "parted /dev/sda mklabel gpt",
            expected: Partitioning,
            binary: "parted",
            args: &["/dev/sda", "mklabel", "gpt"],
        },
        ProhibitedFixture {
            label: "formatting",
            command: "mkfs.ext4 /dev/sdb1",
            expected: Formatting,
            binary: "mkfs.ext4",
            args: &["/dev/sdb1"],
        },
        ProhibitedFixture {
            label: "filesystem-resizing",
            command: "resize2fs /dev/sda1 20G",
            expected: FilesystemResizing,
            binary: "resize2fs",
            args: &["/dev/sda1", "20G"],
        },
        ProhibitedFixture {
            label: "secure-erase",
            command: "shred -n 3 /dev/sdb",
            expected: SecureErase,
            binary: "shred",
            args: &["-n", "3", "/dev/sdb"],
        },
        ProhibitedFixture {
            label: "full-disk-encryption",
            command: "cryptsetup luksFormat /dev/sdb1",
            expected: FullDiskEncryptionProvisioning,
            binary: "cryptsetup",
            args: &["luksFormat", "/dev/sdb1"],
        },
        // ── Storage-destructive administration (Task 3.2, OSC-012.6, OSC-030) ──
        // These are the exact destructive disk-administration categories the
        // storage lifecycle (`list_storage_devices`/`mount_device`/
        // `unmount_device`/`eject_device`/`get_storage_health`) explicitly
        // never implements. UDisks2's own `format-mkfs`-style operations still
        // route through the raw-shell/generic-command boundary (never through
        // the typed storage tools), so this fixture proves that boundary is
        // covered for the storage-adjacent forms too.
        ProhibitedFixture {
            label: "storage/format-via-mkfs",
            command: "mkfs.ext4 -F /dev/sdb1",
            expected: Formatting,
            binary: "mkfs.ext4",
            args: &["-F", "/dev/sdb1"],
        },
        ProhibitedFixture {
            label: "storage/partition-via-parted",
            command: "parted /dev/sdb mklabel gpt",
            expected: Partitioning,
            binary: "parted",
            args: &["/dev/sdb", "mklabel", "gpt"],
        },
        ProhibitedFixture {
            label: "storage/resize-via-resize2fs",
            command: "resize2fs /dev/sdb1 100G",
            expected: FilesystemResizing,
            binary: "resize2fs",
            args: &["/dev/sdb1", "100G"],
        },
        ProhibitedFixture {
            label: "storage/secure-erase-via-wipefs",
            command: "wipefs -a /dev/sdb1",
            expected: SecureErase,
            binary: "wipefs",
            args: &["-a", "/dev/sdb1"],
        },
        ProhibitedFixture {
            label: "storage/encryption-provisioning-via-cryptsetup",
            command: "cryptsetup luksFormat /dev/sdb2",
            expected: FullDiskEncryptionProvisioning,
            binary: "cryptsetup",
            args: &["luksFormat", "/dev/sdb2"],
        },
        ProhibitedFixture {
            label: "grub/bootloader",
            command: "grub-install /dev/sda",
            expected: BootloaderOrSecureBoot,
            binary: "grub-install",
            args: &["/dev/sda"],
        },
        ProhibitedFixture {
            label: "secure-boot",
            command: "mokutil --disable-validation",
            expected: BootloaderOrSecureBoot,
            binary: "mokutil",
            args: &["--disable-validation"],
        },
        ProhibitedFixture {
            label: "kernel/module",
            command: "modprobe kvm_intel",
            expected: KernelManagement,
            binary: "modprobe",
            args: &["kvm_intel"],
        },
        ProhibitedFixture {
            label: "kernel/tuning",
            command: "sysctl -w kernel.randomize_va_space=0",
            expected: KernelManagement,
            binary: "sysctl",
            args: &["-w", "kernel.randomize_va_space=0"],
        },
        ProhibitedFixture {
            label: "kernel/install",
            command: "apt install linux-image-generic",
            expected: KernelManagement,
            binary: "apt",
            args: &["install", "linux-image-generic"],
        },
        ProhibitedFixture {
            label: "users",
            command: "useradd -m intruder",
            expected: UserGroupPasswordSudoAdministration,
            binary: "useradd",
            args: &["-m", "intruder"],
        },
        ProhibitedFixture {
            label: "users/passwd",
            command: "passwd root",
            expected: UserGroupPasswordSudoAdministration,
            binary: "passwd",
            args: &["root"],
        },
        ProhibitedFixture {
            label: "selinux",
            command: "setenforce 0",
            expected: SecurityPolicyEditing,
            binary: "setenforce",
            args: &["0"],
        },
        ProhibitedFixture {
            label: "apparmor",
            command: "apparmor_parser -r /etc/apparmor.d/usr.bin.foo",
            expected: SecurityPolicyEditing,
            binary: "apparmor_parser",
            args: &["-r", "/etc/apparmor.d/usr.bin.foo"],
        },
        ProhibitedFixture {
            label: "pki",
            command: "update-ca-certificates --fresh",
            expected: PkiAdministration,
            binary: "update-ca-certificates",
            args: &["--fresh"],
        },
        ProhibitedFixture {
            label: "raw-firewall/iptables",
            command: "iptables -A INPUT -p tcp --dport 22 -j DROP",
            expected: RawFirewallRules,
            binary: "iptables",
            args: &["-A", "INPUT", "-p", "tcp", "--dport", "22", "-j", "DROP"],
        },
        ProhibitedFixture {
            label: "raw-firewall/ufw-rule",
            command: "ufw allow 22/tcp",
            expected: RawFirewallRules,
            binary: "ufw",
            args: &["allow", "22/tcp"],
        },
        ProhibitedFixture {
            label: "firmware-flash",
            command: "fwupdmgr update",
            expected: VendorFirmwareFlashing,
            binary: "fwupdmgr",
            args: &["update"],
        },
        ProhibitedFixture {
            label: "fan-control",
            command: "fancontrol /etc/fancontrol",
            expected: FanOrEmbeddedControllerWrites,
            binary: "fancontrol",
            args: &["/etc/fancontrol"],
        },
        ProhibitedFixture {
            label: "overclocking",
            command: "cpupower frequency-set -g performance",
            expected: Overclocking,
            binary: "cpupower",
            args: &["frequency-set", "-g", "performance"],
        },
        ProhibitedFixture {
            label: "systemd-unit-creation/run",
            command: "systemd-run --unit=backdoor sleep 3600",
            expected: ArbitrarySystemdUnitCreation,
            binary: "systemd-run",
            args: &["--unit=backdoor", "sleep", "3600"],
        },
        ProhibitedFixture {
            label: "systemd-unit-creation/edit",
            command: "systemctl edit sshd",
            expected: ArbitrarySystemdUnitCreation,
            binary: "systemctl",
            args: &["edit", "sshd"],
        },
        ProhibitedFixture {
            label: "privilege-bypass/sudo-shell",
            command: "sudo -i",
            expected: PrivilegeBypass,
            binary: "sudo",
            args: &["-i"],
        },
        ProhibitedFixture {
            label: "privilege-bypass/setuid",
            command: "chmod u+s /bin/bash",
            expected: PrivilegeBypass,
            binary: "chmod",
            args: &["u+s", "/bin/bash"],
        },
    ]
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

/// The classifier attributes each fixture to its exact prohibited capability.
#[test]
fn classifier_attributes_every_category() {
    for f in fixtures() {
        assert_eq!(
            black_scope::classify_command(f.command),
            Some(f.expected),
            "command form `{}` ({}) misclassified",
            f.command,
            f.label
        );
        assert_eq!(
            black_scope::classify_structured(f.binary, &owned(f.args)),
            Some(f.expected),
            "structured form `{} {:?}` ({}) misclassified",
            f.binary,
            f.args,
            f.label
        );
    }
}

/// PolicyEngine (prompt/loop-engine admission) blocks every prohibited fixture
/// as BLACK before any tool call.
#[test]
fn policy_engine_blocks_every_prohibited_fixture() {
    let policy = PolicyEngine::new();
    for f in fixtures() {
        let decision =
            policy.evaluate("execute_bash", &serde_json::json!({ "command": f.command }));
        assert!(
            decision.blocked,
            "PolicyEngine failed to block `{}` ({})",
            f.command, f.label
        );
        assert_eq!(
            decision.risk_level,
            RiskLevel::Black,
            "PolicyEngine risk for `{}` ({}) should be BLACK",
            f.command,
            f.label
        );
        assert!(
            !decision.requires_approval,
            "prohibited `{}` ({}) must be denied, never routed to approval",
            f.command, f.label
        );
    }
}

/// CapabilityPolicyGate (structured subprocess admission) blocks every
/// prohibited fixture — both as a structured `(binary, args)` pair and when
/// smuggled through `sh -c "…"`.
#[test]
fn capability_policy_gate_blocks_every_prohibited_fixture() {
    let gate = CapabilityPolicyGate::new();
    for f in fixtures() {
        let structured = gate.evaluate(f.binary, &owned(f.args));
        assert!(
            structured.is_blocked(),
            "gate failed to block structured `{} {:?}` ({})",
            f.binary,
            f.args,
            f.label
        );

        let via_shell = gate.evaluate("sh", &owned(&["-c", f.command]));
        assert!(
            via_shell.is_blocked(),
            "gate failed to block `sh -c \"{}\"` ({})",
            f.command,
            f.label
        );
    }
}

/// The raw-shell tool boundary also refuses every prohibited fixture, even for
/// a direct attended user in Expert Mode (defense in depth).
#[test]
fn raw_shell_boundary_refuses_every_prohibited_fixture() {
    for f in fixtures() {
        let admission = evaluate_raw_shell_admission(f.command, TriggerProvenance::User, true);
        match admission {
            RawShellAdmission::Refused { code, .. } => assert_eq!(
                code, "prohibited_scope",
                "raw-shell refusal code for `{}` ({}) unexpected",
                f.command, f.label
            ),
            RawShellAdmission::Allowed => {
                panic!("raw shell allowed prohibited `{}` ({})", f.command, f.label)
            }
        }
    }
}

/// The containment invariant: structured/benign commands are NOT blocked.
/// These reversible or read-only commands remain admissible so structured,
/// approved actions are never blocked merely because a system operation is
/// dangerous.
#[test]
fn benign_and_reversible_commands_remain_admissible() {
    let policy = PolicyEngine::new();
    let allowed = [
        "systemctl status nginx",
        "systemctl restart nginx",
        "ls -la /var/log",
        "cat /etc/os-release",
        "ip addr show",
        "ufw status",
        "systemctl list-units",
    ];
    for cmd in allowed {
        assert!(
            black_scope::classify_command(cmd).is_none(),
            "`{cmd}` must not be flagged as prohibited BLACK scope"
        );
        let decision = policy.evaluate("execute_bash", &serde_json::json!({ "command": cmd }));
        assert_ne!(
            decision.risk_level,
            RiskLevel::Black,
            "`{cmd}` must not be BLACK-blocked by the prohibited-scope layer"
        );
    }
}
