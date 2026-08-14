//! Polkit authorization and fixed native-operation seams used by the broker.
//!
//! linux-os-control-production **Task 1.5**, design §12
//! (OSC-001, OSC-004, OSC-030, OSC-033).
//!
//! The broker validates a request, authorizes it through **Polkit**, then
//! performs exactly one **fixed native operation** and queries resulting state.
//! Both seams are traits so deny-live tests inject fakes:
//!
//! * [`PolkitAuthorizer`] — the live implementation activates the broker's
//!   registered Polkit action; a denial has no broader fallback (design §12: "A
//!   denied Polkit request remains denied").
//! * [`NativeBrokerOperations`] — performs the fixed operation for the six
//!   variants only, returning a [`BrokerDispatchOutcome`] with bounded evidence
//!   and never raw output.
//!
//! The live implementations require a [`LiveHostAccessToken`] and trip the
//! deny-live sentinel, so no completion test can reach a live Polkit call or a
//! privileged child process.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};

use super::caller::PeerCredentials;
use super::protocol::{
    BoundedBrokerEvidence, BrokerBoundPath, BrokerDispatchOutcome, BrokerOperation,
    ChargeThresholdAdapterId, EvidenceField, ExistingLocalIdentity,
};
use crate::os_control::contract::{Digest, ProviderId, SafeField, SafeText};
use crate::os_control::receipt::UncertainEffectCause;

/// The Polkit authorization decision. A denial is terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolkitDecision {
    /// The authenticated caller is authorized for the action.
    Authorized,
    /// The authority denied the caller; there is no broader fallback.
    Denied,
}

/// Authorizes a broker operation through Polkit for an authenticated caller.
pub trait PolkitAuthorizer: Send + Sync {
    /// Authorize `action_id` (the operation's registered Polkit action) for the
    /// given authenticated peer.
    fn authorize(&self, action_id: &str, caller: &PeerCredentials) -> PolkitDecision;
}

/// Performs the fixed native operation and queries resulting state.
pub trait NativeBrokerOperations: Send + Sync {
    /// Whether this backend supports the operation's adapter/provider on the
    /// current host. An unsupported adapter is rejected before dispatch.
    fn supports(&self, operation: &BrokerOperation) -> bool;

    /// Validate operation-specific freshness against current state *before*
    /// dispatch (design §12: plan/path/provider/identity/options/percentage
    /// checks). The default accepts; a live backend re-queries the approved plan
    /// digest / bound path device+inode+owner immediately before the operation
    /// and returns [`BrokerPreDispatchError::StalePlan`] /
    /// [`BrokerPreDispatchError::StaleTargetIdentity`] on drift.
    ///
    /// [`BrokerPreDispatchError::StalePlan`]: super::protocol::BrokerPreDispatchError::StalePlan
    /// [`BrokerPreDispatchError::StaleTargetIdentity`]: super::protocol::BrokerPreDispatchError::StaleTargetIdentity
    fn precheck(
        &self,
        operation: &BrokerOperation,
    ) -> Result<(), super::protocol::BrokerPreDispatchError> {
        let _ = operation;
        Ok(())
    }

    /// Perform the fixed operation exactly once and return an effect-aware
    /// outcome. Called only after Polkit authorization.
    fn perform(&self, operation: &BrokerOperation) -> BrokerDispatchOutcome;
}

// ─────────────────────────────────────────────────────────────────────────────
// Deny-live fakes
// ─────────────────────────────────────────────────────────────────────────────

/// A Polkit fake with a fixed decision, for deny-live tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedPolkit(pub PolkitDecision);

impl FixedPolkit {
    /// Always authorize.
    #[must_use]
    pub fn allow() -> Self {
        Self(PolkitDecision::Authorized)
    }

    /// Always deny.
    #[must_use]
    pub fn deny() -> Self {
        Self(PolkitDecision::Denied)
    }
}

impl PolkitAuthorizer for FixedPolkit {
    fn authorize(&self, _action_id: &str, _caller: &PeerCredentials) -> PolkitDecision {
        self.0
    }
}

/// A scripted native-operations fake. It declares a supported-operation
/// predicate and a FIFO of outcomes, so deny-live tests drive Applied /
/// Uncertain / PartiallyApplied without any live process.
type SupportFn = Box<dyn Fn(&BrokerOperation) -> bool + Send + Sync>;
type PrecheckFn = Box<
    dyn Fn(&BrokerOperation) -> Result<(), super::protocol::BrokerPreDispatchError> + Send + Sync,
>;

pub struct ScriptedNativeOperations {
    supported: SupportFn,
    precheck: PrecheckFn,
    outcomes: Mutex<VecDeque<BrokerDispatchOutcome>>,
}

impl ScriptedNativeOperations {
    /// Create a fake that supports every operation and returns `outcomes` in
    /// order (a missing outcome panics loudly — a test must script every call).
    #[must_use]
    pub fn new(outcomes: impl IntoIterator<Item = BrokerDispatchOutcome>) -> Self {
        Self {
            supported: Box::new(|_| true),
            precheck: Box::new(|_| Ok(())),
            outcomes: Mutex::new(outcomes.into_iter().collect()),
        }
    }

    /// Like [`Self::new`] but with a custom support predicate (to exercise the
    /// unsupported-adapter path).
    #[must_use]
    pub fn with_support(
        supported: impl Fn(&BrokerOperation) -> bool + Send + Sync + 'static,
        outcomes: impl IntoIterator<Item = BrokerDispatchOutcome>,
    ) -> Self {
        Self {
            supported: Box::new(supported),
            precheck: Box::new(|_| Ok(())),
            outcomes: Mutex::new(outcomes.into_iter().collect()),
        }
    }

    /// Set a custom precheck (to exercise the stale-plan / stale-target-identity
    /// / timeout-before-dispatch paths).
    #[must_use]
    pub fn with_precheck(
        mut self,
        precheck: impl Fn(&BrokerOperation) -> Result<(), super::protocol::BrokerPreDispatchError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.precheck = Box::new(precheck);
        self
    }
}

impl NativeBrokerOperations for ScriptedNativeOperations {
    fn supports(&self, operation: &BrokerOperation) -> bool {
        (self.supported)(operation)
    }

    fn precheck(
        &self,
        operation: &BrokerOperation,
    ) -> Result<(), super::protocol::BrokerPreDispatchError> {
        (self.precheck)(operation)
    }

    fn perform(&self, _operation: &BrokerOperation) -> BrokerDispatchOutcome {
        self.outcomes
            .lock()
            .expect("scripted native ops poisoned")
            .pop_front()
            .expect("scripted native operation outcome missing")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Live stubs (require a token; trip the deny-live sentinel)
// ─────────────────────────────────────────────────────────────────────────────

/// The live Polkit authority transport. Constructible only in a live
/// composition (it borrows a [`LiveHostAccessToken`]); every authorization trips
/// the deny-live sentinel first, so it is unreachable under `os-control-test`.
/// The live Polkit authorizer.
///
/// # Why `pkcheck` and not D-Bus
///
/// [`PolkitAuthorizer::authorize`] is synchronous, and the crate's `zbus` is
/// built tokio-only — calling it here would mean nesting a runtime inside a
/// blocking call, which can deadlock the broker. `pkcheck` is Polkit's own
/// supported synchronous client and performs exactly the same
/// `CheckAuthorization` call.
///
/// The caller is identified by **pid and uid together**, and `--process
/// pid,start_time,uid` is used rather than a bare pid: a bare pid is reusable, so
/// a caller could exit and let an unrelated privileged process inherit its
/// authorization. The start time makes the subject unforgeable.
pub struct LivePolkitAuthorizer {
    _seal: (),
}

/// Polkit's synchronous authorization client.
const PKCHECK: &str = "/usr/bin/pkcheck";

/// How long an unanswered Polkit prompt may stay open. Long enough for a human to
/// read the dialog and type a password; short enough that an abandoned prompt does
/// not pin a privileged worker.
const PROMPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

impl LivePolkitAuthorizer {
    /// Construct in a live composition root.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }
}

impl PolkitAuthorizer for LivePolkitAuthorizer {
    fn authorize(&self, action_id: &str, caller: &PeerCredentials) -> PolkitDecision {
        deny_live_transport(RawTransportKind::Polkit);

        // The subject's start time comes from the kernel, not the request. A
        // caller that has exited cannot be authorized at all.
        let Ok(start_time) = crate::os_control::linux::signal::read_start_time(u32::try_from(caller.pid).unwrap_or(0)) else {
            return PolkitDecision::Denied;
        };
        let subject = format!("{},{},{}", caller.pid, start_time, caller.uid);

        // `--allow-user-interaction` is required, not optional: the policy file
        // declares every action `auth_admin`, so authorization can only be
        // obtained through an interactive check. Without it `pkcheck` would always
        // report "interaction required" and the broker could never authorize
        // anything.
        //
        // Crucially the prompt is **Polkit's own system dialog**, not a KRIA one:
        // the password is typed into a trusted OS component that this process
        // never sees. That is the entire reason to front privilege with Polkit
        // rather than collecting a password in the app.
        let child = std::process::Command::new(PKCHECK)
            .arg("--action-id")
            .arg(action_id)
            .arg("--process")
            .arg(&subject)
            .arg("--allow-user-interaction")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        let Ok(mut child) = child else {
            return PolkitDecision::Denied;
        };

        // Bounded: a human needs time to read the dialog and type a password, but
        // an unanswered prompt must not pin a privileged worker forever.
        let deadline = std::time::Instant::now() + PROMPT_BUDGET;
        loop {
            match child.try_wait() {
                // Exit 0 is the only authorization. Anything else — not
                // authorized, dismissed, or `pkcheck` itself failing — is denial.
                Ok(Some(status)) => {
                    return if status.success() {
                        PolkitDecision::Authorized
                    } else {
                        PolkitDecision::Denied
                    }
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        // Abandoned prompt: kill the check and deny.
                        let _ = child.kill();
                        let _ = child.wait();
                        return PolkitDecision::Denied;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => return PolkitDecision::Denied,
            }
        }
    }
}

/// The live native-operations backend. Constructible only in a live composition;
/// performing an operation trips the deny-live sentinel (raw process / bus), so
/// it is unreachable under `os-control-test`.
pub struct LiveNativeOperations {
    _seal: (),
}

impl LiveNativeOperations {
    /// Construct in a live composition root.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }
}

impl NativeBrokerOperations for LiveNativeOperations {
    fn supports(&self, operation: &BrokerOperation) -> bool {
        deny_live_transport(RawTransportKind::Process);
        // Support is declared **honestly and narrowly**. Only operations with a
        // fully implemented, verifiable privileged path are claimed; the broker
        // turns an unsupported operation into a clean bound rejection, which is
        // far better than a half-implemented root-capable path.
        match operation {
            BrokerOperation::SetBoundPathOwnership { .. } => true,
            BrokerOperation::SetBatteryChargeThresholds { adapter, .. } => {
                std::path::Path::new(charge_threshold_dir(adapter)).is_dir()
            }
            // Support is claimed only when the tool is actually installed, so an
            // absent `ufw` is a clean rejection rather than a failure after
            // dispatch.
            BrokerOperation::SetFirewallEnabled { provider, .. } => {
                std::path::Path::new(match provider {
                    super::protocol::FirewallProviderId::Ufw => "/usr/sbin/ufw",
                    super::protocol::FirewallProviderId::Firewalld => "/usr/bin/firewall-cmd",
                })
                .is_file()
            }
            BrokerOperation::ConfigureDiscoveredPrinter { .. } => {
                std::path::Path::new("/usr/sbin/lpadmin").is_file()
            }
            BrokerOperation::ApplyPackagePlan { provider, .. } => {
                std::path::Path::new(match provider {
                    super::protocol::PackageProviderId::Apt => "/usr/bin/apt-get",
                    super::protocol::PackageProviderId::Snap => "/usr/bin/snap",
                    super::protocol::PackageProviderId::Flatpak => "/usr/bin/flatpak",
                })
                .is_file()
            }
            // Deliberately NOT supported: a privacy toggle is a per-user setting,
            // and root writing it would change root's own settings while reporting
            // success. The unprivileged provider is authoritative for it.
            BrokerOperation::SetPrivacyControl { .. } => false,
        }
    }

    fn precheck(
        &self,
        operation: &BrokerOperation,
    ) -> Result<(), super::protocol::BrokerPreDispatchError> {
        deny_live_transport(RawTransportKind::Process);
        match operation {
            // Re-verify the bound path's identity immediately before dispatch. A
            // drift here means the path no longer refers to the object the user
            // approved, and `StaleTargetIdentity` is a `NotDispatched` response —
            // provably no effect, which is exactly right.
            BrokerOperation::SetBoundPathOwnership { path, .. } => {
                let facts = path_identity(&path.path)
                    .ok_or(super::protocol::BrokerPreDispatchError::StaleTargetIdentity)?;
                if facts != (path.device, path.inode, path.owner_uid) {
                    return Err(super::protocol::BrokerPreDispatchError::StaleTargetIdentity);
                }
                Ok(())
            }
            BrokerOperation::SetBatteryChargeThresholds {
                lower_percent,
                upper_percent,
                ..
            } => {
                // A lower bound above the upper bound is rejected by the kernel
                // anyway; catching it here keeps it a clean no-effect refusal.
                if lower_percent.get() > upper_percent.get() {
                    return Err(super::protocol::BrokerPreDispatchError::InvalidParameters);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn perform(&self, operation: &BrokerOperation) -> BrokerDispatchOutcome {
        deny_live_transport(RawTransportKind::Process);
        match operation {
            BrokerOperation::SetBoundPathOwnership { path, owner } => {
                set_bound_path_ownership(path, owner)
            }
            BrokerOperation::SetBatteryChargeThresholds {
                adapter,
                lower_percent,
                upper_percent,
            } => set_charge_thresholds(adapter, lower_percent.get(), upper_percent.get()),
            BrokerOperation::SetFirewallEnabled { provider, enabled } => {
                super::privileged_ops::set_firewall_enabled(provider, *enabled)
            }
            BrokerOperation::ConfigureDiscoveredPrinter { printer, options } => {
                super::privileged_ops::configure_discovered_printer(printer, options)
            }
            BrokerOperation::ApplyPackagePlan {
                provider,
                transaction,
                ..
            } => super::privileged_ops::apply_package_plan(provider, transaction),
            // `supports` returns false for this one, so the broker rejects it
            // before dispatch; the arm exists so the match stays exhaustive.
            BrokerOperation::SetPrivacyControl { control, enabled } => {
                super::privileged_ops::set_privacy_control(control, *enabled)
            }
        }
    }
}

/// Read a path's (device, inode, owner uid) **without following a symlink**.
fn path_identity(path: &str) -> Option<(u64, u64, u32)> {
    use std::os::unix::fs::MetadataExt;
    // `symlink_metadata` does not follow the final component, so a symlink
    // swapped in after approval cannot masquerade as the approved file.
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino(), meta.uid()))
}

/// Bounded, typed evidence. There is structurally no field for raw output, so
/// nothing a privileged command printed can leak through here.
fn broker_evidence(key: &str, value: &str) -> BoundedBrokerEvidence {
    BoundedBrokerEvidence::new(
        ProviderId::new("kria-os-broker"),
        Digest::of_str(&format!("{key}:{value}")),
        [EvidenceField {
            key: SafeField::new(key),
            value: SafeText::new(value),
        }],
    )
}

/// The sysfs directory carrying a recognized adapter's charge thresholds.
fn charge_threshold_dir(adapter: &ChargeThresholdAdapterId) -> &'static str {
    // A closed set: the id is a recognized adapter, never a caller-supplied path,
    // so no request can direct a privileged write at an arbitrary sysfs node.
    // A closed enum, so no request can direct a privileged write at an arbitrary
    // sysfs node — only one of these two fixed locations.
    match adapter {
        ChargeThresholdAdapterId::ThinkpadAcpi => "/sys/class/power_supply/BAT0",
        ChargeThresholdAdapterId::SysfsStandard => "/sys/class/power_supply/BAT0",
    }
}

/// Set ownership of an identity-bound path without following symlinks.
///
/// # The hazard this defends against
///
/// Between approval and execution the path could be replaced with a symlink to
/// something else — the classic TOCTOU escalation, and it matters far more here
/// because this code runs as root. Two defences apply:
///
/// 1. the path is opened with `O_NOFOLLOW`, so a symlink at the final component
///    fails outright rather than being followed; and
/// 2. the opened descriptor's device+inode+owner are compared against the approved
///    identity, and ownership is then changed **through that descriptor** — so the
///    object modified is provably the one approved, not whatever now occupies the
///    path.
fn set_bound_path_ownership(
    path: &BrokerBoundPath,
    owner: &ExistingLocalIdentity,
) -> BrokerDispatchOutcome {
    use std::ffi::CString;

    let Ok(c_path) = CString::new(path.path.as_bytes()) else {
        return uncertain_no_effect("path is not representable");
    };

    // O_PATH + O_NOFOLLOW: take a handle without opening contents, and refuse a
    // symlink at the final component.
    // SAFETY: `c_path` is a valid NUL-terminated string for the call's duration
    // and the flags are constants.
    let fd =
        unsafe { libc::open(c_path.as_ptr(), libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return uncertain_no_effect("path could not be opened without following a symlink");
    }
    // Closed on every exit path below, so no privileged handle leaks.
    let guard = OwnedFd(fd);

    // SAFETY: `guard.0` is an open descriptor and `stat` is a valid out-pointer.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(guard.0, &mut stat) } != 0 {
        return uncertain_no_effect("path identity could not be confirmed");
    }

    // Defence in depth: precheck already verified this, but re-checking through
    // the descriptor we are about to modify closes the remaining window.
    if stat.st_dev as u64 != path.device
        || stat.st_ino as u64 != path.inode
        || stat.st_uid != path.owner_uid
    {
        return uncertain_no_effect("path identity changed since approval");
    }

    // `/proc/self/fd/<n>` is used because `fchown` is not permitted on an `O_PATH`
    // descriptor, while this route still resolves to the same inode rather than
    // re-walking the path.
    let Ok(c_proc) = CString::new(format!("/proc/self/fd/{}", guard.0)) else {
        return uncertain_no_effect("descriptor path is not representable");
    };
    // `u32::MAX` (-1) leaves the group unchanged: the operation is about the owner,
    // and silently rewriting the group would be a change nobody approved.
    // SAFETY: the descriptor is open and owned by `guard`; the uid comes from a
    // validated existing local identity.
    if unsafe { libc::chown(c_proc.as_ptr(), owner.uid, u32::MAX) } != 0 {
        // `chown` is atomic: on failure ownership is unchanged.
        return uncertain_no_effect("ownership could not be changed");
    }

    BrokerDispatchOutcome::Applied {
        receipt_digest: Digest::of_str(&format!(
            "chown:{}:{}:{}",
            path.device, path.inode, owner.uid
        )),
        evidence: broker_evidence("ownership", "set on the identity-verified path"),
    }
}

/// A failure inside `perform` where nothing was changed.
///
/// `Uncertain` is used rather than a success because the outcome type has no
/// "not applied" variant by design: once `perform` is entered the broker no longer
/// claims proof of no effect. The verifier re-observes and settles it, so an
/// unchanged file is reported as unchanged rather than as a false success.
fn uncertain_no_effect(reason: &str) -> BrokerDispatchOutcome {
    BrokerDispatchOutcome::Uncertain {
        receipt_digest: None,
        cause: UncertainEffectCause::ProviderReportedFailureAfterDispatch,
        evidence: broker_evidence("refused", reason),
    }
}

/// Write battery charge thresholds to sysfs.
fn set_charge_thresholds(
    adapter: &ChargeThresholdAdapterId,
    lower: u8,
    upper: u8,
) -> BrokerDispatchOutcome {
    let dir = charge_threshold_dir(adapter);
    // The upper threshold is written first: writing a lower bound above the
    // current upper bound is rejected by the kernel, so this order avoids a
    // transient invalid pair.
    if std::fs::write(
        format!("{dir}/charge_control_end_threshold"),
        format!("{upper}\n"),
    )
    .is_err()
    {
        return uncertain_no_effect("the upper charge threshold could not be written");
    }
    if std::fs::write(
        format!("{dir}/charge_control_start_threshold"),
        format!("{lower}\n"),
    )
    .is_err()
    {
        // The upper bound DID change, so this is a real partial effect rather than
        // nothing having happened.
        return BrokerDispatchOutcome::Uncertain {
            receipt_digest: None,
            cause: UncertainEffectCause::ProviderReportedFailureAfterDispatch,
            evidence: broker_evidence(
                "partial",
                "the upper threshold was written but the lower threshold was not",
            ),
        };
    }
    BrokerDispatchOutcome::Applied {
        receipt_digest: Digest::of_str(&format!("charge:{}:{lower}:{upper}", adapter.tag())),
        evidence: broker_evidence("charge_thresholds", "written"),
    }
}

/// A descriptor closed when it goes out of scope.
struct OwnedFd(libc::c_int);

impl Drop for OwnedFd {
    fn drop(&mut self) {
        // SAFETY: obtained from `open` and closed exactly once.
        unsafe {
            libc::close(self.0);
        }
    }
}
