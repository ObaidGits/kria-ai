//! Explicit BLACK-scope prohibited-administration classifier.
//!
//! # Purpose (linux-os-control-production Task 0.2)
//!
//! Requirement OSC-030 freezes an *explicit, closed* set of operating-system
//! administration that the normal assistant SHALL NOT expose, route to, or
//! reconstruct through any generic primitive. This module is the single
//! code-owned enumeration of those prohibited capabilities plus a
//! command-string classifier that recognizes attempts to reach them through
//! **generic shell execution** (`execute_bash` / `execute_powershell`) or the
//! structured-command policy gate.
//!
//! ## What this module is (and is not)
//!
//! - It is the authoritative list of prohibited capability IDs (BLACK scope).
//! - It only inspects *generic* command strings and structured `(binary, args)`
//!   pairs. It is deliberately NOT applied to typed OS-control capabilities.
//!   A structured, approved action is never blocked merely because its
//!   underlying system operation is dangerous — that governance lives in the
//!   typed capability/grant path (OSC-002, OSC-004). Generic shell is a
//!   separately-restricted surface, and this classifier hardens *only* that
//!   surface plus the raw command policy gate.
//!
//! ## Coverage (OSC-030 acceptance criterion 1)
//!
//! Partitioning, formatting, filesystem resizing, secure erase, full-disk
//! encryption provisioning, bootloader/Secure Boot mutation, kernel
//! installation/selection/tuning/modules, full user/group/password/sudo
//! administration, SELinux/AppArmor policy editing, CA/PKI administration, raw
//! firewall rules, vendor firmware flashing, fan/embedded-controller writes,
//! overclocking, arbitrary systemd-unit creation — plus privilege-bypass
//! escalation to an arbitrary root shell (OSC-004 least-privilege).
//!
//! The list is closed: adding a new prohibited category requires editing this
//! enum and its detection table, which keeps the boundary reviewable in one
//! place.

/// A single prohibited administration capability (BLACK scope).
///
/// Each variant maps to a stable capability ID (`black.*`) and a concise
/// user-facing boundary explanation. These IDs never appear in the normal
/// capability manifest; they exist solely to *deny* and to explain the
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProhibitedCapability {
    /// Disk partitioning / partition-table editing.
    Partitioning,
    /// Creating filesystems (formatting) on a device.
    Formatting,
    /// Growing/shrinking filesystems or logical volumes.
    FilesystemResizing,
    /// Secure erase / device sanitize / disk wiping.
    SecureErase,
    /// Provisioning full-disk encryption (LUKS/VeraCrypt) containers.
    FullDiskEncryptionProvisioning,
    /// Bootloader or Secure Boot mutation (GRUB, EFI, MOK, shim).
    BootloaderOrSecureBoot,
    /// Kernel installation, selection, tuning, or module load/unload.
    KernelManagement,
    /// Full user / group / password / sudoers administration.
    UserGroupPasswordSudoAdministration,
    /// SELinux / AppArmor mandatory-access-control policy editing.
    SecurityPolicyEditing,
    /// Certificate-authority / PKI trust-store administration.
    PkiAdministration,
    /// Raw firewall rule programming (iptables/nftables/raw ufw rules).
    RawFirewallRules,
    /// Vendor firmware flashing / update.
    VendorFirmwareFlashing,
    /// Fan / embedded-controller register writes.
    FanOrEmbeddedControllerWrites,
    /// CPU/GPU overclocking / undervolting / frequency forcing.
    Overclocking,
    /// Creating arbitrary systemd units (unit files, transient units, edit/link).
    ArbitrarySystemdUnitCreation,
    /// Escalating to an arbitrary root shell or setuid bypass.
    PrivilegeBypass,
}

impl ProhibitedCapability {
    /// Stable capability ID string (used in reasons, audit, and tests).
    pub fn id(self) -> &'static str {
        match self {
            Self::Partitioning => "black.partitioning",
            Self::Formatting => "black.formatting",
            Self::FilesystemResizing => "black.filesystem_resizing",
            Self::SecureErase => "black.secure_erase",
            Self::FullDiskEncryptionProvisioning => "black.full_disk_encryption",
            Self::BootloaderOrSecureBoot => "black.bootloader_secure_boot",
            Self::KernelManagement => "black.kernel_management",
            Self::UserGroupPasswordSudoAdministration => "black.user_group_sudo_admin",
            Self::SecurityPolicyEditing => "black.security_policy_editing",
            Self::PkiAdministration => "black.pki_administration",
            Self::RawFirewallRules => "black.raw_firewall_rules",
            Self::VendorFirmwareFlashing => "black.firmware_flashing",
            Self::FanOrEmbeddedControllerWrites => "black.fan_ec_writes",
            Self::Overclocking => "black.overclocking",
            Self::ArbitrarySystemdUnitCreation => "black.systemd_unit_creation",
            Self::PrivilegeBypass => "black.privilege_bypass",
        }
    }

    /// Concise, user-facing boundary explanation. This is what a normal prompt
    /// receives instead of a tool call: a refusal plus a pointer to a trusted
    /// specialist utility (OSC-030 acceptance criteria 2 and 3).
    pub fn boundary_explanation(self) -> String {
        let what = match self {
            Self::Partitioning => "disk partitioning",
            Self::Formatting => "formatting a filesystem",
            Self::FilesystemResizing => "resizing a filesystem or logical volume",
            Self::SecureErase => "securely erasing a storage device",
            Self::FullDiskEncryptionProvisioning => "provisioning full-disk encryption",
            Self::BootloaderOrSecureBoot => "modifying the bootloader or Secure Boot",
            Self::KernelManagement => "installing, selecting, tuning, or loading kernel modules",
            Self::UserGroupPasswordSudoAdministration => {
                "administering users, groups, passwords, or sudo access"
            }
            Self::SecurityPolicyEditing => "editing SELinux or AppArmor security policy",
            Self::PkiAdministration => "administering certificate-authority / PKI trust",
            Self::RawFirewallRules => "programming raw firewall rules",
            Self::VendorFirmwareFlashing => "flashing vendor firmware",
            Self::FanOrEmbeddedControllerWrites => {
                "writing to fan or embedded-controller registers"
            }
            Self::Overclocking => "overclocking or altering CPU/GPU power limits",
            Self::ArbitrarySystemdUnitCreation => "creating arbitrary systemd units",
            Self::PrivilegeBypass => "escalating to an unrestricted root shell",
        };
        format!(
            "{what} is outside KRIA's supported scope and is never performed automatically. \
             I can share read-only diagnostics or point you to a trusted specialist utility, \
             but I will not carry out this operation myself."
        )
    }
}

/// Classify a raw command string for prohibited BLACK-scope administration.
///
/// Returns `Some(capability)` when the command targets a prohibited operation.
/// The classifier tokenizes the (lowercased) command, tolerates a leading
/// `sudo`/`doas`/`pkexec` privilege prefix, and matches on the effective
/// program name plus argument tokens. It intentionally errs toward matching
/// well-known administration binaries — false positives here degrade to a
/// refusal on *generic shell only*, never on structured capabilities.
pub fn classify_command(command: &str) -> Option<ProhibitedCapability> {
    let lower = command.to_ascii_lowercase();
    let raw_tokens: Vec<&str> = lower.split_whitespace().collect();
    if raw_tokens.is_empty() {
        return None;
    }

    // Privilege bypass is detected on the *original* token stream (before we
    // strip the privilege prefix) so that `sudo bash`, `sudo -i`, `su -`, and
    // `pkexec sh` are recognized as escalation to an arbitrary root shell.
    if let Some(cap) = detect_privilege_bypass(&raw_tokens) {
        return Some(cap);
    }

    // Strip a single leading privilege prefix (and its immediate `-n`/`-u user`
    // style options are left in place as tokens; program detection scans for
    // the first token that is not a prefix/flag).
    let effective = strip_privilege_prefix(&raw_tokens);
    if effective.is_empty() {
        return None;
    }
    let program = program_name(effective[0]);
    let args = &effective[1..];

    // Path-based detection first: writing to a protected admin path via a
    // generic writer (tee/cp/mv/dd/ln/install/sed -i) is prohibited by the
    // owning category regardless of the tool used.
    if let Some(cap) = detect_protected_path_write(program, args, &lower) {
        return Some(cap);
    }

    classify_program(program, args)
}

/// Classify a structured `(binary, args)` pair (policy-gate path).
///
/// `binary` may be an absolute path; only its file name is considered.
pub fn classify_structured(binary: &str, args: &[String]) -> Option<ProhibitedCapability> {
    let mut tokens: Vec<&str> = Vec::with_capacity(args.len() + 1);
    tokens.push(binary);
    for a in args {
        tokens.push(a.as_str());
    }
    let lowered: Vec<String> = tokens.iter().map(|t| t.to_ascii_lowercase()).collect();
    let refs: Vec<&str> = lowered.iter().map(|s| s.as_str()).collect();

    if let Some(cap) = detect_privilege_bypass(&refs) {
        return Some(cap);
    }
    let effective = strip_privilege_prefix(&refs);
    if effective.is_empty() {
        return None;
    }
    let program = program_name(effective[0]);
    let joined = refs.join(" ");
    if let Some(cap) = detect_protected_path_write(program, &effective[1..], &joined) {
        return Some(cap);
    }
    classify_program(program, &effective[1..])
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Reduce an absolute or relative path to a bare program name.
fn program_name(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

/// Strip a single leading privilege prefix (`sudo`, `doas`, `pkexec`) and any
/// of its own leading options, returning the effective command tokens.
fn strip_privilege_prefix<'a>(tokens: &[&'a str]) -> Vec<&'a str> {
    let mut idx = 0;
    if let Some(first) = tokens.first() {
        let p = program_name(first);
        if matches!(p, "sudo" | "doas" | "pkexec") {
            idx = 1;
            // Skip sudo/doas options and their arguments conservatively.
            while idx < tokens.len() {
                let t = tokens[idx];
                if t.starts_with('-') {
                    // `-u user`, `-g group` consume the next token.
                    if matches!(t, "-u" | "-g" | "--user" | "--group") {
                        idx += 2;
                    } else {
                        idx += 1;
                    }
                } else {
                    break;
                }
            }
        }
    }
    tokens[idx.min(tokens.len())..].to_vec()
}

/// Detect escalation to an arbitrary root shell or a setuid/capability bypass.
fn detect_privilege_bypass(tokens: &[&str]) -> Option<ProhibitedCapability> {
    let first = program_name(tokens.first().copied().unwrap_or(""));

    // Direct privilege-bypass binaries.
    if matches!(first, "pkexec" | "doas") {
        return Some(ProhibitedCapability::PrivilegeBypass);
    }
    if first == "su" {
        return Some(ProhibitedCapability::PrivilegeBypass);
    }
    if matches!(first, "capsh" | "setcap") {
        return Some(ProhibitedCapability::PrivilegeBypass);
    }

    // `chmod` granting setuid/setgid bits.
    if first == "chmod" {
        for t in &tokens[1..] {
            if t.contains("+s") || t.contains("u+s") || t.contains("g+s") {
                return Some(ProhibitedCapability::PrivilegeBypass);
            }
            // Numeric mode with a leading setuid/setgid/sticky digit (>= 4 digits).
            if t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
                let lead = t.as_bytes()[0];
                if matches!(lead, b'2' | b'4' | b'6' | b'7') {
                    return Some(ProhibitedCapability::PrivilegeBypass);
                }
            }
        }
    }

    // `sudo`/`doas` invoking an interactive/arbitrary shell.
    if matches!(first, "sudo" | "doas") {
        // Options that spawn a shell.
        for t in &tokens[1..] {
            if matches!(t, &"-i" | &"-s" | &"--login" | &"--shell") {
                return Some(ProhibitedCapability::PrivilegeBypass);
            }
            if !t.starts_with('-') {
                let prog = program_name(t);
                if matches!(
                    prog,
                    "su" | "bash" | "sh" | "zsh" | "fish" | "csh" | "tcsh" | "dash"
                ) {
                    return Some(ProhibitedCapability::PrivilegeBypass);
                }
                // First non-flag token decides; stop scanning further args.
                break;
            }
        }
    }

    None
}

/// Detect prohibited writes to protected administration paths performed via a
/// generic file writer.
fn detect_protected_path_write(
    program: &str,
    args: &[&str],
    full_lower: &str,
) -> Option<ProhibitedCapability> {
    // Only treat as a write when a generic writer / redirect is involved.
    // A read (`cat /etc/passwd`) is not a write. Require an actual writer
    // program or an output redirect (`>`), which is caught here rather than by
    // reading tools.
    let is_generic_writer = matches!(
        program,
        "tee" | "cp" | "mv" | "dd" | "ln" | "install" | "truncate"
    ) || (program == "sed" && args.iter().any(|a| a.starts_with("-i")))
        || full_lower.contains('>');

    if !is_generic_writer {
        return None;
    }

    if full_lower.contains("/etc/systemd/system/") || full_lower.contains("/.config/systemd/") {
        return Some(ProhibitedCapability::ArbitrarySystemdUnitCreation);
    }
    if full_lower.contains("/etc/sudoers")
        || full_lower.contains("/etc/passwd")
        || full_lower.contains("/etc/shadow")
        || full_lower.contains("/etc/group")
    {
        return Some(ProhibitedCapability::UserGroupPasswordSudoAdministration);
    }
    if full_lower.contains("/usr/local/share/ca-certificates")
        || full_lower.contains("/etc/pki/")
        || full_lower.contains("/etc/ca-certificates")
    {
        return Some(ProhibitedCapability::PkiAdministration);
    }
    if full_lower.contains("/sys/class/hwmon") || full_lower.contains("/etc/fancontrol") {
        return Some(ProhibitedCapability::FanOrEmbeddedControllerWrites);
    }
    if full_lower.contains("/sys/devices/system/cpu/")
        && (full_lower.contains("scaling_setspeed") || full_lower.contains("cpufreq"))
    {
        return Some(ProhibitedCapability::Overclocking);
    }
    None
}

/// Core program-name + argument classifier.
fn classify_program(program: &str, args: &[&str]) -> Option<ProhibitedCapability> {
    let first_arg = args.first().copied().unwrap_or("");

    // ── Partitioning ──────────────────────────────────────────────────────
    if matches!(
        program,
        "fdisk"
            | "sfdisk"
            | "cfdisk"
            | "gdisk"
            | "sgdisk"
            | "cgdisk"
            | "partprobe"
            | "partx"
            | "kpartx"
    ) {
        return Some(ProhibitedCapability::Partitioning);
    }
    // `parted` is a partition editor; treat its mutating verbs as partitioning
    // and its resize verb as filesystem resizing. A bare/print invocation is
    // still a partition tool and stays out of scope.
    if program == "parted" {
        if args.iter().any(|a| a.contains("resizepart")) {
            return Some(ProhibitedCapability::FilesystemResizing);
        }
        return Some(ProhibitedCapability::Partitioning);
    }

    // ── Formatting ──────────────────────────────────────────────────────────
    if program == "mkfs"
        || program.starts_with("mkfs.")
        || matches!(
            program,
            "mke2fs" | "mkswap" | "mkntfs" | "mkdosfs" | "newfs" | "mkfs.ext4" | "mkfs.xfs"
        )
    {
        return Some(ProhibitedCapability::Formatting);
    }
    // Windows-style `format X:`
    if program == "format" {
        return Some(ProhibitedCapability::Formatting);
    }

    // ── Filesystem resizing ──────────────────────────────────────────────────
    if matches!(
        program,
        "resize2fs"
            | "xfs_growfs"
            | "lvresize"
            | "lvextend"
            | "lvreduce"
            | "pvresize"
            | "vgextend"
            | "ntfsresize"
    ) {
        return Some(ProhibitedCapability::FilesystemResizing);
    }
    if program == "btrfs" && args.iter().any(|a| a.contains("resize")) {
        return Some(ProhibitedCapability::FilesystemResizing);
    }

    // ── Secure erase ─────────────────────────────────────────────────────────
    if matches!(program, "shred" | "wipefs" | "blkdiscard" | "scrub" | "srm") {
        return Some(ProhibitedCapability::SecureErase);
    }
    if program == "nvme" && matches!(first_arg, "format" | "sanitize") {
        return Some(ProhibitedCapability::SecureErase);
    }
    if program == "hdparm" && args.iter().any(|a| a.contains("security-erase")) {
        return Some(ProhibitedCapability::SecureErase);
    }

    // ── Full-disk encryption provisioning ────────────────────────────────────
    if matches!(
        program,
        "cryptsetup" | "veracrypt" | "luksformat" | "zulucrypt-cli"
    ) {
        return Some(ProhibitedCapability::FullDiskEncryptionProvisioning);
    }

    // ── Bootloader / Secure Boot ─────────────────────────────────────────────
    if matches!(
        program,
        "grub-install"
            | "grub2-install"
            | "grub-mkconfig"
            | "grub2-mkconfig"
            | "update-grub"
            | "update-grub2"
            | "grubby"
            | "efibootmgr"
            | "mokutil"
            | "sbctl"
            | "sbsign"
    ) {
        return Some(ProhibitedCapability::BootloaderOrSecureBoot);
    }
    if program == "bootctl" && matches!(first_arg, "install" | "update" | "remove") {
        return Some(ProhibitedCapability::BootloaderOrSecureBoot);
    }

    // ── Kernel management ────────────────────────────────────────────────────
    if matches!(
        program,
        "modprobe"
            | "insmod"
            | "rmmod"
            | "depmod"
            | "dkms"
            | "update-initramfs"
            | "mkinitcpio"
            | "dracut"
    ) {
        return Some(ProhibitedCapability::KernelManagement);
    }
    // Kernel tuning via sysctl writes.
    if program == "sysctl"
        && (args.iter().any(|a| *a == "-w") || args.iter().any(|a| a.contains('=')))
    {
        return Some(ProhibitedCapability::KernelManagement);
    }
    // Installing kernel packages.
    if matches!(
        program,
        "apt" | "apt-get" | "dnf" | "yum" | "zypper" | "pacman"
    ) && args.iter().any(|a| {
        a.starts_with("linux-image")
            || a.starts_with("linux-headers")
            || a.starts_with("linux-modules")
            || a.starts_with("linux-generic")
            || a.starts_with("kernel-")
            || *a == "linux"
    }) {
        return Some(ProhibitedCapability::KernelManagement);
    }

    // ── User / group / password / sudo administration ────────────────────────
    if matches!(
        program,
        "useradd"
            | "userdel"
            | "usermod"
            | "adduser"
            | "deluser"
            | "groupadd"
            | "groupdel"
            | "groupmod"
            | "addgroup"
            | "delgroup"
            | "passwd"
            | "chpasswd"
            | "gpasswd"
            | "newusers"
            | "vipw"
            | "vigr"
            | "visudo"
            | "chage"
    ) {
        return Some(ProhibitedCapability::UserGroupPasswordSudoAdministration);
    }

    // ── SELinux / AppArmor policy editing ────────────────────────────────────
    if matches!(
        program,
        "setenforce"
            | "semanage"
            | "setsebool"
            | "semodule"
            | "checkmodule"
            | "audit2allow"
            | "apparmor_parser"
    ) || program.starts_with("aa-")
    {
        return Some(ProhibitedCapability::SecurityPolicyEditing);
    }

    // ── CA / PKI administration ──────────────────────────────────────────────
    if matches!(
        program,
        "update-ca-certificates" | "update-ca-trust" | "certutil" | "trust"
    ) {
        return Some(ProhibitedCapability::PkiAdministration);
    }
    if program == "openssl" && matches!(first_arg, "ca" | "req" | "x509") {
        return Some(ProhibitedCapability::PkiAdministration);
    }

    // ── Raw firewall rules ───────────────────────────────────────────────────
    if matches!(
        program,
        "iptables"
            | "ip6tables"
            | "arptables"
            | "ebtables"
            | "nft"
            | "nftables"
            | "iptables-restore"
    ) {
        return Some(ProhibitedCapability::RawFirewallRules);
    }
    // `ufw` high-level enable/disable/status is allowed via the structured
    // firewall capability; raw rule verbs are prohibited.
    if program == "ufw"
        && matches!(
            first_arg,
            "allow" | "deny" | "reject" | "limit" | "insert" | "route" | "prepend"
        )
    {
        return Some(ProhibitedCapability::RawFirewallRules);
    }
    if program == "firewall-cmd"
        && args.iter().any(|a| {
            a.starts_with("--direct")
                || a.starts_with("--add-rich-rule")
                || a.starts_with("--add-rule")
                || a.starts_with("--remove-rule")
        })
    {
        return Some(ProhibitedCapability::RawFirewallRules);
    }

    // ── Vendor firmware flashing ─────────────────────────────────────────────
    if matches!(
        program,
        "flashrom" | "fwupdtool" | "dfu-util" | "mstflint" | "fwupdmgr"
    ) {
        return Some(ProhibitedCapability::VendorFirmwareFlashing);
    }
    if program == "nvme"
        && args
            .iter()
            .any(|a| a.starts_with("fw-") || a.contains("firmware"))
    {
        return Some(ProhibitedCapability::VendorFirmwareFlashing);
    }

    // ── Fan / embedded-controller writes ─────────────────────────────────────
    if matches!(program, "fancontrol" | "pwmconfig" | "ectool" | "nbfc") {
        return Some(ProhibitedCapability::FanOrEmbeddedControllerWrites);
    }

    // ── Overclocking / power-limit forcing ───────────────────────────────────
    if matches!(program, "ryzenadj" | "wrmsr") {
        return Some(ProhibitedCapability::Overclocking);
    }
    if program == "cpupower" && args.iter().any(|a| a.contains("frequency-set")) {
        return Some(ProhibitedCapability::Overclocking);
    }
    if program == "nvidia-settings" && args.iter().any(|a| *a == "-a" || *a == "--assign") {
        return Some(ProhibitedCapability::Overclocking);
    }
    if program == "rocm-smi"
        && args.iter().any(|a| {
            a.contains("setsclk") || a.contains("setmclk") || a.contains("setpoweroverdrive")
        })
    {
        return Some(ProhibitedCapability::Overclocking);
    }

    // ── Arbitrary systemd-unit creation ──────────────────────────────────────
    if program == "systemd-run" {
        return Some(ProhibitedCapability::ArbitrarySystemdUnitCreation);
    }
    if program == "systemctl" && matches!(first_arg, "edit" | "link") {
        return Some(ProhibitedCapability::ArbitrarySystemdUnitCreation);
    }
    // A unit-file token pointed at a systemd unit directory.
    if args.iter().any(|a| {
        (a.ends_with(".service")
            || a.ends_with(".timer")
            || a.ends_with(".socket")
            || a.ends_with(".mount")
            || a.ends_with(".path")
            || a.ends_with(".target"))
            && (a.contains("/etc/systemd/") || a.contains("/.config/systemd/"))
    }) {
        return Some(ProhibitedCapability::ArbitrarySystemdUnitCreation);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(command: &str) -> ProhibitedCapability {
        classify_command(command)
            .unwrap_or_else(|| panic!("expected `{command}` to be prohibited, but it was allowed"))
    }

    #[test]
    fn ids_are_unique_and_black_prefixed() {
        use std::collections::HashSet;
        let all = [
            ProhibitedCapability::Partitioning,
            ProhibitedCapability::Formatting,
            ProhibitedCapability::FilesystemResizing,
            ProhibitedCapability::SecureErase,
            ProhibitedCapability::FullDiskEncryptionProvisioning,
            ProhibitedCapability::BootloaderOrSecureBoot,
            ProhibitedCapability::KernelManagement,
            ProhibitedCapability::UserGroupPasswordSudoAdministration,
            ProhibitedCapability::SecurityPolicyEditing,
            ProhibitedCapability::PkiAdministration,
            ProhibitedCapability::RawFirewallRules,
            ProhibitedCapability::VendorFirmwareFlashing,
            ProhibitedCapability::FanOrEmbeddedControllerWrites,
            ProhibitedCapability::Overclocking,
            ProhibitedCapability::ArbitrarySystemdUnitCreation,
            ProhibitedCapability::PrivilegeBypass,
        ];
        let ids: HashSet<&str> = all.iter().map(|c| c.id()).collect();
        assert_eq!(ids.len(), all.len(), "capability IDs must be unique");
        assert!(all.iter().all(|c| c.id().starts_with("black.")));
        assert!(all.iter().all(|c| !c.boundary_explanation().is_empty()));
    }

    #[test]
    fn partitioning_is_prohibited() {
        assert_eq!(cap("fdisk /dev/sda"), ProhibitedCapability::Partitioning);
        assert_eq!(
            cap("parted /dev/sda mklabel gpt"),
            ProhibitedCapability::Partitioning
        );
        assert_eq!(
            cap("sgdisk -n 1:0:0 /dev/nvme0n1"),
            ProhibitedCapability::Partitioning
        );
    }

    #[test]
    fn formatting_is_prohibited() {
        assert_eq!(cap("mkfs.ext4 /dev/sdb1"), ProhibitedCapability::Formatting);
        assert_eq!(
            cap("mkfs -t xfs /dev/sdb1"),
            ProhibitedCapability::Formatting
        );
        assert_eq!(cap("mkswap /dev/sdb2"), ProhibitedCapability::Formatting);
    }

    #[test]
    fn filesystem_resizing_is_prohibited() {
        assert_eq!(
            cap("resize2fs /dev/sda1"),
            ProhibitedCapability::FilesystemResizing
        );
        assert_eq!(
            cap("lvextend -L +10G /dev/vg/lv"),
            ProhibitedCapability::FilesystemResizing
        );
        assert_eq!(
            cap("parted /dev/sda resizepart 2 100%"),
            ProhibitedCapability::FilesystemResizing
        );
    }

    #[test]
    fn secure_erase_is_prohibited() {
        assert_eq!(
            cap("shred -n 3 /dev/sdb"),
            ProhibitedCapability::SecureErase
        );
        assert_eq!(cap("wipefs -a /dev/sdb"), ProhibitedCapability::SecureErase);
        assert_eq!(
            cap("nvme format /dev/nvme0n1"),
            ProhibitedCapability::SecureErase
        );
    }

    #[test]
    fn full_disk_encryption_is_prohibited() {
        assert_eq!(
            cap("cryptsetup luksFormat /dev/sdb1"),
            ProhibitedCapability::FullDiskEncryptionProvisioning
        );
    }

    #[test]
    fn bootloader_and_secure_boot_is_prohibited() {
        assert_eq!(
            cap("grub-install /dev/sda"),
            ProhibitedCapability::BootloaderOrSecureBoot
        );
        assert_eq!(
            cap("update-grub"),
            ProhibitedCapability::BootloaderOrSecureBoot
        );
        assert_eq!(
            cap("efibootmgr -o 0002,0001"),
            ProhibitedCapability::BootloaderOrSecureBoot
        );
        assert_eq!(
            cap("mokutil --disable-validation"),
            ProhibitedCapability::BootloaderOrSecureBoot
        );
    }

    #[test]
    fn kernel_management_is_prohibited() {
        assert_eq!(cap("modprobe kvm"), ProhibitedCapability::KernelManagement);
        assert_eq!(cap("rmmod nvidia"), ProhibitedCapability::KernelManagement);
        assert_eq!(
            cap("sysctl -w kernel.randomize_va_space=0"),
            ProhibitedCapability::KernelManagement
        );
        assert_eq!(
            cap("apt install linux-image-generic"),
            ProhibitedCapability::KernelManagement
        );
    }

    #[test]
    fn user_admin_is_prohibited() {
        assert_eq!(
            cap("useradd bob"),
            ProhibitedCapability::UserGroupPasswordSudoAdministration
        );
        assert_eq!(
            cap("passwd root"),
            ProhibitedCapability::UserGroupPasswordSudoAdministration
        );
        assert_eq!(
            cap("usermod -aG sudo bob"),
            ProhibitedCapability::UserGroupPasswordSudoAdministration
        );
        assert_eq!(
            cap("visudo"),
            ProhibitedCapability::UserGroupPasswordSudoAdministration
        );
    }

    #[test]
    fn security_policy_editing_is_prohibited() {
        assert_eq!(
            cap("setenforce 1"),
            ProhibitedCapability::SecurityPolicyEditing
        );
        assert_eq!(
            cap("semanage port -a -t http_port_t -p tcp 8085"),
            ProhibitedCapability::SecurityPolicyEditing
        );
        assert_eq!(
            cap("aa-disable /etc/apparmor.d/usr.bin.firefox"),
            ProhibitedCapability::SecurityPolicyEditing
        );
        assert_eq!(
            cap("apparmor_parser -r /etc/apparmor.d/foo"),
            ProhibitedCapability::SecurityPolicyEditing
        );
    }

    #[test]
    fn pki_administration_is_prohibited() {
        assert_eq!(
            cap("update-ca-certificates"),
            ProhibitedCapability::PkiAdministration
        );
        assert_eq!(
            cap("trust anchor rogue.crt"),
            ProhibitedCapability::PkiAdministration
        );
        assert_eq!(
            cap("openssl ca -in req.pem"),
            ProhibitedCapability::PkiAdministration
        );
    }

    #[test]
    fn raw_firewall_rules_are_prohibited() {
        assert_eq!(
            cap("iptables -A INPUT -p tcp --dport 22 -j DROP"),
            ProhibitedCapability::RawFirewallRules
        );
        assert_eq!(
            cap("nft add rule inet filter input drop"),
            ProhibitedCapability::RawFirewallRules
        );
        assert_eq!(
            cap("ufw allow 22/tcp"),
            ProhibitedCapability::RawFirewallRules
        );
    }

    #[test]
    fn firmware_flashing_is_prohibited() {
        assert_eq!(
            cap("fwupdmgr update"),
            ProhibitedCapability::VendorFirmwareFlashing
        );
        assert_eq!(
            cap("flashrom -w bios.rom"),
            ProhibitedCapability::VendorFirmwareFlashing
        );
    }

    #[test]
    fn fan_control_is_prohibited() {
        assert_eq!(
            cap("fancontrol"),
            ProhibitedCapability::FanOrEmbeddedControllerWrites
        );
        assert_eq!(
            cap("pwmconfig"),
            ProhibitedCapability::FanOrEmbeddedControllerWrites
        );
    }

    #[test]
    fn overclocking_is_prohibited() {
        assert_eq!(
            cap("cpupower frequency-set -g performance"),
            ProhibitedCapability::Overclocking
        );
        assert_eq!(
            cap("ryzenadj --tctl-temp=90"),
            ProhibitedCapability::Overclocking
        );
    }

    #[test]
    fn systemd_unit_creation_is_prohibited() {
        assert_eq!(
            cap("systemd-run --unit=evil sleep 60"),
            ProhibitedCapability::ArbitrarySystemdUnitCreation
        );
        assert_eq!(
            cap("systemctl edit sshd"),
            ProhibitedCapability::ArbitrarySystemdUnitCreation
        );
        assert_eq!(
            cap("tee /etc/systemd/system/evil.service"),
            ProhibitedCapability::ArbitrarySystemdUnitCreation
        );
    }

    #[test]
    fn privilege_bypass_is_prohibited() {
        assert_eq!(cap("sudo -i"), ProhibitedCapability::PrivilegeBypass);
        assert_eq!(cap("sudo bash"), ProhibitedCapability::PrivilegeBypass);
        assert_eq!(cap("sudo su -"), ProhibitedCapability::PrivilegeBypass);
        assert_eq!(cap("su - root"), ProhibitedCapability::PrivilegeBypass);
        assert_eq!(cap("pkexec bash"), ProhibitedCapability::PrivilegeBypass);
        assert_eq!(
            cap("chmod u+s /bin/bash"),
            ProhibitedCapability::PrivilegeBypass
        );
        assert_eq!(
            cap("chmod 4755 /usr/bin/foo"),
            ProhibitedCapability::PrivilegeBypass
        );
    }

    #[test]
    fn structured_form_matches_command_form() {
        assert_eq!(
            classify_structured(
                "iptables",
                &["-A".into(), "INPUT".into(), "-j".into(), "DROP".into()]
            ),
            Some(ProhibitedCapability::RawFirewallRules)
        );
        assert_eq!(
            classify_structured("/usr/sbin/useradd", &["bob".into()]),
            Some(ProhibitedCapability::UserGroupPasswordSudoAdministration)
        );
    }

    // ── Negative cases: normal / structured-adjacent commands stay allowed ──

    #[test]
    fn benign_and_reversible_commands_are_allowed() {
        for cmd in [
            "ls -la /var/log",
            "cat /etc/hosts",
            "systemctl status nginx",
            "systemctl restart nginx",
            "systemctl stop nginx",
            "sudo systemctl status nginx",
            "apt install nginx",
            "git commit -m \"add setcap notes to firewall doc\"",
            "ufw status",
            "ufw enable",
            "ufw disable",
            "ip addr show",
            "cp report.txt backup.txt",
            "echo hello",
            "systemctl list-units",
        ] {
            assert!(
                classify_command(cmd).is_none(),
                "`{cmd}` must NOT be classified as prohibited BLACK scope"
            );
        }
    }
}
