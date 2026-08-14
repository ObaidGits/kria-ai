//! `OsControlRuntime` — the injectable composition seam that keeps raw
//! `HostOsControl` private behind runtime composition.
//!
//! linux-os-control-production **Task 1.2** (OSC-001, OSC-003, OSC-009,
//! OSC-033), design §§4, 15.
//!
//! # Scope boundary
//!
//! The full governed lifecycle — mutation-permit sealing
//! ([`crate::os_control::context::AdmittedMutationContext`] construction),
//! observe/apply/verify/rollback orchestration, and terminal receipt
//! construction — is owned by **Task 1.7**. The concrete domain provider ports
//! and the `HostOsControl` aggregate are fleshed out across **Tasks 1.3–3.x**.
//!
//! Task 1.2 introduces `OsControlRuntime` as the **composition seam** only:
//!
//! * it is the single type composition roots inject (via the registry
//!   setter/getter and [`crate::tools::ToolContext`]);
//! * it holds any composed [`HostOsControl`] aggregate **privately** — there is
//!   no accessor that hands a tool or skill the raw `Arc<dyn HostOsControl>`, so
//!   the only way to reach host effects is through governed runtime methods;
//! * when no provider is composed (core-only registry, or before the live
//!   desktop/server composition wires one in a later task) every OS-facing entry
//!   point returns the frozen [`OsControlError::Unavailable`] envelope and
//!   **never** falls back to `LocalEnvironment` or a direct host subprocess.

use std::sync::Arc;

use crate::agent::resource_lease::ResourceRequirement;
use crate::agent::turn_memory::ExecutionTarget;
use crate::os_control::context::{
    AdmittedMutationContext, AuditAdmissionToken, ExecutionGrant, HostExecutionContext,
    MutationPermit,
};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, ReceiptId,
    SafeErrorCode, SafeStepId, SafeText, SnapshotRevision, Tolerance,
};
use crate::os_control::error::{GrantInvalidReason, OsControlError};
use crate::os_control::manifest::RollbackClaim;
use crate::os_control::receipt::{
    ApplyOutcome, AuditCompletionState, CompensationReport, ContradictedDispatch,
    FailureRollbackState, MutationReceipt, MutationResult, PartialDispatch, ReceiptCommon,
    RedactedObservation, RollbackAvailability, RollbackEligibleFailure, RollbackFailure,
    RollbackOutcome, RollbackReceipt, RollbackToken, RollbackTokenRejection, UnverifiedCause,
    UnverifiedDispatch, VerificationContradiction, VerificationReport,
};
use crate::os_control::redaction::parameter_digest;
use crate::os_control::resource::{write_resource_set_digest, AcquiredResourceLeaseSet};

/// Minimal aggregate handle for the local host OS control plane (design §4).
///
/// Task 1.2 defines only the identity method every provider must expose; the
/// full typed domain-port aggregate (`files()`, `audio()`, `power()`, …) is
/// added by Tasks 1.3–3.x as those ports land. Keeping the trait here — rather
/// than exposing a raw provider handle — means the aggregate can only ever be
/// reached through [`OsControlRuntime`], never handed out directly.
pub trait HostOsControl: Send + Sync {
    /// The stable provider identity of this aggregate (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The capability snapshot this aggregate was composed against, when the
    /// composition root probed the host (Task 1.3).
    ///
    /// `None` for a detached or unprobed aggregate: callers then fall back to
    /// environment hints rather than claiming probe-confirmed facts. The snapshot
    /// carries the revision that binds an admitted action to the capability state
    /// it was decided under.
    fn capability_snapshot(&self) -> Option<&crate::os_control::capability::CapabilitySnapshot> {
        None
    }

    /// The audio domain port (design §4 `fn audio(&self) -> &dyn AudioControl`,
    /// Task 2.1). `None` when no audio provider is composed into this
    /// aggregate; callers fall back to the frozen `Unavailable` envelope rather
    /// than any ungoverned subprocess. Returns the object-safe
    /// [`crate::os_control::audio::AudioControlPort`] supertrait so any
    /// concrete [`crate::os_control::audio::AudioControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn audio(&self) -> Option<&dyn crate::os_control::audio::AudioControlPort> {
        None
    }

    /// The display domain port (design §4 `fn display(&self) -> &dyn
    /// DisplayControl`, Task 2.2). `None` when no display provider is composed
    /// into this aggregate; callers fall back to the frozen `Unavailable`
    /// envelope rather than any ungoverned subprocess. Returns the
    /// object-safe [`crate::os_control::display::DisplayControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::display::DisplayControl`] transport instantiation
    /// can be composed behind one erased reference.
    fn display(&self) -> Option<&dyn crate::os_control::display::DisplayControlPort> {
        None
    }

    /// The connectivity domain port (design §4 `fn connectivity(&self) -> &dyn
    /// ConnectivityControl`, Task 2.3). `None` when no connectivity provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned subprocess. Returns
    /// the object-safe
    /// [`crate::os_control::connectivity::ConnectivityControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::connectivity::ConnectivityControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn connectivity(
        &self,
    ) -> Option<&dyn crate::os_control::connectivity::ConnectivityControlPort> {
        None
    }

    /// The power domain port (design §4 `fn power(&self) -> &dyn PowerControl`,
    /// Task 2.3 profile slice). `None` when no power provider is composed into
    /// this aggregate; callers fall back to the frozen `Unavailable` envelope
    /// rather than any ungoverned subprocess. Returns the object-safe
    /// [`crate::os_control::power::PowerControlPort`] supertrait so any
    /// concrete [`crate::os_control::power::PowerControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn power(&self) -> Option<&dyn crate::os_control::power::PowerControlPort> {
        None
    }

    /// The power-session domain port (design §4, Task 2.4 session/lifecycle
    /// slice: lock/suspend/hibernate/shutdown/reboot). `None` when no power-
    /// session provider is composed into this aggregate; callers fall back to
    /// the frozen `Unavailable` envelope rather than any ungoverned subprocess
    /// or sudo fallback. Returns the object-safe
    /// [`crate::os_control::power::session::PowerSessionControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::power::session::PowerSessionControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn power_session(
        &self,
    ) -> Option<&dyn crate::os_control::power::session::PowerSessionControlPort> {
        None
    }

    /// The process domain port (Task 2.5, design §4, §9.5: `kill_process`/
    /// `set_process_priority`). `None` when no process provider is composed
    /// into this aggregate; callers fall back to the frozen `Unavailable`
    /// envelope rather than any ungoverned syscall. Returns the object-safe
    /// [`crate::os_control::processes::ProcessControlPort`] supertrait so any
    /// concrete [`crate::os_control::processes::ProcessControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn processes(&self) -> Option<&dyn crate::os_control::processes::ProcessControlPort> {
        None
    }

    /// The Bluetooth domain port (Task 3.7, OSC-021). `None` when no BlueZ
    /// provider is composed; callers get the frozen `Unavailable` envelope rather
    /// than an ungoverned `bluetoothctl` invocation.
    fn bluetooth(&self) -> Option<&dyn crate::os_control::bluetooth::BluetoothControlPort> {
        None
    }

    /// The credential store (Task 3.10, OSC-025). `None` when no Secret Service
    /// is composed, so a caller gets the frozen `Unavailable` envelope rather
    /// than a silent failure to store a credential.
    fn secrets(&self) -> Option<&dyn crate::os_control::secrets::CredentialStore> {
        None
    }

    /// Battery charge thresholds (Task 5.4, broker-backed).
    fn charge_thresholds(
        &self,
    ) -> Option<&dyn crate::os_control::power::charge::ChargeThresholdControlPort> {
        None
    }

    /// Desktop search (Task 4.1).
    fn search_control(&self) -> Option<&dyn crate::os_control::search::SearchControlPort> {
        None
    }

    /// System health (Task 4.6).
    fn health(&self) -> Option<&dyn crate::os_control::health::SystemHealthControlPort> {
        None
    }

    /// Backup integration and scanning (Task 5.5).
    fn backup_scan(&self) -> Option<&dyn crate::os_control::backup::BackupScanControlPort> {
        None
    }

    /// Firmware awareness (Task 5.4, read-only).
    fn firmware(&self) -> Option<&dyn crate::os_control::hardware::FirmwareAwarenessPort> {
        None
    }

    /// Hardware sensors (Task 5.4, read-only).
    fn hardware(&self) -> Option<&dyn crate::os_control::hardware::HardwareControlPort> {
        None
    }

    /// Printing (Task 4.7). `None` when no print service is composed.
    fn print_control(&self) -> Option<&dyn crate::os_control::print::PrintControlPort> {
        None
    }

    /// Privacy controls (Task 4.7).
    fn privacy(&self) -> Option<&dyn crate::os_control::privacy::PrivacyControlPort> {
        None
    }

    /// Firewall (Tasks 4.3 / 5.3).
    fn firewall(&self) -> Option<&dyn crate::os_control::firewall::FirewallControlPort> {
        None
    }

    /// Monitor configuration and night light (Task 5.1). `None` when no
    /// compositor display-config service is composed.
    fn display_configuration(
        &self,
    ) -> Option<&dyn crate::os_control::display::configuration::DisplayConfigControlPort> {
        None
    }

    /// Direct file mutations: permissions, append, permanent delete (Task 3.1).
    fn file_attributes(
        &self,
    ) -> Option<&dyn crate::os_control::files::attributes::FileAttributeControlPort> {
        None
    }

    /// The application graceful-close domain port (Task 2.5, design §4,
    /// §9.3: `graceful_close_application`). `None` when no provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned syscall. Returns the
    /// object-safe
    /// [`crate::os_control::applications::ApplicationCloseControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::applications::ApplicationCloseControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn application_close(
        &self,
    ) -> Option<&dyn crate::os_control::applications::ApplicationCloseControlPort> {
        None
    }

    /// The clipboard domain port (Task 2.5, design §4, §9.10:
    /// `set_clipboard`/`get_clipboard`). `None` when no clipboard provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned device access.
    /// Returns the object-safe
    /// [`crate::os_control::clipboard::ClipboardControlPort`] supertrait so
    /// any concrete [`crate::os_control::clipboard::ClipboardControl`]
    /// transport instantiation can be composed behind one erased reference.
    fn clipboard(&self) -> Option<&dyn crate::os_control::clipboard::ClipboardControlPort> {
        None
    }

    /// The notification domain port (Task 2.5, design §4, §9.10:
    /// `send_notification`). `None` when no notification provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned `notify-send`
    /// subprocess fallback. Returns the object-safe
    /// [`crate::os_control::notifications::NotificationControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::notifications::NotificationControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn notifications(
        &self,
    ) -> Option<&dyn crate::os_control::notifications::NotificationControlPort> {
        None
    }

    /// The automation-listing domain port (Task 2.5, design §4, §9.13:
    /// `list_scheduled_tasks`). `None` when no automation provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned `crontab`/
    /// `systemctl` subprocess. Returns the object-safe
    /// [`crate::os_control::automation::AutomationControlPort`] supertrait so
    /// any concrete [`crate::os_control::automation::AutomationControl`]
    /// transport instantiation can be composed behind one erased reference.
    fn automation(&self) -> Option<&dyn crate::os_control::automation::AutomationControlPort> {
        None
    }

    /// The Trash domain port (design §4, Task 3.1). `None` when no Trash
    /// provider is composed into this aggregate; callers fall back to the
    /// frozen `Unavailable` envelope rather than any ungoverned
    /// `std::fs::remove_*` fallback. Returns the object-safe
    /// [`crate::os_control::files::TrashControlPort`] supertrait so any
    /// concrete [`crate::os_control::files::TrashControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn trash(&self) -> Option<&dyn crate::os_control::files::TrashControlPort> {
        None
    }

    /// The archive domain port (design §4, Task 3.1). `None` when no archive
    /// provider is composed into this aggregate; callers fall back to the
    /// frozen `Unavailable` envelope. Returns the object-safe
    /// [`crate::os_control::files::ArchiveControlPort`] supertrait so any
    /// concrete [`crate::os_control::files::ArchiveControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn archive(&self) -> Option<&dyn crate::os_control::files::ArchiveControlPort> {
        None
    }

    /// The ownership domain port (design §4, Task 3.1). `None` when no
    /// ownership provider is composed into this aggregate; callers fall back
    /// to the frozen `Unavailable` envelope rather than any ungoverned
    /// `chown` fallback. Returns the object-safe
    /// [`crate::os_control::files::OwnershipControlPort`] supertrait so any
    /// concrete [`crate::os_control::files::OwnershipControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn ownership(&self) -> Option<&dyn crate::os_control::files::OwnershipControlPort> {
        None
    }

    /// The storage domain port (design §4, Task 3.2). `None` when no
    /// storage provider is composed into this aggregate; callers fall back
    /// to the frozen `Unavailable` envelope rather than any ungoverned
    /// `udisksctl`/`mount`/`umount`/`eject` fallback. Returns the
    /// object-safe [`crate::os_control::storage::StorageControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::storage::StorageControl`] transport
    /// instantiation can be composed behind one erased reference.
    fn storage(&self) -> Option<&dyn crate::os_control::storage::StorageControlPort> {
        None
    }

    /// The desktop-association domain port (design §4, §9.2, Task 3.3:
    /// `set_default_application`/`manage_autostart`). `None` when no
    /// provider is composed into this aggregate; callers fall back to the
    /// frozen `Unavailable` envelope rather than any ungoverned
    /// `xdg-mime`/direct-write fallback. Returns the object-safe
    /// [`crate::os_control::applications::DesktopAssociationControlPort`]
    /// supertrait so any concrete
    /// [`crate::os_control::applications::DesktopAssociationControl`]
    /// transport instantiation can be composed behind one erased reference.
    fn desktop_association(
        &self,
    ) -> Option<&dyn crate::os_control::applications::DesktopAssociationControlPort> {
        None
    }

    /// The packages domain port (design §4 `fn packages(&self) -> &dyn
    /// PackageControl`, Task 3.4). `None` when no packages provider is
    /// composed into this aggregate; callers fall back to the frozen
    /// `Unavailable` envelope rather than any ungoverned
    /// `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak`/`pkexec`/`sudo`
    /// subprocess fallback. Returns the object-safe
    /// [`crate::os_control::packages::PackageControlPort`] supertrait so
    /// any concrete [`crate::os_control::packages::PackageControl`]
    /// transport instantiation can be composed behind one erased
    /// reference.
    fn packages(&self) -> Option<&dyn crate::os_control::packages::PackageControlPort> {
        None
    }
}

/// Runtime-only sealing witness (Task 1.7, design §4).
///
/// Its single field is **private to this module**, so a value can be
/// constructed only inside `os_control::runtime` — i.e. only by
/// [`OsControlRuntime`]. Every mutation-context sealing constructor
/// ([`AdmittedMutationContext::seal`], [`MutationPermit::seal`]) and every narrow
/// terminal-receipt constructor (`MutationReceipt::verified`, …) borrows a
/// `&RuntimeSealAuthority`. No provider, handler, adapter, or other crate module
/// can obtain one, so "only the runtime seals a mutation context" and "only the
/// runtime constructs a terminal receipt" are compile-time guarantees.
///
/// [`AdmittedMutationContext::seal`]: crate::os_control::context::AdmittedMutationContext
/// [`MutationPermit::seal`]: crate::os_control::context::MutationPermit
///
/// # No external construction
///
/// The witness's field is private, so it cannot be constructed outside this
/// module:
///
/// ```compile_fail
/// use kria_core::os_control::RuntimeSealAuthority;
/// // error[E0603]: the tuple struct constructor `RuntimeSealAuthority` is private
/// let _forged = RuntimeSealAuthority(());
/// ```
///
/// And with no witness, a provider/handler cannot forge a mutation context (its
/// fields are private and it exposes no public constructor):
///
/// ```compile_fail
/// use kria_core::os_control::AdmittedMutationContext;
/// // error: no associated public constructor exists on `AdmittedMutationContext`.
/// let _forged = AdmittedMutationContext::new();
/// ```
pub struct RuntimeSealAuthority(());

#[cfg(feature = "os-control-test")]
impl RuntimeSealAuthority {
    /// Mint a sealing witness for deny-live tests that exercise the receipt /
    /// context constructors directly. Gated to `os-control-test`; the production
    /// witness is minted only inside runtime sealing.
    #[must_use]
    pub fn for_test() -> Self {
        Self(())
    }
}

/// The governed OS-control runtime seam injected into OS tool handlers.
///
/// Raw [`HostOsControl`] is a **private** field: no method returns it. Handlers
/// receive `Arc<OsControlRuntime>` (through [`crate::tools::ToolContext`]) and
/// call governed methods; they can never obtain the underlying provider.
pub struct OsControlRuntime {
    /// The composed host aggregate, if any. Private and never handed out.
    host: Option<Arc<dyn HostOsControl>>,
}

impl std::fmt::Debug for OsControlRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsControlRuntime")
            .field("provider", &self.provider_id())
            .finish()
    }
}

impl OsControlRuntime {
    /// A runtime with **no** composed provider (core-only registry, or before a
    /// later task's live composition injects one). Every OS entry point returns
    /// the frozen `Unavailable` envelope.
    #[must_use]
    pub fn detached() -> Self {
        Self { host: None }
    }

    /// Compose a runtime around a live [`HostOsControl`] aggregate. Used by the
    /// desktop/server composition roots (and by fakes in `os-control-test`). The
    /// aggregate is stored privately and is never exposed to tools/skills.
    #[must_use]
    pub fn with_host(host: Arc<dyn HostOsControl>) -> Self {
        Self { host: Some(host) }
    }

    /// Whether a host provider aggregate is composed. Handlers use this to
    /// decide between a governed call and the `Unavailable` envelope; it never
    /// exposes the provider itself.
    #[must_use]
    pub fn provider_present(&self) -> bool {
        self.host.is_some()
    }

    /// The composed provider identity, if any. Returns only the redacted id
    /// newtype — never a handle to the aggregate.
    #[must_use]
    pub fn provider_id(&self) -> Option<ProviderId> {
        self.host.as_ref().map(|h| h.provider_id())
    }

    /// The frozen `Unavailable` error for `capability` when no provider is
    /// composed (design §15). This is the single pre-admission failure an OS
    /// handler returns instead of any `LocalEnvironment`/subprocess fallback.
    #[must_use]
    pub fn unavailable(&self, capability: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new(format!(
                "OS control provider for `{capability}` is not composed in this build"
            )),
            retryable: false,
        }
    }

    /// Governed capability probe used to demonstrate (and test) that OS handlers
    /// route through the runtime rather than any raw environment/provider.
    ///
    /// Returns the composed provider's identity, or the frozen `Unavailable`
    /// envelope when nothing is composed. It hands back only the redacted
    /// [`ProviderId`], proving that even a successful governed call never yields
    /// the raw [`HostOsControl`] handle. Later tasks add the full observe/apply/
    /// verify/rollback surface on this same private-`host` foundation.
    pub fn probe_provider(&self, capability: &str) -> Result<ProviderId, OsControlError> {
        match &self.host {
            Some(host) => Ok(host.provider_id()),
            None => Err(self.unavailable(capability)),
        }
    }

    /// The governed audio domain port (Task 2.1, design §4). Returns the
    /// composed [`crate::os_control::audio::AudioControlPort`] when a host
    /// aggregate with an audio provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never a raw subprocess
    /// fallback. This is the single path an audio tool handler uses to reach
    /// [`crate::os_control::audio::AudioControl`] through the runtime.
    /// The capability snapshot the composed aggregate was probed against, if any.
    ///
    /// Handlers and the governed-call layer read it through the runtime so the
    /// probe result reaches the observation context without a second seam.
    #[must_use]
    pub fn capability_snapshot(&self) -> Option<crate::os_control::capability::CapabilitySnapshot> {
        self.host
            .as_ref()
            .and_then(|host| host.capability_snapshot().cloned())
    }

    /// Borrow the charge-threshold port for `tool`, or the frozen `Unavailable`
    /// error.
    pub fn charge_thresholds(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::power::charge::ChargeThresholdControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.charge_thresholds())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the search port for `tool`, or the frozen `Unavailable` error.
    pub fn search_control(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::search::SearchControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.search_control())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the health port for `tool`, or the frozen `Unavailable` error.
    pub fn health(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::health::SystemHealthControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.health())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the backup/scan port for `tool`, or the frozen `Unavailable` error.
    pub fn backup_scan(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::backup::BackupScanControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.backup_scan())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the firmware port for `tool`, or the frozen `Unavailable` error.
    pub fn firmware(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::hardware::FirmwareAwarenessPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.firmware())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the sensors port for `tool`, or the frozen `Unavailable` error.
    pub fn hardware(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::hardware::HardwareControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.hardware())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the print port for `tool`, or the frozen `Unavailable` error.
    pub fn print_control(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::print::PrintControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.print_control())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the privacy port for `tool`, or the frozen `Unavailable` error.
    pub fn privacy(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::privacy::PrivacyControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.privacy())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the firewall port for `tool`, or the frozen `Unavailable` error.
    pub fn firewall(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::firewall::FirewallControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.firewall())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the display-configuration port for `tool`, or the frozen
    /// `Unavailable` error.
    pub fn display_configuration(
        &self,
        tool: &str,
    ) -> Result<
        &dyn crate::os_control::display::configuration::DisplayConfigControlPort,
        OsControlError,
    > {
        self.host
            .as_ref()
            .and_then(|host| host.display_configuration())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the file-attributes port for `tool`, or the frozen `Unavailable`
    /// error.
    pub fn file_attributes(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::files::attributes::FileAttributeControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.file_attributes())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the credential store for `tool`, or the frozen `Unavailable` error.
    pub fn secrets(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::secrets::CredentialStore, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.secrets())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the Bluetooth domain port for `tool`, or the frozen `Unavailable`
    /// error.
    pub fn bluetooth(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::bluetooth::BluetoothControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.bluetooth())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// Borrow the audio domain port for `tool`, or the frozen `Unavailable` error.
    pub fn audio(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::audio::AudioControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.audio())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed display domain port (Task 2.2, design §4). Returns the
    /// composed [`crate::os_control::display::DisplayControlPort`] when a host
    /// aggregate with a display provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never a raw subprocess
    /// fallback. This is the single path a display/brightness tool handler
    /// uses to reach [`crate::os_control::display::DisplayControl`] through
    /// the runtime.
    pub fn display(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::display::DisplayControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.display())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed connectivity domain port (Task 2.3, design §4). Returns the
    /// composed [`crate::os_control::connectivity::ConnectivityControlPort`]
    /// when a host aggregate with a connectivity provider is composed, or the
    /// frozen `Unavailable` envelope for `tool` otherwise — never a raw
    /// subprocess fallback. This is the single path a Wi-Fi tool handler uses
    /// to reach [`crate::os_control::connectivity::ConnectivityControl`]
    /// through the runtime.
    pub fn connectivity(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::connectivity::ConnectivityControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.connectivity())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed power domain port (Task 2.3 profile slice, design §4).
    /// Returns the composed [`crate::os_control::power::PowerControlPort`]
    /// when a host aggregate with a power provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never a raw subprocess
    /// fallback. This is the single path a power-plan tool handler uses to
    /// reach [`crate::os_control::power::PowerControl`] through the runtime.
    pub fn power(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::power::PowerControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.power())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed power-session domain port (Task 2.4, design §4). Returns
    /// the composed
    /// [`crate::os_control::power::session::PowerSessionControlPort`] when a
    /// host aggregate with a power-session provider is composed, or the
    /// frozen `Unavailable` envelope for `tool` otherwise — never a raw
    /// subprocess fallback and never a sudo/privilege-escalation fallback.
    /// This is the single path `lock_screen`/`sleep`/`hibernate`/
    /// `shutdown_system`/`reboot_system` tool handlers use to reach
    /// [`crate::os_control::power::session::PowerSessionControl`] through the
    /// runtime.
    pub fn power_session(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::power::session::PowerSessionControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.power_session())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed process domain port (Task 2.5, design §4). Returns the
    /// composed [`crate::os_control::processes::ProcessControlPort`] when a
    /// host aggregate with a process provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never a raw syscall
    /// fallback. This is the single path `kill_process`/
    /// `set_process_priority` tool handlers use to reach
    /// [`crate::os_control::processes::ProcessControl`] through the runtime.
    pub fn processes(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::processes::ProcessControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.processes())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed application graceful-close domain port (Task 2.5, design
    /// §4). Returns the composed
    /// [`crate::os_control::applications::ApplicationCloseControlPort`] when
    /// a host aggregate with a provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise. This is the single path
    /// `graceful_close_application` uses to reach
    /// [`crate::os_control::applications::ApplicationCloseControl`] through
    /// the runtime.
    pub fn application_close(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::applications::ApplicationCloseControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.application_close())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed clipboard domain port (Task 2.5, design §4). Returns the
    /// composed [`crate::os_control::clipboard::ClipboardControlPort`] when a
    /// host aggregate with a clipboard provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise. This is the single path
    /// `get_clipboard`/`set_clipboard` tool handlers use to reach
    /// [`crate::os_control::clipboard::ClipboardControl`] through the
    /// runtime.
    pub fn clipboard(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::clipboard::ClipboardControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.clipboard())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed notification domain port (Task 2.5, design §4). Returns
    /// the composed
    /// [`crate::os_control::notifications::NotificationControlPort`] when a
    /// host aggregate with a notification provider is composed, or the
    /// frozen `Unavailable` envelope for `tool` otherwise — never a
    /// `notify-send` subprocess fallback. This is the single path
    /// `send_notification` uses to reach
    /// [`crate::os_control::notifications::NotificationControl`] through the
    /// runtime.
    pub fn notifications(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::notifications::NotificationControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.notifications())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed automation-listing domain port (Task 2.5, design §4).
    /// Returns the composed
    /// [`crate::os_control::automation::AutomationControlPort`] when a host
    /// aggregate with an automation provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never a `crontab`/
    /// `systemctl` subprocess fallback. This is the single path
    /// `list_scheduled_tasks` uses to reach
    /// [`crate::os_control::automation::AutomationControl`] through the
    /// runtime.
    pub fn automation(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::automation::AutomationControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.automation())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed Trash domain port (Task 3.1, design §4). Returns the
    /// composed [`crate::os_control::files::TrashControlPort`] when a host
    /// aggregate with a Trash provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never an ungoverned
    /// `std::fs::remove_*` fallback. This is the single path `trash_file`/
    /// `restore_trash_item` tool handlers use to reach
    /// [`crate::os_control::files::TrashControl`] through the runtime.
    pub fn trash(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::files::TrashControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.trash())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed archive domain port (Task 3.1, design §4). Returns the
    /// composed [`crate::os_control::files::ArchiveControlPort`] when a host
    /// aggregate with an archive provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise. This is the single path
    /// `create_archive`/`list_archive`/`extract_archive` tool handlers use to
    /// reach [`crate::os_control::files::ArchiveControl`] through the
    /// runtime.
    pub fn archive(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::files::ArchiveControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.archive())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed ownership domain port (Task 3.1, design §4). Returns the
    /// composed [`crate::os_control::files::OwnershipControlPort`] when a
    /// host aggregate with an ownership provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never an ungoverned
    /// `chown` fallback. This is the single path `set_file_ownership` uses to
    /// reach [`crate::os_control::files::OwnershipControl`] through the
    /// runtime.
    pub fn ownership(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::files::OwnershipControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.ownership())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed storage domain port (Task 3.2, design §4). Returns the
    /// composed [`crate::os_control::storage::StorageControlPort`] when a
    /// host aggregate with a storage provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never an ungoverned
    /// `udisksctl`/`mount`/`umount`/`eject` fallback. This is the single
    /// path `list_storage_devices`/`mount_device`/`unmount_device`/
    /// `eject_device`/`get_storage_health` tool handlers use to reach
    /// [`crate::os_control::storage::StorageControl`] through the runtime.
    pub fn storage(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::storage::StorageControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.storage())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed desktop-association domain port (Task 3.3, design §4).
    /// Returns the composed
    /// [`crate::os_control::applications::DesktopAssociationControlPort`]
    /// when a host aggregate with a provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never an ungoverned
    /// direct-write fallback. This is the single path
    /// `set_default_application`/`manage_autostart` tool handlers use to
    /// reach
    /// [`crate::os_control::applications::DesktopAssociationControl`]
    /// through the runtime.
    pub fn desktop_association(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::applications::DesktopAssociationControlPort, OsControlError>
    {
        self.host
            .as_ref()
            .and_then(|host| host.desktop_association())
            .ok_or_else(|| self.unavailable(tool))
    }

    /// The governed packages domain port (Task 3.4, design §4). Returns the
    /// composed [`crate::os_control::packages::PackageControlPort`] when a
    /// host aggregate with a packages provider is composed, or the frozen
    /// `Unavailable` envelope for `tool` otherwise — never an ungoverned
    /// `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak`/`pkexec`/`sudo`
    /// subprocess fallback. This is the single path
    /// `search_package`/`get_package_info`/`list_installed_packages`/
    /// `plan_package_changes`/`install_package`/`uninstall_package`/
    /// `check_system_updates`/`get_reboot_required` tool handlers use to
    /// reach [`crate::os_control::packages::PackageControl`] through the
    /// runtime.
    pub fn packages(
        &self,
        tool: &str,
    ) -> Result<&dyn crate::os_control::packages::PackageControlPort, OsControlError> {
        self.host
            .as_ref()
            .and_then(|host| host.packages())
            .ok_or_else(|| self.unavailable(tool))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Normalized observation, postcondition predicates, and evidence ordering
// (Task 1.7, design §5, §13)
// ─────────────────────────────────────────────────────────────────────────────

/// A normalized domain observation the runtime can compare for idempotency and
/// verification. Provider observation types implement it; the digest is the
/// stable identity used for `Exact`/`Membership` comparison, and the optional
/// numeric value is used for `WithinTolerance` comparison (e.g. audio %).
pub trait NormalizedObservation {
    /// The stable, redaction-safe digest of this normalized observation.
    fn observation_digest(&self) -> Digest;

    /// The comparable numeric value, when the domain state is numeric.
    fn numeric_value(&self) -> Option<f64> {
        None
    }
}

/// Whether `observed` satisfies the `desired` state under a comparator/tolerance
/// (design §13). `Exact`/`Membership` compare normalized digests; `WithinTolerance`
/// compares numeric values within an absolute delta, falling back to digest
/// equality when either side is non-numeric or no tolerance is supplied.
#[must_use]
pub fn observation_satisfies<O: NormalizedObservation>(
    comparator: ComparatorKind,
    tolerance: Option<Tolerance>,
    desired: &O,
    observed: &O,
) -> bool {
    match comparator {
        ComparatorKind::Exact | ComparatorKind::Membership => {
            observed.observation_digest() == desired.observation_digest()
        }
        ComparatorKind::WithinTolerance => {
            match (observed.numeric_value(), desired.numeric_value(), tolerance) {
                (Some(o), Some(d), Some(t)) => (o - d).abs() <= t.abs,
                _ => observed.observation_digest() == desired.observation_digest(),
            }
        }
    }
}

/// Whether an observation is fresh enough to satisfy the postcondition deadline
/// (design §5.2/§13): its measured freshness must be within the bounded deadline.
#[must_use]
pub fn evidence_is_fresh(freshness_ms: u64, deadline_ms: u64) -> bool {
    freshness_ms <= deadline_ms
}

/// The strongest OS evidence source among the candidates (design §13 evidence
/// ordering). Authoritative service/property or filesystem state outranks an
/// independent provider query, which outranks structured-command (shell) query
/// output, which outranks user attestation — so shell output can never outrank
/// authoritative state.
#[must_use]
pub fn strongest_os_evidence(sources: &[OsEvidenceSource]) -> Option<OsEvidenceSource> {
    sources.iter().copied().max()
}

// ─────────────────────────────────────────────────────────────────────────────
// Sealing inputs and mutation plan (Task 1.7, design §4, §6)
// ─────────────────────────────────────────────────────────────────────────────

/// The live bindings the runtime re-derives at seal time to prove the fresh
/// grant and the committed audit admission describe the same logical action
/// (design §4). These are recomputed from live values (never trusted from the
/// grant alone), so a changed action/parameter/target/resource/revision is caught
/// before any provider mutation.
#[derive(Debug, Clone, Copy)]
pub struct SealBinding<'b> {
    /// The live user session id.
    pub session_id: &'b str,
    /// The live canonical action name.
    pub action: &'b str,
    /// The live canonical parameters.
    pub params: &'b serde_json::Value,
    /// The live resolved execution target (must be `Host`).
    pub target: ExecutionTarget,
    /// The live derived exclusive resource requirements.
    pub resource_requirements: &'b [ResourceRequirement],
    /// The live capability-snapshot revision.
    pub capability_snapshot_revision: SnapshotRevision,
}

/// How rollback is provisioned for a mutation (design §4, §13.1). The static
/// per-operation `RollbackClaim` lives in the manifest; this is the *runtime*
/// disposition after prior state was (or was not) captured.
#[derive(Debug, Clone)]
pub enum RollbackPlan {
    /// No reliable inverse or insufficient prior state — never advertise rollback.
    Unavailable,
    /// Rollback is advertised with an opaque token. `auto` requests an automatic
    /// bounded rollback attempt when fresh evidence contradicts the desired state.
    Available {
        /// The opaque, bounded, session-scoped rollback token.
        token: RollbackToken,
        /// Whether to attempt automatic rollback on a contradiction.
        auto: bool,
    },
}

/// Everything the runtime needs to classify a completed apply into the single
/// valid terminal receipt state (design §4).
#[derive(Debug, Clone)]
pub struct MutationPlan {
    /// The opaque receipt identity.
    pub receipt_id: ReceiptId,
    /// The verifying provider identity.
    pub provider: ProviderId,
    /// Idempotency/verification comparator.
    pub comparator: ComparatorKind,
    /// Optional numeric tolerance.
    pub tolerance: Option<Tolerance>,
    /// Bounded re-observation freshness deadline.
    pub deadline_ms: u64,
    /// Rollback disposition.
    pub rollback: RollbackPlan,
    /// Measured action latency (diagnostics only).
    pub latency_ms: u64,
}

impl MutationPlan {
    fn availability(&self) -> RollbackAvailability {
        match &self.rollback {
            RollbackPlan::Unavailable => RollbackAvailability::Unavailable,
            RollbackPlan::Available { token, .. } => RollbackAvailability::Available(token.clone()),
        }
    }
}

fn redacted<O: NormalizedObservation + Clone>(value: &O) -> RedactedObservation<O> {
    RedactedObservation::new(value.clone(), value.observation_digest())
}

// ─────────────────────────────────────────────────────────────────────────────
// Runtime sealing + governed mutation orchestration (Task 1.7)
// ─────────────────────────────────────────────────────────────────────────────

impl OsControlRuntime {
    /// Mint the sealing witness. Private to the runtime: this is the only place
    /// in the crate a [`RuntimeSealAuthority`] value comes into existence.
    fn seal_authority(&self) -> RuntimeSealAuthority {
        RuntimeSealAuthority(())
    }

    /// Seal a mutation-capable [`AdmittedMutationContext`] (design §4, §6).
    ///
    /// This is the **only** constructor of a mutation context. It verifies that:
    ///
    /// * the grant has not expired and is bound to `ExecutionTarget::Host`;
    /// * the fresh grant matches the live action / parameter / target / resource
    ///   bindings (via [`ExecutionGrant::matches`]) — a changed argv/action/target
    ///   or resource set fails here;
    /// * the grant and the committed [`AuditAdmissionToken`] agree on session,
    ///   action, parameter, resource-set, and capability-revision bindings, and
    ///   the borrowed observation context was lent from that same admission;
    /// * the exact canonical resource set named by `grant.resource_set_digest` is
    ///   currently held (the live [`AcquiredResourceLeaseSet`] digest matches).
    ///
    /// On any mismatch it returns a pre-mutation [`OsControlError`] and constructs
    /// nothing, so no provider mutation can follow. Only on full agreement does it
    /// seal the borrowed leases + audit admission into a [`MutationPermit`] and
    /// return the context, whose borrows keep `apply` from outliving any authority.
    pub fn seal_mutation_context<'a>(
        &self,
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        lease_set: &'a AcquiredResourceLeaseSet,
        audit_admission: &'a AuditAdmissionToken,
        binding: &SealBinding<'_>,
    ) -> Result<AdmittedMutationContext<'a>, OsControlError> {
        // (0) Expiry and host-only target — proven no effect.
        if grant.is_expired(std::time::SystemTime::now()) {
            return Err(OsControlError::ApprovalExpired);
        }
        if grant.target() != ExecutionTarget::Host || binding.target != ExecutionTarget::Host {
            return Err(OsControlError::InvalidRequest {
                field: crate::os_control::contract::SafeField::new("target"),
                reason: SafeText::new(
                    "mutation sealing is host-only; non-host targets are rejected",
                ),
            });
        }

        // (a) The fresh grant must match the live action/parameter/target/resource
        //     bindings. A changed argv/action/target/resource set fails here.
        if !grant.matches(
            binding.session_id,
            binding.action,
            binding.params,
            binding.target,
            binding.resource_requirements,
        ) {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }

        // Recompute the admission bindings from live values, exactly as
        // `OsAuditStore::admit_action` did, and compare against the committed token.
        let live_action_hash = Digest::of_str(binding.action);
        let live_parameter_hash = parameter_digest(binding.params);
        let live_resource_digest = write_resource_set_digest(binding.action, binding.params);

        // (c) Session binding.
        if grant.session_id() != audit_admission.session_id().as_str()
            || binding.session_id != audit_admission.session_id().as_str()
        {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::SessionMismatch,
            });
        }
        // (c) Action binding (grant.action == admission action == live action).
        if grant.action() != binding.action || live_action_hash != *audit_admission.action_hash() {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }
        // (c) Parameter binding: live params must reproduce the admission digest.
        if live_parameter_hash != *audit_admission.parameter_hash() {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }
        // (a)+(c) Capability-revision binding.
        if grant.capability_snapshot_revision() != audit_admission.capability_snapshot_revision()
            || binding.capability_snapshot_revision
                != audit_admission.capability_snapshot_revision()
        {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::StaleSnapshot,
            });
        }
        // (c) Observation context must be lent from this same admission.
        if observation.observation_audit().admission_id() != audit_admission.admission_id() {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }

        // (a)+(c) Resource binding across grant, admission, and live derivation.
        let grant_resource_digest = Digest::from_hex(grant.resource_set_digest());
        if grant_resource_digest != *audit_admission.resource_set_digest()
            || live_resource_digest != *audit_admission.resource_set_digest()
        {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }

        // (b) The exact named resource set must be currently HELD.
        if lease_set.resource_set_digest() != audit_admission.resource_set_digest() {
            return Err(OsControlError::ResourceBusy {
                resource: crate::os_control::contract::SafeResource::new("os-control-write-set"),
                owner: None,
            });
        }

        // All authorities agree — seal the permit and context. Runtime-only.
        let auth = self.seal_authority();
        let permit = MutationPermit::seal(&auth, lease_set, audit_admission, grant_resource_digest);
        Ok(AdmittedMutationContext::seal(
            &auth,
            observation,
            grant,
            permit,
            binding.action.to_string(),
            binding.params.clone(),
        ))
    }

    /// Run one governed desired-state mutation end to end (design §6):
    /// observe → idempotency → seal → under-lease re-observe → apply-once →
    /// verify → optional bounded rollback → construct the single valid terminal
    /// receipt. The verifier is never retried and the provider is never
    /// redispatched; a second-mutator "retry" is unrepresentable here.
    ///
    /// A pre-mutation [`OsControlError`] from `observe`/seal/`apply` means no host
    /// effect started. Once `apply` returns an [`ApplyOutcome`], every result is a
    /// [`MutationReceipt`] state — including uncertain/contradictory/partial.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_mutation<R, O, P>(
        &self,
        provider: &P,
        observation: &HostExecutionContext,
        grant: &ExecutionGrant,
        lease_set: &AcquiredResourceLeaseSet,
        audit_admission: &AuditAdmissionToken,
        binding: &SealBinding<'_>,
        request: &R,
        desired: &O,
        plan: &MutationPlan,
        audit_completion: AuditCompletionState,
    ) -> MutationResult<O>
    where
        R: Send + Sync,
        O: NormalizedObservation + Clone + Send + Sync,
        P: DesiredStateControl<R, O> + ?Sized,
    {
        let auth = self.seal_authority();
        let common = ReceiptCommon::new(
            plan.receipt_id.clone(),
            Digest::of_str(binding.action),
            Digest::of_str(binding.target.as_str()),
            plan.provider.clone(),
            plan.latency_ms,
        );

        // 1. Pre-observation (read authority only, no grant).
        let before_val = provider.observe(observation, request).await?;
        let before = redacted(&before_val);

        // 2. Idempotency: desired already holds → Unchanged, zero apply calls.
        if observation_satisfies(plan.comparator, plan.tolerance, desired, &before_val) {
            return Ok(MutationReceipt::unchanged(
                &auth,
                common,
                before,
                audit_completion,
            ));
        }

        // 3. Seal the mutation permit. Any binding mismatch returns here with NO
        //    provider mutation performed.
        let ctx =
            self.seal_mutation_context(observation, grant, lease_set, audit_admission, binding)?;

        // 4. Under-lease re-observation closes the TOCTOU gap; converged → Unchanged.
        let reobserved = provider.observe(observation, request).await?;
        if observation_satisfies(plan.comparator, plan.tolerance, desired, &reobserved) {
            return Ok(MutationReceipt::unchanged(
                &auth,
                common,
                redacted(&reobserved),
                audit_completion,
            ));
        }

        // 5. Apply exactly once. `Err` here is proven-no-effect (pre-dispatch).
        let outcome = provider.apply(&ctx, request, desired).await?;

        match outcome {
            // Session-ending / async: Accepted, from acceptance evidence only.
            ApplyOutcome::Accepted(accepted) => Ok(MutationReceipt::accepted(
                &auth,
                common,
                Some(before),
                accepted,
                audit_completion,
            )),
            // Known multi-step residue → PartiallyApplied.
            ApplyOutcome::PartiallyApplied(partial) => Ok(MutationReceipt::partially_applied(
                &auth,
                common,
                Some(before),
                None,
                partial,
                FailureRollbackState::NotAttempted(plan.availability()),
                audit_completion,
            )),
            ApplyOutcome::Applied(applied) => {
                let dispatch = UnverifiedDispatch::Applied(applied.clone());
                let contradicted = ContradictedDispatch::Applied(applied.clone());
                self.finalize_verifiable(
                    &ctx,
                    provider,
                    observation,
                    request,
                    desired,
                    &before_val,
                    before,
                    common,
                    Some(applied),
                    dispatch,
                    contradicted,
                    plan,
                    audit_completion,
                )
                .await
            }
            ApplyOutcome::Uncertain(uncertain) => {
                let dispatch = UnverifiedDispatch::Uncertain(uncertain.clone());
                let contradicted = ContradictedDispatch::Uncertain(uncertain);
                self.finalize_verifiable(
                    &ctx,
                    provider,
                    observation,
                    request,
                    desired,
                    &before_val,
                    before,
                    common,
                    None, // uncertain dispatch can never reach Verified
                    dispatch,
                    contradicted,
                    plan,
                    audit_completion,
                )
                .await
            }
        }
    }

    /// Finalize an `Applied`/`Uncertain` dispatch: fresh re-observe + typed
    /// verification → the one valid terminal state. `verified_apply` is `Some`
    /// only for an `AppliedDispatch` (the only fact that may reach `Verified`).
    #[allow(clippy::too_many_arguments)]
    async fn finalize_verifiable<R, O, P>(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        provider: &P,
        observation: &HostExecutionContext,
        request: &R,
        desired: &O,
        before_val: &O,
        before: RedactedObservation<O>,
        common: ReceiptCommon,
        verified_apply: Option<crate::os_control::receipt::AppliedDispatch>,
        dispatch: UnverifiedDispatch,
        contradicted: ContradictedDispatch,
        plan: &MutationPlan,
        audit_completion: AuditCompletionState,
    ) -> MutationResult<O>
    where
        R: Send + Sync,
        O: NormalizedObservation + Clone + Send + Sync,
        P: DesiredStateControl<R, O> + ?Sized,
    {
        let auth = self.seal_authority();

        // Fresh, independent re-observation. If it is unavailable we cannot build
        // a decisive terminal → Unverified(ObservationUnavailable).
        let after_val = match provider.observe(observation, request).await {
            Ok(v) => v,
            Err(_) => {
                return Ok(MutationReceipt::unverified(
                    &auth,
                    common,
                    Some(before),
                    None,
                    dispatch,
                    UnverifiedCause::ObservationUnavailable,
                    FailureRollbackState::NotAttempted(plan.availability()),
                    audit_completion,
                ));
            }
        };
        let after = redacted(&after_val);

        // Typed postcondition verification. A verify error is post-dispatch and
        // must not masquerade as a pre-mutation error → Unverified.
        let report = match provider.verify(observation, request, desired).await {
            Ok(report) => report,
            Err(_) => {
                return Ok(MutationReceipt::unverified(
                    &auth,
                    common,
                    Some(before),
                    Some(after),
                    dispatch,
                    UnverifiedCause::NoDecisiveObservation,
                    FailureRollbackState::NotAttempted(plan.availability()),
                    audit_completion,
                ));
            }
        };

        match report {
            VerificationReport::Satisfied(evidence) => {
                let fresh = evidence_is_fresh(evidence.freshness_ms(), plan.deadline_ms);
                match (verified_apply, fresh) {
                    // Applied + fresh satisfying evidence → Verified.
                    (Some(applied), true) => Ok(MutationReceipt::verified(
                        &auth,
                        common,
                        before,
                        evidence.observation().clone(),
                        applied,
                        evidence,
                        plan.availability(),
                        audit_completion,
                    )),
                    // Applied but stale, or Uncertain (never Verified) → Unverified.
                    _ => Ok(MutationReceipt::unverified(
                        &auth,
                        common,
                        Some(before),
                        Some(after),
                        dispatch,
                        UnverifiedCause::NoDecisiveObservation,
                        FailureRollbackState::NotAttempted(plan.availability()),
                        audit_completion,
                    )),
                }
            }
            VerificationReport::Inconclusive { .. } => Ok(MutationReceipt::unverified(
                &auth,
                common,
                Some(before),
                Some(after),
                dispatch,
                UnverifiedCause::NoDecisiveObservation,
                FailureRollbackState::NotAttempted(plan.availability()),
                audit_completion,
            )),
            VerificationReport::Contradicted(contradiction) => {
                self.resolve_contradiction(
                    ctx,
                    provider,
                    observation,
                    request,
                    before_val,
                    before,
                    after,
                    common,
                    contradicted,
                    contradiction,
                    plan,
                    audit_completion,
                )
                .await
            }
        }
    }

    /// Resolve a fresh contradiction: attempt a bounded rollback only when
    /// predeclared, then verify the restore. Never redispatches the forward
    /// mutation. Yields `RolledBack` on verified restore, else `VerificationFailed`
    /// (carrying the rollback failure when one was attempted).
    #[allow(clippy::too_many_arguments)]
    async fn resolve_contradiction<R, O, P>(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        provider: &P,
        observation: &HostExecutionContext,
        request: &R,
        before_val: &O,
        before: RedactedObservation<O>,
        after: RedactedObservation<O>,
        common: ReceiptCommon,
        dispatch: ContradictedDispatch,
        contradiction: VerificationContradiction,
        plan: &MutationPlan,
        audit_completion: AuditCompletionState,
    ) -> MutationResult<O>
    where
        R: Send + Sync,
        O: NormalizedObservation + Clone + Send + Sync,
        P: DesiredStateControl<R, O> + ?Sized,
    {
        let auth = self.seal_authority();

        if let RollbackPlan::Available { token, auto: true } = &plan.rollback {
            let rollback_failed = |code: &'static str| {
                FailureRollbackState::Failed(RollbackFailure::new(
                    SafeErrorCode::from_static(code),
                    None,
                ))
            };

            match provider.rollback(ctx, token).await {
                Ok(ApplyOutcome::Applied(_)) | Ok(ApplyOutcome::Accepted(_)) => {
                    // Verify the restore against the original before-state.
                    match provider.verify(observation, request, before_val).await {
                        Ok(VerificationReport::Satisfied(v))
                            if evidence_is_fresh(v.freshness_ms(), plan.deadline_ms) =>
                        {
                            Ok(MutationReceipt::rolled_back(
                                &auth,
                                common,
                                before,
                                Some(after),
                                RollbackEligibleFailure::new(dispatch, contradiction),
                                v,
                                audit_completion,
                            ))
                        }
                        _ => Ok(MutationReceipt::verification_failed(
                            &auth,
                            common,
                            before,
                            after,
                            dispatch,
                            contradiction,
                            rollback_failed("os_control.incident.rollback_verification_failed"),
                            audit_completion,
                        )),
                    }
                }
                // Rollback itself uncertain / partial / failed → truthful failure.
                _ => Ok(MutationReceipt::verification_failed(
                    &auth,
                    common,
                    before,
                    after,
                    dispatch,
                    contradiction,
                    rollback_failed("os_control.incident.rollback_failed"),
                    audit_completion,
                )),
            }
        } else {
            // No automatic rollback: report the contradiction with availability.
            Ok(MutationReceipt::verification_failed(
                &auth,
                common,
                before,
                after,
                dispatch,
                contradiction,
                FailureRollbackState::NotAttempted(plan.availability()),
                audit_completion,
            ))
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rollback coordinator and compensation contract (Task 1.9, design §4, §13.1,
// §14.5; OSC-006, OSC-028)
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a manifest [`RollbackClaim`] permits *ever* advertising rollback
/// (OSC-006.6). Permanent deletion, process kill, shutdown, reboot, and routine
/// updates resolve to [`RollbackClaim::NoRollback`] and therefore never claim
/// rollback, regardless of what a provider captured.
#[must_use]
pub fn rollback_claim_advertisable(claim: RollbackClaim) -> bool {
    !matches!(claim, RollbackClaim::NoRollback)
}

/// Reconcile the manifest [`RollbackClaim`] with the runtime [`RollbackPlan`] so
/// the advertised [`RollbackAvailability`] in a receipt **exactly** matches both
/// the static claim and the provider's captured prior state (the Task 1.9
/// completion proof).
///
/// * A [`RollbackClaim::NoRollback`] operation is forced to
///   [`RollbackAvailability::Unavailable`] even if a provider mistakenly built a
///   token — a non-reversible action can never advertise rollback.
/// * Otherwise the provider's runtime disposition is honored: `Available` only
///   when the provider actually captured sufficient prior state and minted a
///   token, else `Unavailable`.
#[must_use]
pub fn reconcile_rollback_availability(
    claim: RollbackClaim,
    plan: &RollbackPlan,
) -> RollbackAvailability {
    if !rollback_claim_advertisable(claim) {
        return RollbackAvailability::Unavailable;
    }
    match plan {
        RollbackPlan::Unavailable => RollbackAvailability::Unavailable,
        RollbackPlan::Available { token, .. } => RollbackAvailability::Available(token.clone()),
    }
}

/// A per-step compensator for reverse-order multi-step compensation (Task 1.9,
/// OSC-006.7/OSC-028). It declares which completed steps are reversible and
/// compensates one step at a time under a sealed mutation context. The
/// coordinator ([`OsControlRuntime::compensate_partial`]) drives it in reverse
/// order, compensating each step at most once and only where declared reversible.
#[async_trait::async_trait]
pub trait StepCompensator: Send + Sync {
    /// Whether a completed step is declared reversible. Non-reversible completed
    /// work is left in place (never compensated).
    fn is_reversible(&self, step: &SafeStepId) -> bool;

    /// Compensate exactly one completed step. `Err` reports that this step's
    /// compensation failed; the coordinator stops and reports partial progress.
    async fn compensate_step(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        step: &SafeStepId,
    ) -> Result<(), OsControlError>;
}

/// The inputs a rollback logical action needs to classify its outcome (design
/// §4, §14.5). The rollback is a **separate** action linked to the original
/// receipt via `linked_receipt`.
#[derive(Debug, Clone)]
pub struct RollbackExecPlan {
    /// The rollback action's own opaque receipt identity.
    pub rollback_receipt_id: ReceiptId,
    /// The original receipt this rollback undoes (OSC-006.5 linkage).
    pub linked_receipt: ReceiptId,
    /// The action-name digest of the original operation being undone. The token
    /// must be linked to exactly this action.
    pub original_action_hash: Digest,
    /// The provider capability that owns the reversible operation. The token
    /// must be owned by exactly this capability.
    pub capability: ProviderId,
    /// Comparator used to verify the restore against the captured prior state.
    pub comparator: ComparatorKind,
    /// Optional numeric tolerance for restore verification.
    pub tolerance: Option<Tolerance>,
    /// Bounded restore re-observation freshness deadline.
    pub deadline_ms: u64,
    /// Measured rollback latency (diagnostics only).
    pub latency_ms: u64,
}

impl OsControlRuntime {
    /// Map a pre-rollback token rejection to the frozen pre-mutation error set.
    /// Every rejection is proven-no-compensation.
    fn token_rejection_error(rejection: RollbackTokenRejection) -> OsControlError {
        match rejection {
            RollbackTokenRejection::Expired => OsControlError::ApprovalExpired,
            RollbackTokenRejection::SessionMismatch => OsControlError::GrantInvalid {
                reason: GrantInvalidReason::SessionMismatch,
            },
            RollbackTokenRejection::ActionMismatch | RollbackTokenRejection::CapabilityMismatch => {
                OsControlError::GrantInvalid {
                    reason: GrantInvalidReason::BindingMismatch,
                }
            }
        }
    }

    /// Run one governed, separately-audited rollback logical action (design §4,
    /// §14.5; OSC-006.3–.5, OSC-028.7):
    ///
    /// 1. **Validate the opaque token** (expiry, session scope, action linkage,
    ///    capability ownership) *before* touching the provider. A mismatched or
    ///    expired token is a pre-rollback [`OsControlError`] that performs **no**
    ///    compensation.
    /// 2. **Seal the rollback's own mutation context** — the rollback passes
    ///    through the same grant/lease/audit-admission policy as a forward
    ///    mutation (a separate admitted action linked to the original receipt).
    /// 3. **Dispatch `provider.rollback` exactly once**, then verify the restore
    ///    against the captured prior state with fresh evidence.
    ///
    /// The forward mutation is never redispatched. The returned
    /// [`RollbackReceipt`] links to the original receipt and never overwrites it,
    /// so the original failure and any rollback failure are preserved separately.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_rollback<R, O, P>(
        &self,
        provider: &P,
        observation: &HostExecutionContext,
        grant: &ExecutionGrant,
        lease_set: &AcquiredResourceLeaseSet,
        audit_admission: &AuditAdmissionToken,
        binding: &SealBinding<'_>,
        request: &R,
        prior_state: &O,
        token: &RollbackToken,
        plan: &RollbackExecPlan,
        audit_completion: AuditCompletionState,
    ) -> Result<RollbackReceipt<O>, OsControlError>
    where
        R: Send + Sync,
        O: NormalizedObservation + Clone + Send + Sync,
        P: DesiredStateControl<R, O> + ?Sized,
    {
        // (1) Pre-rollback token validation. No provider call, no compensation.
        token
            .validate(
                std::time::SystemTime::now(),
                binding.session_id,
                &plan.original_action_hash,
                &plan.capability,
            )
            .map_err(Self::token_rejection_error)?;

        // (2) Seal the rollback's own mutation context (policy/resource/audit).
        //     Any binding mismatch returns a pre-rollback error with no dispatch.
        let ctx =
            self.seal_mutation_context(observation, grant, lease_set, audit_admission, binding)?;

        let auth = self.seal_authority();
        let build = |outcome: RollbackOutcome<O>| {
            RollbackReceipt::new(
                &auth,
                plan.rollback_receipt_id.clone(),
                plan.linked_receipt.clone(),
                plan.capability.clone(),
                outcome,
                audit_completion.clone(),
                plan.latency_ms,
            )
        };

        // (3) Dispatch the inverse exactly once. `Err` is proven-no-effect.
        let dispatched = provider.rollback(&ctx, token).await?;

        match dispatched {
            // The inverse dispatched (or was accepted): verify the restore.
            ApplyOutcome::Applied(_) | ApplyOutcome::Accepted(_) => {
                match provider.verify(observation, request, prior_state).await {
                    Ok(VerificationReport::Satisfied(v))
                        if evidence_is_fresh(v.freshness_ms(), plan.deadline_ms) =>
                    {
                        let observation = v.observation().clone();
                        Ok(build(RollbackOutcome::Restored {
                            observation,
                            verification: v,
                        }))
                    }
                    // Fresh evidence says the prior state was NOT restored.
                    Ok(VerificationReport::Contradicted(_)) => {
                        Ok(build(RollbackOutcome::Failed(RollbackFailure::new(
                            SafeErrorCode::from_static(
                                "os_control.incident.rollback_restore_contradicted",
                            ),
                            None,
                        ))))
                    }
                    // Stale-but-satisfying, inconclusive, or unavailable evidence.
                    _ => Ok(build(RollbackOutcome::Unverified {
                        cause: UnverifiedCause::NoDecisiveObservation,
                    })),
                }
            }
            // The inverse itself was uncertain — no restore may be claimed.
            ApplyOutcome::Uncertain(_) => Ok(build(RollbackOutcome::Failed(RollbackFailure::new(
                SafeErrorCode::from_static("os_control.incident.rollback_uncertain"),
                None,
            )))),
            // A rollback that itself left partial residue is a failed rollback.
            ApplyOutcome::PartiallyApplied(_) => {
                Ok(build(RollbackOutcome::Failed(RollbackFailure::new(
                    SafeErrorCode::from_static("os_control.incident.rollback_partial"),
                    None,
                ))))
            }
        }
    }

    /// Compensate a multi-step partial effect in **reverse order** (Task 1.9,
    /// OSC-006.7/OSC-028). Only completed steps declared reversible by the
    /// compensator are compensated; non-reversible completed work is left in
    /// place and reported. Each step is compensated **at most once** (single
    /// reverse pass), and the first compensation failure stops the pass so
    /// partial progress is reported precisely. The forward `PartialDispatch`
    /// (the child receipt evidence) is never mutated.
    pub async fn compensate_partial<C>(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        compensator: &C,
        partial: &PartialDispatch,
    ) -> CompensationReport
    where
        C: StepCompensator + ?Sized,
    {
        // Completed steps in application order (head first), then reversed so we
        // compensate the most-recently-applied reversible step first.
        let steps = partial.completed_steps();
        let mut ordered: Vec<SafeStepId> = Vec::with_capacity(steps.len());
        ordered.push(steps.head().clone());
        ordered.extend(steps.tail().iter().cloned());

        let mut report = CompensationReport::with_cap(steps.len());
        for step in ordered.into_iter().rev() {
            if !compensator.is_reversible(&step) {
                report.record_skipped(step);
                continue;
            }
            match compensator.compensate_step(ctx, &step).await {
                Ok(()) => report.record_compensated(step),
                Err(err) => {
                    // Report partial completion precisely and stop; the remaining
                    // (earlier) steps are neither compensated nor claimed.
                    report.record_failure(step, SafeErrorCode::from_static(err.code()));
                    break;
                }
            }
        }
        report
    }
}

impl Default for OsControlRuntime {
    fn default() -> Self {
        Self::detached()
    }
}

// Compile-time proof the seam is thread-safe (design §18 `Send + Sync` surface).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OsControlRuntime>();
};

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::testing::FakeHostOsControl;

    #[test]
    fn detached_runtime_returns_unavailable_and_no_provider() {
        let rt = OsControlRuntime::detached();
        assert!(!rt.provider_present());
        assert!(rt.provider_id().is_none());
        let err = rt.probe_provider("set_volume").unwrap_err();
        assert_eq!(err.code(), "os_control.unavailable");
        // The frozen envelope names no provider and no LocalEnvironment fallback.
        let env = err.to_envelope();
        assert!(env["os_control"]["provider"].is_null());
        assert_eq!(env["os_control"]["availability"], "unavailable");
    }

    #[test]
    fn composed_runtime_routes_through_fake_and_records_calls() {
        let fake = Arc::new(FakeHostOsControl::new("pipewire"));
        let recorder = fake.recorder();
        let rt = OsControlRuntime::with_host(fake);

        assert!(rt.provider_present());
        let id = rt.probe_provider("set_volume").expect("composed provider");
        assert_eq!(id.as_str(), "pipewire");
        // The fake recorded the governed call — proving fake-testability through
        // the runtime with no live transport.
        assert_eq!(recorder.labels(), vec!["provider_id".to_string()]);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 1.7 code-level validation: sealing, verification predicates, terminal
// receipt construction, forbidden cross-product, and forge-resistance.
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(all(test, feature = "os-control-test"))]
mod seal_tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime};

    use tokio_util::sync::CancellationToken;

    use crate::agent::execution_gate::OsActionGrant;
    use crate::agent::resource_lease::ResourceRequirement;
    use crate::os_control::context::{
        AuditAdmissionToken, HostExecutionContext, RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{
        ActionId, AuditAdmissionId, AuditRecoveryKey, BoundedVec, CorrelationId, GrantNonce,
        NonEmptyBoundedVec, ReceiptId, SafeStepId, SessionId, VerificationReliability,
    };
    use crate::os_control::receipt::{
        AcceptanceEvidence, AcceptedDispatch, ActionLifecycle, AppliedDispatch, PartialDispatch,
        PartialEffectCause, RedactedObservation, SatisfyingVerification, UncertainDispatch,
        UncertainEffectCause,
    };
    use crate::os_control::redaction::parameter_digest;
    use crate::os_control::resource::{
        write_resource_set_digest, AcquiredResourceLeaseSet, OsResourceKind,
    };
    use crate::safety::RiskLevel;

    const SESSION: &str = "sess-1";
    const ACTION: &str = "set_volume";

    fn params() -> serde_json::Value {
        serde_json::json!({ "level": 40 })
    }

    fn reqs() -> Vec<ResourceRequirement> {
        crate::os_control::resource::os_write_requirements(ACTION, &params())
    }

    // ── Normalized observation used by the fake provider ────────────────────
    #[derive(Debug, Clone, PartialEq)]
    struct TestObs {
        tag: String,
        value: Option<f64>,
    }

    impl TestObs {
        fn new(tag: &str, value: Option<f64>) -> Self {
            Self {
                tag: tag.to_string(),
                value,
            }
        }
    }

    impl NormalizedObservation for TestObs {
        fn observation_digest(&self) -> Digest {
            Digest::of_str(&self.tag)
        }
        fn numeric_value(&self) -> Option<f64> {
            self.value
        }
    }

    fn robs(tag: &str) -> RedactedObservation<TestObs> {
        let o = TestObs::new(tag, None);
        RedactedObservation::new(o.clone(), o.observation_digest())
    }

    fn satisfying(freshness_ms: u64) -> VerificationReport<TestObs> {
        VerificationReport::Satisfied(SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            ProviderId::new("fake"),
            robs("verified-after"),
            None,
            SystemTime::now(),
            freshness_ms,
        ))
    }

    fn contradicted() -> VerificationReport<TestObs> {
        VerificationReport::Contradicted(VerificationContradiction::new(
            Digest::of_str("expected"),
            Some(Digest::of_str("observed")),
            SafeErrorCode::from_static("os_control.incident.contradicted"),
        ))
    }

    fn inconclusive() -> VerificationReport<TestObs> {
        VerificationReport::Inconclusive {
            reason: SafeText::new("no decisive evidence"),
        }
    }

    // ── Scripted fake provider (records the governed call order) ─────────────
    struct ScriptedProvider {
        recorder: crate::os_control::testing::CallRecorder,
        observe: Mutex<VecDeque<Result<TestObs, ()>>>,
        apply: Mutex<VecDeque<Result<ApplyOutcome, ()>>>,
        verify: Mutex<VecDeque<Result<VerificationReport<TestObs>, ()>>>,
        rollback: Mutex<VecDeque<Result<ApplyOutcome, ()>>>,
    }

    impl ScriptedProvider {
        fn new() -> Self {
            Self {
                recorder: crate::os_control::testing::CallRecorder::new(),
                observe: Mutex::new(VecDeque::new()),
                apply: Mutex::new(VecDeque::new()),
                verify: Mutex::new(VecDeque::new()),
                rollback: Mutex::new(VecDeque::new()),
            }
        }
        fn observe_ok(self, tag: &str) -> Self {
            self.observe
                .lock()
                .unwrap()
                .push_back(Ok(TestObs::new(tag, None)));
            self
        }
        fn observe_err(self) -> Self {
            self.observe.lock().unwrap().push_back(Err(()));
            self
        }
        fn apply(self, outcome: ApplyOutcome) -> Self {
            self.apply.lock().unwrap().push_back(Ok(outcome));
            self
        }
        fn apply_err(self) -> Self {
            self.apply.lock().unwrap().push_back(Err(()));
            self
        }
        fn verify(self, report: VerificationReport<TestObs>) -> Self {
            self.verify.lock().unwrap().push_back(Ok(report));
            self
        }
        fn rollback(self, outcome: ApplyOutcome) -> Self {
            self.rollback.lock().unwrap().push_back(Ok(outcome));
            self
        }
        #[allow(dead_code)] // test-only seam
        fn labels(&self) -> Vec<String> {
            self.recorder.labels()
        }
        fn count(&self, label: &str) -> usize {
            self.recorder
                .labels()
                .iter()
                .filter(|l| *l == label)
                .count()
        }
    }

    #[async_trait::async_trait]
    impl DesiredStateControl<(), TestObs> for ScriptedProvider {
        async fn observe(
            &self,
            _ctx: &HostExecutionContext,
            _request: &(),
        ) -> Result<TestObs, OsControlError> {
            self.recorder.record("observe");
            match self.observe.lock().unwrap().pop_front() {
                Some(Ok(o)) => Ok(o),
                _ => Err(OsControlError::Unavailable {
                    provider: None,
                    reason: SafeText::new("no scripted observe"),
                    retryable: false,
                }),
            }
        }
        async fn apply(
            &self,
            _ctx: &AdmittedMutationContext<'_>,
            _request: &(),
            _desired: &TestObs,
        ) -> Result<ApplyOutcome, OsControlError> {
            self.recorder.record("apply");
            match self.apply.lock().unwrap().pop_front() {
                Some(Ok(o)) => Ok(o),
                _ => Err(OsControlError::CancelledBeforeMutation),
            }
        }
        async fn verify(
            &self,
            _ctx: &HostExecutionContext,
            _request: &(),
            _desired: &TestObs,
        ) -> Result<VerificationReport<TestObs>, OsControlError> {
            self.recorder.record("verify");
            match self.verify.lock().unwrap().pop_front() {
                Some(Ok(r)) => Ok(r),
                _ => Err(OsControlError::Unavailable {
                    provider: None,
                    reason: SafeText::new("no scripted verify"),
                    retryable: false,
                }),
            }
        }
        async fn rollback(
            &self,
            _ctx: &AdmittedMutationContext<'_>,
            _token: &RollbackToken,
        ) -> Result<ApplyOutcome, OsControlError> {
            self.recorder.record("rollback");
            match self.rollback.lock().unwrap().pop_front() {
                Some(Ok(o)) => Ok(o),
                _ => Err(OsControlError::CancelledBeforeMutation),
            }
        }
    }

    // ── Authority fixture: builds a matching grant/observation/lease/token ───
    struct Fixture {
        grant: OsActionGrant,
        host_ctx: HostExecutionContext,
        lease_set: AcquiredResourceLeaseSet,
        token: AuditAdmissionToken,
        reqs: Vec<ResourceRequirement>,
    }

    impl Fixture {
        fn build(expired: bool) -> Self {
            let p = params();
            let reqs = reqs();
            let grant = if expired {
                OsActionGrant::for_test_expired(
                    SESSION,
                    ACTION,
                    &p,
                    ExecutionTarget::Host,
                    &reqs,
                    RiskLevel::Yellow,
                )
            } else {
                OsActionGrant::for_test(
                    SESSION,
                    ACTION,
                    &p,
                    ExecutionTarget::Host,
                    &reqs,
                    RiskLevel::Yellow,
                )
            };
            let resource_digest = write_resource_set_digest(ACTION, &p);
            let token = AuditAdmissionToken::seal(
                AuditAdmissionId::new("adm-1"),
                AuditRecoveryKey::new("rk-1"),
                SessionId::new(SESSION),
                Digest::of_str(ACTION),
                parameter_digest(&p),
                Digest::of_str("host"),
                SnapshotRevision(1),
                resource_digest.clone(),
            );
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-1"),
                ActionId::new("act-1"),
                token.observation_authority(),
                Arc::new(SessionContext::new(SessionId::new(SESSION))),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(30),
                RedactionPolicy::default(),
            );
            let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest);
            Self {
                grant,
                host_ctx,
                lease_set,
                token,
                reqs,
            }
        }

        fn ok() -> Self {
            Self::build(false)
        }

        fn binding<'b>(
            &'b self,
            session: &'b str,
            revision: SnapshotRevision,
            params: &'b serde_json::Value,
        ) -> SealBinding<'b> {
            SealBinding {
                session_id: session,
                action: ACTION,
                params,
                target: ExecutionTarget::Host,
                resource_requirements: &self.reqs,
                capability_snapshot_revision: revision,
            }
        }
    }

    fn plan(rollback: RollbackPlan) -> MutationPlan {
        MutationPlan {
            receipt_id: ReceiptId::new("r-1"),
            provider: ProviderId::new("fake"),
            comparator: ComparatorKind::Exact,
            tolerance: None,
            deadline_ms: 500,
            rollback,
            latency_ms: 5,
        }
    }

    fn recorded() -> AuditCompletionState {
        AuditCompletionState::Recorded {
            record_id: crate::os_control::contract::AuditRecordId::new("rec-1"),
        }
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::new(
            Digest::of_str("tok"),
            SessionId::new(SESSION),
            Digest::of_str(ACTION),
            ProviderId::new("fake"),
            ReceiptId::new("r-1"),
            GrantNonce::new("n"),
            SystemTime::now() + Duration::from_secs(60),
        )
    }

    // ── Sealing: success and the forbidden mismatch cross-product ────────────

    #[test]
    fn seal_succeeds_when_every_binding_matches() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let ctx = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .expect("seal should succeed when all authorities agree");
        assert_eq!(ctx.grant().action(), ACTION);
    }

    #[test]
    fn seal_rejects_expired_grant() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::build(true);
        let p = params();
        let err = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.approval_expired");
    }

    #[test]
    fn seal_rejects_non_host_binding_target() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let mut binding = fx.binding(SESSION, SnapshotRevision(1), &p);
        binding.target = ExecutionTarget::Vm;
        let err = rt
            .seal_mutation_context(&fx.host_ctx, &fx.grant, &fx.lease_set, &fx.token, &binding)
            .unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
    }

    #[test]
    fn seal_rejects_session_mismatch() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let err = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding("other-session", SnapshotRevision(1), &p),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
    }

    #[test]
    fn seal_rejects_parameter_mismatch() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        // Different live params → grant.matches fails AND admission param digest
        // fails. No mutation context is produced.
        let other = serde_json::json!({ "level": 99 });
        let err = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &other),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
    }

    #[test]
    fn seal_rejects_stale_capability_revision() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let err = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(2), &p),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
    }

    #[test]
    fn seal_rejects_unheld_resource_set() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        // A lease set for a *different* resource digest is not the named held set.
        let wrong_lease = AcquiredResourceLeaseSet::for_test(Digest::of_str("different-set"));
        let err = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &wrong_lease,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.resource_busy");
    }

    #[test]
    fn seal_rejects_observation_from_a_different_admission() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        // An observation context lent from a *different* admission token.
        let other_token = AuditAdmissionToken::seal(
            AuditAdmissionId::new("adm-OTHER"),
            AuditRecoveryKey::new("rk-1"),
            SessionId::new(SESSION),
            Digest::of_str(ACTION),
            parameter_digest(&p),
            Digest::of_str("host"),
            SnapshotRevision(1),
            write_resource_set_digest(ACTION, &p),
        );
        let foreign_ctx = HostExecutionContext::for_test(
            CorrelationId::new("corr-x"),
            ActionId::new("act-x"),
            other_token.observation_authority(),
            Arc::new(SessionContext::new(SessionId::new(SESSION))),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            RedactionPolicy::default(),
        );
        let err = rt
            .seal_mutation_context(
                &foreign_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
    }

    // ── run_mutation: idempotency and no-provider-mutation guarantees ────────

    #[tokio::test]
    async fn idempotent_state_returns_unchanged_and_never_applies() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let desired = TestObs::new("already", None);
        let provider = ScriptedProvider::new().observe_ok("already"); // before == desired
        let receipt = rt
            .run_mutation(
                &provider,
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
                &(),
                &desired,
                &plan(RollbackPlan::Unavailable),
                recorded(),
            )
            .await
            .expect("unchanged receipt");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
        assert_eq!(provider.count("apply"), 0, "no apply on already-satisfied");
    }

    #[tokio::test]
    async fn invalid_binding_calls_no_provider_mutation() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let desired = TestObs::new("desired", None);
        // before != desired → proceeds to seal, which fails on session mismatch.
        let provider = ScriptedProvider::new().observe_ok("before");
        let err = rt
            .run_mutation(
                &provider,
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding("wrong-session", SnapshotRevision(1), &p),
                &(),
                &desired,
                &plan(RollbackPlan::Unavailable),
                recorded(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
        assert_eq!(provider.count("apply"), 0, "seal failure must not apply");
        assert_eq!(provider.count("rollback"), 0);
    }

    #[tokio::test]
    async fn converged_after_lease_reobservation_is_unchanged() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let desired = TestObs::new("desired", None);
        // before != desired (proceeds), re-observe == desired (converged).
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("desired");
        let receipt = rt
            .run_mutation(
                &provider,
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
                &(),
                &desired,
                &plan(RollbackPlan::Unavailable),
                recorded(),
            )
            .await
            .expect("unchanged after convergence");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unchanged);
        assert_eq!(provider.count("apply"), 0);
    }

    // ── run_mutation: every terminal state is reachable ──────────────────────

    async fn run(provider: &ScriptedProvider, plan: &MutationPlan) -> MutationResult<TestObs> {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let desired = TestObs::new("desired", None);
        rt.run_mutation(
            provider,
            &fx.host_ctx,
            &fx.grant,
            &fx.lease_set,
            &fx.token,
            &fx.binding(SESSION, SnapshotRevision(1), &p),
            &(),
            &desired,
            plan,
            recorded(),
        )
        .await
    }

    fn applied() -> ApplyOutcome {
        ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
    }

    #[tokio::test]
    async fn applied_with_fresh_satisfying_evidence_is_verified() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(satisfying(10));
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("verified");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Verified);
        assert!(receipt.verification().is_some());
        // apply exactly once, verify not retried.
        assert_eq!(provider.count("apply"), 1);
        assert_eq!(provider.count("verify"), 1);
    }

    #[tokio::test]
    async fn applied_with_stale_evidence_is_unverified() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(satisfying(5_000)); // freshness far beyond the 500ms deadline
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("unverified");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unverified);
    }

    #[tokio::test]
    async fn applied_with_inconclusive_verification_is_unverified() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(inconclusive());
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("unverified");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unverified);
    }

    #[tokio::test]
    async fn applied_but_after_observation_unavailable_is_unverified() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .apply(applied())
            .observe_err(); // the post-apply re-observation is unavailable
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("unverified");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unverified);
        // Verification is never attempted when the after-observation is missing.
        assert_eq!(provider.count("verify"), 0);
    }

    #[tokio::test]
    async fn accepted_dispatch_yields_accepted_without_verifying() {
        let accepted = ApplyOutcome::Accepted(AcceptedDispatch::new(
            None,
            AcceptanceEvidence {
                detail: SafeText::new("logind accepted"),
                accepted_at: SystemTime::now(),
            },
            BoundedVec::new(),
        ));
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .apply(accepted);
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("accepted");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Accepted);
        assert_eq!(provider.count("verify"), 0, "accepted never verifies");
    }

    #[tokio::test]
    async fn uncertain_dispatch_with_timeout_is_unverified_with_incident() {
        let uncertain = ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::TimedOutAfterDispatch,
            BoundedVec::new(),
        ));
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(uncertain)
            .verify(inconclusive());
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("unverified");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::Unverified);
        let codes: Vec<String> = receipt
            .safe_summary()
            .incident_codes()
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
        assert!(codes.iter().any(|c| c.contains("timed_out_after_dispatch")));
    }

    #[tokio::test]
    async fn uncertain_contradiction_is_verification_failed() {
        let uncertain = ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::TransportLostAfterDispatch,
            BoundedVec::new(),
        ));
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(uncertain)
            .verify(contradicted());
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("verification failed");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::VerificationFailed);
    }

    #[tokio::test]
    async fn partial_dispatch_yields_partially_applied() {
        let partial = ApplyOutcome::PartiallyApplied(PartialDispatch::new(
            None,
            NonEmptyBoundedVec::single(SafeStepId::new("step-1")),
            SafeStepId::new("step-2"),
            PartialEffectCause::StepFailedAfterCommit,
            BoundedVec::new(),
        ));
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .apply(partial);
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("partial");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::PartiallyApplied);
        assert_eq!(provider.count("verify"), 0);
    }

    #[tokio::test]
    async fn contradiction_without_rollback_is_verification_failed() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(contradicted());
        let receipt = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .expect("verification failed");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::VerificationFailed);
        assert_eq!(provider.count("rollback"), 0);
    }

    #[tokio::test]
    async fn contradiction_with_verified_rollback_is_rolled_back() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(contradicted()) // forward verify contradicts
            .rollback(applied()) // rollback dispatch succeeds
            .verify(satisfying(10)); // rollback restore verified fresh
        let receipt = run(
            &provider,
            &plan(RollbackPlan::Available {
                token: rollback_token(),
                auto: true,
            }),
        )
        .await
        .expect("rolled back");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::RolledBack);
        assert!(!receipt.changed(), "successful rollback is net-unchanged");
        assert_eq!(provider.count("rollback"), 1, "rollback attempted once");
        assert_eq!(
            provider.count("apply"),
            1,
            "forward mutation never redispatched"
        );
    }

    #[tokio::test]
    async fn contradiction_with_failed_rollback_stays_verification_failed() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .observe_ok("after")
            .apply(applied())
            .verify(contradicted())
            .rollback(applied())
            .verify(contradicted()); // rollback restore NOT verified
        let receipt = run(
            &provider,
            &plan(RollbackPlan::Available {
                token: rollback_token(),
                auto: true,
            }),
        )
        .await
        .expect("verification failed with failed rollback");
        assert_eq!(receipt.lifecycle(), ActionLifecycle::VerificationFailed);
        // Forward mutation applied exactly once; rollback attempted exactly once.
        assert_eq!(provider.count("apply"), 1);
        assert_eq!(provider.count("rollback"), 1);
    }

    #[tokio::test]
    async fn pre_dispatch_apply_error_produces_no_receipt() {
        let provider = ScriptedProvider::new()
            .observe_ok("before")
            .observe_ok("mid")
            .apply_err();
        let err = run(&provider, &plan(RollbackPlan::Unavailable))
            .await
            .unwrap_err();
        // A proven-no-effect apply error is a pre-mutation OsControlError.
        assert_eq!(err.code(), "os_control.cancelled_before_mutation");
        assert_eq!(provider.count("verify"), 0);
    }

    // ── Verification predicates ──────────────────────────────────────────────

    #[test]
    fn exact_comparator_uses_digest_equality() {
        let d = TestObs::new("x", None);
        assert!(observation_satisfies(
            ComparatorKind::Exact,
            None,
            &d,
            &TestObs::new("x", None)
        ));
        assert!(!observation_satisfies(
            ComparatorKind::Exact,
            None,
            &d,
            &TestObs::new("y", None)
        ));
    }

    #[test]
    fn tolerance_comparator_respects_absolute_delta() {
        let desired = TestObs::new("v40", Some(40.0));
        let tol = Some(Tolerance { abs: 2.0 });
        assert!(observation_satisfies(
            ComparatorKind::WithinTolerance,
            tol,
            &desired,
            &TestObs::new("v41", Some(41.0))
        ));
        assert!(!observation_satisfies(
            ComparatorKind::WithinTolerance,
            tol,
            &desired,
            &TestObs::new("v45", Some(45.0))
        ));
    }

    #[test]
    fn freshness_respects_the_deadline() {
        assert!(evidence_is_fresh(100, 500));
        assert!(evidence_is_fresh(500, 500));
        assert!(!evidence_is_fresh(501, 500));
    }

    #[test]
    fn shell_output_never_outranks_authoritative_state() {
        let strongest = strongest_os_evidence(&[
            OsEvidenceSource::StructuredCommandQuery, // "shell output"
            OsEvidenceSource::AuthoritativeServiceState,
            OsEvidenceSource::UserAttestation,
        ]);
        assert_eq!(strongest, Some(OsEvidenceSource::AuthoritativeServiceState));
        // The OS-state ranking is strictly ordered (unlike the GUI ranking that
        // ties filesystem and shell at 100).
        use crate::agent::execution_verifier::os_control_authority_rank;
        assert!(
            os_control_authority_rank(OsEvidenceSource::AuthoritativeServiceState)
                > os_control_authority_rank(OsEvidenceSource::StructuredCommandQuery)
        );
    }

    // ── Source-enumeration: mutating ports require &AdmittedMutationContext,
    //    reads require &HostExecutionContext, and contexts cannot be forged. ──

    fn read_src(rel: &str) -> String {
        let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel);
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("read {path}"))
    }

    /// Collapse all runs of ASCII whitespace to single spaces so signature
    /// assertions are robust to formatting/rustfmt line breaks.
    fn flatten_ws(src: &str) -> String {
        src.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn desired_state_control_signatures_enforce_context_split() {
        let src = flatten_ws(&read_src("src/os_control/contract.rs"));
        // Reads take the observation-only context.
        assert!(src.contains("async fn observe(&self, ctx: &HostExecutionContext"));
        assert!(src.contains("async fn verify( &self, ctx: &HostExecutionContext"));
        // Mutators take the sealed mutation context (observation-only would be
        // rejected by the compiler on any mutating port).
        assert!(src.contains("async fn apply( &self, ctx: &AdmittedMutationContext<'_>"));
        assert!(src.contains("async fn rollback( &self, ctx: &AdmittedMutationContext<'_>"));
    }

    #[test]
    fn existing_mutating_ports_consume_admitted_context() {
        // The two shipped mutating dispatchers both take a borrowed sealed context.
        let structured = flatten_ws(&read_src("src/os_control/linux/structured_command.rs"));
        assert!(structured.contains("pub fn from_admitted( ctx: &AdmittedMutationContext<'_>"));
        let broker = flatten_ws(&read_src("src/os_control/broker/client.rs"));
        assert!(broker.contains("ctx: &AdmittedMutationContext<'_>"));
    }

    #[test]
    fn mutation_context_and_permit_have_no_public_constructor() {
        let src = flatten_ws(&read_src("src/os_control/context.rs"));
        // No public constructor for either sealed type: only `pub(crate) fn seal`
        // (runtime witness) and `#[cfg(os-control-test)] pub fn for_test`.
        assert!(!src.contains("AdmittedMutationContext<'a> { pub fn new"));
        assert!(!src.contains("MutationPermit<'a> { pub fn new"));
        assert!(src.contains("pub(crate) fn seal( _authority: &RuntimeSealAuthority"));
    }

    #[test]
    fn runtime_seal_authority_field_is_private() {
        // The witness's only field is a private unit, so it cannot be constructed
        // outside `os_control::runtime` — the compile-fail doctests on the type
        // prove the negative; here we assert the shape textually.
        let src = read_src("src/os_control/runtime.rs");
        assert!(src.contains("pub struct RuntimeSealAuthority(());"));
    }

    #[test]
    fn every_mutating_tool_maps_to_a_typed_write_resource_kind() {
        // Sanity check that the seal fixture's tool derives a typed write set
        // (so the resource-digest triangle in the seal is meaningful).
        assert!(
            !reqs().is_empty(),
            "set_volume must declare a write resource"
        );
        assert!(OsResourceKind::from_token("audio-state").is_some());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Task 1.9 code-level validation: rollback coordinator + compensation
    // ─────────────────────────────────────────────────────────────────────────

    fn rollback_exec_plan() -> RollbackExecPlan {
        RollbackExecPlan {
            rollback_receipt_id: ReceiptId::new("rb-1"),
            linked_receipt: ReceiptId::new("r-orig"),
            original_action_hash: Digest::of_str(ACTION),
            capability: ProviderId::new("fake"),
            comparator: ComparatorKind::Exact,
            tolerance: None,
            deadline_ms: 500,
            latency_ms: 5,
        }
    }

    fn token_with(action: &str, cap: &str, expires_in_secs: i64) -> RollbackToken {
        let now = SystemTime::now();
        let expires = if expires_in_secs >= 0 {
            now + Duration::from_secs(expires_in_secs as u64)
        } else {
            now - Duration::from_secs((-expires_in_secs) as u64)
        };
        RollbackToken::new(
            Digest::of_str("tok"),
            SessionId::new(SESSION),
            Digest::of_str(action),
            ProviderId::new(cap),
            ReceiptId::new("r-orig"),
            GrantNonce::new("n"),
            expires,
        )
    }

    async fn run_rb(
        provider: &ScriptedProvider,
        token: &RollbackToken,
        plan: &RollbackExecPlan,
    ) -> Result<RollbackReceipt<TestObs>, OsControlError> {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let prior = TestObs::new("prior", None);
        rt.run_rollback(
            provider,
            &fx.host_ctx,
            &fx.grant,
            &fx.lease_set,
            &fx.token,
            &fx.binding(SESSION, SnapshotRevision(1), &p),
            &(),
            &prior,
            token,
            plan,
            recorded(),
        )
        .await
    }

    #[tokio::test]
    async fn rollback_expired_token_is_pre_rollback_error_and_no_compensation() {
        let provider = ScriptedProvider::new()
            .rollback(applied())
            .verify(satisfying(10));
        let err = run_rb(
            &provider,
            &token_with(ACTION, "fake", -1),
            &rollback_exec_plan(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "os_control.approval_expired");
        assert_eq!(
            provider.count("rollback"),
            0,
            "expired token must perform no compensation"
        );
        assert_eq!(provider.count("verify"), 0);
    }

    #[tokio::test]
    async fn rollback_action_mismatch_token_is_rejected_before_provider() {
        let provider = ScriptedProvider::new()
            .rollback(applied())
            .verify(satisfying(10));
        // Token linked to a *different* action than the one being undone.
        let err = run_rb(
            &provider,
            &token_with("kill_process", "fake", 60),
            &rollback_exec_plan(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
        assert_eq!(provider.count("rollback"), 0, "no compensation on mismatch");
    }

    #[tokio::test]
    async fn rollback_capability_mismatch_token_is_rejected_before_provider() {
        let provider = ScriptedProvider::new()
            .rollback(applied())
            .verify(satisfying(10));
        // Token owned by a *different* capability than the operation's provider.
        let err = run_rb(
            &provider,
            &token_with(ACTION, "logind", 60),
            &rollback_exec_plan(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
        assert_eq!(provider.count("rollback"), 0);
    }

    #[tokio::test]
    async fn rollback_with_verified_restore_is_restored_and_linked() {
        let provider = ScriptedProvider::new()
            .rollback(applied()) // inverse dispatches
            .verify(satisfying(10)); // restore verified fresh
        let receipt = run_rb(
            &provider,
            &token_with(ACTION, "fake", 60),
            &rollback_exec_plan(),
        )
        .await
        .expect("rollback receipt");
        assert!(receipt.succeeded(), "prior state verifiably restored");
        assert!(receipt.verification().is_some());
        // Rollback is a separate action linked to the original receipt.
        assert_eq!(receipt.linked_receipt().as_str(), "r-orig");
        assert_eq!(receipt.rollback_receipt_id().as_str(), "rb-1");
        assert_eq!(receipt.safe_summary().status(), "restored");
        assert_eq!(provider.count("rollback"), 1, "inverse dispatched once");
    }

    #[tokio::test]
    async fn rollback_restore_contradiction_is_failed_and_preserves_separation() {
        let provider = ScriptedProvider::new()
            .rollback(applied())
            .verify(contradicted()); // fresh evidence: prior state NOT restored
        let receipt = run_rb(
            &provider,
            &token_with(ACTION, "fake", 60),
            &rollback_exec_plan(),
        )
        .await
        .expect("rollback receipt");
        assert!(!receipt.succeeded());
        match receipt.outcome() {
            RollbackOutcome::Failed(_) => {}
            other => panic!("expected Failed, got {other:?}"),
        }
        // The rollback receipt is distinct from the original (kept separate).
        assert_eq!(receipt.linked_receipt().as_str(), "r-orig");
        assert_eq!(receipt.safe_summary().status(), "failed");
        assert!(receipt.safe_summary().incident_code().is_some());
    }

    #[tokio::test]
    async fn rollback_uncertain_dispatch_is_failed() {
        let uncertain = ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::TransportLostAfterDispatch,
            BoundedVec::new(),
        ));
        let provider = ScriptedProvider::new().rollback(uncertain);
        let receipt = run_rb(
            &provider,
            &token_with(ACTION, "fake", 60),
            &rollback_exec_plan(),
        )
        .await
        .expect("rollback receipt");
        assert!(!receipt.succeeded());
        assert_eq!(
            provider.count("verify"),
            0,
            "uncertain rollback never verifies"
        );
    }

    // ── Non-reversible declarations (OSC-006.6) ──────────────────────────────

    #[test]
    fn non_reversible_claims_never_advertise_rollback() {
        assert!(!rollback_claim_advertisable(RollbackClaim::NoRollback));
        assert!(rollback_claim_advertisable(RollbackClaim::UserRequestable));
        assert!(rollback_claim_advertisable(RollbackClaim::Automatic));
        assert!(rollback_claim_advertisable(RollbackClaim::CompensationOnly));

        // Even if a provider mistakenly minted a token, a `None` claim (kill /
        // permanent delete / shutdown / reboot / update) forces Unavailable.
        let available = RollbackPlan::Available {
            token: rollback_token(),
            auto: false,
        };
        assert!(matches!(
            reconcile_rollback_availability(RollbackClaim::NoRollback, &available),
            RollbackAvailability::Unavailable
        ));
        // A reversible claim honors the provider's captured-state disposition.
        assert!(matches!(
            reconcile_rollback_availability(RollbackClaim::UserRequestable, &available),
            RollbackAvailability::Available(_)
        ));
        assert!(matches!(
            reconcile_rollback_availability(
                RollbackClaim::UserRequestable,
                &RollbackPlan::Unavailable
            ),
            RollbackAvailability::Unavailable
        ));
    }

    // ── Reverse-order multi-step compensation (OSC-006.7 / OSC-028) ──────────

    struct FakeCompensator {
        reversible: std::collections::HashSet<String>,
        fail_on: Option<String>,
        recorder: crate::os_control::testing::CallRecorder,
    }

    impl FakeCompensator {
        fn new(reversible: &[&str], fail_on: Option<&str>) -> Self {
            Self {
                reversible: reversible.iter().map(|s| (*s).to_string()).collect(),
                fail_on: fail_on.map(str::to_string),
                recorder: crate::os_control::testing::CallRecorder::new(),
            }
        }
    }

    #[async_trait::async_trait]
    impl StepCompensator for FakeCompensator {
        fn is_reversible(&self, step: &SafeStepId) -> bool {
            self.reversible.contains(step.as_str())
        }
        async fn compensate_step(
            &self,
            _ctx: &AdmittedMutationContext<'_>,
            step: &SafeStepId,
        ) -> Result<(), OsControlError> {
            self.recorder.record(step.as_str());
            if self.fail_on.as_deref() == Some(step.as_str()) {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            Ok(())
        }
    }

    fn partial_of(completed: &[&str], failed: &str) -> PartialDispatch {
        let mut steps = completed.iter();
        let head = SafeStepId::new(*steps.next().expect("at least one completed step"));
        let mut tail = BoundedVec::with_cap(16);
        for s in steps {
            tail.try_push(SafeStepId::new(*s));
        }
        PartialDispatch::new(
            None,
            NonEmptyBoundedVec::new(head, tail),
            SafeStepId::new(failed),
            PartialEffectCause::StepFailedAfterCommit,
            BoundedVec::new(),
        )
    }

    #[tokio::test]
    async fn compensation_runs_reversible_steps_in_reverse_order() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let ctx = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .expect("seal for compensation");

        // Completed in application order: s1, s2, s3 (s4 failed).
        let partial = partial_of(&["s1", "s2", "s3"], "s4");
        let comp = FakeCompensator::new(&["s1", "s2", "s3"], None);
        let report = rt.compensate_partial(&ctx, &comp, &partial).await;

        assert!(report.fully_compensated());
        // Compensated most-recently-applied first: s3, s2, s1.
        let comped: Vec<&str> = report.compensated().iter().map(|s| s.as_str()).collect();
        assert_eq!(comped, vec!["s3", "s2", "s1"]);
        // The compensator was invoked in that same reverse order.
        assert_eq!(comp.recorder.labels(), vec!["s3", "s2", "s1"]);
        assert!(report.skipped_irreversible().is_empty());
    }

    #[tokio::test]
    async fn compensation_skips_irreversible_and_stops_at_first_failure() {
        let rt = OsControlRuntime::detached();
        let fx = Fixture::ok();
        let p = params();
        let ctx = rt
            .seal_mutation_context(
                &fx.host_ctx,
                &fx.grant,
                &fx.lease_set,
                &fx.token,
                &fx.binding(SESSION, SnapshotRevision(1), &p),
            )
            .expect("seal for compensation");

        // s1,s2,s3,s4 completed. s3 is NOT reversible → skipped. s2 fails.
        let partial = partial_of(&["s1", "s2", "s3", "s4"], "s5");
        let comp = FakeCompensator::new(&["s1", "s2", "s4"], Some("s2"));
        let report = rt.compensate_partial(&ctx, &comp, &partial).await;

        // Reverse pass: s4 (comp ok), s3 (irreversible → skipped),
        // s2 (fails → stop). s1 is never reached.
        assert!(!report.fully_compensated());
        let comped: Vec<&str> = report.compensated().iter().map(|s| s.as_str()).collect();
        assert_eq!(comped, vec!["s4"]);
        let skipped: Vec<&str> = report
            .skipped_irreversible()
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(skipped, vec!["s3"]);
        assert_eq!(report.failed_step().map(|s| s.as_str()), Some("s2"));
        // s1 (applied before the failure) is neither compensated nor claimed.
        assert!(!comped.contains(&"s1"));
        // Each attempted step invoked at most once.
        assert_eq!(comp.recorder.labels(), vec!["s4", "s2"]);
    }
}
