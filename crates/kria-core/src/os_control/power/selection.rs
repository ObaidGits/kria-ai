//! Power-profile backend selection and captured-argv construction.
//!
//! linux-os-control-production **Task 2.3** (OSC-020, OSC-031), design §9.7.
//!
//! `power-profiles-daemon`'s D-Bus API
//! (`org.freedesktop.UPower.PowerProfiles`) is the preferred, authoritative
//! provider. Its `powerprofilesctl` CLI front-end is retained as a **declared
//! degraded** structured-command fallback until the live D-Bus transport is
//! wired by a desktop composition root.

use crate::os_control::contract::{CapabilityId, Digest, ProviderId, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

use super::PowerProfile;

/// The concrete host power-profile backend a provider selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfileBackend {
    /// `power-profiles-daemon` D-Bus. Preferred.
    PowerProfilesDaemon,
    /// `powerprofilesctl` structured-command fallback. Degraded.
    Powerprofilesctl,
}

impl PowerProfileBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [PowerProfileBackend; 2] = [
        PowerProfileBackend::PowerProfilesDaemon,
        PowerProfileBackend::Powerprofilesctl,
    ];

    /// The stable label used in the `backend` result field and traces.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PowerProfileBackend::PowerProfilesDaemon => "power_profiles_daemon",
            PowerProfileBackend::Powerprofilesctl => "powerprofilesctl",
        }
    }

    /// Whether this backend is a declared **degraded** provider (not the
    /// preferred authoritative D-Bus path).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        !matches!(self, PowerProfileBackend::PowerProfilesDaemon)
    }

    /// The trusted absolute executable path for this backend's structured
    /// command (only the `powerprofilesctl` fallback dispatches through a
    /// process).
    #[must_use]
    fn executable_path(self) -> &'static str {
        "/usr/bin/powerprofilesctl"
    }

    /// A stable trusted-executable identity used by the fallback adapter. Live
    /// transports compare the on-disk identity against this to detect drift; the
    /// deny-live provider tests use it directly.
    #[must_use]
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred available backend, or `None` when no session
/// power-profile backend is present (→ the provider reports `Unavailable`).
#[must_use]
pub fn select_backend(available: &[PowerProfileBackend]) -> Option<PowerProfileBackend> {
    PowerProfileBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// The argv for reading the current power profile.
#[must_use]
pub fn query_profile_argv() -> Vec<String> {
    vec!["get".into()]
}

/// The argv for setting the power profile.
#[must_use]
pub fn set_profile_argv(_backend: PowerProfileBackend, profile: PowerProfile) -> Vec<String> {
    vec!["set".into(), profile.as_str().into()]
}

/// A failed profile observation. Never a substituted profile: reporting a
/// plausible-looking profile the daemon never named would let `set_power_plan`
/// verify against a fabricated fact.
fn unreadable(backend: PowerProfileBackend, reason: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(format!("power-{}", backend.as_str()))),
        reason: SafeText::new(reason),
        retryable: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Battery health (Task 3.8, `get_battery_health`, OSC-020)
// ─────────────────────────────────────────────────────────────────────────────

/// UPower's `org.freedesktop.UPower.Device.Type` value for a battery.
///
/// Matched numerically rather than on the human-visible model string, because a
/// device *label* ("BAT0", "Dell Primary Battery") is neither unique nor stable.
pub const UPOWER_DEVICE_TYPE_BATTERY: u32 = 2;

/// The frozen `health_state` tokens for `get_battery_health`.
///
/// The boundaries are a declared classification of UPower's design-capacity
/// ratio, not a measurement: the measured fact is `capacity_percent`, and this
/// token only names the band it falls in. `absent` is deliberately **not** in
/// this set — a missing battery is reported by presence, never as a health band.
#[must_use]
pub fn classify_battery_health(capacity_percent: u8) -> &'static str {
    match capacity_percent {
        80..=100 => "good",
        60..=79 => "fair",
        40..=59 => "degraded",
        _ => "poor",
    }
}

/// Interpret UPower's `Capacity` property — the battery's current full-charge
/// capacity as a percentage of its **design** capacity (D-Bus type `d`).
///
/// Fails closed on anything outside `(0, 100]`, and that lower bound is the
/// point of this function: UPower reports `0.0` when the kernel driver does not
/// expose a design capacity at all, which is extremely common. Reporting that as
/// `0%` health would describe a perfectly healthy battery as completely dead, so
/// an unreported capacity stays unknown.
pub fn parse_battery_capacity(
    backend: PowerProfileBackend,
    raw: f64,
) -> Result<u8, OsControlError> {
    if !raw.is_finite() {
        return Err(unreadable(
            backend,
            "UPower reported a non-finite battery capacity; battery health is unknown",
        ));
    }
    if raw <= 0.0 {
        return Err(unreadable(
            backend,
            "UPower reported no design capacity for this battery (0), so its health is unknown, not zero",
        ));
    }
    if raw > 100.0 {
        return Err(unreadable(
            backend,
            "UPower reported a battery capacity above 100 percent of design; refusing to assume a scale",
        ));
    }
    Ok(raw.round() as u8)
}

/// Interpret UPower's `ChargeCycles` property (`i`).
///
/// UPower answers `-1` when the driver does not expose a cycle count. `None`
/// means "not reported"; it is never collapsed into `0`, which would claim a
/// brand-new battery.
#[must_use]
pub fn parse_charge_cycles(raw: i32) -> Option<u64> {
    if raw < 0 {
        None
    } else {
        Some(raw as u64)
    }
}

/// Normalize a profile token for comparison only (never for reporting): the
/// manifest spells `power_saver`, `power-profiles-daemon` spells `power-saver`,
/// and either may arrive capitalized.
fn normalize_profile_token(token: &str) -> String {
    token.trim().to_ascii_lowercase().replace('_', "-")
}

/// Resolve the daemon's `ActiveProfile` **against the profile list this machine
/// actually advertises** (`Profiles`).
///
/// The advertised set is hardware- and driver-dependent: a desktop with no
/// battery commonly advertises only `balanced` and `performance`, and some
/// vendors add profiles outside this contract's closed set. So:
///
/// * an active token absent from `advertised` is a failed read, not a profile —
///   the daemon and its own profile list disagree, and picking either would be
///   an invention;
/// * an empty `advertised` list can never confirm anything;
/// * an advertised profile outside the frozen `set_power_plan` set is
///   [`OsControlError::Unsupported`] — real, but not addressable through this
///   contract, which is different from unreadable.
pub fn parse_active_profile(
    backend: PowerProfileBackend,
    active: &str,
    advertised: &[String],
) -> Result<PowerProfile, OsControlError> {
    let active_token = normalize_profile_token(active);
    if active_token.is_empty() {
        return Err(unreadable(
            backend,
            "the power-profile daemon reported an empty active profile",
        ));
    }
    if advertised.is_empty() {
        return Err(unreadable(
            backend,
            "the power-profile daemon advertised no profiles; the active profile cannot be confirmed",
        ));
    }
    let advertised_here = advertised
        .iter()
        .any(|candidate| normalize_profile_token(candidate) == active_token);
    if !advertised_here {
        return Err(unreadable(
            backend,
            "the reported active profile is absent from this machine's advertised profile list",
        ));
    }
    PowerProfile::parse(&active_token).ok_or_else(|| OsControlError::Unsupported {
        capability: CapabilityId::new("get_power_plan"),
        reason: SafeText::new(
            "this machine advertises a power profile outside the contract's closed set; reporting a supported profile instead would be a false observation",
        ),
    })
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    const BACKEND: PowerProfileBackend = PowerProfileBackend::PowerProfilesDaemon;

    #[test]
    fn battery_capacity_normal_readings_parse() {
        assert_eq!(parse_battery_capacity(BACKEND, 100.0).unwrap(), 100);
        assert_eq!(parse_battery_capacity(BACKEND, 87.4).unwrap(), 87);
        assert_eq!(parse_battery_capacity(BACKEND, 87.6).unwrap(), 88);
        assert_eq!(parse_battery_capacity(BACKEND, 0.6).unwrap(), 1);
    }

    #[test]
    fn unreported_design_capacity_is_unknown_not_zero_percent_health() {
        // The reading a driver that exposes no design capacity actually gives.
        // Reporting it as 0% would describe a healthy battery as dead.
        assert!(parse_battery_capacity(BACKEND, 0.0).is_err());
    }

    #[test]
    fn unrecognised_capacity_scale_is_an_error_not_a_default() {
        for raw in [-1.0, -0.0001, 100.5, 4200.0, f64::NAN, f64::INFINITY] {
            assert!(
                parse_battery_capacity(BACKEND, raw).is_err(),
                "raw {raw} must not parse"
            );
        }
    }

    #[test]
    fn unknown_cycle_count_is_none_not_zero() {
        assert_eq!(parse_charge_cycles(-1), None);
        assert_eq!(parse_charge_cycles(0), Some(0));
        assert_eq!(parse_charge_cycles(412), Some(412));
    }

    #[test]
    fn health_bands_are_ordered_and_never_report_absent() {
        assert_eq!(classify_battery_health(100), "good");
        assert_eq!(classify_battery_health(80), "good");
        assert_eq!(classify_battery_health(79), "fair");
        assert_eq!(classify_battery_health(60), "fair");
        assert_eq!(classify_battery_health(59), "degraded");
        assert_eq!(classify_battery_health(40), "degraded");
        assert_eq!(classify_battery_health(39), "poor");
        for percent in 0..=100u8 {
            assert_ne!(classify_battery_health(percent), "absent");
        }
    }

    fn advertised(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| (*t).to_string()).collect()
    }

    #[test]
    fn normal_reading_resolves_against_the_advertised_list() {
        let list = advertised(&["power-saver", "balanced", "performance"]);
        assert_eq!(
            parse_active_profile(BACKEND, "balanced", &list).unwrap(),
            PowerProfile::Balanced
        );
        assert_eq!(
            parse_active_profile(BACKEND, "power-saver", &list).unwrap(),
            PowerProfile::PowerSaver
        );
    }

    #[test]
    fn spelling_and_case_variants_normalize_before_comparison() {
        // The manifest spells `power_saver`; the daemon spells `power-saver`.
        let list = advertised(&["power_saver", "Balanced"]);
        assert_eq!(
            parse_active_profile(BACKEND, "power-saver", &list).unwrap(),
            PowerProfile::PowerSaver
        );
        assert_eq!(
            parse_active_profile(BACKEND, " BALANCED \n", &list).unwrap(),
            PowerProfile::Balanced
        );
    }

    #[test]
    fn hardware_without_a_profile_never_reports_it_active() {
        // A real desktop/VM case: power-profiles-daemon advertises only two
        // profiles because the platform driver offers no low-power state.
        let list = advertised(&["balanced", "performance"]);
        assert_eq!(
            parse_active_profile(BACKEND, "performance", &list).unwrap(),
            PowerProfile::Performance
        );
        assert!(parse_active_profile(BACKEND, "power-saver", &list).is_err());
    }

    #[test]
    fn empty_advertised_list_confirms_nothing() {
        assert!(parse_active_profile(BACKEND, "balanced", &[]).is_err());
        assert!(parse_active_profile(BACKEND, "", &advertised(&["balanced"])).is_err());
    }

    #[test]
    fn vendor_profile_outside_the_contract_is_unsupported_not_substituted() {
        let list = advertised(&["quiet", "balanced"]);
        let error = parse_active_profile(BACKEND, "quiet", &list).unwrap_err();
        assert!(matches!(error, OsControlError::Unsupported { .. }));
    }

    #[test]
    fn unrecognised_output_is_an_error_not_a_default() {
        let list = advertised(&["power-saver", "balanced", "performance"]);
        for active in ["", "  ", "unknown", "Profile: balanced", "3"] {
            assert!(
                parse_active_profile(BACKEND, active, &list).is_err(),
                "active {active:?} must not parse"
            );
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_matrix_prefers_daemon_then_ctl() {
        use PowerProfileBackend::*;
        let cases: &[(&[PowerProfileBackend], Option<PowerProfileBackend>)] = &[
            (&[PowerProfilesDaemon, Powerprofilesctl], Some(PowerProfilesDaemon)),
            (&[Powerprofilesctl], Some(Powerprofilesctl)),
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(select_backend(available), *expected, "available {available:?}");
        }
    }

    #[test]
    fn degraded_classification() {
        assert!(!PowerProfileBackend::PowerProfilesDaemon.is_degraded());
        assert!(PowerProfileBackend::Powerprofilesctl.is_degraded());
    }

    #[test]
    fn captured_argv_golden() {
        assert_eq!(query_profile_argv(), vec!["get"]);
        assert_eq!(
            set_profile_argv(PowerProfileBackend::Powerprofilesctl, PowerProfile::Balanced),
            vec!["set", "balanced"]
        );
        assert_eq!(
            set_profile_argv(PowerProfileBackend::Powerprofilesctl, PowerProfile::PowerSaver),
            vec!["set", "power-saver"]
        );
    }

    #[test]
    fn trusted_executables_are_absolute_and_valid() {
        for backend in PowerProfileBackend::PREFERENCE {
            let exe = backend.trusted_executable().expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
    }
}
