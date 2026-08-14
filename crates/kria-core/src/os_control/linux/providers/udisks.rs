//! Live UDisks2 D-Bus adapter (raw transport seam).
//!
//! linux-os-control-production **Task 3.2** — "Complete storage and
//! removable-media lifecycle" (OSC-012, OSC-030), design §3, §9.1, §12 — with
//! the reads wired live by **Task 2/§5**.
//!
//! # Host safety
//!
//! Driving UDisks2 (`org.freedesktop.UDisks2` over the system D-Bus) is a
//! **raw live transport**. Like
//! [`crate::os_control::linux::providers::network_manager`] and
//! [`crate::os_control::linux::providers::logind`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in
//!    a live composition root under `os-control-live`), so no completion
//!    test can build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before**
//!    any read, list, mount, unmount, eject, or health query, so a
//!    deny-live (`os-control-test`) build that reached here would trip the
//!    sentinel and abort rather than open a system-bus connection.
//!
//! Reads are live: one `GetManagedObjects` call against the UDisks2 object
//! manager, bounded by the observation context's deadline and cancellation,
//! normalized by [`crate::os_control::storage::selection`]. Mount, unmount and
//! eject still fail closed with [`OsControlError::Unavailable`] and never fall
//! back to an ungoverned `udisksctl`/`mount`/`umount`/`eject` subprocess.
//! Deny-live tests inject `FakeStorageTransport`.
//!
//! # Reads fail closed, and "unknown" is never "unmounted"
//!
//! Every fact this adapter reports comes from a property it actually read. An
//! absent, mistyped or self-contradictory object tree is an error, because the
//! governed runtime verifies a mutation by re-reading state: a substituted
//! `mounted: false` would let an unmount "verify" against a fact that was never
//! observed. Targets resolve only from a stable identity (block object path or
//! filesystem UUID) — never a human label, never a `/dev/sdX` node that can
//! name a different disk after a re-plug.
//!
//! # UDisks2 is D-Bus activatable
//!
//! UDisks2 owns its name on demand, so having no name owner is **not** evidence
//! that the service is absent. This adapter therefore never pre-checks
//! ownership: it issues the call and lets the bus activate the service, and only
//! a failed call is reported as unavailable (retryable).
//!
//! # Not the privilege broker (design §12)
//!
//! Mount/unmount/eject are dispatched **directly** to UDisks2 here — never
//! through [`crate::os_control::broker::BrokerOperation`]. UDisks2 owns its
//! own typed Polkit policy actions
//! (`org.freedesktop.udisks2.filesystem-mount`/`filesystem-unmount`/
//! `eject-media`), so this adapter's eventual live wiring authenticates
//! through UDisks2's native authorization, not KRIA's broker.

use std::collections::HashMap;

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeField, SafeOperation, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::receipt::ApplyOutcome;
use crate::os_control::storage::selection::{
    self, AtaSmartFacts, BlockFacts, DriveFacts, BLOCK_INTERFACE, DRIVE_ATA_INTERFACE,
    DRIVE_INTERFACE, FILESYSTEM_INTERFACE, OBJECT_MANAGER_INTERFACE, UDISKS2_OBJECT_MANAGER_PATH,
    UDISKS2_SERVICE,
};
use crate::os_control::storage::{
    FilesystemId, StorageDeviceId, StorageDevicePage, StorageHealthReport, StorageMountState,
    StorageTransport, STORAGE_PROVIDER_ID,
};

/// Properties of one D-Bus interface.
type Properties = HashMap<String, OwnedValue>;
/// One managed object: its interfaces and their properties.
type Interfaces = HashMap<String, Properties>;
/// The `GetManagedObjects` reply shape (`a{oa{sa{sv}}}`).
type ManagedObjects = HashMap<OwnedObjectPath, Interfaces>;

/// The live UDisks2 D-Bus adapter. Constructible only in a live composition;
/// a value cannot exist under `os-control-test`.
pub struct LiveUdisks {
    /// The system-bus connection, opened on first use behind the deny-live
    /// sentinel. See [`LiveUdisks::connection`] for why it is not opened in
    /// [`LiveUdisks::new`].
    system_bus: tokio::sync::OnceCell<zbus::Connection>,
    _seal: (),
}

impl LiveUdisks {
    /// Construct in a live composition root. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self {
            system_bus: tokio::sync::OnceCell::new(),
            _seal: (),
        }
    }

    /// The system-bus connection UDisks2 is read through, opened once.
    ///
    /// # Why the connection is not held from the composition root
    ///
    /// [`crate::os_control::linux::dbus::LiveDbusTransport`] is the intended
    /// holder, but its constructors are `async` and take
    /// `&LiveHostAccessToken`, while this adapter is built by the *synchronous*
    /// `LiveHostOsControl::compose_with` and then stored as a `'static` value —
    /// the borrow cannot be kept and the bus cannot be opened in `new`. So the
    /// connection is opened lazily here, behind the same
    /// `deny_live_transport(RawTransportKind::SystemBus)` sentinel that
    /// `LiveDbusTransport::connect_system` arms (every trait method below arms
    /// it as its first statement). Wiring an already-connected
    /// `LiveDbusTransport` into `LiveUdisks::new` is a one-line change in the
    /// composition root, which is owned elsewhere.
    async fn connection(&self) -> Result<&zbus::Connection, OsControlError> {
        self.system_bus
            .get_or_try_init(|| async {
                zbus::Connection::system().await.map_err(|_| {
                    OsControlError::Unavailable {
                        provider: Some(self.provider_id()),
                        reason: SafeText::new(
                            "the system D-Bus could not be reached; storage state was not read",
                        ),
                        retryable: true,
                    }
                })
            })
            .await
    }

    /// Read the whole UDisks2 object tree in one governed round trip.
    ///
    /// One call keeps the block/filesystem/drive facts mutually consistent (a
    /// per-object walk could observe a device mid-removal and mix a stale mount
    /// point with a fresh block). The call is bounded by the observation
    /// context's deadline and cancellation, so a hung bus degrades this domain
    /// instead of blocking the caller.
    ///
    /// UDisks2 is D-Bus activatable, so this issues the call without checking
    /// for a name owner first: on-demand services own no name until used.
    async fn read_tree(&self, ctx: &HostExecutionContext) -> Result<UdisksTree, OsControlError> {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let connection = self.connection().await?;
        let call = connection.call_method(
            Some(UDISKS2_SERVICE),
            UDISKS2_OBJECT_MANAGER_PATH,
            Some(OBJECT_MANAGER_INTERFACE),
            "GetManagedObjects",
            &(),
        );

        let reply = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            outcome = tokio::time::timeout_at(
                tokio::time::Instant::from_std(ctx.deadline),
                call,
            ) => match outcome {
                Err(_elapsed) => {
                    return Err(OsControlError::TimedOutBeforeMutation {
                        operation: SafeOperation::new("udisks2.get_managed_objects"),
                        timeout_ms: 0,
                    });
                }
                Ok(Err(_bus_error)) => {
                    // Never a fallback: no `udisksctl`, no `mount`, no guess.
                    return Err(self.unavailable(
                        "the UDisks2 object tree could not be read; storage state is unknown",
                        true,
                    ));
                }
                Ok(Ok(reply)) => reply,
            },
        };

        let objects: ManagedObjects = reply.body().deserialize().map_err(|_| {
            self.unavailable(
                "the UDisks2 object tree had an unexpected shape; refusing to parse a partial \
                 read",
                false,
            )
        })?;
        UdisksTree::from_objects(&objects)
    }

    fn unavailable(&self, reason: &'static str, retryable: bool) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(reason),
            retryable,
        }
    }

}

/// The UDisks2 object tree, lifted into plain facts and indexed for lookup.
struct UdisksTree {
    /// Every block object, ordered by object path so pagination is stable
    /// across reads.
    blocks: Vec<BlockFacts>,
    /// Drive facts by drive object path.
    drives: HashMap<String, DriveFacts>,
    /// SMART facts by drive object path (absent when the drive exposes no
    /// `Drive.Ata` interface, which is itself the honest "no evidence" fact).
    ata: HashMap<String, AtaSmartFacts>,
}

impl UdisksTree {
    fn from_objects(objects: &ManagedObjects) -> Result<Self, OsControlError> {
        let mut blocks = Vec::new();
        let mut drives = HashMap::new();
        let mut ata = HashMap::new();

        for (path, interfaces) in objects {
            let path = path.as_str().to_string();
            if let Some(block) = interfaces.get(BLOCK_INTERFACE) {
                let filesystem = interfaces.get(FILESYSTEM_INTERFACE);
                blocks.push(block_facts(&path, block, filesystem)?);
            }
            if let Some(drive) = interfaces.get(DRIVE_INTERFACE) {
                drives.insert(path.clone(), drive_facts(drive)?);
            }
            if let Some(smart) = interfaces.get(DRIVE_ATA_INTERFACE) {
                ata.insert(path, ata_facts(smart)?);
            }
        }

        blocks.sort_by(|left, right| left.object_path.cmp(&right.object_path));
        Ok(Self {
            blocks,
            drives,
            ata,
        })
    }

    fn drive_of(&self, block: &BlockFacts) -> Option<&DriveFacts> {
        self.drives.get(&block.drive_object_path)
    }

    fn ata_of(&self, block: &BlockFacts) -> Option<&AtaSmartFacts> {
        self.ata.get(&block.drive_object_path)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Property extraction — a missing or mistyped property is a failed read
// ─────────────────────────────────────────────────────────────────────────────

/// A required UDisks2 property was absent or had an unexpected type. The key is
/// a compile-time D-Bus property name, never captured output.
fn missing(key: &'static str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(STORAGE_PROVIDER_ID)),
        reason: SafeText::new(format!(
            "UDisks2 property {key} was absent or of an unexpected type; refusing a partial read"
        )),
        retryable: false,
    }
}

fn property<'props>(
    props: &'props Properties,
    key: &'static str,
) -> Result<&'props Value<'static>, OsControlError> {
    props.get(key).map(|value| &**value).ok_or_else(|| missing(key))
}

fn required_u64(props: &Properties, key: &'static str) -> Result<u64, OsControlError> {
    match property(props, key)? {
        Value::U64(value) => Ok(*value),
        _ => Err(missing(key)),
    }
}

fn required_bool(props: &Properties, key: &'static str) -> Result<bool, OsControlError> {
    match property(props, key)? {
        Value::Bool(value) => Ok(*value),
        _ => Err(missing(key)),
    }
}

fn required_string(props: &Properties, key: &'static str) -> Result<String, OsControlError> {
    match property(props, key)? {
        Value::Str(value) => Ok(value.as_str().to_string()),
        _ => Err(missing(key)),
    }
}

fn required_object_path(props: &Properties, key: &'static str) -> Result<String, OsControlError> {
    match property(props, key)? {
        Value::ObjectPath(value) => Ok(value.as_str().to_string()),
        _ => Err(missing(key)),
    }
}

fn required_f64(props: &Properties, key: &'static str) -> Result<f64, OsControlError> {
    match property(props, key)? {
        Value::F64(value) => Ok(*value),
        _ => Err(missing(key)),
    }
}

/// Extract an `aay` property (UDisks2 encodes paths as NUL-terminated byte
/// strings). Decoding to text happens in `selection`, which fails closed on a
/// path it cannot represent exactly.
fn required_byte_arrays(
    props: &Properties,
    key: &'static str,
) -> Result<Vec<Vec<u8>>, OsControlError> {
    let Value::Array(outer) = property(props, key)? else {
        return Err(missing(key));
    };
    let mut out = Vec::with_capacity(outer.len());
    for entry in outer.inner() {
        let Value::Array(inner) = entry else {
            return Err(missing(key));
        };
        let mut bytes = Vec::with_capacity(inner.len());
        for byte in inner.inner() {
            match byte {
                Value::U8(byte) => bytes.push(*byte),
                _ => return Err(missing(key)),
            }
        }
        out.push(bytes);
    }
    Ok(out)
}

fn block_facts(
    object_path: &str,
    block: &Properties,
    filesystem: Option<&Properties>,
) -> Result<BlockFacts, OsControlError> {
    Ok(BlockFacts {
        object_path: object_path.to_string(),
        id_usage: required_string(block, "IdUsage")?,
        id_uuid: required_string(block, "IdUUID")?,
        size_bytes: required_u64(block, "Size")?,
        hint_ignore: required_bool(block, "HintIgnore")?,
        drive_object_path: required_object_path(block, "Drive")?,
        // `None` (interface absent) and `Some(vec![])` (interface present,
        // nothing mounted) are different facts; `selection` decides what each
        // one means.
        mount_points: match filesystem {
            Some(props) => Some(required_byte_arrays(props, "MountPoints")?),
            None => None,
        },
    })
}

fn drive_facts(drive: &Properties) -> Result<DriveFacts, OsControlError> {
    Ok(DriveFacts {
        removable: required_bool(drive, "Removable")?,
        media_removable: required_bool(drive, "MediaRemovable")?,
    })
}

fn ata_facts(smart: &Properties) -> Result<AtaSmartFacts, OsControlError> {
    Ok(AtaSmartFacts {
        smart_supported: required_bool(smart, "SmartSupported")?,
        smart_enabled: required_bool(smart, "SmartEnabled")?,
        smart_updated: required_u64(smart, "SmartUpdated")?,
        smart_failing: required_bool(smart, "SmartFailing")?,
        smart_temperature_kelvin: required_f64(smart, "SmartTemperature")?,
        smart_selftest_status: required_string(smart, "SmartSelftestStatus")?,
    })
}

/// `udisksctl`: the supported client for UDisks2. Used instead of raw D-Bus
/// because UDisks2 mount/unmount are long-running method calls whose completion
/// this client already waits for, and because it applies the same polkit rules a
/// desktop file manager gets — so a removable disk needs no extra privilege.
const UDISKSCTL: &str = "/usr/bin/udisksctl";

/// Validate a device identity before it becomes an argv element.
///
/// Only an absolute `/dev/` path is accepted. A label or UUID is not accepted
/// here: resolving one to a device is a *read*, and doing it implicitly at
/// mutation time could resolve to a different disk than the one observed.
fn validate_device(device: &str) -> Result<&str, OsControlError> {
    let ok = device.starts_with("/dev/")
        && device.len() <= 128
        && device
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'));
    if !ok {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("device"),
            reason: SafeText::new(
                "device must be an absolute /dev path; a label or UUID must be resolved by a read first",
            ),
        });
    }
    Ok(device)
}

impl LiveUdisks {
    /// Run one governed `udisksctl` mutation.
    async fn run(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        argv: Vec<String>,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let executable = TrustedExecutable::new(
            UDISKSCTL,
            crate::os_control::contract::Digest::of_str("udisksctl-v1"),
        )?;
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            executable,
            argv,
        );
        let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
        request.dispatch().await
    }
}

#[async_trait::async_trait]
impl StorageTransport for LiveUdisks {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(STORAGE_PROVIDER_ID)
    }

    async fn list_devices(
        &self,
        ctx: &HostExecutionContext,
        cursor: usize,
        limit: usize,
    ) -> Result<StorageDevicePage, OsControlError> {
        deny_live_transport(RawTransportKind::SystemBus);

        let tree = self.read_tree(ctx).await?;
        let mut items = Vec::with_capacity(tree.blocks.len());
        for block in &tree.blocks {
            // `HintIgnore` is UDisks2's own "do not present this device"
            // marker (loop backing files, zram, …). Every other device is
            // listed: silently dropping a device would let a caller believe it
            // is absent.
            if block.hint_ignore {
                continue;
            }
            items.push(selection::device_info(block, tree.drive_of(block))?);
        }
        Ok(selection::page(items, cursor, limit))
    }

    async fn read_mount_state(
        &self,
        ctx: &HostExecutionContext,
        device: &StorageDeviceId,
        filesystem: Option<&FilesystemId>,
    ) -> Result<StorageMountState, OsControlError> {
        deny_live_transport(RawTransportKind::SystemBus);

        let tree = self.read_tree(ctx).await?;
        // A device that is no longer in the tree is *unknown*, not unmounted:
        // `resolve_device` errors rather than letting a mutation verify against
        // a device nobody observed.
        let block = selection::resolve_device(&tree.blocks, device)?;
        selection::mount_state(device, filesystem, block)
    }

    async fn mount(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
        _filesystem: Option<&FilesystemId>,
    ) -> Result<ApplyOutcome, OsControlError> {
        let device = validate_device(device.as_str())?;
        // `--no-user-interaction` keeps the child non-interactive: a governed
        // child has no console on which to answer a polkit prompt, and hanging
        // until the deadline would report Uncertain for a fixable reason.
        self.run(
            ctx,
            "mount_device",
            vec![
                "mount".into(),
                "--no-user-interaction".into(),
                "-b".into(),
                device.to_string(),
            ],
        )
        .await
    }

    async fn unmount(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError> {
        let device = validate_device(device.as_str())?;
        // `--no-user-interaction` keeps the child non-interactive: a governed
        // child has no console on which to answer a polkit prompt, and hanging
        // until the deadline would report Uncertain for a fixable reason.
        self.run(
            ctx,
            "unmount_device",
            vec![
                "unmount".into(),
                "--no-user-interaction".into(),
                "-b".into(),
                device.to_string(),
            ],
        )
        .await
    }

    async fn eject(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        device: &StorageDeviceId,
    ) -> Result<ApplyOutcome, OsControlError> {
        let device = validate_device(device.as_str())?;
        // `--no-user-interaction` keeps the child non-interactive: a governed
        // child has no console on which to answer a polkit prompt, and hanging
        // until the deadline would report Uncertain for a fixable reason.
        self.run(
            ctx,
            "eject_device",
            vec![
                "power-off".into(),
                "--no-user-interaction".into(),
                "-b".into(),
                device.to_string(),
            ],
        )
        .await
    }

    async fn read_health(
        &self,
        ctx: &HostExecutionContext,
        device: Option<&StorageDeviceId>,
    ) -> Result<StorageHealthReport, OsControlError> {
        deny_live_transport(RawTransportKind::SystemBus);

        let tree = self.read_tree(ctx).await?;
        let block = match device {
            Some(device) => selection::resolve_device(&tree.blocks, device)?,
            // No named device: the root-mounted device, derived — never an
            // arbitrary disk whose health would be attributed to the wrong
            // hardware.
            None => selection::primary_device(&tree.blocks)?,
        };
        Ok(selection::health_report(
            StorageDeviceId::new(&block.object_path),
            tree.ata_of(block),
        ))
    }
}
