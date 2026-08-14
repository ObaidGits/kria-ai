//! Storage domain: the `StorageControl` desired-state provider (design §3,
//! §9.1, §10.1 `StorageControl.{list,mount,unmount,eject,health}`).
//!
//! linux-os-control-production **Task 3.2** — "Complete storage and
//! removable-media lifecycle" (OSC-012, OSC-030).
//!
//! # Scope
//!
//! Adds typed discovery, mount, unmount, eject, and health reporting for
//! removable/fixed storage, with **no** path to raw device administration:
//!
//! * [`list_storage_devices`]/[`StorageDevicePage`] — a pure read (outside
//!   the mutation lifecycle, mirroring
//!   `ConnectivityControl::scan_wifi_networks`) returning stable UDisks2
//!   object-path-derived [`StorageDeviceId`]s, capacity/free bytes,
//!   filesystem type, mount state, and a removable flag. Never a raw
//!   `/dev/sdX` device-node string (OSC-012.2).
//! * [`mount_device`]/[`unmount_device`]/[`eject_device`] —
//!   [`DesiredStateControl`] mutations over [`StorageMountState`], a
//!   normalized observation binding the device (+ optional filesystem)
//!   identity to its current mount state. Mounting an already-mounted
//!   device (at any point, or at the caller-specified `filesystem` when
//!   given) is `Unchanged`; unmounting/ejecting an already-unmounted device
//!   is `Unchanged` — the runtime's idempotency short-circuit means neither
//!   ever dispatches (OSC-012.7's "verify mount topology after mutation" is
//!   the *runtime's* fresh re-observation on the mutating path; the
//!   `Unchanged` path never touches the device at all).
//! * A busy unmount/eject (an open file handle blocks the operation) is
//!   reported through the existing [`OsControlError::ResourceBusy`] variant
//!   (Task 1.1) — never a force-unmount. There is no `force` parameter on
//!   any tool in this module and no code path that could accept one
//!   (OSC-012.3, OSC-012.4).
//! * [`get_storage_health`]/[`StorageHealthReport`] — a pure read returning
//!   available SMART/health evidence as a distinct, honestly-reported
//!   [`HealthAvailability::Unavailable`] state when the device has no
//!   `DriveAta` interface, never a fabricated healthy/unhealthy status
//!   (OSC-012.5, OSC-031).
//! * **BLACK containment (OSC-012.6, OSC-030):** partition, format, resize,
//!   secure-erase, and encryption-provisioning are not merely undispatched
//!   here — there is no [`StorageOp`] variant, no tool, and no transport
//!   method in this module that could express them. They remain reachable
//!   only through the existing raw-shell BLACK-scope containment
//!   ([`crate::safety::black_scope`]), which already classifies
//!   `mkfs*`/`resize2fs`/`cryptsetup luksFormat`/`wipefs`/`fdisk`/`parted`
//!   as prohibited (Task 0.2). This module's completion proof is the
//!   *absence* of any such capability, not a redundant re-block of it.
//!
//! # Not the broker (OSC-012.2, design §12)
//!
//! Unlike [`crate::os_control::files::ownership`] (whose mutation is
//! dispatched exclusively through
//! `BrokerOperation::SetBoundPathOwnership`), storage mount/unmount/eject
//! use **UDisks2's own typed Polkit authorization**
//! (`org.freedesktop.udisks2.filesystem-mount`/`filesystem-unmount`/
//! `eject-media` policy actions) — design §12 states this explicitly:
//! "Mount/eject uses UDisks2's own typed Polkit authorization and is not
//! reconstructed in this broker." So [`StorageTransport::dispatch`] talks
//! directly to the (deny-live-gated) UDisks2 D-Bus adapter, never through
//! [`crate::os_control::broker::BrokerOperation`].
//!
//! # Why a `StorageTransport` seam (not a pure `std::fs` domain)
//!
//! Unlike [`crate::os_control::files`] (plain `std::fs` against an
//! injectable root), storage devices need genuinely fake-transport-testable
//! D-Bus semantics: a UDisks2 object tree (block/filesystem/drive
//! interfaces) and `PropertiesChanged` signals. This mirrors
//! [`crate::os_control::audio`]/[`crate::os_control::display`]/
//! [`crate::os_control::connectivity`]/[`crate::os_control::power`]: a
//! [`StorageTransport`] trait, [`fake::FakeStorageTransport`] for
//! completion tests, and a
//! [`crate::os_control::linux::providers::udisks::LiveUdisks`]
//! deny-live-gated stub that fails closed with
//! [`OsControlError::Unavailable`] — never a real D-Bus call in this task.

use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

pub mod selection;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

/// The stable provider identity for the UDisks2-backed storage transport.
pub const STORAGE_PROVIDER_ID: &str = "storage-udisks2";

/// Maximum number of items returned in one [`StorageDevicePage`] (mirrors the
/// frozen manifest's `page_size` bound).
pub const MAX_STORAGE_DEVICE_PAGE: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Typed identities (OSC-012.2: never a raw device-node string)
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum length (chars) of a [`StorageDeviceId`]/[`FilesystemId`].
const STORAGE_ID_MAX_CHARS: usize = 128;

fn sanitize_storage_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(STORAGE_ID_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= STORAGE_ID_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// A stable, typed storage-device identity derived from the UDisks2
/// `Block`/`Drive` object path — never a raw `/dev/sdX` device-node string
/// (OSC-012.2). Two devices with the same object-path-derived identity are
/// the same device across observations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StorageDeviceId(String);

impl StorageDeviceId {
    /// Construct from a raw identity string (bounded, control-char-free).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(sanitize_storage_id(&raw.into()))
    }

    /// Borrow the identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StorageDeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stable, typed filesystem identity (the UDisks2 `Filesystem` interface's
/// identity for a given block device), distinct from the device identity so a
/// device with multiple partitions/filesystems never collides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FilesystemId(String);

impl FilesystemId {
    /// Construct from a raw identity string (bounded, control-char-free).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(sanitize_storage_id(&raw.into()))
    }

    /// Borrow the identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FilesystemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Discovery (list_storage_devices) — pure read, outside the mutation lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// One discovered storage device (design's `StorageDevicePage` item shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageDeviceInfo {
    /// The stable device identity.
    pub device_id: StorageDeviceId,
    /// The filesystem identity, when the device carries a filesystem.
    pub filesystem_id: Option<FilesystemId>,
    /// Total device capacity in bytes.
    pub capacity_bytes: u64,
    /// Free space in bytes (0 when unmounted/unknown).
    pub free_bytes: u64,
    /// The current mount state as a bounded, stable label
    /// (`"mounted"`/`"unmounted"`).
    pub mount_state: MountLabel,
    /// The current mount point, when mounted.
    pub mount_point: Option<String>,
    /// Whether UDisks2 reports this device as removable (USB/optical/SD).
    pub removable: bool,
}

/// The closed, stable mount-state label surfaced in results (never free
/// prose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountLabel {
    /// The device/filesystem is currently mounted.
    Mounted,
    /// The device/filesystem is currently unmounted.
    Unmounted,
}

impl MountLabel {
    /// The stable string label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MountLabel::Mounted => "mounted",
            MountLabel::Unmounted => "unmounted",
        }
    }
}

/// A bounded page of discovered storage devices (design's
/// `StorageDevicePage`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StorageDevicePage {
    /// The devices in this page.
    pub items: Vec<StorageDeviceInfo>,
    /// Whether more devices exist beyond this page.
    pub truncated: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Health (get_storage_health) — pure read, degraded/unavailable is honest
// ─────────────────────────────────────────────────────────────────────────────

/// Whether SMART/health evidence is available for a device (design's
/// `Availability` enum, OSC-012.5, OSC-031). Missing evidence is a distinct,
/// honestly-reported state — never a fabricated healthy/unhealthy status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthAvailability {
    /// SMART/health evidence was read successfully.
    Available,
    /// A provider query ran but returned a reduced-fidelity result.
    Degraded,
    /// No health evidence exists for this device (e.g. no `DriveAta`
    /// interface).
    Unavailable,
}

/// A single storage device's health report (design's `StorageHealth`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageHealthReport {
    /// The device this report covers.
    pub device_id: StorageDeviceId,
    /// Whether SMART/health evidence is available.
    pub availability: HealthAvailability,
    /// The closed, stable health-state label, when available
    /// (e.g. `"ok"`/`"warning"`/`"failing"` — provider-normalized, never raw
    /// SMART attribute text).
    pub health_state: Option<String>,
    /// Reported temperature in millikelvin, when available.
    pub temperature_millikelvin: Option<u64>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutation lifecycle: mount_device / unmount_device / eject_device
// ─────────────────────────────────────────────────────────────────────────────

/// Which dimension of storage-device state a request compares against, so a
/// mount idempotency check is never perturbed by an unrelated device's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFocus {
    /// Compare the mount state (and mount point) of one device/filesystem.
    Mount,
}

/// A normalized storage-device mount observation (design §5, §10.1). Binds
/// the device (+ optional filesystem) identity to its mount state, so an
/// observation for one device never satisfies a postcondition for another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageMountState {
    /// The device identity.
    pub device_id: StorageDeviceId,
    /// The filesystem identity, when the request named one.
    pub filesystem_id: Option<FilesystemId>,
    /// Whether the device/filesystem is currently mounted.
    pub mounted: bool,
    /// The current mount point, when mounted.
    pub mount_point: Option<String>,
}

impl StorageMountState {
    /// Construct a mount-focused observation.
    #[must_use]
    pub fn new(
        device_id: StorageDeviceId,
        filesystem_id: Option<FilesystemId>,
        mounted: bool,
        mount_point: Option<String>,
    ) -> Self {
        Self {
            device_id,
            filesystem_id,
            mounted,
            mount_point,
        }
    }
}

impl NormalizedObservation for StorageMountState {
    fn observation_digest(&self) -> Digest {
        // Deliberately excludes `mount_point`: the exact mount point a fresh
        // `mount_device` lands at is chosen by UDisks2 and unknown in
        // advance, so `desired_state()` for `Mount` cannot pin it. Per this
        // task's idempotency contract, mounting an already-mounted device
        // "at any point" is `Unchanged` — the digest binds device +
        // filesystem + mounted-boolean only, never the specific path.
        Digest::of_str(&format!(
            "storage:mount:{}:{}:{}",
            self.device_id,
            self.filesystem_id
                .as_ref()
                .map(FilesystemId::as_str)
                .unwrap_or(""),
            self.mounted,
        ))
    }
}

/// The concrete storage mutation this task implements. **Closed**: there is
/// no `Force`/`Format`/`Partition`/`Resize`/`SecureErase`/
/// `EncryptionProvisioning` variant and never will be added under this
/// module (OSC-012.4, OSC-012.6, OSC-030) — destructive disk administration
/// is handed off to trusted system utilities outside KRIA's tool surface.
#[derive(Debug, Clone)]
pub enum StorageOp {
    /// Mount `device`, optionally at a specific `filesystem`.
    Mount {
        /// The target device identity.
        device: StorageDeviceId,
        /// The target filesystem identity, when the caller specified one.
        filesystem: Option<FilesystemId>,
    },
    /// Unmount `device`. Never accepts a force flag (OSC-012.4).
    Unmount {
        /// The target device identity.
        device: StorageDeviceId,
    },
    /// Eject `device` (unmount + power-down for removable media). Never
    /// accepts a force flag (OSC-012.4).
    Eject {
        /// The target device identity.
        device: StorageDeviceId,
    },
}

/// A fully-described storage request. Carries the canonical `action`/
/// `params` so the governed lifecycle can bind them against the grant.
#[derive(Debug, Clone)]
pub struct StorageRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: StorageOp,
}

impl StorageRequest {
    /// The comparison focus implied by the operation (always [`StorageFocus::Mount`]
    /// for this task's three mutations).
    #[must_use]
    pub fn focus(&self) -> StorageFocus {
        StorageFocus::Mount
    }

    /// The device identity the operation targets.
    #[must_use]
    pub fn device_id(&self) -> &StorageDeviceId {
        match &self.op {
            StorageOp::Mount { device, .. }
            | StorageOp::Unmount { device }
            | StorageOp::Eject { device } => device,
        }
    }

    /// The filesystem identity the operation targets, when named.
    #[must_use]
    pub fn filesystem_id(&self) -> Option<&FilesystemId> {
        match &self.op {
            StorageOp::Mount { filesystem, .. } => filesystem.as_ref(),
            StorageOp::Unmount { .. } | StorageOp::Eject { .. } => None,
        }
    }

    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> StorageMountState {
        match &self.op {
            StorageOp::Mount { device, filesystem } => {
                StorageMountState::new(device.clone(), filesystem.clone(), true, None)
            }
            StorageOp::Unmount { device } => {
                StorageMountState::new(device.clone(), None, false, None)
            }
            StorageOp::Eject { device } => {
                StorageMountState::new(device.clone(), None, false, None)
            }
        }
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for all three storage mutations).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw storage transport seam over UDisks2. The live implementation
/// ([`crate::os_control::linux::providers::udisks::LiveUdisks`]) is a raw,
/// deny-live-gated D-Bus adapter; deny-live tests inject
/// [`fake::FakeStorageTransport`], which models a fake UDisks2 object tree
/// (block/filesystem/drive identities) and scripted `PropertiesChanged`-style
/// state without ever opening a live bus.
///
/// Mount/unmount/eject dispatch **directly** to UDisks2 — never through
/// [`crate::os_control::broker::BrokerOperation`] (design §12: "Mount/eject
/// uses UDisks2's own typed Polkit authorization and is not reconstructed in
/// this broker").
#[async_trait]
pub trait StorageTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// List discovered storage devices (`list_storage_devices`; a pure read
    /// outside the mutation lifecycle).
    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError>;

    /// Read the current mount state of `device` (+ optional `filesystem`).
    /// `Ok(None)` for `filesystem` means "read the device's own current
    /// filesystem identity, if any" rather than a caller-specified target.
    async fn read_mount_state(
        &self,
        ctx: &HostExecutionContext,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<StorageMountState, OsControlError>;

    /// Mount `device` (optionally at `filesystem`) through UDisks2's own
    /// typed Polkit authorization. A busy/blocking condition must surface as
    /// [`OsControlError::ResourceBusy`] — never a forced retry.
    async fn mount(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Unmount `device` through UDisks2's own typed Polkit authorization. A
    /// busy device (open file handle) reports
    /// [`OsControlError::ResourceBusy`] rather than forcing (OSC-012.3,
    /// OSC-012.4) — there is no force parameter to accept.
    async fn unmount(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Eject `device` (unmount + power-down for removable media) through
    /// UDisks2's own typed Polkit authorization. A busy device reports
    /// [`OsControlError::ResourceBusy`] rather than forcing.
    async fn eject(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Read available SMART/health evidence for `device` (`get_storage_health`;
    /// a pure read outside the mutation lifecycle). When `device` is `None`,
    /// implementations report the first/primary device's health (the
    /// frozen manifest's `device` parameter is optional).
    async fn read_health(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError>;
}

/// The `StorageControl` desired-state provider (design §3, §4, §10.1, §12).
/// Generic over the [`StorageTransport`] so the same governed logic runs over
/// the live UDisks2 adapter and the deny-live fake.
pub struct StorageControl<T: StorageTransport> {
    transport: T,
}

impl<T: StorageTransport> StorageControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// List discovered storage devices (`list_storage_devices`; a pure read
    /// outside the mutation lifecycle).
    pub async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError> {
        let limit = limit.clamp(1, MAX_STORAGE_DEVICE_PAGE);
        self.transport.list_devices(ctx, cursor, limit).await
    }

    /// Read available SMART/health evidence (`get_storage_health`; a pure
    /// read outside the mutation lifecycle).
    pub async fn read_health(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError> {
        self.transport.read_health(ctx, device).await
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(&self, observed: &StorageMountState) -> SatisfyingVerification<StorageMountState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: StorageTransport> DesiredStateControl<StorageRequest, StorageMountState>
    for StorageControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &StorageRequest,
    ) -> Result<StorageMountState, OsControlError> {
        self.transport
            .read_mount_state(ctx, request.device_id(), request.filesystem_id())
            .await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StorageRequest,
        _desired: &StorageMountState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            StorageOp::Mount { device, filesystem } => {
                self.transport.mount(ctx, device, filesystem.as_ref()).await
            }
            StorageOp::Unmount { device } => self.transport.unmount(ctx, device).await,
            StorageOp::Eject { device } => self.transport.eject(ctx, device).await,
        }
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &StorageRequest,
        desired: &StorageMountState,
    ) -> Result<VerificationReport<StorageMountState>, OsControlError> {
        // OSC-012.7: every storage mutation's verification re-reads the
        // *actual* mount topology through the transport — never a cached
        // flag from the apply call.
        let observed = self
            .transport
            .read_mount_state(ctx, request.device_id(), request.filesystem_id())
            .await?;

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The frozen manifest declares `rollbackClaim: None` for
        // `mount_device`/`unmount_device`/`eject_device`: never actually
        // invoked. Reports the truthful "no inverse" fact if it ever were.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the `mount_device` result fields.
#[must_use]
pub fn mount_device_result(
    receipt: &MutationReceipt<StorageMountState>,
    device: &str,
    filesystem: Option<&str>,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "device": device,
        "filesystem": filesystem,
        "mounted": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `unmount_device` result fields.
#[must_use]
pub fn unmount_device_result(
    receipt: &MutationReceipt<StorageMountState>,
    device: &str,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "device": device,
        "unmounted": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `eject_device` result fields.
#[must_use]
pub fn eject_device_result(
    receipt: &MutationReceipt<StorageMountState>,
    device: &str,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "device": device,
        "ejected": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a [`StorageDevicePage`] to the `list_storage_devices` result fields.
#[must_use]
pub fn list_storage_devices_result(page: &StorageDevicePage) -> serde_json::Value {
    let items: Vec<serde_json::Value> = page
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "device_id": item.device_id.as_str(),
                "filesystem_id": item.filesystem_id.as_ref().map(FilesystemId::as_str),
                "capacity_bytes": item.capacity_bytes,
                "free_bytes": item.free_bytes,
                "mount_state": item.mount_state.as_str(),
                "mount_point": item.mount_point,
                "removable": item.removable,
            })
        })
        .collect();
    serde_json::json!({
        "items": items,
        "truncated": page.truncated,
    })
}

/// Map a [`StorageHealthReport`] to the `get_storage_health` result fields.
/// Missing evidence is reported as `"unavailable"` — never a fabricated
/// healthy/unhealthy status (OSC-012.5, OSC-031).
#[must_use]
pub fn get_storage_health_result(report: &StorageHealthReport) -> serde_json::Value {
    let availability = match report.availability {
        HealthAvailability::Available => "available",
        HealthAvailability::Degraded => "degraded",
        HealthAvailability::Unavailable => "unavailable",
    };
    serde_json::json!({
        "device_id": report.device_id.as_str(),
        "availability": availability,
        "health_state": report.health_state,
        "temperature_millikelvin": report.temperature_millikelvin,
    })
}

/// The frozen [`OsControlError::ResourceBusy`] "device is busy" error
/// (OSC-012.3): a blocking unmount/eject reports this distinct blocking
/// state — never a forced retry. Constructed here so every transport (real +
/// fake) reports the identical error shape.
#[must_use]
pub fn device_busy_error(device: &StorageDeviceId) -> OsControlError {
    OsControlError::ResourceBusy {
        resource: crate::os_control::contract::SafeResource::new(format!(
            "storage/{}",
            device.as_str()
        )),
        owner: Some(SafeText::new("open file handle")),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::storage()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible storage domain port. Because the concrete
/// [`StorageControl`] provider struct above is generic over its
/// [`StorageTransport`], `HostOsControl::storage()` returns this object-safe
/// supertrait instead so any transport (live UDisks2, or a deny-live fake)
/// can be composed behind one erased reference.
#[async_trait]
pub trait StorageControlPort: DesiredStateControl<StorageRequest, StorageMountState> {
    /// Read-only device discovery (erased passthrough for the read-only
    /// `list_storage_devices` tool).
    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError>;

    /// Read-only health reporting (erased passthrough for the read-only
    /// `get_storage_health` tool).
    async fn read_health(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError>;
}

#[async_trait]
impl<T: StorageTransport> StorageControlPort for StorageControl<T> {
    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError> {
        StorageControl::list_devices(self, ctx, cursor, limit).await
    }

    async fn read_health(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError> {
        StorageControl::read_health(self, ctx, device).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_device_filesystem_and_mount_state() {
        let a = StorageMountState::new(StorageDeviceId::new("dev-1"), None, true, Some("/media/a".into()));
        let b = StorageMountState::new(StorageDeviceId::new("dev-1"), None, true, Some("/media/a".into()));
        assert_eq!(a.observation_digest(), b.observation_digest());

        let c = StorageMountState::new(StorageDeviceId::new("dev-1"), None, false, None);
        assert_ne!(a.observation_digest(), c.observation_digest());

        let d = StorageMountState::new(StorageDeviceId::new("dev-2"), None, true, Some("/media/a".into()));
        assert_ne!(a.observation_digest(), d.observation_digest());

        let e = StorageMountState::new(
            StorageDeviceId::new("dev-1"),
            Some(FilesystemId::new("fs-1")),
            true,
            Some("/media/a".into()),
        );
        assert_ne!(a.observation_digest(), e.observation_digest());
    }

    #[test]
    fn digest_ignores_mount_point_so_mount_idempotency_holds_at_any_path() {
        // Mounting an already-mounted device "at any point" is Unchanged: the
        // digest must not distinguish two observations that differ only in
        // mount_point (design note: UDisks2 chooses the mount point, so
        // `desired_state()` cannot pin one in advance).
        let a = StorageMountState::new(StorageDeviceId::new("dev-1"), None, true, Some("/media/a".into()));
        let b = StorageMountState::new(StorageDeviceId::new("dev-1"), None, true, Some("/media/b".into()));
        assert_eq!(a.observation_digest(), b.observation_digest());
    }

    #[test]
    fn desired_state_mount_targets_mounted_true() {
        let req = StorageRequest {
            action: "mount_device".to_string(),
            params: serde_json::json!({ "device": "dev-1" }),
            op: StorageOp::Mount {
                device: StorageDeviceId::new("dev-1"),
                filesystem: None,
            },
        };
        assert!(req.desired_state().mounted);
    }

    #[test]
    fn desired_state_unmount_and_eject_target_mounted_false() {
        let unmount = StorageRequest {
            action: "unmount_device".to_string(),
            params: serde_json::json!({ "device": "dev-1" }),
            op: StorageOp::Unmount {
                device: StorageDeviceId::new("dev-1"),
            },
        };
        assert!(!unmount.desired_state().mounted);

        let eject = StorageRequest {
            action: "eject_device".to_string(),
            params: serde_json::json!({ "device": "dev-1" }),
            op: StorageOp::Eject {
                device: StorageDeviceId::new("dev-1"),
            },
        };
        assert!(!eject.desired_state().mounted);
    }

    #[test]
    fn storage_device_id_strips_control_chars_and_bounds_length() {
        let id = StorageDeviceId::new("dev\n\u{1b}[31m".to_string() + &"z".repeat(500));
        assert!(!id.as_str().contains('\n'));
        assert!(!id.as_str().contains('\u{1b}'));
        assert!(id.as_str().chars().count() <= STORAGE_ID_MAX_CHARS);
    }
}
