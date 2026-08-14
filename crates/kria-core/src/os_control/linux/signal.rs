//! The governed **signal** transport — the single place KRIA sends a signal or
//! changes a process's priority.
//!
//! linux-os-control-production design §3/§9; OSC-013.
//!
//! # Why a PID is not an identity
//!
//! A PID is reused. Between the moment KRIA observed a process and the moment it
//! signals one, the original may have exited and an unrelated process may hold
//! the same number. Signalling on the PID alone would then kill *the wrong
//! process* — potentially something the user cares about far more than the one
//! they asked to close. This is the central hazard of the whole domain.
//!
//! So every operation here takes a
//! [`crate::os_control::processes::ProcessIdentity`], which pairs the PID with
//! the process's **start time** from `/proc/<pid>/stat`. The start time is
//! re-read immediately before acting, and a mismatch is a hard error rather than
//! a silent match. The window between the check and the syscall cannot be closed
//! entirely on Linux without pidfds, but it is reduced to microseconds and the
//! failure mode becomes "refused" instead of "killed the wrong thing".
//!
//! # Containment
//!
//! * Guarded by [`deny_live_transport`], so a deny-live test can never signal.
//! * PID 1 and the whole of KRIA's own process group are refused: signalling
//!   `init` can take the machine down, and signalling ourselves would kill the
//!   agent mid-audit, losing the record of what it just did.
//! * Only a small allowlist of signals is expressible; there is no arbitrary
//!   signal-number parameter.

use crate::os_control::access::{deny_live_transport, RawTransportKind};
use crate::os_control::contract::{SafeField, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::processes::ProcessIdentity;

/// The signals this transport can send. Deliberately closed: an arbitrary signal
/// number is not a capability any tool contract asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GovernedSignal {
    /// Polite termination; the process may clean up and refuse.
    Term,
    /// Unconditional kill; no cleanup is possible.
    Kill,
}

impl GovernedSignal {
    fn as_libc(self) -> i32 {
        match self {
            GovernedSignal::Term => libc::SIGTERM,
            GovernedSignal::Kill => libc::SIGKILL,
        }
    }

    /// A redacted label for audit/receipt text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            GovernedSignal::Term => "SIGTERM",
            GovernedSignal::Kill => "SIGKILL",
        }
    }
}

fn refused(reason: &str) -> OsControlError {
    OsControlError::PolicyDenied {
        reason: SafeText::new(reason),
    }
}

fn invalid(reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new("process"),
        reason: SafeText::new(reason),
    }
}

/// Read a process's start time (field 22 of `/proc/<pid>/stat`).
///
/// The `comm` field is parenthesised and may itself contain spaces **and**
/// parentheses, so the fields after it are located from the **last** `)` rather
/// than by splitting the whole line — a process named `foo) (bar` would otherwise
/// shift every subsequent field and produce a wrong start time, which is exactly
/// the value the reuse check depends on.
pub fn read_start_time(pid: u32) -> Result<u64, OsControlError> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| invalid("process does not exist"))?;
    let tail = raw
        .rfind(')')
        .map(|index| &raw[index + 1..])
        .ok_or_else(|| invalid("process stat output is malformed"))?;
    // After `comm`, fields restart at 3 (`state`), so start time (22) is the
    // 20th whitespace-separated token here.
    tail.split_whitespace()
        .nth(19)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| invalid("process start time could not be read"))
}

/// Confirm the process at `identity.pid` is still the one that was observed.
///
/// A start-time mismatch means the PID was reused: a **different** process now
/// holds it, and acting would hit the wrong target.
pub fn verify_identity(identity: ProcessIdentity) -> Result<(), OsControlError> {
    let observed = read_start_time(identity.pid)?;
    if observed != identity.start_time {
        return Err(OsControlError::TargetChanged);
    }
    Ok(())
}

/// Refuse targets that must never be signalled.
fn guard_target(pid: u32) -> Result<(), OsControlError> {
    if pid <= 1 {
        return Err(refused(
            "refusing to signal pid 1 (init): it would terminate the session or the machine",
        ));
    }
    // Never signal ourselves or a sibling in our own process group: killing the
    // agent mid-operation would destroy the audit record of what it just did.
    let own_pid = std::process::id();
    if pid == own_pid {
        return Err(refused("refusing to signal KRIA's own process"));
    }
    // SAFETY: `getpgid` only reads scheduler metadata and cannot fail
    // destructively; a negative return simply means the pid is gone.
    let (target_group, own_group) =
        unsafe { (libc::getpgid(pid as libc::pid_t), libc::getpgid(own_pid as libc::pid_t)) };
    if target_group >= 0 && target_group == own_group {
        return Err(refused(
            "refusing to signal a process in KRIA's own process group",
        ));
    }
    Ok(())
}

/// Send a signal to a verified process.
pub fn send_signal(
    identity: ProcessIdentity,
    signal: GovernedSignal,
) -> Result<(), OsControlError> {
    deny_live_transport(RawTransportKind::ProcessSignal);
    guard_target(identity.pid)?;
    // Re-verify immediately before the syscall to keep the reuse window minimal.
    verify_identity(identity)?;

    // SAFETY: the pid was just confirmed to exist and to be the observed process,
    // and the signal is from a closed allowlist.
    let result = unsafe { libc::kill(identity.pid as libc::pid_t, signal.as_libc()) };
    if result != 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return match errno {
            // The process exited between the check and the syscall. Nothing was
            // signalled, so this is provably a pre-mutation failure.
            libc::ESRCH => Err(OsControlError::TargetChanged),
            libc::EPERM => Err(OsControlError::PermissionDenied {
                authority: SafeText::new(
                    "signalling this process requires privilege KRIA does not hold",
                ),
                remediation: SafeText::new(
                    "the process belongs to another user or to the system; close it from its own session",
                ),
            }),
            _ => Err(invalid("the signal could not be delivered")),
        };
    }
    Ok(())
}

/// Change a verified process's nice value.
///
/// Lowering the nice value (raising priority) requires privilege and is refused
/// here rather than silently failing: this transport is unprivileged by design,
/// and a privileged change belongs to the broker.
pub fn set_priority(identity: ProcessIdentity, nice: i32) -> Result<(), OsControlError> {
    // Input validation opens no transport, so it runs before the deny-live guard:
    // a nonsensical request is rejected without touching the system at all.
    if !(-20..=19).contains(&nice) {
        return Err(invalid("nice value must be between -20 and 19"));
    }
    deny_live_transport(RawTransportKind::ProcessSignal);
    guard_target(identity.pid)?;
    verify_identity(identity)?;

    // SAFETY: the pid was just verified; `setpriority` only adjusts scheduling.
    let result = unsafe {
        libc::setpriority(
            libc::PRIO_PROCESS as libc::c_uint,
            identity.pid,
            nice,
        )
    };
    if result != 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return match errno {
            libc::ESRCH => Err(OsControlError::TargetChanged),
            libc::EACCES | libc::EPERM => Err(OsControlError::PermissionDenied {
                authority: SafeText::new("changing this priority requires privilege"),
                remediation: SafeText::new(
                    "raising a process's priority needs the privileged broker; lowering it does not",
                ),
            }),
            _ => Err(invalid("the priority could not be changed")),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_refused() {
        let error = guard_target(1).expect_err("pid 1 must be refused");
        assert_eq!(error.code(), "os_control.policy_denied");
        assert!(guard_target(0).is_err());
    }

    #[test]
    fn our_own_process_is_refused() {
        // Killing ourselves mid-operation would destroy the audit record.
        let error = guard_target(std::process::id()).expect_err("self must be refused");
        assert_eq!(error.code(), "os_control.policy_denied");
    }

    #[test]
    fn a_start_time_mismatch_is_target_changed() {
        // Our own start time is readable; a deliberately wrong one must be
        // rejected rather than accepted as a match.
        let pid = std::process::id();
        let real = read_start_time(pid).expect("own start time is readable");
        assert!(verify_identity(ProcessIdentity::new(pid, real)).is_ok());

        let error = verify_identity(ProcessIdentity::new(pid, real.wrapping_add(1)))
            .expect_err("a reused pid must not verify");
        assert_eq!(error.code(), "os_control.target_changed");
    }

    #[test]
    fn a_nonexistent_pid_is_an_error_not_a_silent_success() {
        // u32::MAX is above any real pid_max.
        assert!(read_start_time(u32::MAX).is_err());
    }

    #[test]
    fn an_out_of_range_nice_value_is_rejected() {
        let identity = ProcessIdentity::new(std::process::id(), 0);
        assert!(set_priority(identity, 50).is_err());
        assert!(set_priority(identity, -50).is_err());
    }

    #[test]
    fn a_comm_containing_parens_does_not_shift_the_start_time() {
        // Regression guard for the parsing rule: fields are located from the LAST
        // ')' so a process named ") (" cannot shift field 22.
        let line = "1234 (weird) (name) S 1 1234 1234 0 -1 4194304 0 0 0 0 \
                    1 2 3 4 20 0 1 0 999888 0 0 0 0";
        let tail = &line[line.rfind(')').unwrap() + 1..];
        let start = tail.split_whitespace().nth(19).unwrap();
        assert_eq!(start, "999888");
    }
}
