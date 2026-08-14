//! The live firmware-awareness and hardware-sensor providers.
//!
//! linux-os-control-production task **5.4**.
//!
//! # Firmware is read-only here, on purpose
//!
//! [`LiveFirmware`] implements [`FirmwareAwarenessPort`] — there is no flash path
//! anywhere in this file. A failed firmware write can leave a machine that will not
//! boot, which is the one outcome no verification or rollback in this architecture
//! could recover from. KRIA reports what is installed and what is offered; the user
//! applies it with `fwupdmgr` or the Software app.
//!
//! # "No updates" and "could not check" are different facts
//!
//! `fwupdmgr get-updates` exits non-zero both when everything is current and when
//! the metadata is stale or the network is unreachable. Collapsing those into "up to
//! date" would be a false assurance about a security-relevant component, so
//! [`FirmwareStatus::update_source_reachable`] carries the distinction and
//! `available_version` stays `None` for "not checked".
//!
//! # Sensors come from sysfs, not a parser
//!
//! `lm-sensors` output is a human-readable format that changes between releases and
//! localizations. `/sys/class/hwmon` is a stable kernel ABI with one value per file,
//! so this reads that instead: no parsing ambiguity, no locale dependence, and no
//! process spawn per reading.

use async_trait::async_trait;

use crate::os_control::context::HostExecutionContext;
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::hardware::{
    FirmwareAwarenessPort, FirmwareDevice, FirmwareStatus, HardwareControlPort, SensorKind,
    SensorPage, SensorReading,
};
use crate::os_control::linux::providers::cli_query as cli;

const FWUPDMGR_PATHS: &[&str] = &["/usr/bin/fwupdmgr"];

/// The live firmware-awareness provider.
pub struct LiveFirmware {
    fwupdmgr: &'static str,
}

impl LiveFirmware {
    /// Compose the provider when `fwupdmgr` is present.
    #[must_use]
    pub fn discover() -> Option<Self> {
        Some(Self {
            fwupdmgr: cli::first_present(FWUPDMGR_PATHS)?,
        })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("fwupd")
    }
}

/// Parse `fwupdmgr get-devices --json` into firmware devices.
///
/// A device without a stable GUID or instance id is skipped: a human name like
/// "System Firmware" is not an identity, and two machines can present the same
/// label for different components.
fn parse_fwupd_devices(json: &str) -> Option<Vec<FirmwareDevice>> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    let devices = root.get("Devices")?.as_array()?;
    let mut out = Vec::new();
    for device in devices {
        // `DeviceId` is fwupd's stable per-device handle.
        let Some(id) = device.get("DeviceId").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let label = device
            .get("Name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(id);
        let installed = device
            .get("Version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        // Flags carry whether a reboot is needed to activate an update.
        let needs_reboot = device.get("Flags").and_then(serde_json::Value::as_array).map(|flags| {
            flags
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|flag| flag == "needs-reboot")
        });
        out.push(FirmwareDevice {
            device: id.to_string(),
            label: SafeText::new(label),
            installed_version: installed,
            // Filled in only from `get-updates`, so it stays `None` = "not
            // checked" rather than implying the device is current.
            available_version: None,
            needs_reboot,
        });
    }
    Some(out)
}

/// Merge available versions from `fwupdmgr get-updates --json` into devices.
fn merge_available(devices: &mut [FirmwareDevice], json: &str) {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return;
    };
    let Some(rows) = root.get("Devices").and_then(serde_json::Value::as_array) else {
        return;
    };
    for row in rows {
        let Some(id) = row.get("DeviceId").and_then(serde_json::Value::as_str) else {
            continue;
        };
        // The newest release offered for this device.
        let version = row
            .get("Releases")
            .and_then(serde_json::Value::as_array)
            .and_then(|releases| releases.first())
            .and_then(|release| release.get("Version"))
            .and_then(serde_json::Value::as_str);
        if let (Some(version), Some(device)) = (
            version,
            devices.iter_mut().find(|device| device.device == id),
        ) {
            device.available_version = Some(version.to_string());
        }
    }
}

#[async_trait]
impl FirmwareAwarenessPort for LiveFirmware {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn status(&self, ctx: &HostExecutionContext) -> Result<FirmwareStatus, OsControlError> {
        let raw = cli::query(
            ctx,
            self.id(),
            "firmware.get_devices",
            self.fwupdmgr,
            vec!["get-devices".into(), "--json".into()],
        )
        .await?;
        let Some(mut devices) = parse_fwupd_devices(&raw) else {
            // Unparseable output is an error, never an empty device list: "no
            // firmware devices" would wrongly read as a clean bill of health.
            return Err(OsControlError::Unavailable {
                provider: Some(self.id()),
                reason: SafeText::new("could not parse the firmware device list"),
                retryable: true,
            });
        };

        // `get-updates` exits non-zero when there is nothing to do AND when the
        // metadata could not be refreshed. Only a parseable payload proves the
        // update source was actually consulted.
        let (updates_raw, updates_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "firmware.get_updates",
            self.fwupdmgr,
            vec!["get-updates".into(), "--json".into()],
        )
        .await?;
        let parsed = serde_json::from_str::<serde_json::Value>(&updates_raw).is_ok();
        if parsed {
            merge_available(&mut devices, &updates_raw);
        }
        Ok(FirmwareStatus {
            devices,
            // Reachable only when fwupd gave us a payload we could read. A
            // non-zero exit with valid JSON is the normal "no updates" case.
            update_source_reachable: parsed || updates_ok,
        })
    }
}

/// The live hardware-sensor provider, reading `/sys/class/hwmon`.
pub struct LiveHardwareSensors {
    root: std::path::PathBuf,
}

impl Default for LiveHardwareSensors {
    fn default() -> Self {
        Self::new("/sys/class/hwmon")
    }
}

impl LiveHardwareSensors {
    /// Compose over an explicit hwmon root (tests pass a fixture directory).
    #[must_use]
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Compose the provider when hwmon is present.
    #[must_use]
    pub fn discover() -> Option<Self> {
        let root = std::path::PathBuf::from("/sys/class/hwmon");
        root.is_dir().then_some(Self { root })
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("hwmon")
    }

    /// Collect every readable sensor under the hwmon root.
    fn collect(&self) -> Vec<SensorReading> {
        let mut out = Vec::new();
        let Ok(chips) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for chip in chips.flatten() {
            let chip_path = chip.path();
            let chip_name = std::fs::read_to_string(chip_path.join("name"))
                .map(|text| text.trim().to_string())
                .unwrap_or_else(|_| {
                    chip_path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            let Ok(entries) = std::fs::read_dir(&chip_path) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
                .filter(|name| name.ends_with("_input"))
                .collect();
            // Stable order: the kernel does not guarantee readdir order, and an
            // unstable order would make the page cursor meaningless.
            names.sort();
            for input in names {
                let stem = input.trim_end_matches("_input");
                let Some(kind) = sensor_kind(stem) else {
                    continue;
                };
                let Some(raw) = read_i64(&chip_path.join(&input)) else {
                    continue;
                };
                let label = std::fs::read_to_string(chip_path.join(format!("{stem}_label")))
                    .map(|text| text.trim().to_string())
                    .unwrap_or_else(|_| format!("{chip_name} {stem}"));
                // hwmon reports millidegrees, millivolts, microwatts and RPM. The
                // domain stores tenths, so each kind is scaled by its own factor —
                // sharing one factor would silently mis-scale three of the four.
                let value_tenths = match kind {
                    SensorKind::Temperature | SensorKind::Voltage => raw / 100,
                    SensorKind::Power => raw / 100_000,
                    SensorKind::FanSpeed => raw * 10,
                };
                let threshold = match kind {
                    SensorKind::Temperature => read_i64(&chip_path.join(format!("{stem}_crit")))
                        .or_else(|| read_i64(&chip_path.join(format!("{stem}_max"))))
                        .map(|value| value / 100),
                    SensorKind::FanSpeed | SensorKind::Voltage | SensorKind::Power => None,
                };
                out.push(SensorReading {
                    sensor: format!("{}/{}", chip_path.to_string_lossy(), stem),
                    label: SafeText::new(label),
                    kind,
                    value_tenths: i32::try_from(value_tenths).unwrap_or(i32::MAX),
                    high_threshold_tenths: threshold
                        .and_then(|value| i32::try_from(value).ok()),
                });
            }
        }
        out.sort_by(|a, b| a.sensor.cmp(&b.sensor));
        out
    }
}

/// Map an hwmon attribute stem to a sensor kind. Unknown stems are skipped.
fn sensor_kind(stem: &str) -> Option<SensorKind> {
    if stem.starts_with("temp") {
        Some(SensorKind::Temperature)
    } else if stem.starts_with("fan") {
        Some(SensorKind::FanSpeed)
    } else if stem.starts_with("in") || stem.starts_with("cpu") {
        Some(SensorKind::Voltage)
    } else if stem.starts_with("power") {
        Some(SensorKind::Power)
    } else {
        None
    }
}

/// Read a single integer from a sysfs attribute.
fn read_i64(path: &std::path::Path) -> Option<i64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[async_trait]
impl HardwareControlPort for LiveHardwareSensors {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn sensors(
        &self,
        _ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SensorPage, OsControlError> {
        // The domain owns the clamp, so an absent limit is not an unbounded read.
        let limit = crate::os_control::hardware::sensor_page_limit(limit);
        let all = self.collect();
        // The cursor is the last sensor path returned. Resuming by identity rather
        // than by index means a sensor appearing or disappearing between pages
        // cannot silently skip a row.
        let start = match cursor {
            Some(last) => all
                .iter()
                .position(|reading| reading.sensor == last)
                .map_or(0, |index| index + 1),
            None => 0,
        };
        let window: Vec<SensorReading> = all.iter().skip(start).take(limit).cloned().collect();
        let consumed = start + window.len();
        let truncated = consumed < all.len();
        let next_cursor = if truncated {
            window.last().map(|reading| reading.sensor.clone())
        } else {
            None
        };
        Ok(SensorPage {
            items: window,
            next_cursor,
            truncated,
        })
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn write(path: &std::path::Path, text: &str) {
        std::fs::write(path, text).expect("write fixture");
    }

    /// Build a fake hwmon tree: one chip, a temperature and a fan.
    fn fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("kria-hwmon-{}", uuid::Uuid::new_v4()));
        let chip = root.join("hwmon0");
        std::fs::create_dir_all(&chip).expect("mkdir");
        write(&chip.join("name"), "coretemp\n");
        write(&chip.join("temp1_input"), "45000\n");
        write(&chip.join("temp1_label"), "Package id 0\n");
        write(&chip.join("temp1_crit"), "100000\n");
        write(&chip.join("fan1_input"), "2400\n");
        // An attribute with no recognized kind must be skipped, not mis-typed.
        write(&chip.join("mystery1_input"), "7\n");
        root
    }

    #[test]
    fn each_sensor_kind_uses_its_own_scale() {
        let root = fixture();
        let readings = LiveHardwareSensors::new(&root).collect();
        let temp = readings
            .iter()
            .find(|r| r.kind == SensorKind::Temperature)
            .expect("temperature present");
        // 45000 millidegrees = 45.0 °C = 450 tenths.
        assert_eq!(temp.value_tenths, 450);
        assert_eq!(temp.high_threshold_tenths, Some(1000));
        let fan = readings
            .iter()
            .find(|r| r.kind == SensorKind::FanSpeed)
            .expect("fan present");
        // 2400 RPM = 24000 tenths. Sharing the temperature scale would report 24.
        assert_eq!(fan.value_tenths, 24_000);
        // The unknown attribute was skipped rather than guessed.
        assert_eq!(readings.len(), 2);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_fan_without_a_limit_reports_no_verdict() {
        let root = fixture();
        let readings = LiveHardwareSensors::new(&root).collect();
        let fan = readings
            .iter()
            .find(|r| r.kind == SensorKind::FanSpeed)
            .expect("fan present");
        // No threshold file exists, so no alarm may be invented.
        assert!(fan.high_threshold_tenths.is_none());
        assert!(fan.over_threshold().is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn paging_resumes_by_identity_not_index() {
        let root = fixture();
        let provider = LiveHardwareSensors::new(&root);
        let ctx = crate::os_control::testing::observation_context_for_test();
        let first = provider.sensors(&ctx, None, Some(1)).await.expect("page");
        assert_eq!(first.items.len(), 1);
        assert!(first.truncated);
        let cursor = first.next_cursor.clone().expect("cursor");
        let second = provider
            .sensors(&ctx, Some(&cursor), Some(10))
            .await
            .expect("page");
        // The second page must not repeat the first row.
        assert_eq!(second.items.len(), 1);
        assert_ne!(second.items[0].sensor, first.items[0].sensor);
        assert!(!second.truncated);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn firmware_devices_need_a_stable_id() {
        // A device with only a human name is skipped: a label is not an identity.
        let json = r#"{"Devices":[{"Name":"System Firmware"},
                        {"DeviceId":"abc123","Name":"SSD","Version":"1.2",
                         "Flags":["needs-reboot"]}]}"#;
        let devices = parse_fwupd_devices(json).expect("parses");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].device, "abc123");
        assert_eq!(devices[0].needs_reboot, Some(true));
        // Never checked for updates yet, so this must be None, not "current".
        assert!(devices[0].available_version.is_none());
    }

    #[test]
    fn available_versions_merge_onto_the_matching_device_only() {
        let mut devices = parse_fwupd_devices(
            r#"{"Devices":[{"DeviceId":"a","Name":"A","Version":"1"},
                            {"DeviceId":"b","Name":"B","Version":"1"}]}"#,
        )
        .expect("parses");
        merge_available(
            &mut devices,
            r#"{"Devices":[{"DeviceId":"b","Releases":[{"Version":"2"}]}]}"#,
        );
        assert!(devices[0].available_version.is_none(), "A has no update");
        assert_eq!(devices[1].available_version.as_deref(), Some("2"));
    }

    #[test]
    fn malformed_firmware_output_is_an_error_not_an_empty_list() {
        assert!(parse_fwupd_devices("not json").is_none());
        assert!(parse_fwupd_devices(r#"{"NoDevicesKey":1}"#).is_none());
    }
}
