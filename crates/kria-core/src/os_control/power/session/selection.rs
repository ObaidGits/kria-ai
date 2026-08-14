//! Power-session (`org.freedesktop.login1`) backend selection and captured-argv
//! construction for lock/suspend/hibernate/shutdown/reboot.
//!
//! linux-os-control-production **Task 2.4** — "Migrate lock, suspend,
//! hibernate, shutdown and reboot" (OSC-004, OSC-005, OSC-020), design §9.7.
//!
//! `org.freedesktop.login1`'s D-Bus API is the preferred, authoritative
//! transport for every session-lifecycle operation this slice covers. Its
//! `loginctl` CLI front-end is retained as a declared **degraded**
//! structured-command fallback until the live D-Bus transport is wired by a
//! desktop composition root — `loginctl` exposes exactly the five subcommands
//! this slice needs (`lock-session` / `suspend` / `hibernate` / `poweroff` /
//! `reboot`), so one fixed executable covers the whole slice, mirroring how
//! [`crate::os_control::power::selection`] and
//! [`crate::os_control::connectivity::selection`] each pick one trusted binary
//! per domain.

use crate::os_control::contract::{Digest, ProviderId, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

/// The concrete host power-session backend a provider selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSessionBackend {
    /// `org.freedesktop.login1` D-Bus. Preferred.
    LogindDbus,
    /// `loginctl` structured-command fallback. Degraded.
    Loginctl,
}

impl PowerSessionBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [PowerSessionBackend; 2] = [
        PowerSessionBackend::LogindDbus,
        PowerSessionBackend::Loginctl,
    ];

    /// The stable label used in traces and the `backend` result field.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            PowerSessionBackend::LogindDbus => "logind",
            PowerSessionBackend::Loginctl => "loginctl",
        }
    }

    /// Whether this backend is a declared **degraded** provider (not the
    /// preferred authoritative D-Bus path).
    #[must_use]
    pub fn is_degraded(self) -> bool {
        !matches!(self, PowerSessionBackend::LogindDbus)
    }

    /// The trusted absolute executable path for this backend's structured
    /// command (only the `loginctl` fallback dispatches through a process).
    #[must_use]
    fn executable_path(self) -> &'static str {
        "/usr/bin/loginctl"
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
/// power-session backend is present (→ the provider reports `Unavailable`).
#[must_use]
pub fn select_backend(available: &[PowerSessionBackend]) -> Option<PowerSessionBackend> {
    PowerSessionBackend::PREFERENCE
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// The argv for `lock_screen` (`loginctl lock-session`).
#[must_use]
pub fn lock_argv() -> Vec<String> {
    vec!["lock-session".into()]
}

/// The argv for `sleep` (`loginctl suspend`).
#[must_use]
pub fn suspend_argv() -> Vec<String> {
    vec!["suspend".into()]
}

/// The argv for `hibernate` (`loginctl hibernate`).
#[must_use]
pub fn hibernate_argv() -> Vec<String> {
    vec!["hibernate".into()]
}

/// The argv for `shutdown_system` (`loginctl poweroff`). Delay scheduling is
/// **Task 3.8's** scope (KRIA-owned cancellable scheduler entry / delayed
/// logind call); this slice dispatches an immediate poweroff and threads the
/// requested delay only through the canonical request/action parameters (never
/// a shell `shutdown +N` string).
#[must_use]
pub fn shutdown_argv() -> Vec<String> {
    vec!["poweroff".into()]
}

/// The argv for `reboot_system` (`loginctl reboot`).
#[must_use]
pub fn reboot_argv() -> Vec<String> {
    vec!["reboot".into()]
}

// ─────────────────────────────────────────────────────────────────────────────
// Task 3.8: logout and scheduled-shutdown cancellation
// ─────────────────────────────────────────────────────────────────────────────

/// The maximum accepted length of a logind session id (the frozen
/// `CurrentSessionId` bound).
const MAX_SESSION_ID_CHARS: usize = 128;

/// The argv for `logout_session` (`loginctl terminate-session <id>`).
///
/// `session_id` is a logind **session id** resolved from the live session
/// manager — never a user-visible name, and never a fabricated default. Callers
/// must pass it through [`validate_session_id`] first.
#[must_use]
pub fn logout_argv(session_id: &str) -> Vec<String> {
    vec!["terminate-session".into(), session_id.to_string()]
}

/// The argv for `cancel_scheduled_shutdown` (`shutdown -c`).
///
/// systemd's `shutdown` front-end is the only CLI that reaches logind's
/// `CancelScheduledShutdown`; `loginctl` exposes no equivalent subcommand, which
/// is why this operation uses [`shutdown_schedule_executable`] rather than the
/// slice's `loginctl` binary. `-c` is a fixed literal flag chosen by this
/// module, never a caller-supplied value.
#[must_use]
pub fn cancel_scheduled_shutdown_argv() -> Vec<String> {
    vec!["-c".into()]
}

/// The trusted executable for scheduled-shutdown cancellation.
///
/// A distinct binary from [`PowerSessionBackend::trusted_executable`] because
/// `loginctl` cannot cancel a shutdown; on systemd hosts this path is the
/// `systemctl` multi-call front-end.
pub fn shutdown_schedule_executable() -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new(
        "/usr/sbin/shutdown",
        Digest::of_str("shutdown-schedule-fallback-v1"),
    )
}

/// A scheduled system shutdown, as logind reports it.
///
/// [`Self::schedule_id`] is derived from the authoritative
/// `(type, scheduled-time)` pair rather than assigned by KRIA, so the same
/// pending shutdown always has the same identity and a KRIA restart does not
/// lose the ability to name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledShutdown {
    /// The derived stable identity for this pending shutdown.
    pub schedule_id: String,
    /// logind's own action token (`poweroff`, `reboot`, `halt`, `kexec`, …).
    pub action: String,
    /// The scheduled wall-clock time, in milliseconds since the Unix epoch.
    pub scheduled_at_ms: u64,
}

/// logind's closed set of `ScheduledShutdown` action tokens.
const SHUTDOWN_ACTION_TOKENS: [&str; 8] = [
    "poweroff",
    "reboot",
    "halt",
    "kexec",
    "soft-reboot",
    "dry-poweroff",
    "dry-reboot",
    "dry-halt",
];

/// Derive the stable identity of a pending shutdown from authoritative state.
#[must_use]
pub fn derive_schedule_id(action: &str, scheduled_at_usec: u64) -> String {
    Digest::of_str(&format!("shutdown-schedule:{action}:{scheduled_at_usec}"))
        .as_hex()
        .to_string()
}

/// Interpret logind's `ScheduledShutdown` property (`(st)`: action token +
/// microseconds since the epoch).
///
/// * `("", 0)` — logind's answer when **nothing is scheduled**. That is a fact,
///   so it parses to `Ok(None)`, and `cancel_scheduled_shutdown` treats it as
///   already being in the desired state rather than as a failure.
/// * a recognized token with a real time — `Ok(Some(..))`.
/// * anything else (an unknown token, or a token with no time, or a time with no
///   token) — an error. logind and itself disagree, and picking an
///   interpretation would invent a pending shutdown or hide a real one.
pub fn parse_scheduled_shutdown(
    backend: PowerSessionBackend,
    action: &str,
    scheduled_at_usec: u64,
) -> Result<Option<ScheduledShutdown>, OsControlError> {
    let token = action.trim().to_ascii_lowercase();
    match (token.is_empty(), scheduled_at_usec) {
        (true, 0) => Ok(None),
        (true, _) => Err(unreadable(
            backend,
            "logind reported a shutdown time with no action token; the pending shutdown is unknown, not absent",
        )),
        (false, 0) => Err(unreadable(
            backend,
            "logind reported a shutdown action with no scheduled time; the pending shutdown is unknown, not absent",
        )),
        (false, usec) => {
            if !SHUTDOWN_ACTION_TOKENS.contains(&token.as_str()) {
                return Err(unreadable(
                    backend,
                    "logind reported an unrecognized scheduled-shutdown action token; refusing to assume what is pending",
                ));
            }
            Ok(Some(ScheduledShutdown {
                schedule_id: derive_schedule_id(&token, usec),
                action: token,
                scheduled_at_ms: usec / 1_000,
            }))
        }
    }
}

/// Validate a logind session id before it reaches argv (design §8, rule "validate
/// before argv"): non-empty, bounded, no control characters, and never something
/// that would be read as an option.
///
/// Rejected, never escaped.
pub fn validate_session_id(candidate: &str) -> Result<&str, OsControlError> {
    if candidate.is_empty() {
        return Err(invalid_session("a session id must not be empty"));
    }
    if candidate.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(invalid_session("session id exceeds the maximum length"));
    }
    if candidate.starts_with('-') {
        return Err(invalid_session(
            "a session id starting with '-' would be read as an option",
        ));
    }
    if candidate.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid_session(
            "session id contains control or whitespace characters",
        ));
    }
    Ok(candidate)
}

fn invalid_session(reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: crate::os_control::contract::SafeField::new("session"),
        reason: SafeText::new(reason),
    }
}

/// A failed session-state observation. "Unknown" must never collapse into a
/// concrete answer: `read_locked` returning `false` asserts the session is
/// *unlocked*, which is a different fact from "the lock state could not be
/// read".
fn unreadable(backend: PowerSessionBackend, reason: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(format!(
            "power-session-{}",
            backend.as_str()
        ))),
        reason: SafeText::new(reason),
        retryable: false,
    }
}

/// Classify an `org.freedesktop.login1.Manager.CanHibernate` reply (also the
/// text `loginctl can-hibernate` prints).
///
/// logind answers with one of four tokens:
///
/// * `yes` — supported and already permitted for this caller;
/// * `challenge` — supported, but Polkit will require authentication first;
/// * `no` — the operation exists but is not permitted;
/// * `na` — not available at all (no swap, or no firmware/kernel support).
///
/// `challenge` is **available**: hibernate is genuinely supported, and an
/// authorization refusal at dispatch surfaces as `PermissionDenied` with no
/// privilege-escalation fallback (OSC-004). Pre-empting it as "unsupported"
/// here would report a capability fact logind never stated. Any other token is
/// a failed read rather than a default, because answering `false` would claim
/// the host lacks hibernate support on no evidence.
pub fn parse_can_hibernate(
    backend: PowerSessionBackend,
    reply: &str,
) -> Result<bool, OsControlError> {
    match reply.trim().to_ascii_lowercase().as_str() {
        "yes" | "challenge" => Ok(true),
        "no" | "na" => Ok(false),
        _ => Err(unreadable(
            backend,
            "logind returned an unrecognized hibernate-capability token; refusing to assume hibernate support either way",
        )),
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    const BACKEND: PowerSessionBackend = PowerSessionBackend::LogindDbus;

    #[test]
    fn normal_yes_and_no_are_classified() {
        assert!(parse_can_hibernate(BACKEND, "yes").unwrap());
        assert!(!parse_can_hibernate(BACKEND, "no").unwrap());
    }

    #[test]
    fn na_means_the_platform_cannot_hibernate() {
        // The reply on a machine with no swap — the case this probe exists for.
        assert!(!parse_can_hibernate(BACKEND, "na").unwrap());
    }

    #[test]
    fn challenge_is_available_because_polkit_decides_authorization() {
        // Real reply when the action needs authentication; still supported.
        assert!(parse_can_hibernate(BACKEND, "challenge").unwrap());
        // Trailing newline is what `loginctl can-hibernate` actually prints.
        assert!(parse_can_hibernate(BACKEND, "Challenge\n").unwrap());
    }

    #[test]
    fn unrecognised_output_is_an_error_not_a_default() {
        for reply in ["", "  ", "maybe", "yes please", "1", "oui"] {
            assert!(
                parse_can_hibernate(BACKEND, reply).is_err(),
                "reply {reply:?} must not parse"
            );
        }
    }

    // ── scheduled shutdown ──────────────────────────────────────────────────

    #[test]
    fn nothing_scheduled_is_a_fact_not_an_error() {
        // logind's literal answer when no shutdown is pending.
        assert_eq!(parse_scheduled_shutdown(BACKEND, "", 0).unwrap(), None);
    }

    #[test]
    fn a_pending_shutdown_parses_with_a_derived_stable_identity() {
        let usec = 1_760_000_000_000_000_u64;
        let first = parse_scheduled_shutdown(BACKEND, "poweroff", usec)
            .unwrap()
            .expect("pending");
        let again = parse_scheduled_shutdown(BACKEND, "POWEROFF\n", usec)
            .unwrap()
            .expect("pending");
        assert_eq!(first, again, "identity must be stable across formatting");
        assert_eq!(first.action, "poweroff");
        assert_eq!(first.scheduled_at_ms, usec / 1_000);

        // A different time is a different schedule.
        let later = parse_scheduled_shutdown(BACKEND, "poweroff", usec + 1_000_000)
            .unwrap()
            .expect("pending");
        assert_ne!(first.schedule_id, later.schedule_id);
        // …and so is a different action at the same time.
        let reboot = parse_scheduled_shutdown(BACKEND, "reboot", usec)
            .unwrap()
            .expect("pending");
        assert_ne!(first.schedule_id, reboot.schedule_id);
    }

    #[test]
    fn unrecognised_schedule_output_is_an_error_not_a_default() {
        // An unknown token must not become "nothing is scheduled" (which would
        // let a cancel report success while a shutdown is still pending), nor a
        // fabricated poweroff.
        assert!(parse_scheduled_shutdown(BACKEND, "self-destruct", 1_000_000).is_err());
        // Half-answers are contradictions, not absence.
        assert!(parse_scheduled_shutdown(BACKEND, "poweroff", 0).is_err());
        assert!(parse_scheduled_shutdown(BACKEND, "", 1_000_000).is_err());
    }

    #[test]
    fn session_ids_are_validated_before_argv() {
        assert_eq!(validate_session_id("c2").unwrap(), "c2");
        assert_eq!(validate_session_id("session-42").unwrap(), "session-42");
        for bad in ["", "-c2", "--force", "c2 c3", "c2\n", "c2\u{0}"] {
            assert!(
                validate_session_id(bad).is_err(),
                "session id {bad:?} must be rejected, not escaped"
            );
        }
        assert!(validate_session_id(&"c".repeat(129)).is_err());
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn selection_matrix_prefers_logind_then_loginctl() {
        use PowerSessionBackend::*;
        let cases: &[(&[PowerSessionBackend], Option<PowerSessionBackend>)] = &[
            (&[LogindDbus, Loginctl], Some(LogindDbus)),
            (&[Loginctl], Some(Loginctl)),
            (&[], None),
        ];
        for (available, expected) in cases {
            assert_eq!(select_backend(available), *expected, "available {available:?}");
        }
    }

    #[test]
    fn degraded_classification() {
        assert!(!PowerSessionBackend::LogindDbus.is_degraded());
        assert!(PowerSessionBackend::Loginctl.is_degraded());
    }

    #[test]
    fn captured_argv_golden() {
        assert_eq!(lock_argv(), vec!["lock-session"]);
        assert_eq!(suspend_argv(), vec!["suspend"]);
        assert_eq!(hibernate_argv(), vec!["hibernate"]);
        assert_eq!(shutdown_argv(), vec!["poweroff"]);
        assert_eq!(reboot_argv(), vec!["reboot"]);
        assert_eq!(logout_argv("c2"), vec!["terminate-session", "c2"]);
        assert_eq!(cancel_scheduled_shutdown_argv(), vec!["-c"]);
    }

    #[test]
    fn shutdown_schedule_executable_is_absolute_and_distinct_from_loginctl() {
        let exe = shutdown_schedule_executable().expect("valid trusted executable");
        assert!(exe.path().starts_with('/'));
        assert_ne!(
            exe.path(),
            PowerSessionBackend::Loginctl
                .trusted_executable()
                .unwrap()
                .path()
        );
    }

    #[test]
    fn trusted_executables_are_absolute_and_valid() {
        for backend in PowerSessionBackend::PREFERENCE {
            let exe = backend.trusted_executable().expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
    }
}
