//! Brightness backend selection and captured-argv construction.
//!
//! linux-os-control-production **Task 2.2** (OSC-019, OSC-031, OSC-032),
//! design §9.6.
//!
//! Backend preference distinguishes **physical backlight** control
//! (`brightnessctl`, and the GNOME `SettingsDaemon.Power.Screen` D-Bus
//! property) from the **XRandR gamma** fallback, which only simulates
//! brightness by scaling an output's gamma ramp and is explicitly labeled
//! degraded (OSC-019.2, OSC-019.3). The GNOME session D-Bus property is
//! preferred (authoritative service state); `brightnessctl` is the hardware
//! fallback; XRandR is the **X11-only**, last-resort degraded adapter and is
//! never eligible in a native Wayland session.

use crate::os_control::capability::DisplayServer;
use crate::os_control::contract::{Digest, ProviderId, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

/// The concrete host brightness backend a provider selected. The string form
/// is kept **compatible with the pre-migration `set_brightness` `backend`
/// field** (`"gnome-settingsd"` / `"brightnessctl"` / `"xrandr-gamma"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrightnessBackend {
    /// logind's `Session.SetBrightness` over the **system** bus, paired with a
    /// `/sys/class/backlight` read. Physical backlight; **the preferred backend**.
    ///
    /// Preferred over GNOME because it is desktop-independent and still present:
    /// GNOME removed `SettingsDaemon.Power.Screen` in recent versions, so that
    /// interface is missing on a current Ubuntu even though the hardware is fine.
    /// logind allows the write unprivileged for the **active** session, which is
    /// exactly the case that matters here.
    LogindSession,
    /// GNOME `SettingsDaemon.Power.Screen` session D-Bus property. Physical
    /// backlight; usable under both GNOME X11 and Wayland **when the interface
    /// exists** — it was removed in newer GNOME releases.
    GnomeSettingsDaemon,
    /// `brightnessctl` hardware sysfs-backed CLI. Physical backlight; the
    /// desktop-independent fallback; usable under both X11 and Wayland.
    Brightnessctl,
    /// XRandR `--brightness` gamma-ramp scaling. **Software gamma, not
    /// physical backlight.** Degraded; X11-only — never eligible on Wayland
    /// (OSC-019.3, OSC-032.3).
    XrandrGamma,
}

impl BrightnessBackend {
    /// The full, ordered preference list (most preferred first). Callers must
    /// still filter through [`select_backend`] for the X11-only XRandR guard.
    pub const PREFERENCE: [BrightnessBackend; 4] = [
        BrightnessBackend::LogindSession,
        BrightnessBackend::GnomeSettingsDaemon,
        BrightnessBackend::Brightnessctl,
        BrightnessBackend::XrandrGamma,
    ];

    /// The stable label used in the `backend` result field and traces.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BrightnessBackend::LogindSession => "logind-session",
            BrightnessBackend::GnomeSettingsDaemon => "gnome-settingsd",
            BrightnessBackend::Brightnessctl => "brightnessctl",
            BrightnessBackend::XrandrGamma => "xrandr-gamma",
        }
    }

    /// Whether this backend controls the **physical** backlight, as opposed to
    /// a software gamma simulation (OSC-019.2).
    #[must_use]
    pub fn is_physical_backlight(self) -> bool {
        !matches!(self, BrightnessBackend::XrandrGamma)
    }

    /// Whether this backend is a declared **degraded** provider (OSC-019.3:
    /// only the XRandR software-gamma fallback is degraded; `brightnessctl` is
    /// a legitimate physical-backlight fallback, not a degraded one).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        matches!(self, BrightnessBackend::XrandrGamma)
    }

    /// The X11/Wayland eligibility for this backend (OSC-032.2). XRandR is
    /// X11-only; the others are display-server-neutral for this operation.
    #[must_use]
    pub fn eligible_for(self, display_server: DisplayServer) -> bool {
        match self {
            BrightnessBackend::XrandrGamma => display_server == DisplayServer::X11,
            // logind is display-server neutral: it talks to the seat, not the
            // compositor, so it is eligible under Wayland where XRandR is not.
            BrightnessBackend::LogindSession
            | BrightnessBackend::GnomeSettingsDaemon
            | BrightnessBackend::Brightnessctl => true,
        }
    }

    /// The trusted absolute executable path for this backend's structured
    /// command (the GNOME backend dispatches through `gdbus`, matching the
    /// pre-migration handler's transport).
    #[must_use]
    fn executable_path(self) -> &'static str {
        match self {
            // Only the WRITE uses this; the read is a direct sysfs file read.
            BrightnessBackend::LogindSession => "/usr/bin/busctl",
            BrightnessBackend::GnomeSettingsDaemon => "/usr/bin/gdbus",
            BrightnessBackend::Brightnessctl => "/usr/bin/brightnessctl",
            BrightnessBackend::XrandrGamma => "/usr/bin/xrandr",
        }
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

/// Select the most-preferred **eligible** backend for the confirmed display
/// server, or `None` when no eligible backend is available in this session
/// (→ the provider reports `Unavailable`).
///
/// This is the single choke point enforcing OSC-019.3 / OSC-032.3: XRandR is
/// filtered out by [`BrightnessBackend::eligible_for`] before preference
/// ordering is applied, so it can never be selected in a native Wayland
/// session — even if it is the only backend reported as "available".
#[must_use]
pub fn select_backend(
    display_server: DisplayServer,
    available: &[BrightnessBackend],
) -> Option<BrightnessBackend> {
    BrightnessBackend::PREFERENCE
        .into_iter()
        .filter(|candidate| candidate.eligible_for(display_server))
        .find(|candidate| available.contains(candidate))
}

/// The argv for reading the current brightness on the selected backend.
#[must_use]
pub fn query_brightness_argv(backend: BrightnessBackend) -> Vec<String> {
    match backend {
        // logind exposes no brightness getter. The provider reads
        // `/sys/class/backlight/<device>/brightness` directly and never calls this,
        // so there is deliberately no argv to return.
        BrightnessBackend::LogindSession => Vec::new(),
        BrightnessBackend::GnomeSettingsDaemon => vec![
            "call".into(),
            "--session".into(),
            "--dest".into(),
            "org.gnome.SettingsDaemon.Power".into(),
            "--object-path".into(),
            "/org/gnome/SettingsDaemon/Power".into(),
            "--method".into(),
            "org.freedesktop.DBus.Properties.Get".into(),
            "org.gnome.SettingsDaemon.Power.Screen".into(),
            "Brightness".into(),
        ],
        BrightnessBackend::Brightnessctl => vec!["get".into()],
        BrightnessBackend::XrandrGamma => vec!["--verbose".into()],
    }
}

/// The argv for setting brightness through logind.
///
/// # Why the value is a raw device number, not a percentage
///
/// `Session.SetBrightness(subsystem, name, value)` takes the value in the device's
/// own units, whose range is whatever `max_brightness` says — commonly 100, but also
/// 255, 937 or 65535 depending on the panel. Passing a percentage straight through
/// would set a laptop with `max_brightness = 65535` to almost fully dark and look
/// like a broken feature.
///
/// `auto` is used as the session path so logind resolves the **caller's own**
/// session — falling back to the user's display session when the caller has none.
/// `self` is not a valid object here, and naming a session id explicitly would let a
/// request target a different user's screen.
#[must_use]
pub fn logind_set_brightness_argv(device: &str, max_value: u32, percent: u8) -> Vec<String> {
    // Round to nearest rather than truncating: 99% of 255 truncates to 252 while
    // the user asked for very nearly full brightness.
    let scaled = if max_value == 0 {
        0
    } else {
        (u32::from(percent) * max_value + 50) / 100
    };
    vec![
        "call".into(),
        "--system".into(),
        "org.freedesktop.login1".into(),
        "/org/freedesktop/login1/session/auto".into(),
        "org.freedesktop.login1.Session".into(),
        "SetBrightness".into(),
        "ssu".into(),
        "backlight".into(),
        device.to_string(),
        scaled.to_string(),
    ]
}

/// The first `/sys/class/backlight` device that exposes both a current and a maximum
/// brightness, with its maximum.
///
/// Returns `None` when no usable device exists — a desktop with no backlight. That
/// stays `None` rather than becoming a guess, so brightness reports "not available"
/// instead of a number nobody measured.
#[must_use]
pub fn discover_backlight_device() -> Option<(String, u32)> {
    let entries = std::fs::read_dir("/sys/class/backlight").ok()?;
    let mut candidates: Vec<(String, u32)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let base = entry.path();
            let max: u32 = std::fs::read_to_string(base.join("max_brightness"))
                .ok()?
                .trim()
                .parse()
                .ok()?;
            // A zero maximum cannot be scaled against; treat the device as unusable
            // rather than dividing by it later.
            if max == 0 || !base.join("brightness").exists() {
                return None;
            }
            Some((name, max))
        })
        .collect();
    // Stable order: readdir order is not guaranteed, and a machine with two
    // backlights must not silently switch between them between calls.
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.into_iter().next()
}

/// The current brightness of `device` as a percentage of its maximum.
#[must_use]
pub fn read_backlight_percent(device: &str, max_value: u32) -> Option<u8> {
    if max_value == 0 {
        return None;
    }
    let raw: u32 = std::fs::read_to_string(format!("/sys/class/backlight/{device}/brightness"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let percent = (raw * 100 + max_value / 2) / max_value;
    u8::try_from(percent.min(100)).ok()
}

/// The argv for setting brightness to `percent` on the selected backend.
#[must_use]
pub fn set_brightness_argv(backend: BrightnessBackend, percent: u8) -> Vec<String> {
    match backend {
        // logind takes a raw DEVICE value, which cannot be derived from a
        // percentage alone. Callers must use `logind_set_brightness_argv` with the
        // resolved device and its maximum; an empty argv here is deliberate so a
        // caller that forgets produces no command rather than a wrong brightness.
        BrightnessBackend::LogindSession => Vec::new(),
        BrightnessBackend::GnomeSettingsDaemon => {
            let value = format!("<int32 {percent}>");
            vec![
                "call".into(),
                "--session".into(),
                "--dest".into(),
                "org.gnome.SettingsDaemon.Power".into(),
                "--object-path".into(),
                "/org/gnome/SettingsDaemon/Power".into(),
                "--method".into(),
                "org.freedesktop.DBus.Properties.Set".into(),
                "org.gnome.SettingsDaemon.Power.Screen".into(),
                "Brightness".into(),
                value,
            ]
        }
        BrightnessBackend::Brightnessctl => vec!["set".into(), format!("{percent}%")],
        BrightnessBackend::XrandrGamma => {
            let fraction = format!("{:.2}", percent as f64 / 100.0);
            // The concrete connector is resolved by the caller (from a fresh
            // `xrandr` query, never fabricated) and substituted before this
            // argv is captured; see `linux::providers::xrandr_display`.
            vec![
                "--output".into(),
                selection_placeholder_connector(),
                "--brightness".into(),
                fraction,
            ]
        }
    }
}

/// A stable placeholder token for the XRandR output connector position. The
/// live adapter replaces this with the resolved connected-display name before
/// dispatch; kept as a named constant so golden argv tests have a stable
/// expected value for the *shape* of the command.
#[must_use]
fn selection_placeholder_connector() -> String {
    "<resolved-connector>".to_string()
}

/// A failed brightness observation. Never a substituted percentage: a mutation
/// that "verified" against a fabricated brightness would report success it
/// never observed.
fn unreadable(backend: BrightnessBackend, reason: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(format!("display-{}", backend.as_str()))),
        reason: SafeText::new(reason),
        retryable: false,
    }
}

/// Interpret a GNOME `org.gnome.SettingsDaemon.Power.Screen.Brightness`
/// property reading (D-Bus type `i`, documented domain 0–100 percent).
///
/// Fails closed on every reading outside that domain, because **zero is a
/// legitimate brightness**: mapping "no backlight" or a raw sysfs value onto
/// `0` would let `set_brightness` verify against a percentage the host never
/// reported.
///
/// * `0..=100` — a real reading, including a genuine `0`.
/// * negative — `gnome-settings-daemon` answers `-1` when the session has no
///   controllable backlight device at all. Unknown, not dark.
/// * `> 100` — outside the property's documented percent domain (a raw sysfs
///   step count looks like this), so the scale is ambiguous.
pub fn parse_gnome_brightness_percent(
    backend: BrightnessBackend,
    raw: i32,
) -> Result<u8, OsControlError> {
    if raw < 0 {
        return Err(unreadable(
            backend,
            "the GNOME power daemon reported no controllable backlight; brightness is unknown, not zero",
        ));
    }
    if raw > 100 {
        return Err(unreadable(
            backend,
            "the GNOME power daemon reported a brightness outside the 0-100 percent domain; refusing to assume a scale",
        ));
    }
    Ok(raw as u8)
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    const GNOME: BrightnessBackend = BrightnessBackend::GnomeSettingsDaemon;

    #[test]
    fn normal_percentage_is_parsed() {
        assert_eq!(parse_gnome_brightness_percent(GNOME, 42).unwrap(), 42);
        assert_eq!(parse_gnome_brightness_percent(GNOME, 100).unwrap(), 100);
    }

    #[test]
    fn genuine_zero_is_a_real_reading_not_an_error() {
        // A backlight really driven to 0 is a fact the host reported; only an
        // *unreachable* backend is unavailable.
        assert_eq!(parse_gnome_brightness_percent(GNOME, 0).unwrap(), 0);
    }

    #[test]
    fn absent_backlight_is_unavailable_not_zero() {
        // The edge case gnome-settings-daemon actually emits on a machine with
        // no controllable backlight.
        let error = parse_gnome_brightness_percent(GNOME, -1).unwrap_err();
        assert!(matches!(
            error,
            OsControlError::Unavailable {
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn unrecognised_scale_is_an_error_not_a_default() {
        // A raw sysfs step count (or any out-of-domain value) must not be
        // clamped into a plausible-looking percentage.
        for raw in [101, 255, 96_000] {
            assert!(
                parse_gnome_brightness_percent(GNOME, raw).is_err(),
                "raw {raw} must not parse"
            );
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_matrix_prefers_gnome_then_brightnessctl_then_xrandr_on_x11() {
        use BrightnessBackend::*;
        let cases: &[(&[BrightnessBackend], Option<BrightnessBackend>)] = &[
            (
                &[GnomeSettingsDaemon, Brightnessctl, XrandrGamma],
                Some(GnomeSettingsDaemon),
            ),
            (&[Brightnessctl, XrandrGamma], Some(Brightnessctl)),
            (&[XrandrGamma], Some(XrandrGamma)),
            (&[XrandrGamma, Brightnessctl], Some(Brightnessctl)), // order-independent
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(
                select_backend(DisplayServer::X11, available),
                *expected,
                "available {available:?}"
            );
        }
    }

    #[test]
    fn selection_matrix_never_selects_xrandr_on_wayland() {
        use BrightnessBackend::*;
        let cases: &[(&[BrightnessBackend], Option<BrightnessBackend>)] = &[
            (
                &[GnomeSettingsDaemon, Brightnessctl, XrandrGamma],
                Some(GnomeSettingsDaemon),
            ),
            (&[Brightnessctl, XrandrGamma], Some(Brightnessctl)),
            // XRandR is the ONLY reported-available backend — Wayland still
            // yields None rather than selecting an X11-only provider.
            (&[XrandrGamma], None),
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(
                select_backend(DisplayServer::Wayland, available),
                *expected,
                "available {available:?}"
            );
        }
    }

    #[test]
    fn selection_matrix_headless_and_unknown_never_select_xrandr() {
        use BrightnessBackend::*;
        assert_eq!(
            select_backend(DisplayServer::Headless, &[XrandrGamma]),
            None
        );
        assert_eq!(select_backend(DisplayServer::Unknown, &[XrandrGamma]), None);
    }

    #[test]
    fn degraded_and_physical_classification() {
        assert!(!BrightnessBackend::GnomeSettingsDaemon.is_degraded());
        assert!(!BrightnessBackend::Brightnessctl.is_degraded());
        assert!(BrightnessBackend::XrandrGamma.is_degraded());

        assert!(BrightnessBackend::GnomeSettingsDaemon.is_physical_backlight());
        assert!(BrightnessBackend::Brightnessctl.is_physical_backlight());
        assert!(!BrightnessBackend::XrandrGamma.is_physical_backlight());
    }

    #[test]
    fn eligibility_matrix() {
        use BrightnessBackend::*;
        for backend in [GnomeSettingsDaemon, Brightnessctl] {
            assert!(backend.eligible_for(DisplayServer::Wayland));
            assert!(backend.eligible_for(DisplayServer::X11));
            assert!(backend.eligible_for(DisplayServer::Headless));
        }
        assert!(!XrandrGamma.eligible_for(DisplayServer::Wayland));
        assert!(XrandrGamma.eligible_for(DisplayServer::X11));
        assert!(!XrandrGamma.eligible_for(DisplayServer::Headless));
        assert!(!XrandrGamma.eligible_for(DisplayServer::Unknown));
    }

    #[test]
    fn captured_set_brightness_argv_golden() {
        assert_eq!(
            set_brightness_argv(BrightnessBackend::Brightnessctl, 60),
            vec!["set", "60%"]
        );
        assert_eq!(
            set_brightness_argv(BrightnessBackend::GnomeSettingsDaemon, 42),
            vec![
                "call",
                "--session",
                "--dest",
                "org.gnome.SettingsDaemon.Power",
                "--object-path",
                "/org/gnome/SettingsDaemon/Power",
                "--method",
                "org.freedesktop.DBus.Properties.Set",
                "org.gnome.SettingsDaemon.Power.Screen",
                "Brightness",
                "<int32 42>",
            ]
        );
        assert_eq!(
            set_brightness_argv(BrightnessBackend::XrandrGamma, 50),
            vec!["--output", "<resolved-connector>", "--brightness", "0.50"]
        );
    }

    #[test]
    fn captured_query_argv_golden() {
        assert_eq!(
            query_brightness_argv(BrightnessBackend::Brightnessctl),
            vec!["get"]
        );
        assert_eq!(
            query_brightness_argv(BrightnessBackend::XrandrGamma),
            vec!["--verbose"]
        );
    }

    #[test]
    fn trusted_executables_are_absolute_and_valid() {
        for backend in BrightnessBackend::PREFERENCE {
            let exe = backend.trusted_executable().expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
    }
}
