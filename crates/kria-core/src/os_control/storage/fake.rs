//! Deny-live fake [`StorageTransport`] (OSC-012, OSC-030, OSC-033), Task 3.2.
//!
//! Compiled only under `os-control-test`. Never opens a D-Bus connection, so
//! [`crate::os_control::access::deny_live_transport`] is unreachable from this
//! file and the process-wide deny-live sentinel never trips.
//!
//! # Two ways to drive it (documented cascade, never both by accident)
//!
//! 1. **Scripted queue** — [`Self::mount_state_ok`] queues successive
//!    `read_mount_state` answers in call order, because one governed mutation
//!    performs four reads in a fixed order (pre-observation → under-lease
//!    re-observation → post-apply re-observation → `verify`'s own fresh
//!    re-read). When the queue drains the last *observed* value is held (a
//!    steady state), never a newly invented one.
//! 2. **In-memory device model** — [`Self::device`] installs a
//!    [`FakeStorageDevice`] table and [`StorageTransport::mount`]/`unmount`/
//!    `eject` **apply the effect to that table**, so an observe → apply →
//!    re-observe lifecycle exercises the real governed path instead of a
//!    scripted sequence.
//!
//! Resolution order for a read: scripted queue first (while it has entries),
//! then the device table (if any), then the held steady state, then a hard
//! error. An unscripted read is **always** an error — a fake that invented a
//! mount state would let a test certify a mutation against a fact nobody read.
//!
//! # Facts this fake keeps distinct (they are genuinely different)
//!
//! * **"not mounted"** is `Ok(StorageMountState { mounted: false, .. })` — an
//!   observation that happened.
//! * **"could not determine the mount state"** is
//!   [`Self::mount_state_indeterminate`] → a retryable
//!   [`OsControlError::Unavailable`]. Never a `mounted: false` default.
//! * **"the device is gone"** is [`Self::mount_state_device_removed`] /
//!   [`Self::remove_device`] → a non-retryable `Unavailable`. Removable media
//!   really does vanish between an observation and an unmount, so the model
//!   can do exactly that.
//!
//! # Identity is the stable id, never the human label
//!
//! [`FakeStorageDevice::label`] exists precisely so a test can prove it is
//! *not* identity: two devices sharing one volume label stay distinguishable
//! because only [`StorageDeviceId`] (a UUID/object-path-derived id, OSC-012.2)
//! participates in lookup and in
//! [`StorageMountState::observation_digest`]. A scripted observation whose
//! `device_id` does not match the requested device is rejected rather than
//! served, so a read can never be satisfied by another device's fact.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};
use crate::os_control::testing::CallRecorder;

use super::{
    device_busy_error, FilesystemId, MountLabel, StorageDeviceId, StorageDeviceInfo,
    StorageDevicePage, StorageHealthReport, StorageMountState, StorageTransport,
};

/// Provider identity reported by the fake transport.
pub const FAKE_STORAGE_PROVIDER_ID: &str = "fake-storage-udisks2";

/// Which effect a dispatch applies to the in-memory device table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    Mount,
    Unmount,
    Eject,
}

impl Effect {
    fn label(self) -> &'static str {
        match self {
            Effect::Mount => "mount",
            Effect::Unmount => "unmount",
            Effect::Eject => "eject",
        }
    }
}

/// One device in the fake's in-memory UDisks2 object tree.
///
/// `device_id` is the **identity**. `label` is a human volume label that is
/// deliberately *not* identity — two devices may carry the same one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeStorageDevice {
    device_id: StorageDeviceId,
    filesystem_id: Option<FilesystemId>,
    label: Option<String>,
    mounted: bool,
    mount_point: Option<String>,
    removable: bool,
    capacity_bytes: u64,
    free_bytes: u64,
    /// An open file handle blocks unmount/eject (OSC-012.3).
    busy: bool,
    /// The medium was physically removed; every later read/dispatch reports
    /// that fact instead of a stale state.
    removed: bool,
}

impl FakeStorageDevice {
    /// An unmounted, non-removable, zero-capacity device with `device_id`.
    #[must_use]
    pub fn new(device_id: StorageDeviceId) -> Self {
        Self {
            device_id,
            filesystem_id: None,
            label: None,
            mounted: false,
            mount_point: None,
            removable: false,
            capacity_bytes: 0,
            free_bytes: 0,
            busy: false,
            removed: false,
        }
    }

    /// Builder: the filesystem identity this device carries.
    #[must_use]
    pub fn filesystem(mut self, filesystem_id: FilesystemId) -> Self {
        self.filesystem_id = Some(filesystem_id);
        self
    }

    /// Builder: the human volume label. Never used for lookup or identity.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Builder: the device starts mounted at `mount_point`.
    #[must_use]
    pub fn mounted_at(mut self, mount_point: impl Into<String>) -> Self {
        self.mounted = true;
        self.mount_point = Some(mount_point.into());
        self
    }

    /// Builder: UDisks2 reports the device as removable media.
    #[must_use]
    pub fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Builder: capacity/free bytes reported by discovery.
    #[must_use]
    pub fn capacity(mut self, capacity_bytes: u64, free_bytes: u64) -> Self {
        self.capacity_bytes = capacity_bytes;
        self.free_bytes = free_bytes;
        self
    }

    /// Builder: an open file handle blocks unmount/eject.
    #[must_use]
    pub fn busy(mut self) -> Self {
        self.busy = true;
        self
    }

    /// The device identity.
    #[must_use]
    pub fn device_id(&self) -> &StorageDeviceId {
        &self.device_id
    }

    /// The human volume label, when one was set.
    #[must_use]
    pub fn volume_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Whether the device is currently mounted in the model.
    #[must_use]
    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    /// The current mount point in the model.
    #[must_use]
    pub fn mount_point(&self) -> Option<&str> {
        self.mount_point.as_deref()
    }

    /// The discovery row for this device.
    fn info(&self) -> StorageDeviceInfo {
        StorageDeviceInfo {
            device_id: self.device_id.clone(),
            filesystem_id: self.filesystem_id.clone(),
            capacity_bytes: self.capacity_bytes,
            free_bytes: self.free_bytes,
            mount_state: if self.mounted {
                MountLabel::Mounted
            } else {
                MountLabel::Unmounted
            },
            mount_point: self.mount_point.clone(),
            removable: self.removable,
        }
    }
}

/// A scripted, in-memory storage transport over a fake UDisks2 object tree.
pub struct FakeStorageTransport {
    /// Ordered scripted `read_mount_state` answers (see the module doc's
    /// resolution cascade). An `Err` entry models an evidence failure.
    scripted: Mutex<VecDeque<Result<StorageMountState, OsControlError>>>,
    /// The last *observed* state, held when the queue drains.
    last: Mutex<Option<StorageMountState>>,
    /// The in-memory device table that `dispatch` mutates.
    devices: Mutex<Vec<FakeStorageDevice>>,
    /// Scripted health evidence, keyed by device identity.
    health: Mutex<Vec<StorageHealthReport>>,
    /// Scripted dispatch outcomes, consumed in order per operation.
    mount_outcomes: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
    unmount_outcomes: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
    eject_outcomes: Mutex<VecDeque<Result<ApplyOutcome, OsControlError>>>,
    /// When set, every mount-state read fails: the mount state could not be
    /// determined at all.
    read_failure: Mutex<Option<String>>,
    /// Every transport call, in order (`"mount"`, `"read_mount_state"`, …).
    recorder: CallRecorder,
    /// Mutating dispatch attempts, including ones that failed pre-effect.
    dispatches: Mutex<usize>,
}

impl Default for FakeStorageTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeStorageTransport {
    /// A fake with nothing scripted and no device modelled: every read fails
    /// closed until something is scripted.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(VecDeque::new()),
            last: Mutex::new(None),
            devices: Mutex::new(Vec::new()),
            health: Mutex::new(Vec::new()),
            mount_outcomes: Mutex::new(VecDeque::new()),
            unmount_outcomes: Mutex::new(VecDeque::new()),
            eject_outcomes: Mutex::new(VecDeque::new()),
            read_failure: Mutex::new(None),
            recorder: CallRecorder::new(),
            dispatches: Mutex::new(0),
        }
    }

    /// Builder: queue the next `read_mount_state` answer.
    #[must_use]
    pub fn mount_state_ok(self, state: StorageMountState) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(Ok(state));
        self
    }

    /// Builder: queue a read that **could not determine** the mount state.
    ///
    /// Distinct from a scripted `mounted: false` observation: this is the
    /// absence of evidence, so it is a retryable error and never a state.
    #[must_use]
    pub fn mount_state_indeterminate(self, reason: impl Into<String>) -> Self {
        let err = OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
            reason: SafeText::new(format!("mount state indeterminate: {}", reason.into())),
            retryable: true,
        };
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(Err(err));
        self
    }

    /// Builder: queue a read reporting that `device` is no longer present
    /// (removable media pulled between two observations).
    #[must_use]
    pub fn mount_state_device_removed(self, device: &StorageDeviceId) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(Err(device_removed_error(device)));
        self
    }

    /// Builder: make **every** mount-state read fail, proving an ambiguous
    /// UDisks2 property read never becomes a fabricated mount state.
    #[must_use]
    pub fn read_failure(self, reason: impl Into<String>) -> Self {
        *self.read_failure.lock().expect("read_failure mutex") = Some(reason.into());
        self
    }

    /// Builder: install a device into the in-memory model.
    #[must_use]
    pub fn device(self, device: FakeStorageDevice) -> Self {
        self.devices.lock().expect("devices mutex").push(device);
        self
    }

    /// Builder: script health evidence for one device.
    #[must_use]
    pub fn health_ok(self, report: StorageHealthReport) -> Self {
        self.health.lock().expect("health mutex").push(report);
        self
    }

    /// Builder: script the next `mount` dispatch outcome.
    #[must_use]
    pub fn mount_outcome(self, outcome: Result<ApplyOutcome, OsControlError>) -> Self {
        self.mount_outcomes
            .lock()
            .expect("mount_outcomes mutex")
            .push_back(outcome);
        self
    }

    /// Builder: script the next `unmount` dispatch outcome.
    #[must_use]
    pub fn unmount_outcome(self, outcome: Result<ApplyOutcome, OsControlError>) -> Self {
        self.unmount_outcomes
            .lock()
            .expect("unmount_outcomes mutex")
            .push_back(outcome);
        self
    }

    /// Builder: script the next `eject` dispatch outcome.
    #[must_use]
    pub fn eject_outcome(self, outcome: Result<ApplyOutcome, OsControlError>) -> Self {
        self.eject_outcomes
            .lock()
            .expect("eject_outcomes mutex")
            .push_back(outcome);
        self
    }

    /// Physically remove modelled `device` *now* — the removable-media race
    /// between an observation and the unmount that follows it. Returns whether
    /// the device was in the model.
    pub fn remove_device(&self, device: &StorageDeviceId) -> bool {
        let mut devices = self.devices.lock().expect("devices mutex");
        match devices.iter_mut().find(|d| &d.device_id == device) {
            Some(found) => {
                found.removed = true;
                true
            }
            None => false,
        }
    }

    /// Mark modelled `device` as holding an open file handle *now*.
    pub fn mark_busy(&self, device: &StorageDeviceId) -> bool {
        let mut devices = self.devices.lock().expect("devices mutex");
        match devices.iter_mut().find(|d| &d.device_id == device) {
            Some(found) => {
                found.busy = true;
                true
            }
            None => false,
        }
    }

    /// A copy of the modelled device, for asserting that a dispatch really
    /// mutated the model.
    #[must_use]
    pub fn device_snapshot(&self, device: &StorageDeviceId) -> Option<FakeStorageDevice> {
        self.devices
            .lock()
            .expect("devices mutex")
            .iter()
            .find(|d| &d.device_id == device)
            .cloned()
    }

    /// Every transport call label, in order.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.recorder.labels()
    }

    /// How many mutating dispatches were attempted (a pre-effect failure still
    /// counts as an attempt; there is no retry and no force path).
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        *self.dispatches.lock().expect("dispatches mutex")
    }

    /// The error an unscripted read returns. Never a value.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }

    /// Reject an observation that belongs to a different device/filesystem.
    /// Identity is the stable id — a same-labelled sibling never satisfies it.
    fn bind_identity(
        state: &StorageMountState,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<(), OsControlError> {
        if &state.device_id != device {
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
                reason: SafeText::new(
                    "scripted observation belongs to a different device identity",
                ),
                retryable: false,
            });
        }
        if let Some(filesystem) = filesystem {
            if state.filesystem_id.as_ref() != Some(filesystem) {
                return Err(OsControlError::Unavailable {
                    provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
                    reason: SafeText::new(
                        "scripted observation belongs to a different filesystem identity",
                    ),
                    retryable: false,
                });
            }
        }
        Ok(())
    }

    /// Observe the in-memory model.
    ///
    /// The returned observation echoes the *requested* filesystem identity (the
    /// same binding [`super::StorageRequest::desired_state`] uses), so a
    /// device-scoped unmount is never contradicted by a filesystem the caller
    /// never named.
    fn observe_model(
        &self,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<StorageMountState, OsControlError> {
        let devices = self.devices.lock().expect("devices mutex");
        let found = devices
            .iter()
            .find(|d| &d.device_id == device)
            .ok_or_else(|| device_removed_error(device))?;
        if found.removed {
            return Err(device_removed_error(device));
        }
        if let Some(filesystem) = filesystem {
            if found.filesystem_id.as_ref() != Some(filesystem) {
                return Err(self.unscripted("modelled device carries a different filesystem"));
            }
        }
        Ok(StorageMountState::new(
            found.device_id.clone(),
            filesystem.cloned(),
            found.mounted,
            found.mount_point.clone(),
        ))
    }

    /// Apply `effect` to the model, strictly: an absent/removed device and a
    /// busy unmount/eject are reported, never silently succeeded.
    fn dispatch_model(
        &self,
        effect: Effect,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError> {
        let mut devices = self.devices.lock().expect("devices mutex");
        let Some(found) = devices.iter_mut().find(|d| &d.device_id == device) else {
            return Err(device_removed_error(device));
        };
        if found.removed {
            return Err(device_removed_error(device));
        }
        match effect {
            Effect::Mount => {
                found.mounted = true;
                found.mount_point = Some(format!("/media/{}", found.device_id.as_str()));
            }
            Effect::Unmount | Effect::Eject => {
                // OSC-012.3/OSC-012.4: busy is a distinct blocking state and
                // there is no force path to fall back to.
                if found.busy {
                    return Err(device_busy_error(device));
                }
                found.mounted = false;
                found.mount_point = None;
            }
        }
        Ok(default_applied())
    }

    /// Best-effort model update alongside a scripted outcome: a suite that
    /// scripts reads *and* an outcome has no device table, so an unknown device
    /// is simply not modelled here rather than an error.
    fn nudge_model(&self, effect: Effect, device: &StorageDeviceId) {
        let mut devices = self.devices.lock().expect("devices mutex");
        if let Some(found) = devices.iter_mut().find(|d| &d.device_id == device) {
            if found.removed {
                return;
            }
            match effect {
                Effect::Mount => {
                    found.mounted = true;
                    found.mount_point = Some(format!("/media/{}", found.device_id.as_str()));
                }
                Effect::Unmount | Effect::Eject => {
                    found.mounted = false;
                    found.mount_point = None;
                }
            }
        }
    }

    fn read_mount_state_inner(
        &self,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<StorageMountState, OsControlError> {
        if let Some(reason) = self.read_failure.lock().expect("read_failure mutex").clone() {
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
                reason: SafeText::new(format!("mount state indeterminate: {reason}")),
                retryable: true,
            });
        }

        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        if let Some(entry) = next {
            let state = entry?;
            Self::bind_identity(&state, device, filesystem)?;
            *self.last.lock().expect("last mutex") = Some(state.clone());
            return Ok(state);
        }

        if !self.devices.lock().expect("devices mutex").is_empty() {
            return self.observe_model(device, filesystem);
        }

        let held = self.last.lock().expect("last mutex").clone();
        match held {
            Some(state) => {
                Self::bind_identity(&state, device, filesystem)?;
                Ok(state)
            }
            None => Err(self.unscripted("no mount state scripted on the fake transport")),
        }
    }

    fn dispatch_inner(
        &self,
        effect: Effect,
        device: &StorageDeviceId,
        scripted: Option<Result<ApplyOutcome, OsControlError>>,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.recorder.record(effect.label());
        *self.dispatches.lock().expect("dispatches mutex") += 1;
        match scripted {
            Some(outcome) => {
                if matches!(outcome, Ok(ApplyOutcome::Applied(_))) {
                    self.nudge_model(effect, device);
                }
                outcome
            }
            None => self.dispatch_model(effect, device),
        }
    }
}

/// The "device is no longer present" error every transport (real + fake)
/// reports when removable media vanished. Non-retryable: a gone device does
/// not come back by trying again.
#[must_use]
pub fn device_removed_error(device: &StorageDeviceId) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(FAKE_STORAGE_PROVIDER_ID)),
        reason: SafeText::new(format!(
            "device {} is no longer present",
            device.as_str()
        )),
        retryable: false,
    }
}

fn default_applied() -> ApplyOutcome {
    ApplyOutcome::Applied(AppliedDispatch::new(
        Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
        BoundedVec::new(),
    ))
}

#[async_trait]
impl StorageTransport for FakeStorageTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_STORAGE_PROVIDER_ID)
    }

    async fn list_devices(
        &self,
        _ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError> {
        self.recorder.record("list_devices");
        let devices = self.devices.lock().expect("devices mutex");
        if devices.is_empty() {
            return Err(self.unscripted("no device table scripted on the fake transport"));
        }
        // Physically removed media is not discoverable.
        let present: Vec<&FakeStorageDevice> = devices.iter().filter(|d| !d.removed).collect();
        let items: Vec<StorageDeviceInfo> = present
            .iter()
            .skip(cursor)
            .take(limit)
            .map(|d| d.info())
            .collect();
        let truncated = cursor.saturating_add(limit) < present.len();
        Ok(StorageDevicePage { items, truncated })
    }

    async fn read_mount_state(
        &self,
        _ctx: &HostExecutionContext,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<StorageMountState, OsControlError> {
        self.recorder.record("read_mount_state");
        self.read_mount_state_inner(device, filesystem)
    }

    async fn mount(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<ApplyOutcome, OsControlError> {
        let _ = filesystem;
        let scripted = self
            .mount_outcomes
            .lock()
            .expect("mount_outcomes mutex")
            .pop_front();
        self.dispatch_inner(Effect::Mount, device, scripted)
    }

    async fn unmount(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError> {
        let scripted = self
            .unmount_outcomes
            .lock()
            .expect("unmount_outcomes mutex")
            .pop_front();
        self.dispatch_inner(Effect::Unmount, device, scripted)
    }

    async fn eject(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError> {
        let scripted = self
            .eject_outcomes
            .lock()
            .expect("eject_outcomes mutex")
            .pop_front();
        self.dispatch_inner(Effect::Eject, device, scripted)
    }

    async fn read_health(
        &self,
        _ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError> {
        self.recorder.record("read_health");
        let health = self.health.lock().expect("health mutex");
        match device {
            // Evidence is per device: another device's SMART report never
            // stands in for this one.
            Some(device) => health
                .iter()
                .find(|report| &report.device_id == device)
                .cloned()
                .ok_or_else(|| {
                    self.unscripted("no health evidence scripted for that device identity")
                }),
            None => health
                .first()
                .cloned()
                .ok_or_else(|| self.unscripted("no health evidence scripted on the fake transport")),
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio_util::sync::CancellationToken;

    use crate::agent::execution_gate::OsActionGrant;
    use crate::agent::turn_memory::ExecutionTarget;
    use crate::os_control::context::{
        AuditAdmissionToken, MutationPermit, RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{ActionId, AuditAdmissionId, CorrelationId, SessionId};
    use crate::os_control::resource::AcquiredResourceLeaseSet;
    use crate::os_control::runtime::NormalizedObservation;
    use crate::os_control::storage::HealthAvailability;
    use crate::safety::RiskLevel;

    use super::*;

    const SESSION: &str = "session-storage-fake";

    /// Owns every authority so a borrowed [`AdmittedMutationContext`] can be
    /// handed to a dispatch without lifetime trouble.
    struct Fixture {
        grant: OsActionGrant,
        host_ctx: HostExecutionContext,
        lease_set: AcquiredResourceLeaseSet,
        audit_token: AuditAdmissionToken,
        resource_digest: Digest,
    }

    impl Fixture {
        fn build() -> Self {
            let params = serde_json::json!({ "device": "usb-1" });
            let grant = OsActionGrant::for_test(
                SESSION,
                "unmount_device",
                &params,
                ExecutionTarget::Host,
                &[],
                RiskLevel::Red,
            );
            let resource_digest = Digest::of_str(grant.resource_set_digest());
            let audit_token = AuditAdmissionToken::for_test(
                AuditAdmissionId::new("adm-storage-fake"),
                resource_digest.clone(),
            );
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-storage-fake"),
                ActionId::new("act-storage-fake"),
                audit_token.observation_authority(),
                Arc::new(SessionContext::new(SessionId::new(SESSION))),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(30),
                RedactionPolicy::default(),
            );
            let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest.clone());
            Self {
                grant,
                host_ctx,
                lease_set,
                audit_token,
                resource_digest,
            }
        }

        fn host(&self) -> &HostExecutionContext {
            &self.host_ctx
        }

        fn admitted(&self) -> AdmittedMutationContext<'_> {
            let permit = MutationPermit::for_test(
                &self.lease_set,
                &self.audit_token,
                self.resource_digest.clone(),
            );
            AdmittedMutationContext::for_test(&self.host_ctx, &self.grant, permit)
        }
    }

    #[tokio::test]
    async fn unscripted_read_fails_closed_and_never_invents_a_mount_state() {
        let fx = Fixture::build();
        let fake = FakeStorageTransport::new();
        let err = fake
            .read_mount_state(fx.host(), &StorageDeviceId::new("usb-1"), None)
            .await
            .expect_err("an unscripted read must fail, not default to unmounted");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn not_mounted_and_indeterminate_are_different_facts() {
        let fx = Fixture::build();
        let device = StorageDeviceId::new("usb-1");

        // "not mounted": an observation that happened.
        let observed = FakeStorageTransport::new()
            .mount_state_ok(StorageMountState::new(device.clone(), None, false, None))
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect("a scripted not-mounted observation is a fact");
        assert!(!observed.mounted);

        // "could not determine": no observation at all → retryable error.
        let err = FakeStorageTransport::new()
            .mount_state_indeterminate("udisks2 Filesystem.MountPoints unreadable")
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect_err("indeterminate evidence must not become mounted:false");
        match err {
            OsControlError::Unavailable { retryable, .. } => assert!(retryable),
            other => panic!("expected a retryable Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_devices_sharing_a_label_stay_distinguishable() {
        let fx = Fixture::build();
        let left = StorageDeviceId::new("usb-uuid-1111-1111");
        let right = StorageDeviceId::new("usb-uuid-2222-2222");
        let fake = FakeStorageTransport::new()
            .device(
                FakeStorageDevice::new(left.clone())
                    .label("KINGSTON")
                    .removable()
                    .mounted_at("/media/left"),
            )
            .device(
                FakeStorageDevice::new(right.clone())
                    .label("KINGSTON")
                    .removable(),
            );

        let left_state = fake
            .read_mount_state(fx.host(), &left, None)
            .await
            .expect("left device observed");
        let right_state = fake
            .read_mount_state(fx.host(), &right, None)
            .await
            .expect("right device observed");

        assert_eq!(
            fake.device_snapshot(&left).unwrap().volume_label(),
            fake.device_snapshot(&right).unwrap().volume_label(),
            "the two devices deliberately share one human label"
        );
        assert!(left_state.mounted);
        assert!(!right_state.mounted);
        assert_ne!(
            left_state.observation_digest(),
            right_state.observation_digest(),
            "identity is the stable id, so a shared label must not merge them"
        );
    }

    #[tokio::test]
    async fn a_scripted_observation_for_another_device_is_refused() {
        let fx = Fixture::build();
        let fake = FakeStorageTransport::new().mount_state_ok(StorageMountState::new(
            StorageDeviceId::new("usb-other"),
            None,
            true,
            Some("/media/other".to_string()),
        ));
        let err = fake
            .read_mount_state(fx.host(), &StorageDeviceId::new("usb-1"), None)
            .await
            .expect_err("another device's fact must never satisfy this read");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn dispatch_applies_the_effect_to_the_model() {
        let fx = Fixture::build();
        let device = StorageDeviceId::new("usb-1");
        let fake =
            FakeStorageTransport::new().device(FakeStorageDevice::new(device.clone()).removable());

        let before = fake
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect("observed");
        assert!(!before.mounted);

        fake.mount(&fx.admitted(), &device, None)
            .await
            .expect("mount applied");

        let after = fake
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect("re-observed");
        assert!(after.mounted, "dispatch must mutate the fake's own state");
        assert_eq!(fake.dispatch_count(), 1);
        assert!(fake.labels().contains(&"mount".to_string()));
    }

    #[tokio::test]
    async fn device_removed_between_observation_and_unmount_is_reported_not_faked() {
        let fx = Fixture::build();
        let device = StorageDeviceId::new("usb-1");
        let fake = FakeStorageTransport::new().device(
            FakeStorageDevice::new(device.clone())
                .removable()
                .mounted_at("/media/usb-1"),
        );

        let observed = fake
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect("mounted before the medium is pulled");
        assert!(observed.mounted);

        // The user pulls the stick between the observation and the unmount.
        assert!(fake.remove_device(&device));

        let err = fake
            .unmount(&fx.admitted(), &device)
            .await
            .expect_err("a vanished device must be reported, never reported as unmounted");
        match err {
            OsControlError::Unavailable { retryable, .. } => {
                assert!(!retryable, "a gone device does not come back on retry");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert_eq!(fake.dispatch_count(), 1, "no forced retry");

        let read_err = fake
            .read_mount_state(fx.host(), &device, None)
            .await
            .expect_err("a gone device has no mount state to report");
        assert!(matches!(read_err, OsControlError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn busy_device_reports_resource_busy_with_no_force_path() {
        let fx = Fixture::build();
        let device = StorageDeviceId::new("usb-1");
        let fake = FakeStorageTransport::new().device(
            FakeStorageDevice::new(device.clone())
                .removable()
                .mounted_at("/media/usb-1")
                .busy(),
        );

        let err = fake
            .unmount(&fx.admitted(), &device)
            .await
            .expect_err("an open handle blocks the unmount");
        assert!(matches!(err, OsControlError::ResourceBusy { .. }));
        assert!(
            fake.device_snapshot(&device).unwrap().is_mounted(),
            "a blocked unmount must leave the device mounted"
        );
    }

    #[tokio::test]
    async fn health_evidence_is_per_device_and_never_fabricated() {
        let fx = Fixture::build();
        let scripted = StorageDeviceId::new("nvme-1");
        let other = StorageDeviceId::new("nvme-2");
        let fake = FakeStorageTransport::new().health_ok(StorageHealthReport {
            device_id: scripted.clone(),
            availability: HealthAvailability::Unavailable,
            health_state: None,
            temperature_millikelvin: None,
        });

        let report = fake
            .read_health(fx.host(), Some(&scripted))
            .await
            .expect("scripted device has evidence");
        assert_eq!(report.availability, HealthAvailability::Unavailable);
        assert!(report.health_state.is_none());

        assert!(
            fake.read_health(fx.host(), Some(&other)).await.is_err(),
            "one device's health report must never stand in for another's"
        );
    }

    #[tokio::test]
    async fn removed_media_is_not_discoverable() {
        let fx = Fixture::build();
        let device = StorageDeviceId::new("usb-1");
        let fake = FakeStorageTransport::new().device(
            FakeStorageDevice::new(device.clone())
                .removable()
                .capacity(16, 8),
        );
        assert_eq!(
            fake.list_devices(fx.host(), 0, 16)
                .await
                .expect("one modelled device")
                .items
                .len(),
            1
        );
        fake.remove_device(&device);
        assert!(fake
            .list_devices(fx.host(), 0, 16)
            .await
            .expect("page still readable")
            .items
            .is_empty());
    }
}
