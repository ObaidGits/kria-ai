//! UDisks2 read normalization for the storage domain (design §9.1, §12).
//!
//! linux-os-control-production **Task 2/§5** — live provider reads. The live
//! adapter ([`crate::os_control::linux::providers::udisks::LiveUdisks`]) lifts
//! UDisks2 D-Bus properties into the plain fact structs below, then every
//! decision about what those facts *mean* happens here, in pure functions with
//! unit tests.
//!
//! # Why every function fails closed
//!
//! This domain decides whether a device is mounted and which device an
//! instruction targets. A fabricated answer is not a cosmetic defect:
//!
//! * "not mounted" and "could not determine the mount state" must never be
//!   conflated. The runtime verifies a mutation by re-reading state, so a
//!   substituted `mounted: false` would let an unmount "verify" against a fact
//!   that was never observed.
//! * a target is resolved **only** from a stable identity (the UDisks2 block
//!   object path, or the filesystem UUID). Never from `IdLabel`, which is
//!   neither unique nor stable, and never from a `/dev/sdX` node, which can
//!   name a different disk after a re-plug or reboot.
//!
//! So: missing, mistyped, or self-contradictory UDisks2 facts return an
//! [`OsControlError`], never a default value.

use crate::os_control::contract::{
    BoundedVec, Digest, ProviderId, SafeCandidate, SafeField, SafeText,
};
use crate::os_control::error::OsControlError;
use crate::os_control::storage::{
    FilesystemId, HealthAvailability, MountLabel, StorageDeviceId, StorageDeviceInfo,
    StorageDevicePage, StorageHealthReport, StorageMountState, MAX_STORAGE_DEVICE_PAGE,
    STORAGE_PROVIDER_ID,
};

// ─────────────────────────────────────────────────────────────────────────────
// UDisks2 bus names (single definition site, shared by the live adapter)
// ─────────────────────────────────────────────────────────────────────────────

/// The UDisks2 well-known bus name (system bus, **D-Bus activatable**: it may
/// own no name until the first call, so an absent name owner is not an absent
/// service).
pub const UDISKS2_SERVICE: &str = "org.freedesktop.UDisks2";

/// The UDisks2 object-manager path.
pub const UDISKS2_OBJECT_MANAGER_PATH: &str = "/org/freedesktop/UDisks2";

/// `org.freedesktop.DBus.ObjectManager`.
pub const OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";

/// `org.freedesktop.UDisks2.Block`.
pub const BLOCK_INTERFACE: &str = "org.freedesktop.UDisks2.Block";

/// `org.freedesktop.UDisks2.Filesystem`.
pub const FILESYSTEM_INTERFACE: &str = "org.freedesktop.UDisks2.Filesystem";

/// `org.freedesktop.UDisks2.Drive`.
pub const DRIVE_INTERFACE: &str = "org.freedesktop.UDisks2.Drive";

/// `org.freedesktop.UDisks2.Drive.Ata` — the only source of SMART evidence.
pub const DRIVE_ATA_INTERFACE: &str = "org.freedesktop.UDisks2.Drive.Ata";

/// The `Block.IdUsage` value that means "this block device carries a mountable
/// filesystem".
pub const ID_USAGE_FILESYSTEM: &str = "filesystem";

/// The `Block.Drive` value UDisks2 uses for "no backing drive object".
pub const NO_DRIVE_OBJECT_PATH: &str = "/";

/// Cap on candidates reported for an ambiguous storage target.
const MAX_AMBIGUOUS_CANDIDATES: usize = 16;

/// Plausible drive-temperature window in Kelvin (≈ −73 °C … 127 °C). A reading
/// outside it is reported as *no* temperature plus a degraded availability,
/// never as a number a user might act on.
const MIN_PLAUSIBLE_KELVIN: f64 = 200.0;
/// Upper bound of [`MIN_PLAUSIBLE_KELVIN`]'s window.
const MAX_PLAUSIBLE_KELVIN: f64 = 400.0;

// ─────────────────────────────────────────────────────────────────────────────
// Plain facts lifted out of D-Bus variants
// ─────────────────────────────────────────────────────────────────────────────

/// The `org.freedesktop.UDisks2.Block` (+ optional `Filesystem`) facts one
/// block object contributes to a storage read, already lifted out of D-Bus
/// variants so every decision below is pure and unit-testable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockFacts {
    /// The block object path — the canonical [`StorageDeviceId`] source.
    pub object_path: String,
    /// `Block.IdUsage` (`"filesystem"`, `"raw"`, `"crypto"`, `""`, …).
    pub id_usage: String,
    /// `Block.IdUUID` — the stable filesystem identity when non-empty.
    pub id_uuid: String,
    /// `Block.Size` in bytes.
    pub size_bytes: u64,
    /// `Block.HintIgnore` — the device should not be presented to the user.
    pub hint_ignore: bool,
    /// `Block.Drive` object path, or [`NO_DRIVE_OBJECT_PATH`] when the block
    /// has no backing drive (loop, ramdisk, device-mapper).
    pub drive_object_path: String,
    /// `Some(raw_mount_points)` when the object carries the `Filesystem`
    /// interface; `None` when it does **not**.
    ///
    /// The distinction is load-bearing: `Some(vec![])` is a positive
    /// observation of "not mounted", while `None` means the interface that
    /// reports mount points was absent, which is only an observation of "not
    /// mounted" if the block also reports no filesystem.
    pub mount_points: Option<Vec<Vec<u8>>>,
}

/// The `org.freedesktop.UDisks2.Drive` facts a listing needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DriveFacts {
    /// `Drive.Removable` — the drive itself can be detached (USB, SD).
    pub removable: bool,
    /// `Drive.MediaRemovable` — the media can leave the drive (optical).
    pub media_removable: bool,
}

/// The `org.freedesktop.UDisks2.Drive.Ata` SMART facts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AtaSmartFacts {
    /// `SmartSupported`.
    pub smart_supported: bool,
    /// `SmartEnabled`.
    pub smart_enabled: bool,
    /// `SmartUpdated` (µs since the epoch; `0` = never updated).
    pub smart_updated: u64,
    /// `SmartFailing`.
    pub smart_failing: bool,
    /// `SmartTemperature` in Kelvin (`0.0` = unknown, per UDisks2).
    pub smart_temperature_kelvin: f64,
    /// `SmartSelftestStatus` (`"success"`, `"error_read"`, `"fatal"`, …).
    pub smart_selftest_status: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Error constructors
// ─────────────────────────────────────────────────────────────────────────────

fn provider() -> ProviderId {
    ProviderId::new(STORAGE_PROVIDER_ID)
}

/// A read that could not be completed honestly. `retryable` is true only when
/// re-reading a settled object tree could plausibly succeed (a torn read).
fn unreadable(reason: &'static str, retryable: bool) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(provider()),
        reason: SafeText::new(reason),
        retryable,
    }
}

fn invalid(field: &'static str, reason: &'static str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsers / normalizers
// ─────────────────────────────────────────────────────────────────────────────

/// Decode `Filesystem.MountPoints` (`aay`: an array of NUL-terminated byte
/// strings) into mount-point paths.
///
/// Fails closed rather than approximating: a path that is not valid UTF-8, or
/// an entry that decodes to nothing, is an error. Reporting a lossy path would
/// name a directory the caller could act on that is not the one that is
/// mounted.
pub fn decode_mount_points(raw: &[Vec<u8>]) -> Result<Vec<String>, OsControlError> {
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        let trimmed: &[u8] = match entry.iter().position(|byte| *byte == 0) {
            Some(nul) => &entry[..nul],
            None => entry.as_slice(),
        };
        if trimmed.is_empty() {
            return Err(unreadable(
                "UDisks2 reported an empty mount-point entry; the mount state could not be \
                 determined",
                true,
            ));
        }
        let path = std::str::from_utf8(trimmed).map_err(|_| {
            unreadable(
                "the reported mount point is not valid UTF-8; refusing to report an \
                 approximated path",
                false,
            )
        })?;
        out.push(path.to_string());
    }
    Ok(out)
}

/// The stable filesystem identity a block device currently carries, if any.
///
/// This is the filesystem **UUID**, never `IdLabel`: a label is a human string
/// that two devices can share and that a user can change between observations.
/// A filesystem with no UUID has no addressable identity here (`None`) — it is
/// still mountable by device identity.
#[must_use]
pub fn observed_filesystem_id(block: &BlockFacts) -> Option<FilesystemId> {
    if block.id_usage == ID_USAGE_FILESYSTEM && !block.id_uuid.trim().is_empty() {
        Some(FilesystemId::new(block.id_uuid.trim()))
    } else {
        None
    }
}

/// The mount points a block object is currently mounted at.
///
/// * `Filesystem` interface present → its `MountPoints` are authoritative
///   (empty ⇒ positively not mounted).
/// * interface absent **and** the block reports no filesystem → positively not
///   mounted: there is nothing on it to mount.
/// * interface absent **but** the block reports `IdUsage == "filesystem"` →
///   contradictory facts, so the mount state is *unknown* and this errors.
///   Returning "not mounted" here is the exact conflation this domain must
///   never make.
fn mount_points_of(block: &BlockFacts) -> Result<Vec<String>, OsControlError> {
    match &block.mount_points {
        Some(raw) => decode_mount_points(raw),
        None => {
            if block.id_usage == ID_USAGE_FILESYSTEM {
                return Err(unreadable(
                    "UDisks2 reports a filesystem on this device but exposes no Filesystem \
                     interface; the mount state could not be determined",
                    true,
                ));
            }
            Ok(Vec::new())
        }
    }
}

/// Normalize one block object into the governed [`StorageMountState`]
/// observation for `device` (+ the `filesystem` the request named).
///
/// # Why the request's filesystem identity is echoed back
///
/// [`StorageMountState::observation_digest`] binds device + filesystem +
/// mounted, and the runtime compares that digest against
/// `StorageRequest::desired_state()`, whose filesystem is exactly the one the
/// *request* named (`None` for unmount/eject). Substituting the observed
/// filesystem UUID would make a successful unmount compare unequal and be
/// reported as a verification contradiction. The observed identity is instead
/// checked against the requested one below, so a mismatch is refused outright
/// rather than silently observed on the wrong filesystem.
pub fn mount_state(
    device: &StorageDeviceId,
    filesystem: Option<&FilesystemId>,
    block: &BlockFacts,
) -> Result<StorageMountState, OsControlError> {
    if let Some(requested) = filesystem {
        if observed_filesystem_id(block).as_ref() != Some(requested) {
            return Err(invalid(
                "filesystem_id",
                "the named filesystem is not the one this device currently carries; refusing to \
                 report another filesystem's mount state",
            ));
        }
    }

    let points = mount_points_of(block)?;
    let mounted = !points.is_empty();
    Ok(StorageMountState::new(
        device.clone(),
        filesystem.cloned(),
        mounted,
        points.into_iter().next(),
    ))
}

/// Normalize one block object (+ its backing drive, when it has one) into a
/// listing item.
///
/// `free_bytes` is `0`: UDisks2 exposes no free-space property, and the field's
/// contract documents `0` for "unmounted/unknown". Capacity, mount state and
/// removability are all read facts — none is defaulted.
pub fn device_info(
    block: &BlockFacts,
    drive: Option<&DriveFacts>,
) -> Result<StorageDeviceInfo, OsControlError> {
    let points = mount_points_of(block)?;
    let mounted = !points.is_empty();

    // A block with no backing drive object (loop, ramdisk, LVM) is positively
    // not removable. A block that *names* a drive the object tree does not
    // contain is a torn read, not a fixed disk: refuse instead of telling the
    // caller a USB stick is non-removable.
    let removable = if block.drive_object_path.is_empty()
        || block.drive_object_path == NO_DRIVE_OBJECT_PATH
    {
        false
    } else {
        let drive = drive.ok_or_else(|| {
            unreadable(
                "a block device names a drive that is absent from the UDisks2 object tree; \
                 refusing to report removability from an incomplete read",
                true,
            )
        })?;
        drive.removable || drive.media_removable
    };

    Ok(StorageDeviceInfo {
        device_id: StorageDeviceId::new(&block.object_path),
        filesystem_id: observed_filesystem_id(block),
        capacity_bytes: block.size_bytes,
        free_bytes: 0,
        mount_state: if mounted {
            MountLabel::Mounted
        } else {
            MountLabel::Unmounted
        },
        mount_point: points.into_iter().next(),
        removable,
    })
}

/// Resolve a caller-supplied [`StorageDeviceId`] to exactly one block object.
///
/// Accepts only stable identities:
///
/// 1. the exact UDisks2 block object path (what `list_storage_devices`
///    reports), or
/// 2. the exact filesystem UUID (ASCII-case-insensitive, as FAT/NTFS serials
///    are upper-cased).
///
/// A `/dev/...` node is refused: it is reassigned across re-plug and reboot, so
/// acting on it can target a different disk than the one the caller saw. A
/// human label is never matched at all. Two devices sharing a UUID (a cloned
/// disk) is [`OsControlError::AmbiguousTarget`], never an arbitrary pick.
pub fn resolve_device<'facts>(
    blocks: &'facts [BlockFacts],
    device: &StorageDeviceId,
) -> Result<&'facts BlockFacts, OsControlError> {
    let raw = device.as_str().trim();
    if raw.is_empty() {
        return Err(invalid("device", "a storage device identity is required"));
    }
    if raw.starts_with("/dev/") {
        return Err(invalid(
            "device",
            "a kernel device node is not a stable device identity; use the device_id or \
             filesystem UUID reported by list_storage_devices",
        ));
    }

    if let Some(found) = blocks.iter().find(|block| block.object_path == raw) {
        return Ok(found);
    }

    let by_uuid: Vec<&BlockFacts> = blocks
        .iter()
        .filter(|block| {
            !block.id_uuid.trim().is_empty() && block.id_uuid.trim().eq_ignore_ascii_case(raw)
        })
        .collect();
    match by_uuid.len() {
        1 => Ok(by_uuid[0]),
        0 => Err(invalid(
            "device",
            "no storage device with this identity is present; refusing to guess a target",
        )),
        _ => Err(OsControlError::AmbiguousTarget {
            kind: SafeText::new("storage_device"),
            candidates: BoundedVec::from_iter_capped(
                by_uuid.into_iter().map(|block| SafeCandidate {
                    label: SafeText::new("storage device"),
                    identity: Digest::of_str(&block.object_path),
                }),
                MAX_AMBIGUOUS_CANDIDATES,
            ),
        }),
    }
}

/// The device to report health for when the caller named none.
///
/// "Primary" is derived, not guessed: it is the block object mounted at `/`.
/// When no object reports the root mount point, this errors rather than
/// nominating an arbitrary disk whose health the caller would attribute to the
/// wrong hardware.
pub fn primary_device<'facts>(
    blocks: &'facts [BlockFacts],
) -> Result<&'facts BlockFacts, OsControlError> {
    let mut root: Option<&BlockFacts> = None;
    for block in blocks {
        let Some(raw) = block.mount_points.as_ref() else {
            continue;
        };
        if decode_mount_points(raw)?.iter().any(|point| point == "/") {
            if root.is_some() {
                return Err(unreadable(
                    "more than one block device reports the root mount point; refusing to \
                     nominate a primary device",
                    true,
                ));
            }
            root = Some(block);
        }
    }
    root.ok_or_else(|| {
        unreadable(
            "no block device reports the root mount point, so a primary device could not be \
             identified; name a device explicitly",
            false,
        )
    })
}

/// Convert a SMART temperature in Kelvin to millikelvin.
///
/// `None` for UDisks2's documented `0.0` ("unknown") and for any reading
/// outside the plausible window — an implausible number is not evidence.
#[must_use]
pub fn temperature_millikelvin(kelvin: f64) -> Option<u64> {
    if !kelvin.is_finite() || kelvin <= 0.0 {
        return None;
    }
    if !(MIN_PLAUSIBLE_KELVIN..=MAX_PLAUSIBLE_KELVIN).contains(&kelvin) {
        return None;
    }
    Some((kelvin * 1000.0).round() as u64)
}

/// Normalize a SMART self-test status into the closed health vocabulary.
fn selftest_indicates_error(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase();
    status.starts_with("error") || status == "fatal"
}

/// Normalize SMART facts into the governed [`StorageHealthReport`].
///
/// Missing evidence is [`HealthAvailability::Unavailable`] with **no**
/// `health_state` — never a fabricated healthy/unhealthy verdict (OSC-012.5).
/// SMART that is enabled but never updated is [`HealthAvailability::Degraded`]:
/// the query ran, the evidence is not there yet.
#[must_use]
pub fn health_report(
    device_id: StorageDeviceId,
    ata: Option<&AtaSmartFacts>,
) -> StorageHealthReport {
    let unavailable = |device_id| StorageHealthReport {
        device_id,
        availability: HealthAvailability::Unavailable,
        health_state: None,
        temperature_millikelvin: None,
    };

    let Some(ata) = ata else {
        // No `Drive.Ata` interface: no health evidence exists for this device.
        return unavailable(device_id);
    };
    if !ata.smart_supported || !ata.smart_enabled {
        return unavailable(device_id);
    }
    if ata.smart_updated == 0 {
        return StorageHealthReport {
            device_id,
            availability: HealthAvailability::Degraded,
            health_state: None,
            temperature_millikelvin: None,
        };
    }

    let temperature = temperature_millikelvin(ata.smart_temperature_kelvin);
    // A non-zero temperature we refused to trust means reduced fidelity, not a
    // clean read.
    let availability = if ata.smart_temperature_kelvin > 0.0 && temperature.is_none() {
        HealthAvailability::Degraded
    } else {
        HealthAvailability::Available
    };
    let health_state = if ata.smart_failing {
        "failing"
    } else if selftest_indicates_error(&ata.smart_selftest_status) {
        "warning"
    } else {
        "ok"
    };

    StorageHealthReport {
        device_id,
        availability,
        health_state: Some(health_state.to_string()),
        temperature_millikelvin: temperature,
    }
}

/// Slice a deterministically ordered device list into one bounded page.
#[must_use]
pub fn page(items: Vec<StorageDeviceInfo>, cursor: usize, limit: usize) -> StorageDevicePage {
    let limit = limit.clamp(1, MAX_STORAGE_DEVICE_PAGE);
    if cursor >= items.len() {
        return StorageDevicePage {
            items: Vec::new(),
            truncated: false,
        };
    }
    let end = cursor.saturating_add(limit).min(items.len());
    StorageDevicePage {
        truncated: end < items.len(),
        items: items[cursor..end].to_vec(),
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;
    use crate::os_control::runtime::NormalizedObservation;
    use crate::os_control::storage::{StorageOp, StorageRequest};

    fn nul(path: &str) -> Vec<u8> {
        let mut bytes = path.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    fn filesystem_block(path: &str, uuid: &str, mounted_at: Option<&str>) -> BlockFacts {
        BlockFacts {
            object_path: path.to_string(),
            id_usage: ID_USAGE_FILESYSTEM.to_string(),
            id_uuid: uuid.to_string(),
            size_bytes: 64 * 1024 * 1024 * 1024,
            hint_ignore: false,
            drive_object_path: "/org/freedesktop/UDisks2/drives/Generic_Flash_Disk".to_string(),
            mount_points: Some(mounted_at.map(|p| vec![nul(p)]).unwrap_or_default()),
        }
    }

    // ── decode_mount_points ─────────────────────────────────────────────────

    #[test]
    fn mount_points_are_decoded_without_the_nul_terminator() {
        let raw = vec![nul("/run/media/obaid/USB"), nul("/mnt/second")];
        assert_eq!(
            decode_mount_points(&raw).unwrap(),
            vec![
                "/run/media/obaid/USB".to_string(),
                "/mnt/second".to_string()
            ]
        );
    }

    #[test]
    fn no_mount_points_is_an_empty_list_not_an_error() {
        assert!(decode_mount_points(&[]).unwrap().is_empty());
    }

    #[test]
    fn non_utf8_or_empty_mount_point_is_an_error_never_a_default() {
        // A lossy path would name a directory that is not the mounted one.
        assert!(decode_mount_points(&[vec![0x2f, 0xff, 0xfe, 0x00]]).is_err());
        assert!(decode_mount_points(&[vec![0x00]]).is_err());
    }

    // ── mount_state ─────────────────────────────────────────────────────────

    #[test]
    fn mounted_filesystem_reports_its_mount_point() {
        let block = filesystem_block(
            "/org/freedesktop/UDisks2/block_devices/sdb1",
            "A1B2-C3D4",
            Some("/run/media/obaid/USB"),
        );
        let device = StorageDeviceId::new(&block.object_path);
        let state = mount_state(&device, None, &block).unwrap();
        assert!(state.mounted);
        assert_eq!(state.mount_point.as_deref(), Some("/run/media/obaid/USB"));
    }

    #[test]
    fn filesystem_interface_with_no_mount_points_is_positively_unmounted() {
        let block =
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "A1B2-C3D4", None);
        let device = StorageDeviceId::new(&block.object_path);
        let state = mount_state(&device, None, &block).unwrap();
        assert!(!state.mounted);
        assert_eq!(state.mount_point, None);
    }

    #[test]
    fn raw_block_without_a_filesystem_interface_is_positively_unmounted() {
        // Real tools emit this constantly: an extended partition, a swap
        // device, a LUKS container. There is nothing on it to mount.
        let block = BlockFacts {
            object_path: "/org/freedesktop/UDisks2/block_devices/sda2".to_string(),
            id_usage: "crypto".to_string(),
            id_uuid: "0c1d2e3f-4455-6677-8899-aabbccddeeff".to_string(),
            size_bytes: 512,
            hint_ignore: false,
            drive_object_path: NO_DRIVE_OBJECT_PATH.to_string(),
            mount_points: None,
        };
        let device = StorageDeviceId::new(&block.object_path);
        assert!(!mount_state(&device, None, &block).unwrap().mounted);
    }

    #[test]
    fn filesystem_without_the_filesystem_interface_is_unknown_not_unmounted() {
        // The conflation this domain must never make: contradictory facts mean
        // the mount state was NOT read, so an unmount must not "verify" here.
        let block = BlockFacts {
            mount_points: None,
            ..filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "A1B2-C3D4", None)
        };
        let device = StorageDeviceId::new(&block.object_path);
        assert!(mount_state(&device, None, &block).is_err());
    }

    #[test]
    fn a_filesystem_the_device_does_not_carry_is_refused() {
        let block = filesystem_block(
            "/org/freedesktop/UDisks2/block_devices/sdb1",
            "A1B2-C3D4",
            Some("/run/media/obaid/USB"),
        );
        let device = StorageDeviceId::new(&block.object_path);
        let wrong = FilesystemId::new("DEAD-BEEF");
        assert!(mount_state(&device, Some(&wrong), &block).is_err());
    }

    #[test]
    fn observation_digest_matches_the_unmount_desired_state() {
        // Guards a silent verification bug: the observation must echo the
        // request's filesystem identity (None for unmount), not the observed
        // UUID, or a successful unmount would report as contradicted.
        let block =
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "A1B2-C3D4", None);
        let device = StorageDeviceId::new(&block.object_path);
        let observed = mount_state(&device, None, &block).unwrap();
        let request = StorageRequest {
            action: "unmount_device".to_string(),
            params: serde_json::Value::Null,
            op: StorageOp::Unmount {
                device: device.clone(),
            },
        };
        assert_eq!(
            observed.observation_digest(),
            request.desired_state().observation_digest()
        );
    }

    // ── device_info ─────────────────────────────────────────────────────────

    #[test]
    fn removable_media_is_reported_from_the_drive() {
        let block = filesystem_block(
            "/org/freedesktop/UDisks2/block_devices/sdb1",
            "A1B2-C3D4",
            Some("/run/media/obaid/USB"),
        );
        let drive = DriveFacts {
            removable: true,
            media_removable: false,
        };
        let info = device_info(&block, Some(&drive)).unwrap();
        assert!(info.removable);
        assert_eq!(info.mount_state, MountLabel::Mounted);
        assert_eq!(info.capacity_bytes, 64 * 1024 * 1024 * 1024);
        assert_eq!(info.free_bytes, 0, "UDisks2 reports no free space");
        assert_eq!(
            info.filesystem_id.as_ref().map(FilesystemId::as_str),
            Some("A1B2-C3D4")
        );
    }

    #[test]
    fn loop_device_without_a_drive_is_not_removable() {
        let block = BlockFacts {
            drive_object_path: NO_DRIVE_OBJECT_PATH.to_string(),
            ..filesystem_block("/org/freedesktop/UDisks2/block_devices/loop0", "", None)
        };
        assert!(!device_info(&block, None).unwrap().removable);
    }

    #[test]
    fn a_missing_drive_object_is_an_error_never_non_removable() {
        let block =
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "A1B2-C3D4", None);
        assert!(device_info(&block, None).is_err());
    }

    // ── resolve_device ──────────────────────────────────────────────────────

    #[test]
    fn a_device_resolves_by_object_path_or_filesystem_uuid() {
        let blocks = vec![filesystem_block(
            "/org/freedesktop/UDisks2/block_devices/sdb1",
            "a1b2-c3d4",
            None,
        )];
        assert_eq!(
            resolve_device(
                &blocks,
                &StorageDeviceId::new("/org/freedesktop/UDisks2/block_devices/sdb1")
            )
            .unwrap()
            .object_path,
            blocks[0].object_path
        );
        // FAT/NTFS serials come back upper-cased from some tools.
        assert_eq!(
            resolve_device(&blocks, &StorageDeviceId::new("A1B2-C3D4"))
                .unwrap()
                .object_path,
            blocks[0].object_path
        );
    }

    #[test]
    fn a_device_node_or_label_never_resolves() {
        let mut block =
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "a1b2-c3d4", None);
        block.id_usage = ID_USAGE_FILESYSTEM.to_string();
        let blocks = vec![block];
        assert!(resolve_device(&blocks, &StorageDeviceId::new("/dev/sdb1")).is_err());
        // "KRIA-BACKUP" would be a plausible IdLabel; it is not an identity.
        assert!(resolve_device(&blocks, &StorageDeviceId::new("KRIA-BACKUP")).is_err());
        assert!(resolve_device(&blocks, &StorageDeviceId::new("")).is_err());
    }

    #[test]
    fn a_cloned_uuid_is_ambiguous_never_an_arbitrary_pick() {
        let blocks = vec![
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdb1", "same-uuid", None),
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sdc1", "same-uuid", None),
        ];
        assert!(matches!(
            resolve_device(&blocks, &StorageDeviceId::new("same-uuid")),
            Err(OsControlError::AmbiguousTarget { .. })
        ));
    }

    // ── primary_device ──────────────────────────────────────────────────────

    #[test]
    fn the_primary_device_is_the_one_mounted_at_root() {
        let blocks = vec![
            filesystem_block("/org/freedesktop/UDisks2/block_devices/sda2", "root-uuid", Some("/")),
            filesystem_block(
                "/org/freedesktop/UDisks2/block_devices/sdb1",
                "usb-uuid",
                Some("/run/media/obaid/USB"),
            ),
        ];
        assert_eq!(
            primary_device(&blocks).unwrap().object_path,
            "/org/freedesktop/UDisks2/block_devices/sda2"
        );
    }

    #[test]
    fn no_root_mount_means_no_primary_device_not_the_first_disk() {
        let blocks = vec![filesystem_block(
            "/org/freedesktop/UDisks2/block_devices/sdb1",
            "usb-uuid",
            Some("/run/media/obaid/USB"),
        )];
        assert!(primary_device(&blocks).is_err());
    }

    // ── health_report ───────────────────────────────────────────────────────

    #[test]
    fn healthy_smart_is_reported_with_a_temperature() {
        let ata = AtaSmartFacts {
            smart_supported: true,
            smart_enabled: true,
            smart_updated: 1_700_000_000_000_000,
            smart_failing: false,
            smart_temperature_kelvin: 300.15,
            smart_selftest_status: "success".to_string(),
        };
        let report = health_report(StorageDeviceId::new("dev"), Some(&ata));
        assert_eq!(report.availability, HealthAvailability::Available);
        assert_eq!(report.health_state.as_deref(), Some("ok"));
        assert_eq!(report.temperature_millikelvin, Some(300_150));
    }

    #[test]
    fn failing_and_selftest_error_map_onto_the_closed_vocabulary() {
        let base = AtaSmartFacts {
            smart_supported: true,
            smart_enabled: true,
            smart_updated: 1,
            smart_temperature_kelvin: 0.0,
            ..AtaSmartFacts::default()
        };
        let failing = AtaSmartFacts {
            smart_failing: true,
            smart_selftest_status: "success".to_string(),
            ..base.clone()
        };
        assert_eq!(
            health_report(StorageDeviceId::new("dev"), Some(&failing))
                .health_state
                .as_deref(),
            Some("failing")
        );
        let errored = AtaSmartFacts {
            smart_selftest_status: "error_read".to_string(),
            ..base
        };
        assert_eq!(
            health_report(StorageDeviceId::new("dev"), Some(&errored))
                .health_state
                .as_deref(),
            Some("warning")
        );
    }

    #[test]
    fn absent_or_disabled_smart_is_unavailable_never_a_verdict() {
        // No `Drive.Ata` interface at all (NVMe, USB bridge, virtual disk).
        let report = health_report(StorageDeviceId::new("dev"), None);
        assert_eq!(report.availability, HealthAvailability::Unavailable);
        assert_eq!(report.health_state, None);

        let disabled = AtaSmartFacts {
            smart_supported: true,
            smart_enabled: false,
            smart_updated: 1,
            ..AtaSmartFacts::default()
        };
        assert_eq!(
            health_report(StorageDeviceId::new("dev"), Some(&disabled)).availability,
            HealthAvailability::Unavailable
        );
    }

    #[test]
    fn never_updated_smart_is_degraded_with_no_state() {
        let ata = AtaSmartFacts {
            smart_supported: true,
            smart_enabled: true,
            smart_updated: 0,
            ..AtaSmartFacts::default()
        };
        let report = health_report(StorageDeviceId::new("dev"), Some(&ata));
        assert_eq!(report.availability, HealthAvailability::Degraded);
        assert_eq!(report.health_state, None);
    }

    #[test]
    fn implausible_temperature_is_dropped_and_degrades_the_report() {
        assert_eq!(temperature_millikelvin(0.0), None);
        assert_eq!(temperature_millikelvin(5_000.0), None);
        assert_eq!(temperature_millikelvin(f64::NAN), None);
        let ata = AtaSmartFacts {
            smart_supported: true,
            smart_enabled: true,
            smart_updated: 1,
            smart_temperature_kelvin: 5_000.0,
            smart_selftest_status: "success".to_string(),
            smart_failing: false,
        };
        let report = health_report(StorageDeviceId::new("dev"), Some(&ata));
        assert_eq!(report.availability, HealthAvailability::Degraded);
        assert_eq!(report.temperature_millikelvin, None);
        assert_eq!(report.health_state.as_deref(), Some("ok"));
    }

    // ── page ────────────────────────────────────────────────────────────────

    #[test]
    fn pagination_is_bounded_and_reports_truncation() {
        let items: Vec<StorageDeviceInfo> = (0..5)
            .map(|index| StorageDeviceInfo {
                device_id: StorageDeviceId::new(format!("dev-{index}")),
                filesystem_id: None,
                capacity_bytes: 0,
                free_bytes: 0,
                mount_state: MountLabel::Unmounted,
                mount_point: None,
                removable: false,
            })
            .collect();
        let first = page(items.clone(), 0, 2);
        assert_eq!(first.items.len(), 2);
        assert!(first.truncated);

        let last = page(items.clone(), 4, 2);
        assert_eq!(last.items.len(), 1);
        assert!(!last.truncated);

        let past_end = page(items, 9, 2);
        assert!(past_end.items.is_empty());
        assert!(!past_end.truncated);
    }
}
