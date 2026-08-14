//! Do-not-disturb backend selection and output parsing (Task 4.9, OSC-023).
//!
//! There is no freedesktop standard for do-not-disturb: the
//! `org.freedesktop.Notifications` portal can post a notification but cannot say
//! whether the session is currently suppressing alerts. Each desktop keeps the
//! switch in its own place, so this module resolves *which* authority owns the
//! switch for this session and how to read and write it.
//!
//! # Why this fails closed rather than defaulting
//!
//! Do-not-disturb suppresses alerts the user may be relying on — a calendar
//! alarm, a build failure, a message. Two mistakes matter in opposite
//! directions:
//!
//! * reporting DND **off** when it is on tells the user they will be alerted
//!   when they will not be; and
//! * reporting DND **on** when it is off invites a "turn it off" mutation
//!   against a state nobody observed.
//!
//! So an unreadable or unrecognized switch is an **error**, never a default.
//! [`parse_do_not_disturb`] accepts exactly the two boolean spellings each tool
//! emits and rejects everything else, and [`select_dnd_backend`] refuses a
//! session whose desktop family was not conclusively probed rather than guessing
//! which authority owns the switch.

use crate::os_control::capability::DesktopFamily;
use crate::os_control::contract::{Digest, ProviderId, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

use super::NOTIFICATION_PROVIDER_ID;

/// The GNOME schema holding the banner switch.
const GNOME_NOTIFICATION_SCHEMA: &str = "org.gnome.desktop.notifications";
/// The GNOME key holding the banner switch. `show-banners = false` **is** GNOME's
/// do-not-disturb: it is the key the shell's own DND toggle writes.
const GNOME_SHOW_BANNERS_KEY: &str = "show-banners";

/// The concrete authority that owns this session's do-not-disturb switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DndBackend {
    /// GNOME/GTK: the `org.gnome.desktop.notifications show-banners` GSetting,
    /// read and written through `gsettings`. This is the same key the shell's own
    /// Do Not Disturb toggle writes, so it is authoritative rather than advisory.
    GnomeSettings,
    /// `dunst`: `dunstctl is-paused` / `dunstctl set-paused`. Authoritative
    /// because `dunstctl` asks the *running* notification server directly.
    Dunst,
}

impl DndBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [DndBackend; 2] = [DndBackend::GnomeSettings, DndBackend::Dunst];

    /// The stable label used in traces (never model prose).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DndBackend::GnomeSettings => "gnome-gsettings",
            DndBackend::Dunst => "dunstctl",
        }
    }

    /// Desktop-family eligibility.
    ///
    /// Exactly one authority owns the switch in a given session, so eligibility
    /// is exclusive rather than a preference:
    ///
    /// * under GNOME the shell owns notifications, and its GSetting is the
    ///   switch — a `dunstctl` reading there would describe a daemon that is not
    ///   serving notifications;
    /// * elsewhere `dunstctl` is eligible because it interrogates whichever
    ///   `dunst` instance is actually running, which is authoritative
    ///   independently of the desktop family;
    /// * an unprobed (`Unknown`) family is eligible for neither. Guessing the
    ///   owner of a switch that silences the user's alerts is exactly the
    ///   fabricated observation this architecture exists to prevent.
    #[must_use]
    pub fn eligible_for(self, desktop_family: DesktopFamily) -> bool {
        match self {
            DndBackend::GnomeSettings => desktop_family == DesktopFamily::Gnome,
            DndBackend::Dunst => matches!(
                desktop_family,
                DesktopFamily::Kde | DesktopFamily::Wlroots | DesktopFamily::Other
            ),
        }
    }

    /// The trusted absolute path of this backend's tool.
    #[must_use]
    pub fn executable_path(self) -> &'static str {
        match self {
            DndBackend::GnomeSettings => "/usr/bin/gsettings",
            DndBackend::Dunst => "/usr/bin/dunstctl",
        }
    }

    /// A stable trusted-executable identity.
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-dnd-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred **eligible** backend that is also present in
/// `available`, or `None` when this session has no readable do-not-disturb switch
/// (→ the provider reports `Unavailable`, never "not disturbed").
#[must_use]
pub fn select_dnd_backend(
    desktop_family: DesktopFamily,
    available: &[DndBackend],
) -> Option<DndBackend> {
    DndBackend::PREFERENCE
        .into_iter()
        .filter(|candidate| candidate.eligible_for(desktop_family))
        .find(|candidate| available.contains(candidate))
}

/// The argv that reads this session's do-not-disturb switch.
#[must_use]
pub fn read_dnd_argv(backend: DndBackend) -> Vec<String> {
    match backend {
        DndBackend::GnomeSettings => vec![
            "get".into(),
            GNOME_NOTIFICATION_SCHEMA.into(),
            GNOME_SHOW_BANNERS_KEY.into(),
        ],
        DndBackend::Dunst => vec!["is-paused".into()],
    }
}

/// The argv that writes this session's do-not-disturb switch.
///
/// Every element is a compile-time constant chosen by `enabled`: no caller-
/// supplied value ever reaches an argv position, so there is nothing to escape
/// and no value that could be read as an option.
#[must_use]
pub fn write_dnd_argv(backend: DndBackend, enabled: bool) -> Vec<String> {
    match backend {
        // GNOME stores the *positive* fact (show banners), so the desired
        // do-not-disturb state is written inverted. Getting this backwards would
        // silence the user's alerts while reporting that it un-silenced them.
        DndBackend::GnomeSettings => vec![
            "set".into(),
            GNOME_NOTIFICATION_SCHEMA.into(),
            GNOME_SHOW_BANNERS_KEY.into(),
            if enabled { "false" } else { "true" }.into(),
        ],
        DndBackend::Dunst => vec![
            "set-paused".into(),
            if enabled { "true" } else { "false" }.into(),
        ],
    }
}

/// Parse a backend's switch reading into "do not disturb is on".
///
/// **Fail-closed:** anything other than the exact boolean the tool documents is
/// an error. A schema rename, a locale-translated answer, or a "No such key"
/// diagnostic must not resolve to "notifications are enabled" — that would tell
/// the user they will be alerted when they will not be.
pub fn parse_do_not_disturb(backend: DndBackend, stdout: &str) -> Result<bool, OsControlError> {
    let token = stdout.trim();
    match backend {
        // `gsettings get ... show-banners` prints a GVariant boolean: bare
        // `true`/`false`. `show-banners = false` means do-not-disturb is on.
        DndBackend::GnomeSettings => match token {
            "true" => Ok(false),
            "false" => Ok(true),
            _ => Err(unparseable_dnd(backend)),
        },
        // `dunstctl is-paused` prints `true` when notifications are paused.
        DndBackend::Dunst => match token {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(unparseable_dnd(backend)),
        },
    }
}

/// The switch reading could not be interpreted. Names the tool, never any
/// notification content.
fn unparseable_dnd(backend: DndBackend) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(NOTIFICATION_PROVIDER_ID)),
        reason: SafeText::new(format!(
            "the {} do-not-disturb switch did not report a recognized state; refusing to report an alert-suppression state that was not observed",
            backend.as_str()
        )),
        retryable: false,
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn selection_is_exclusive_per_desktop_family() {
        use DndBackend::{Dunst, GnomeSettings};
        let both = [GnomeSettings, Dunst];
        assert_eq!(
            select_dnd_backend(DesktopFamily::Gnome, &both),
            Some(GnomeSettings)
        );
        assert_eq!(select_dnd_backend(DesktopFamily::Wlroots, &both), Some(Dunst));
        assert_eq!(select_dnd_backend(DesktopFamily::Kde, &both), Some(Dunst));
        // Installed but not eligible: dunst does not serve notifications on GNOME.
        assert_eq!(select_dnd_backend(DesktopFamily::Gnome, &[Dunst]), None);
        // GNOME's key is not the switch on a non-GNOME desktop.
        assert_eq!(
            select_dnd_backend(DesktopFamily::Wlroots, &[GnomeSettings]),
            None
        );
        // Unprobed family: refuse rather than guess which authority owns it.
        assert_eq!(select_dnd_backend(DesktopFamily::Unknown, &both), None);
        // Nothing installed: no switch, which is not "not disturbed".
        assert_eq!(select_dnd_backend(DesktopFamily::Gnome, &[]), None);
    }

    #[test]
    fn captured_dnd_argv_golden() {
        assert_eq!(
            read_dnd_argv(DndBackend::GnomeSettings),
            vec![
                "get".to_string(),
                "org.gnome.desktop.notifications".to_string(),
                "show-banners".to_string()
            ]
        );
        assert_eq!(read_dnd_argv(DndBackend::Dunst), vec!["is-paused".to_string()]);
        // Enabling do-not-disturb *disables* GNOME's banners.
        assert_eq!(
            write_dnd_argv(DndBackend::GnomeSettings, true),
            vec![
                "set".to_string(),
                "org.gnome.desktop.notifications".to_string(),
                "show-banners".to_string(),
                "false".to_string()
            ]
        );
        assert_eq!(
            write_dnd_argv(DndBackend::GnomeSettings, false),
            vec![
                "set".to_string(),
                "org.gnome.desktop.notifications".to_string(),
                "show-banners".to_string(),
                "true".to_string()
            ]
        );
        assert_eq!(
            write_dnd_argv(DndBackend::Dunst, true),
            vec!["set-paused".to_string(), "true".to_string()]
        );
        assert_eq!(
            write_dnd_argv(DndBackend::Dunst, false),
            vec!["set-paused".to_string(), "false".to_string()]
        );
    }

    #[test]
    fn gnome_banner_switch_is_inverted() {
        // Captured from `gsettings get org.gnome.desktop.notifications
        // show-banners` on a GNOME session.
        assert_eq!(
            parse_do_not_disturb(DndBackend::GnomeSettings, "true\n").unwrap(),
            false,
            "banners shown → do-not-disturb is off"
        );
        assert_eq!(
            parse_do_not_disturb(DndBackend::GnomeSettings, "false\n").unwrap(),
            true,
            "banners suppressed → do-not-disturb is on"
        );
    }

    #[test]
    fn dunst_paused_is_do_not_disturb() {
        assert_eq!(
            parse_do_not_disturb(DndBackend::Dunst, "true\n").unwrap(),
            true
        );
        assert_eq!(
            parse_do_not_disturb(DndBackend::Dunst, "false\n").unwrap(),
            false
        );
    }

    #[test]
    fn unrecognised_output_is_an_error_not_a_default() {
        // The single most important test in this file: an unreadable switch must
        // never degrade into "notifications are enabled".
        for hostile in [
            "",
            "\n",
            "No such schema \"org.gnome.desktop.notifications\"",
            "yes",
            "1",
            "TRUE",
            "true false",
            "vrai",
        ] {
            assert!(
                parse_do_not_disturb(DndBackend::GnomeSettings, hostile).is_err(),
                "unrecognised gsettings output must be an error, not a state: {hostile:?}"
            );
            assert!(
                parse_do_not_disturb(DndBackend::Dunst, hostile).is_err(),
                "unrecognised dunstctl output must be an error, not a state: {hostile:?}"
            );
        }
    }
}
