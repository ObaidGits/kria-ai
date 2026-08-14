//! Bluetooth backend selection and captured-argv construction.
//!
//! linux-os-control-production **Task 3.7** (OSC-021, OSC-029, OSC-031),
//! design §9.
//!
//! BlueZ's D-Bus API (`org.bluez`, system bus) is the preferred authoritative
//! provider: it exposes adapter and device objects with typed properties, so
//! every state read is an authoritative service observation rather than parsed
//! prose. Its `bluetoothctl` front-end is retained as a **declared degraded**
//! structured-command fallback for hosts where the bus is unreachable.
//!
//! # Why identities are addresses, not names
//!
//! A device's advertised *name* is neither unique nor stable (two headsets ship
//! with the same name; a name can change between advertisements). The adapter
//! address and the device address are the stable identities, so every operation
//! binds an address and the tool layer resolves a human name to one address —
//! refusing ambiguity rather than picking the first match.

use crate::os_control::contract::Digest;
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

/// The concrete host Bluetooth backend a provider selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BluetoothBackend {
    /// BlueZ `org.bluez` system D-Bus. Preferred and authoritative.
    BluezDbus,
    /// `bluetoothctl` structured-command fallback. Degraded.
    Bluetoothctl,
}

impl BluetoothBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [BluetoothBackend; 2] =
        [BluetoothBackend::BluezDbus, BluetoothBackend::Bluetoothctl];

    /// The stable label used in the `backend` result field and traces.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BluetoothBackend::BluezDbus => "bluez_dbus",
            BluetoothBackend::Bluetoothctl => "bluetoothctl",
        }
    }

    /// Whether this backend is a declared **degraded** provider (not the
    /// preferred authoritative D-Bus path).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        !matches!(self, BluetoothBackend::BluezDbus)
    }

    /// The trusted absolute executable path for this backend's structured
    /// command (only the `bluetoothctl` fallback dispatches through a process).
    #[must_use]
    fn executable_path(self) -> &'static str {
        "/usr/bin/bluetoothctl"
    }

    /// A stable trusted-executable identity used by the fallback adapter.
    #[must_use]
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred available backend, or `None` when no Bluetooth
/// backend is present (→ the provider reports `Unavailable`).
#[must_use]
pub fn select_backend(available: &[BluetoothBackend]) -> Option<BluetoothBackend> {
    BluetoothBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// The argv that powers the adapter on or off.
#[must_use]
pub fn set_power_argv(enabled: bool) -> Vec<String> {
    vec!["power".into(), if enabled { "on".into() } else { "off".into() }]
}

/// Validate a Bluetooth device address before it becomes an argv element.
///
/// Rejected rather than escaped. argv is not shell-interpreted, but an address
/// beginning with `-` would be read by `bluetoothctl` as an **option**, which
/// could change the command's meaning entirely. Only the canonical
/// `AA:BB:CC:DD:EE:FF` form is accepted, so a name can never arrive here posing
/// as an identity.
pub fn validate_address(address: &str) -> Result<&str, OsControlError> {
    let invalid = || OsControlError::InvalidRequest {
        field: crate::os_control::contract::SafeField::new("device"),
        reason: crate::os_control::contract::SafeText::new(
            "device must be a Bluetooth address in AA:BB:CC:DD:EE:FF form",
        ),
    };
    let octets: Vec<&str> = address.split(':').collect();
    if octets.len() != 6 {
        return Err(invalid());
    }
    for octet in octets {
        if octet.len() != 2 || !octet.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid());
        }
    }
    Ok(address)
}

/// The argv that pairs a device by address.
///
/// Pairing may require an agent interaction. A passkey is **never** placed in
/// argv or in the canonical params (OSC-029): the confirmation is carried by the
/// existing approval path, so no secret is persisted anywhere.
#[must_use]
pub fn pair_argv(address: &str) -> Vec<String> {
    vec!["pair".into(), address.to_string()]
}

/// The argv that connects an already-known device by address.
#[must_use]
pub fn connect_argv(address: &str) -> Vec<String> {
    vec!["connect".into(), address.to_string()]
}

/// The argv that disconnects a device by address.
#[must_use]
pub fn disconnect_argv(address: &str) -> Vec<String> {
    vec!["disconnect".into(), address.to_string()]
}

/// The argv that sets or clears a device's trust flag.
#[must_use]
pub fn trust_argv(address: &str, trusted: bool) -> Vec<String> {
    vec![
        if trusted { "trust".into() } else { "untrust".into() },
        address.to_string(),
    ]
}

/// The argv that removes (unpairs and forgets) a device by address.
#[must_use]
pub fn remove_argv(address: &str) -> Vec<String> {
    vec!["remove".into(), address.to_string()]
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_prefers_the_authoritative_bus_then_the_cli() {
        use BluetoothBackend::{BluezDbus, Bluetoothctl};
        let cases: &[(&[BluetoothBackend], Option<BluetoothBackend>)] = &[
            (&[BluezDbus, Bluetoothctl], Some(BluezDbus)),
            (&[Bluetoothctl, BluezDbus], Some(BluezDbus)),
            (&[Bluetoothctl], Some(Bluetoothctl)),
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(select_backend(available), *expected, "available {available:?}");
        }
    }

    #[test]
    fn only_the_cli_fallback_is_degraded() {
        assert!(!BluetoothBackend::BluezDbus.is_degraded());
        assert!(BluetoothBackend::Bluetoothctl.is_degraded());
    }

    #[test]
    fn trusted_executable_is_absolute_and_stable() {
        let exe = BluetoothBackend::Bluetoothctl
            .trusted_executable()
            .expect("valid trusted executable");
        assert!(exe.path().starts_with('/'), "path must be absolute");
        assert_eq!(exe.path(), "/usr/bin/bluetoothctl");
    }

    #[test]
    fn argv_never_carries_a_passkey_or_shell_metacharacters() {
        // Every argv element is a fixed verb or a bare address — no passkey, no
        // quoting, nothing a shell could reinterpret (there is no shell).
        let all = [
            set_power_argv(true),
            set_power_argv(false),
            pair_argv("AA:BB:CC:DD:EE:FF"),
            connect_argv("AA:BB:CC:DD:EE:FF"),
            disconnect_argv("AA:BB:CC:DD:EE:FF"),
            trust_argv("AA:BB:CC:DD:EE:FF", true),
            trust_argv("AA:BB:CC:DD:EE:FF", false),
            remove_argv("AA:BB:CC:DD:EE:FF"),
        ];
        for argv in all {
            for arg in argv {
                assert!(
                    !arg.contains(['|', ';', '&', '$', '`', '\n', '"', '\'']),
                    "argv element must be shell-metacharacter free: {arg}"
                );
            }
        }
    }

    #[test]
    fn trust_argv_selects_the_inverse_verb() {
        assert_eq!(trust_argv("A", true)[0], "trust");
        assert_eq!(trust_argv("A", false)[0], "untrust");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Read argv + parsing
// ─────────────────────────────────────────────────────────────────────────────

/// The argv reading adapter state (`bluetoothctl show`).
#[must_use]
pub fn show_adapter_argv() -> Vec<String> {
    vec!["show".into()]
}

/// The argv listing known devices (`bluetoothctl devices`).
#[must_use]
pub fn list_devices_argv() -> Vec<String> {
    vec!["devices".into()]
}

/// The argv reading one device's state (`bluetoothctl info <addr>`).
pub fn device_info_argv(address: &str) -> Result<Vec<String>, OsControlError> {
    Ok(vec!["info".into(), validate_address(address)?.to_string()])
}

/// The argv running a bounded discovery scan.
///
/// `--timeout` is what bounds it: `scan on` alone would leave discovery running
/// after the command returns, which is both a privacy problem and a battery drain
/// the user never asked for.
#[must_use]
pub fn scan_argv(duration_ms: u64) -> Vec<String> {
    let seconds = duration_ms.div_ceil(1_000).clamp(1, 60);
    vec![
        "--timeout".into(),
        seconds.to_string(),
        "scan".into(),
        "on".into(),
    ]
}

fn unparseable(what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(crate::os_control::contract::ProviderId::new("bluetooth")),
        reason: crate::os_control::contract::SafeText::new(format!(
            "bluetooth {what} output could not be parsed; refusing to assume a state"
        )),
        retryable: true,
    }
}

/// Read a `Key: value` line from `bluetoothctl` output.
fn field<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(':'))
            .map(str::trim)
    })
}

fn yes_no(value: &str) -> Option<bool> {
    match value.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// Parse `bluetoothctl show` into (adapter address, powered, discovering).
///
/// Returns `Ok(None)` only when the output positively says there is **no
/// controller**. An unrecognised format is an error: reporting "powered off"
/// because parsing failed would let an enable request verify as already
/// satisfied.
pub fn parse_adapter(stdout: &str) -> Result<Option<(String, bool, bool)>, OsControlError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed.contains("No default controller") {
        return Ok(None);
    }
    // `Controller AA:BB:CC:DD:EE:FF name [default]`
    let address = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Controller "))
        .and_then(|rest| rest.split_whitespace().next())
        .ok_or_else(|| unparseable("adapter"))?;
    let powered = field(stdout, "Powered")
        .and_then(yes_no)
        .ok_or_else(|| unparseable("adapter power"))?;
    // Older bluetoothctl omits `Discovering` while powered off; absent means not
    // discovering, which is consistent with a powered-off adapter.
    let discovering = field(stdout, "Discovering").and_then(yes_no).unwrap_or(false);
    Ok(Some((address.to_string(), powered, discovering)))
}

/// Parse `bluetoothctl devices` into (address, label) pairs.
///
/// The address is the identity; the label is display text only. Two devices may
/// advertise the same name, so a label is never an identity.
pub fn parse_device_list(stdout: &str) -> Result<Vec<(String, String)>, OsControlError> {
    let mut devices = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix("Device ") else {
            // An unexpected row means the output shape changed.
            return Err(unparseable("device list"));
        };
        let mut parts = rest.splitn(2, ' ');
        let address = parts.next().unwrap_or_default().trim();
        validate_address(address).map_err(|_| unparseable("device address"))?;
        let label = parts.next().unwrap_or("").trim();
        devices.push((address.to_string(), label.to_string()));
    }
    Ok(devices)
}

/// Parse `bluetoothctl info <addr>` into (paired, connected, trusted, label).
///
/// `Ok(None)` means the device is not known to the adapter — a real fact,
/// distinct from a failed read.
pub fn parse_device_info(
    stdout: &str,
) -> Result<Option<(bool, bool, bool, String)>, OsControlError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed.contains("not available") {
        return Ok(None);
    }
    let paired = field(stdout, "Paired")
        .and_then(yes_no)
        .ok_or_else(|| unparseable("device paired state"))?;
    let connected = field(stdout, "Connected")
        .and_then(yes_no)
        .ok_or_else(|| unparseable("device connected state"))?;
    // `Trusted` may be absent on some stacks; absent means not trusted, which is
    // the safe reading because it never grants an implicit reconnect.
    let trusted = field(stdout, "Trusted").and_then(yes_no).unwrap_or(false);
    let label = field(stdout, "Name").unwrap_or("").to_string();
    Ok(Some((paired, connected, trusted, label)))
}

#[cfg(test)]
mod read_parse_tests {
    use super::*;

    const SHOW: &str = "Controller AA:BB:CC:DD:EE:FF kria-laptop [default]\n\
                        \tPowered: yes\n\tDiscoverable: no\n\tDiscovering: no\n";

    #[test]
    fn adapter_state_is_parsed() {
        let (address, powered, discovering) = parse_adapter(SHOW).unwrap().unwrap();
        assert_eq!(address, "AA:BB:CC:DD:EE:FF");
        assert!(powered);
        assert!(!discovering);
    }

    #[test]
    fn no_controller_is_none_not_an_error() {
        assert!(parse_adapter("No default controller available\n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn unparseable_adapter_output_is_an_error_never_powered_off() {
        // The hazard: returning "off" here would make an enable verify as done.
        assert!(parse_adapter("Controller AA:BB:CC:DD:EE:FF kria\n\tFoo: bar\n").is_err());
    }

    #[test]
    fn device_list_keeps_address_as_identity() {
        let out = "Device AA:AA:AA:AA:AA:AA Headphones\nDevice BB:BB:BB:BB:BB:BB Headphones\n";
        let devices = parse_device_list(out).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].1, devices[1].1, "labels collide");
        assert_ne!(devices[0].0, devices[1].0, "addresses do not");
    }

    #[test]
    fn a_non_device_row_is_an_error() {
        assert!(parse_device_list("Failed to list devices\n").is_err());
    }

    #[test]
    fn device_info_is_parsed_and_absent_device_is_none() {
        let out = "Device AA:BB:CC:DD:EE:FF (public)\n\tName: Test Headset\n\
                   \tPaired: yes\n\tTrusted: no\n\tConnected: yes\n";
        let (paired, connected, trusted, label) = parse_device_info(out).unwrap().unwrap();
        assert!(paired && connected && !trusted);
        assert_eq!(label, "Test Headset");

        assert!(parse_device_info("Device AA:BB:CC:DD:EE:FF not available\n")
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_paired_field_is_an_error() {
        assert!(parse_device_info("Device AA:BB:CC:DD:EE:FF\n\tConnected: yes\n").is_err());
    }

    #[test]
    fn scan_duration_is_bounded_in_seconds() {
        assert_eq!(scan_argv(5_000)[1], "5");
        assert_eq!(scan_argv(1)[1], "1", "sub-second rounds up to one second");
        assert_eq!(scan_argv(9_999_999)[1], "60", "clamped, never unbounded");
    }
}
