//! Automation backend selection, argv construction and output parsing
//! (Task 4.5, OSC-027).
//!
//! Two facts shape this module:
//!
//! 1. **Only systemd user timers are addressable.** A crontab line has no
//!    stable identity and no revision, so it cannot support the frozen
//!    `expected_revision` compare-and-set that `modify_scheduled_task`
//!    requires. The crontab backend is therefore listable but not modifiable —
//!    refused explicitly rather than approximated.
//! 2. **A unit name is validated, never escaped.** A unit beginning with `-`
//!    would be read by `systemctl` as an option, so it is rejected before it can
//!    become an argv element.

use crate::os_control::contract::{Digest, SafeField, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::TrustedExecutable;

/// Maximum characters in a systemd unit name accepted here.
pub const UNIT_NAME_MAX_CHARS: usize = 96;

/// The automation backends, most-preferred first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomationBackend {
    /// `systemctl --user` timers: stable unit identity, observable enablement
    /// state, and a unit fragment whose mtime is a monotonic config revision.
    SystemdUserTimers,
    /// `crontab -l`: listable text only. No stable per-entry identity.
    Crontab,
}

impl AutomationBackend {
    /// Preference order used by [`select_backend`].
    pub const PREFERENCE: [AutomationBackend; 2] =
        [AutomationBackend::SystemdUserTimers, AutomationBackend::Crontab];

    /// The stable backend token (never model prose).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AutomationBackend::SystemdUserTimers => "systemd-user-timers",
            AutomationBackend::Crontab => "crontab",
        }
    }

    /// Whether this backend can serve a typed modification.
    ///
    /// Crontab cannot: an entry has no stable id and no revision, so a
    /// compare-and-set patch is not expressible over it.
    #[must_use]
    pub const fn supports_modification(self) -> bool {
        matches!(self, AutomationBackend::SystemdUserTimers)
    }

    const fn executable_path(self) -> &'static str {
        match self {
            AutomationBackend::SystemdUserTimers => "/usr/bin/systemctl",
            AutomationBackend::Crontab => "/usr/bin/crontab",
        }
    }

    /// The trusted executable for this backend's structured commands.
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-v1", self.as_str())),
        )
    }
}

/// The trusted executable used to read a unit fragment's modification time.
///
/// The revision a `modify_scheduled_task` compare-and-set is checked against is
/// the unit fragment's mtime in milliseconds — a value that changes on every
/// configuration write and is readable without a second source of truth.
pub fn stat_executable() -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new("/usr/bin/stat", Digest::of_str("coreutils-stat-v1"))
}

/// Select the most-preferred available backend, or `None` when neither is
/// present (→ the provider reports `Unavailable`, never an empty listing).
#[must_use]
pub fn select_backend(available: &[AutomationBackend]) -> Option<AutomationBackend> {
    AutomationBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

/// Validate a scheduled task's identity as a systemd **timer unit** name.
///
/// Rejected, never escaped or repaired:
///
/// * a name starting with `-` would be parsed by `systemctl` as an option;
/// * a name that is not already a `.timer` unit is refused rather than having
///   `.timer` appended, because inventing an identity is how a mutation ends up
///   pointed at something the caller never named;
/// * control characters, path separators and `..` are refused outright.
pub fn validate_timer_unit(task_id: &str) -> Result<&str, OsControlError> {
    let field = "task_id";
    if task_id.is_empty() || task_id.chars().count() > UNIT_NAME_MAX_CHARS {
        return Err(invalid(
            field,
            "task_id must be a systemd timer unit name of 1..=96 characters",
        ));
    }
    if !task_id.ends_with(".timer") {
        return Err(invalid(
            field,
            "task_id must be a fully-qualified systemd user timer unit ending in `.timer`",
        ));
    }
    let mut chars = task_id.chars();
    let first = chars.next().unwrap_or('-');
    if !(first.is_ascii_alphanumeric()) {
        return Err(invalid(
            field,
            "task_id must begin with an alphanumeric character",
        ));
    }
    for ch in task_id.chars() {
        if ch.is_control() {
            return Err(invalid(field, "task_id contains a control character"));
        }
        // A path separator is excluded by this set, so a unit name can never
        // reach out of the unit namespace.
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '@' | ':' | '-')) {
            return Err(invalid(
                field,
                "task_id contains a character systemd unit names do not permit",
            ));
        }
    }
    if task_id.contains("..") {
        return Err(invalid(field, "task_id must not contain `..`"));
    }
    Ok(task_id)
}

/// Validate a filesystem path that came from `systemctl show` output before it
/// becomes an argv element.
///
/// The value is provider output rather than caller input, but it still reaches
/// argv, so it is checked with the same discipline.
pub fn validate_fragment_path(path: &str) -> Result<&str, OsControlError> {
    let field = "fragment_path";
    if path.is_empty() || path.len() > 4096 {
        return Err(invalid(field, "unit fragment path is empty or too long"));
    }
    if !path.starts_with('/') {
        return Err(invalid(field, "unit fragment path must be absolute"));
    }
    if path.chars().any(char::is_control) {
        return Err(invalid(field, "unit fragment path contains a control character"));
    }
    Ok(path)
}

// ─────────────────────────────────────────────────────────────────────────────
// argv builders — fixed shape, no shell, no interpolation
// ─────────────────────────────────────────────────────────────────────────────

/// `systemctl --user list-timers` argv (the existing listing read).
#[must_use]
pub fn list_timers_argv() -> Vec<String> {
    vec![
        "--user".into(),
        "list-timers".into(),
        "--all".into(),
        "--no-pager".into(),
        "--no-legend".into(),
    ]
}

/// `crontab -l` argv (the existing listing read).
#[must_use]
pub fn list_crontab_argv() -> Vec<String> {
    vec!["-l".into()]
}

/// `systemctl --user show <unit>` argv for the properties a task observation
/// needs. The unit is placed last, after every option, and has already been
/// validated so it cannot be read as one.
pub fn show_timer_argv(unit: &str) -> Result<Vec<String>, OsControlError> {
    let unit = validate_timer_unit(unit)?;
    Ok(vec![
        "--user".into(),
        "show".into(),
        "--no-pager".into(),
        "--property=Id".into(),
        "--property=LoadState".into(),
        "--property=UnitFileState".into(),
        "--property=FragmentPath".into(),
        "--property=NextElapseUSecRealtime".into(),
        unit.to_string(),
    ])
}

/// `systemctl --user enable|disable <unit>` argv.
pub fn set_enabled_argv(unit: &str, enabled: bool) -> Result<Vec<String>, OsControlError> {
    let unit = validate_timer_unit(unit)?;
    Ok(vec![
        "--user".into(),
        if enabled { "enable".into() } else { "disable".into() },
        unit.to_string(),
    ])
}

/// `stat -c %Y.%f`-style argv reading a unit fragment's mtime.
///
/// `--` terminates option parsing so a path can never be read as an option.
pub fn fragment_mtime_argv(fragment_path: &str) -> Result<Vec<String>, OsControlError> {
    let path = validate_fragment_path(fragment_path)?;
    Ok(vec![
        "-c".into(),
        "%.3Y".into(),
        "--".into(),
        path.to_string(),
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// parsers — an unrecognised output is an error, never a default
// ─────────────────────────────────────────────────────────────────────────────

fn unparseable(what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: None,
        reason: SafeText::new(format!(
            "{what} could not be parsed; refusing to report a default in place of an unread fact"
        )),
        retryable: false,
    }
}

/// Whether a unit's file state means "enabled", "disabled", or is not a
/// recognised state at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitEnablement {
    /// The timer is enabled (`enabled`, `enabled-runtime`).
    Enabled,
    /// The timer exists but is not enabled (`disabled`, `masked`, `masked-runtime`).
    Disabled,
    /// The unit is a static or generated unit with no enablement state. This is
    /// a *fact*, distinct from "could not determine".
    NotApplicable,
}

/// A parsed `systemctl --user show` observation for one timer unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerShow {
    /// The unit id systemd echoed back.
    pub id: String,
    /// Whether systemd knows this unit at all. `false` is the "absent" fact.
    pub present: bool,
    /// Enablement, when the unit is present.
    pub enablement: Option<UnitEnablement>,
    /// The unit fragment path, when the unit has one.
    pub fragment_path: Option<String>,
    /// Next elapse in Unix epoch milliseconds, when systemd reported one.
    pub next_run_ms: Option<u64>,
}

fn property<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then_some(value.trim())
    })
}

/// Parse `systemctl --user show` key=value output for a timer unit.
///
/// Fails when the output contains none of the requested properties: that means
/// the command did not do what we asked, and reporting "not enabled" from it
/// would let an enable request verify against a fact nobody read.
pub fn parse_timer_show(stdout: &str) -> Result<TimerShow, OsControlError> {
    let load_state = property(stdout, "LoadState");
    let unit_file_state = property(stdout, "UnitFileState");
    let id = property(stdout, "Id");
    if load_state.is_none() && unit_file_state.is_none() && id.is_none() {
        return Err(unparseable("systemctl show output"));
    }

    // `not-found` is systemd stating the unit does not exist — a fact, not a
    // failure to read.
    let present = match load_state {
        Some("not-found") => false,
        Some("loaded") | Some("masked") | Some("bad-setting") | Some("error") => true,
        // No LoadState at all: fall back to whether a unit file state exists.
        None => unit_file_state.is_some_and(|s| !s.is_empty()),
        Some(other) if other.is_empty() => false,
        Some(_) => return Err(unparseable("systemctl LoadState")),
    };

    let enablement = match unit_file_state {
        None => None,
        Some("") => None,
        Some("enabled") | Some("enabled-runtime") => Some(UnitEnablement::Enabled),
        Some("disabled") | Some("masked") | Some("masked-runtime") => {
            Some(UnitEnablement::Disabled)
        }
        Some("static") | Some("indirect") | Some("generated") | Some("transient")
        | Some("alias") | Some("linked") | Some("linked-runtime") => {
            Some(UnitEnablement::NotApplicable)
        }
        Some("not-found") => None,
        Some(_) => return Err(unparseable("systemctl UnitFileState")),
    };

    let fragment_path = property(stdout, "FragmentPath")
        .filter(|p| !p.is_empty())
        .map(str::to_string);

    let next_run_ms = match property(stdout, "NextElapseUSecRealtime") {
        None => None,
        Some("") | Some("n/a") | Some("0") => None,
        Some(raw) => Some(parse_next_elapse_us(raw)? / 1_000),
    };

    Ok(TimerShow {
        id: id.unwrap_or_default().to_string(),
        present,
        enablement,
        fragment_path,
        next_run_ms,
    })
}

/// Parse systemd's `NextElapseUSecRealtime` value (microseconds since epoch).
fn parse_next_elapse_us(raw: &str) -> Result<u64, OsControlError> {
    raw.parse::<u64>()
        .map_err(|_| unparseable("systemctl NextElapseUSecRealtime"))
}

/// Parse `stat -c %.3Y` output (`seconds.milliseconds`) into epoch
/// milliseconds — the provider's configuration revision.
pub fn parse_fragment_mtime_ms(stdout: &str) -> Result<u64, OsControlError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(unparseable("stat mtime output"));
    }
    let (seconds, fraction) = match trimmed.split_once('.') {
        Some((s, f)) => (s, f),
        None => (trimmed, "0"),
    };
    let seconds: u64 = seconds
        .parse()
        .map_err(|_| unparseable("stat mtime seconds"))?;
    let mut millis: u64 = 0;
    for (index, ch) in fraction.chars().take(3).enumerate() {
        let digit = ch
            .to_digit(10)
            .ok_or_else(|| unparseable("stat mtime fraction"))? as u64;
        millis += digit * 10u64.pow(2 - index as u32);
    }
    seconds
        .checked_mul(1_000)
        .and_then(|ms| ms.checked_add(millis))
        .ok_or_else(|| unparseable("stat mtime overflow"))
}

#[cfg(all(test, feature = "os-control-test"))]
mod parse_tests {
    use super::*;

    #[test]
    fn unrecognised_output_is_an_error_not_a_default() {
        // Mandatory: nothing in this blob is a property we asked for.
        assert!(parse_timer_show("garbage from another program\nfoo\n").is_err());
        assert!(parse_timer_show("").is_err());
        // A property we asked for, with a value systemd does not define.
        assert!(parse_timer_show("LoadState=loaded\nUnitFileState=who-knows\n").is_err());
        assert!(parse_fragment_mtime_ms("not-a-number").is_err());
        assert!(parse_fragment_mtime_ms("").is_err());
    }

    #[test]
    fn an_absent_unit_is_a_fact_not_an_error() {
        let show = parse_timer_show("Id=kria-x.timer\nLoadState=not-found\nUnitFileState=\n")
            .expect("absent is parseable");
        assert!(!show.present);
        assert_eq!(show.enablement, None);
    }

    #[test]
    fn enabled_and_disabled_states_are_distinguished() {
        let enabled = parse_timer_show(
            "Id=kria-backup.timer\nLoadState=loaded\nUnitFileState=enabled\nFragmentPath=/home/u/.config/systemd/user/kria-backup.timer\nNextElapseUSecRealtime=1700000000000000\n",
        )
        .expect("parseable");
        assert!(enabled.present);
        assert_eq!(enabled.enablement, Some(UnitEnablement::Enabled));
        assert_eq!(
            enabled.fragment_path.as_deref(),
            Some("/home/u/.config/systemd/user/kria-backup.timer")
        );
        assert_eq!(enabled.next_run_ms, Some(1_700_000_000_000));

        let disabled =
            parse_timer_show("Id=kria-backup.timer\nLoadState=loaded\nUnitFileState=disabled\n")
                .expect("parseable");
        assert_eq!(disabled.enablement, Some(UnitEnablement::Disabled));
        assert_eq!(disabled.next_run_ms, None);
    }

    #[test]
    fn a_static_unit_reports_not_applicable_rather_than_disabled() {
        let show = parse_timer_show("LoadState=loaded\nUnitFileState=static\n").expect("parseable");
        assert_eq!(show.enablement, Some(UnitEnablement::NotApplicable));
    }

    #[test]
    fn mtime_parses_to_milliseconds() {
        assert_eq!(parse_fragment_mtime_ms("1700000000.125\n").expect("ok"), 1_700_000_000_125);
        assert_eq!(parse_fragment_mtime_ms("1700000000").expect("ok"), 1_700_000_000_000);
        assert_eq!(parse_fragment_mtime_ms("1700000000.1").expect("ok"), 1_700_000_000_100);
    }

    #[test]
    fn a_unit_name_that_could_be_read_as_an_option_is_rejected() {
        assert!(validate_timer_unit("--user").is_err());
        assert!(validate_timer_unit("-x.timer").is_err());
        assert!(validate_timer_unit("../../etc/passwd.timer").is_err());
        assert!(validate_timer_unit("kria backup.timer").is_err());
        assert!(validate_timer_unit("kria\nbackup.timer").is_err());
        // A bare name is refused rather than having `.timer` appended.
        assert!(validate_timer_unit("kria-backup").is_err());
        assert!(validate_timer_unit("kria-backup.timer").is_ok());
    }

    #[test]
    fn argv_is_fixed_and_places_the_unit_last() {
        let argv = show_timer_argv("kria-backup.timer").expect("valid unit");
        assert_eq!(argv.first().map(String::as_str), Some("--user"));
        assert_eq!(argv.last().map(String::as_str), Some("kria-backup.timer"));
        let enable = set_enabled_argv("kria-backup.timer", true).expect("valid unit");
        assert_eq!(enable, vec!["--user", "enable", "kria-backup.timer"]);
        let disable = set_enabled_argv("kria-backup.timer", false).expect("valid unit");
        assert_eq!(disable, vec!["--user", "disable", "kria-backup.timer"]);
    }

    #[test]
    fn a_fragment_path_argv_terminates_option_parsing() {
        let argv = fragment_mtime_argv("/home/u/.config/systemd/user/x.timer").expect("valid");
        assert!(argv.contains(&"--".to_string()));
        assert!(validate_fragment_path("relative/path").is_err());
    }

    #[test]
    fn crontab_cannot_serve_a_modification() {
        assert!(!AutomationBackend::Crontab.supports_modification());
        assert!(AutomationBackend::SystemdUserTimers.supports_modification());
        assert_eq!(
            select_backend(&[AutomationBackend::Crontab, AutomationBackend::SystemdUserTimers]),
            Some(AutomationBackend::SystemdUserTimers)
        );
        assert_eq!(select_backend(&[]), None);
    }
}
