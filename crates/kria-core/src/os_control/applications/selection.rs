//! Application-close **observation**: captured-argv construction and
//! fail-closed process-identity parsing.
//!
//! linux-os-control-production **Task 2/§5** (OSC-013, OSC-031), design §9.3.
//!
//! # Identity comes from a stable id, never a window title
//!
//! An application is identified here by its **process identity** — the kernel
//! `comm` name and the `/proc/<pid>/exe` basename — never by a human-visible
//! window title. A window title is neither unique (two windows can share one)
//! nor stable (it changes with the open document), so matching on it would let
//! `graceful_close_application` signal an unrelated process.
//!
//! # Not running is not the same fact as could not be determined
//!
//! The observation runs `ps -e`, which lists every process and exits zero even
//! when nothing matches. So:
//!
//! * a **successful** listing with no matching process → the application is
//!   positively not running → `0`;
//! * a **failed or truncated** listing → the state is unknown → the provider
//!   returns [`OsControlError`], never `0`.
//!
//! A tool like `pidof` is deliberately not used: it exits non-zero when nothing
//! matches, and the governed read path surfaces a non-zero exit as a failed
//! observation, which would collapse "not running" into "could not read".
//!
//! # Why two queries
//!
//! The kernel truncates `comm` to 15 bytes, so a 16-character binary
//! (`gnome-calculator`) can never match on `comm` alone and would be reported
//! as *not running* — a fabricated fact. `ps -o exe=` carries the untruncated
//! path but prints `-` for kernel threads and for processes whose `exe` link
//! cannot be read. Neither field is sufficient alone, so both are read and
//! joined on pid, and matches are counted **per distinct pid**. `ps -o args=`
//! is deliberately never requested: a full command line can contain another
//! program's secret (`--password …`), and this observation needs only identity.

use std::collections::BTreeSet;

use crate::os_control::contract::{Digest, ProviderId, SafeField, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

use super::APPLICATION_CLOSE_PROVIDER_ID;

/// The trusted absolute path of the process-listing tool.
pub const PS_EXECUTABLE_PATH: &str = "/usr/bin/ps";

/// The shortest application name that can identify a process. Below this a
/// prefix match would sweep in unrelated processes, so the observation is
/// refused rather than answered.
pub const MIN_MATCHABLE_NAME_LEN: usize = 3;

/// A stable trusted-executable identity for the process-listing tool.
pub fn trusted_ps_executable() -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new(
        PS_EXECUTABLE_PATH,
        Digest::of_str("ps-application-close-observe-v1"),
    )
}

/// The argv listing every process as `pid comm` (kernel name, 15-byte
/// truncated).
#[must_use]
pub fn query_process_names_argv() -> Vec<String> {
    vec!["-e".into(), "-o".into(), "pid=,comm=".into()]
}

/// The argv listing every process as `pid exe` (untruncated executable path,
/// `-` when unreadable). Carries no command-line arguments by construction.
#[must_use]
pub fn query_process_executables_argv() -> Vec<String> {
    vec!["-e".into(), "-o".into(), "pid=,exe=".into()]
}

fn unparseable(field: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(APPLICATION_CLOSE_PROVIDER_ID)),
        reason: SafeText::new(match field {
            "comm" => "process name listing could not be parsed; refusing to report the application as not running",
            _ => "process executable listing could not be parsed; refusing to report the application as not running",
        }),
        retryable: true,
    }
}

/// Reject a name too short to identify a process. Returning "not running" for
/// an unusable name would be a fabricated observation.
pub fn validate_matchable_name(name: &str) -> Result<String, OsControlError> {
    let canonical = name.trim().to_ascii_lowercase();
    if canonical.chars().count() < MIN_MATCHABLE_NAME_LEN {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("name"),
            reason: SafeText::new(
                "application name is too short to identify a process; refusing to report it as not running",
            ),
        });
    }
    Ok(canonical)
}

/// One parsed `pid <identity>` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessIdentityRow {
    /// The process id the identity was observed for.
    pub pid: u32,
    /// The lowercased identity token (`comm`, or an `exe` basename).
    pub identity: String,
}

/// Split one `pid <rest>` row. The pid is the first whitespace-delimited
/// token; everything after it is the identity, which may itself contain
/// spaces.
fn split_row(line: &str, field: &str) -> Result<Option<ProcessIdentityRow>, OsControlError> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let (pid_token, rest) = line.split_once(char::is_whitespace).ok_or_else(|| unparseable(field))?;
    let pid: u32 = pid_token.parse().map_err(|_| unparseable(field))?;
    let identity = rest.trim();
    if identity.is_empty() {
        return Ok(None);
    }
    Ok(Some(ProcessIdentityRow {
        pid,
        identity: identity.to_ascii_lowercase(),
    }))
}

/// Parse `ps -e -o pid=,comm=` output.
///
/// **Fail-closed:** a row whose first token is not a pid is a format change,
/// which is an error rather than a shorter process list — a partially
/// understood listing would under-count and report a running application as
/// closed.
pub fn parse_pid_comm_rows(stdout: &str) -> Result<Vec<ProcessIdentityRow>, OsControlError> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        if let Some(row) = split_row(line, "comm")? {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        // `ps -e` always lists at least its own process; an empty listing means
        // the output was not what we think it is.
        return Err(unparseable("comm"));
    }
    Ok(rows)
}

/// Parse `ps -e -o pid=,exe=` output, reducing each path to its basename.
///
/// Rows whose `exe` is `-` (kernel threads, unreadable links) are skipped:
/// their identity is simply unknown through this field, and `comm` covers
/// them. An empty result is therefore legitimate here — unlike for `comm` —
/// because a listing can genuinely expose no readable `exe` link at all.
pub fn parse_pid_exe_rows(stdout: &str) -> Result<Vec<ProcessIdentityRow>, OsControlError> {
    let mut rows = Vec::new();
    let mut saw_row = false;
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        saw_row = true;
        let Some(row) = split_row(line, "exe")? else {
            continue;
        };
        if row.identity == "-" {
            continue;
        }
        let basename = row
            .identity
            .rsplit('/')
            .next()
            .unwrap_or(&row.identity)
            .to_string();
        if basename.is_empty() {
            continue;
        }
        rows.push(ProcessIdentityRow {
            pid: row.pid,
            identity: basename,
        });
    }
    if !saw_row {
        return Err(unparseable("exe"));
    }
    Ok(rows)
}

/// Whether an observed identity token matches the requested application name.
///
/// Preserves the pre-migration `CloseApplication` semantics exactly: an exact
/// match, or the observed identity being a longer variant of the requested
/// name (`gedit` matches `gedit-3`). The reverse direction is **not** matched —
/// a truncated `comm` must never sweep in a longer, unrelated name — and a
/// bare substring match is never used.
#[must_use]
pub fn matches_application_name(name_lower: &str, identity_lower: &str) -> bool {
    identity_lower == name_lower || identity_lower.starts_with(name_lower)
}

/// The distinct pids matching `name_lower` across both identity fields.
///
/// A process matching on *both* `comm` and `exe` appears once, so it can never be
/// signalled twice. The set is ordered, which makes termination deterministic.
#[must_use]
pub fn matching_pids(
    name_lower: &str,
    comm_rows: &[ProcessIdentityRow],
    exe_rows: &[ProcessIdentityRow],
) -> BTreeSet<u32> {
    let mut matched: BTreeSet<u32> = BTreeSet::new();
    for row in comm_rows.iter().chain(exe_rows.iter()) {
        if matches_application_name(name_lower, &row.identity) {
            matched.insert(row.pid);
        }
    }
    matched
}

/// Count the distinct processes matching `name_lower` across both identity
/// fields. Counting per pid is what keeps a process that matches on *both*
/// `comm` and `exe` from being counted twice.
#[must_use]
pub fn count_matching_processes(
    name_lower: &str,
    comm_rows: &[ProcessIdentityRow],
    exe_rows: &[ProcessIdentityRow],
) -> u32 {
    // A host cannot hold more than u32::MAX processes; the cast is bounded by
    // the pid space itself.
    u32::try_from(matching_pids(name_lower, comm_rows, exe_rows).len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    const COMM_LISTING: &str = "      1 systemd\n   2 kthreadd\n 4210 gedit-3\n 4211 averyverylongap\n 4212 ps\n";
    const EXE_LISTING: &str = "      1 /usr/lib/systemd/systemd\n   2 -\n 4210 /usr/bin/gedit-3\n 4211 /usr/bin/gnome-calculator\n 4212 /usr/bin/ps\n";

    #[test]
    fn captured_query_argv_golden() {
        assert_eq!(query_process_names_argv(), vec!["-e", "-o", "pid=,comm="]);
        assert_eq!(
            query_process_executables_argv(),
            vec!["-e", "-o", "pid=,exe="]
        );
        // Never `args=`/`cmd=`: a full command line can carry another
        // program's secret.
        assert!(!query_process_executables_argv()
            .iter()
            .any(|a| a.contains("args") || a.contains("cmd")));
    }

    #[test]
    fn trusted_ps_executable_is_absolute() {
        let exe = trusted_ps_executable().expect("valid trusted executable");
        assert_eq!(exe.path(), "/usr/bin/ps");
        assert!(exe.path().starts_with('/'));
    }

    #[test]
    fn comm_rows_are_parsed_and_lowercased() {
        let rows = parse_pid_comm_rows(COMM_LISTING).unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].pid, 1);
        assert_eq!(rows[0].identity, "systemd");
        assert_eq!(rows[2].identity, "gedit-3");
    }

    #[test]
    fn comm_containing_a_space_keeps_the_whole_name() {
        // Rare but legal: a process can set a comm with a space in it.
        let rows = parse_pid_comm_rows(" 99 my app\n").unwrap();
        assert_eq!(rows[0].identity, "my app");
        assert_eq!(rows[0].pid, 99);
    }

    #[test]
    fn exe_rows_skip_unreadable_links_and_keep_basenames() {
        let rows = parse_pid_exe_rows(EXE_LISTING).unwrap();
        // pid 2 has `-`: skipped, not an error.
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.pid != 2));
        assert_eq!(rows[0].identity, "systemd");
        assert_eq!(rows[2].identity, "gnome-calculator");
    }

    #[test]
    fn exe_listing_of_only_unreadable_links_is_not_an_error() {
        // A legitimate observation: nothing readable, but the listing ran.
        let rows = parse_pid_exe_rows("   1 -\n   2 -\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn truncated_comm_is_rescued_by_the_exe_basename() {
        // The real bug this two-query read exists to prevent: the kernel
        // truncates comm to 15 bytes, so `gnome-calculator` (16) can never
        // match on comm and would otherwise be reported as not running.
        let comm = parse_pid_comm_rows(COMM_LISTING).unwrap();
        let exe = parse_pid_exe_rows(EXE_LISTING).unwrap();
        assert_eq!(count_matching_processes("gnome-calculator", &comm, &exe), 1);
    }

    #[test]
    fn prefix_match_is_one_directional() {
        // `gedit` matches the longer `gedit-3` …
        assert!(matches_application_name("gedit", "gedit-3"));
        // … but a longer request must not match a shorter observed name.
        assert!(!matches_application_name("gedit-3", "gedit"));
        assert!(!matches_application_name("edit", "gedit"));
    }

    #[test]
    fn a_process_matching_both_fields_is_counted_once() {
        let comm = parse_pid_comm_rows(COMM_LISTING).unwrap();
        let exe = parse_pid_exe_rows(EXE_LISTING).unwrap();
        // pid 4210 matches on comm ("gedit-3") and on exe ("gedit-3").
        assert_eq!(count_matching_processes("gedit", &comm, &exe), 1);
    }

    #[test]
    fn a_successful_listing_with_no_match_is_zero_not_an_error() {
        let comm = parse_pid_comm_rows(COMM_LISTING).unwrap();
        let exe = parse_pid_exe_rows(EXE_LISTING).unwrap();
        assert_eq!(count_matching_processes("inkscape", &comm, &exe), 0);
    }

    #[test]
    fn short_names_are_refused_rather_than_answered() {
        assert!(validate_matchable_name("ps").is_err());
        assert!(validate_matchable_name("  a ").is_err());
        assert_eq!(validate_matchable_name("  GEdit ").unwrap(), "gedit");
    }

    #[test]
    fn unrecognised_output_is_an_error_never_an_empty_process_list() {
        // A `ps` format change must not become "the application is not
        // running", which would make a close look already-done.
        assert!(parse_pid_comm_rows("ps: unrecognized option '-o'").is_err());
        assert!(parse_pid_comm_rows("PID COMMAND\n").is_err());
        assert!(parse_pid_comm_rows("").is_err());
        assert!(parse_pid_exe_rows("ps: unrecognized option '-o'").is_err());
        assert!(parse_pid_exe_rows("").is_err());
    }

    #[test]
    fn parse_error_text_never_quotes_tool_output() {
        let err = parse_pid_comm_rows("ps: unrecognized option '-o'").expect_err("fail closed");
        let rendered = format!("{err:?}");
        assert!(!rendered.contains("unrecognized option"));
    }
}
