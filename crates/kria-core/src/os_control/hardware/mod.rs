//! Firmware awareness and hardware sensors.
//!
//! linux-os-control-production task **5.4** (OSC-022).
//!
//! # Awareness, deliberately not control
//!
//! The port is named `FirmwareAwareness` rather than `FirmwareControl` because it
//! **only reads**. There is no operation here to flash firmware, and that is a
//! design decision, not an omission: a failed firmware write can leave a machine
//! that will not boot, which is the one outcome no amount of verification or
//! rollback in this architecture could recover from. Reporting what is installed
//! and what updates exist is useful; applying them belongs to the vendor's own
//! tooling with the user present.
//!
//! # A missing sensor is missing, not zero
//!
//! `get_hardware_sensors` reports only sensors the machine actually exposes. A
//! temperature of `0 °C` and "this machine has no such sensor" are entirely
//! different facts, and collapsing the second into the first would look like a
//! reading rather than an absence.

use async_trait::async_trait;

use crate::os_control::context::HostExecutionContext;
use crate::os_control::contract::{ProviderId, SafeField, SafeText};
use crate::os_control::error::OsControlError;

/// The provider identity for firmware.
pub const FIRMWARE_PROVIDER_ID: &str = "firmware-fwupd";

/// The provider identity for sensors.
pub const SENSORS_PROVIDER_ID: &str = "hardware-sensors";

/// Largest sensor page.
pub const SENSOR_PAGE_MAX: usize = 256;

/// Default sensor page size.
pub const SENSOR_PAGE_DEFAULT: usize = 64;

/// One firmware-carrying device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareDevice {
    /// A stable device id.
    pub device: String,
    /// Vendor/model label for display.
    pub label: SafeText,
    /// The installed version, when reported.
    pub installed_version: Option<String>,
    /// The latest available version, when a source reported one.
    ///
    /// `None` means "no update information available" — never "up to date".
    /// Telling a user their firmware is current when nothing was checked would be
    /// a false assurance about a security-relevant component.
    pub available_version: Option<String>,
    /// Whether applying an update would require a reboot.
    pub needs_reboot: Option<bool>,
}

/// The overall firmware picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmwareStatus {
    /// Devices reported.
    pub devices: Vec<FirmwareDevice>,
    /// Whether an update source was reachable at all.
    ///
    /// Reported explicitly so "no updates found" can be told apart from "could not
    /// check".
    pub update_source_reachable: bool,
}

impl FirmwareStatus {
    /// How many devices have a newer version available.
    ///
    /// Counts only devices where **both** versions are known and differ. A device
    /// with unknown availability is not counted as up to date.
    #[must_use]
    pub fn updates_available(&self) -> usize {
        self.devices
            .iter()
            .filter(|d| match (&d.installed_version, &d.available_version) {
                (Some(installed), Some(available)) => installed != available,
                _ => false,
            })
            .count()
    }

    /// Devices whose update state could not be determined.
    #[must_use]
    pub fn undetermined(&self) -> usize {
        self.devices
            .iter()
            .filter(|d| d.available_version.is_none())
            .count()
    }
}

/// What a sensor measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SensorKind {
    /// Temperature in degrees Celsius.
    Temperature,
    /// Fan speed in RPM.
    FanSpeed,
    /// Voltage in volts.
    Voltage,
    /// Power draw in watts.
    Power,
}

impl SensorKind {
    /// A stable token.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Temperature => "temperature",
            Self::FanSpeed => "fan-speed",
            Self::Voltage => "voltage",
            Self::Power => "power",
        }
    }

    /// The unit a reading is expressed in.
    #[must_use]
    pub fn unit(self) -> &'static str {
        match self {
            Self::Temperature => "C",
            Self::FanSpeed => "rpm",
            Self::Voltage => "V",
            Self::Power => "W",
        }
    }
}

/// One sensor reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorReading {
    /// A stable sensor id.
    pub sensor: String,
    /// Display label.
    pub label: SafeText,
    /// What it measures.
    pub kind: SensorKind,
    /// The reading, scaled to the kind's unit and rounded to one decimal as an
    /// integer tenth (so the type stays exact and comparable).
    pub value_tenths: i32,
    /// The manufacturer's high threshold, when the driver reports one.
    pub high_threshold_tenths: Option<i32>,
}

impl SensorReading {
    /// Whether this reading is at or above its reported high threshold.
    ///
    /// `None` when no threshold is reported: without one, there is nothing to
    /// compare against, and inventing a limit would raise false alarms on hardware
    /// that simply runs warm by design.
    #[must_use]
    pub fn over_threshold(&self) -> Option<bool> {
        self.high_threshold_tenths
            .map(|limit| self.value_tenths >= limit)
    }
}

/// A page of sensor readings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorPage {
    /// The readings.
    pub items: Vec<SensorReading>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
    /// Whether the listing was cut short.
    pub truncated: bool,
}

/// The read-only firmware port.
#[async_trait]
pub trait FirmwareAwarenessPort: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read the firmware picture.
    async fn status(&self, ctx: &HostExecutionContext) -> Result<FirmwareStatus, OsControlError>;
}

/// The read-only sensors port.
#[async_trait]
pub trait HardwareControlPort: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read a bounded page of sensors.
    async fn sensors(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SensorPage, OsControlError>;
}

/// Clamp a sensor page size.
#[must_use]
pub fn sensor_page_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(SENSOR_PAGE_DEFAULT).clamp(1, SENSOR_PAGE_MAX)
}

/// Validate a battery charge-threshold pair.
///
/// Both bounds are checked **together**: a lower bound above the upper one is
/// rejected by the kernel anyway, and catching it here keeps it a clean refusal
/// instead of a half-applied pair where only one value took effect.
pub fn validate_charge_thresholds(lower: u8, upper: u8) -> Result<(u8, u8), OsControlError> {
    if lower > 100 || upper > 100 {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("thresholds"),
            reason: SafeText::new("both thresholds must be between 0 and 100"),
        });
    }
    if lower >= upper {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("thresholds"),
            reason: SafeText::new(
                "lower must be below upper; an inverted pair would be half-applied",
            ),
        });
    }
    if upper < 20 {
        // An upper bound this low keeps the battery nearly empty, which is a worse
        // outcome than the wear the setting exists to avoid.
        return Err(OsControlError::PolicyDenied {
            reason: SafeText::new(
                "an upper charge threshold below 20% would leave the battery nearly empty",
            ),
        });
    }
    Ok((lower, upper))
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn device(installed: Option<&str>, available: Option<&str>) -> FirmwareDevice {
        FirmwareDevice {
            device: "dev0".to_string(),
            label: SafeText::new("Test Device"),
            installed_version: installed.map(str::to_string),
            available_version: available.map(str::to_string),
            needs_reboot: None,
        }
    }

    #[test]
    fn unknown_availability_is_not_counted_as_up_to_date() {
        // The false-assurance case: nothing was checked, so nothing is current.
        let status = FirmwareStatus {
            devices: vec![device(Some("1.0"), None)],
            update_source_reachable: false,
        };
        assert_eq!(status.updates_available(), 0);
        assert_eq!(
            status.undetermined(),
            1,
            "an unchecked device must be reported as undetermined, not current"
        );
    }

    #[test]
    fn an_update_is_counted_only_when_both_versions_are_known() {
        let status = FirmwareStatus {
            devices: vec![
                device(Some("1.0"), Some("1.1")),
                device(Some("2.0"), Some("2.0")),
                device(None, Some("3.0")),
            ],
            update_source_reachable: true,
        };
        assert_eq!(status.updates_available(), 1);
        assert_eq!(status.undetermined(), 0);
    }

    #[test]
    fn a_sensor_without_a_threshold_reports_no_verdict() {
        let no_limit = SensorReading {
            sensor: "temp1".to_string(),
            label: SafeText::new("CPU"),
            kind: SensorKind::Temperature,
            value_tenths: 850,
            high_threshold_tenths: None,
        };
        assert_eq!(
            no_limit.over_threshold(),
            None,
            "without a reported limit there is nothing to compare against"
        );

        let with_limit = SensorReading {
            high_threshold_tenths: Some(800),
            ..no_limit.clone()
        };
        assert_eq!(with_limit.over_threshold(), Some(true));
    }

    #[test]
    fn sensor_units_match_their_kind() {
        assert_eq!(SensorKind::Temperature.unit(), "C");
        assert_eq!(SensorKind::FanSpeed.unit(), "rpm");
        assert_eq!(SensorKind::Power.unit(), "W");
    }

    #[test]
    fn charge_thresholds_are_validated_as_a_pair() {
        assert!(validate_charge_thresholds(80, 75).is_err(), "inverted");
        assert!(validate_charge_thresholds(50, 50).is_err(), "equal");
        assert!(validate_charge_thresholds(0, 15).is_err(), "upper too low");
        assert!(validate_charge_thresholds(101, 100).is_err(), "out of range");
        assert_eq!(validate_charge_thresholds(75, 80).unwrap(), (75, 80));
    }

    #[test]
    fn sensor_page_limit_is_clamped() {
        assert_eq!(sensor_page_limit(None), SENSOR_PAGE_DEFAULT);
        assert_eq!(sensor_page_limit(Some(0)), 1);
        assert_eq!(sensor_page_limit(Some(9999)), SENSOR_PAGE_MAX);
    }
}
