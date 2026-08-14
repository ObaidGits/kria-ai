//! Process-domain `procfs` reading parsers.
//!
//! linux-os-control-production **Task 2/§5** (live provider reads), design §3,
//! §9.5 (`processes/selection.rs`).
//!
//! The live adapter
//! ([`crate::os_control::linux::providers::process_control::LiveProcessControl`])
//! observes processes by reading the kernel's `procfs` files. That is a
//! **filesystem read, not a child process**: there is no tool to shell out to,
//! so these readings never go through
//! [`crate::os_control::linux::structured_query::StructuredQueryRequest`]. What
//! that costs us is the governed query path's parsing discipline, so every byte
//! the adapter reads is parsed by a pure function here and unit-tested against
//! real kernel output shapes.
//!
//! Every parser is **fail-closed**: unrecognised input is an
//! [`OsControlError`], never a substituted value. Reporting "nice is 0" because
//! the `stat` layout moved would let a `set_process_priority` mutation "verify"
//! against a fact that was never read, and reporting "not alive" for a process
//! whose `stat` failed to parse would let `kill_process` claim success against a
//! live process. The one place a non-error normalization is allowed is
//! [`parse_lifecycle_state`], because the frozen manifest itself defines an
//! `Unknown` member for provider states outside its closed set.
//!
//! # PID reuse
//!
//! A PID is not an identity. The reuse guard is the process start time from
//! `/proc/<pid>/stat` field 22, normalized to
//! [`crate::os_control::processes::ProcessIdentity::start_time`] by
//! [`start_time_ms`] and compared by [`start_time_matches`]. See that function
//! for the exact strength of the guarantee.

use crate::os_control::contract::{Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::processes::{ProcessLifecycleState, PROCESS_PROVIDER_ID};

/// Tolerance applied when comparing a caller-supplied
/// [`crate::os_control::processes::ProcessIdentity::start_time`] against a
/// freshly observed start time (see [`start_time_matches`]).
///
/// The normalized start time is derived from `/proc/stat`'s `btime`, which the
/// kernel reports in **whole seconds**, so the epoch base of every reading
/// already carries up to a second of quantization and can shift across a clock
/// step or a suspend/resume. An exact `==` comparison would therefore turn a
/// clock adjustment into "the process is gone", which is the dangerous
/// direction of error for `kill_process` verification.
pub const START_TIME_MATCH_TOLERANCE_MS: u64 = 1_000;

/// Maximum bytes of the bounded executable label carried by a
/// [`crate::os_control::processes::ProcessObservation`]. The kernel's `comm` is
/// already at most 15 characters; this bounds the sanitized form regardless.
pub const MAX_EXECUTABLE_LABEL_BYTES: usize = 64;

/// The fail-closed error for a `procfs` reading that could not be parsed.
///
/// `source` is a fixed caller-chosen label (e.g. `"process stat reading"`) —
/// never a raw OS error string and never a captured line of the file, so no
/// process's command content can reach an error message through here.
#[must_use]
pub fn unparseable_proc_reading(source: &'static str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(PROCESS_PROVIDER_ID)),
        reason: SafeText::new(format!(
            "{source} could not be parsed; refusing to assume a process reading"
        )),
        retryable: true,
    }
}

/// The fail-closed error for a process that exposes no command line at all
/// (a kernel thread, or a process that exited before `/proc/<pid>/cmdline`
/// was read).
///
/// An empty `cmdline` is deliberately **not** reported as "zero arguments":
/// the two cases are indistinguishable from the file alone, and claiming an
/// empty argv for a process that in fact has one would be a fabricated
/// observation of the most privacy-sensitive read in this domain.
#[must_use]
pub fn no_command_line_error() -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(PROCESS_PROVIDER_ID)),
        reason: SafeText::new(
            "the process exposes no command line; refusing to report an empty argv as a reading",
        ),
        retryable: false,
    }
}

/// The exact `/proc/<pid>/stat` fields this domain reads. Deliberately a
/// narrow projection: `stat` carries 50+ fields and none of the others are
/// part of any process contract here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStatFields {
    /// Field 1 — the PID the file itself reports.
    pub pid: u32,
    /// Field 2 — the kernel's `comm` (the process *name*, never its argv).
    /// May contain spaces, parentheses, or a newline.
    pub comm: String,
    /// Field 3 — the lifecycle state, normalized to the closed set.
    pub state: ProcessLifecycleState,
    /// Field 19 — the scheduling niceness.
    pub nice: i32,
    /// Fields 14 + 15 (`utime` + `stime`) in clock ticks.
    pub cpu_ticks: u64,
    /// Field 22 — start time in clock ticks since boot (the PID-reuse guard).
    pub start_time_ticks: u64,
    /// Field 24 — resident set size in pages.
    pub rss_pages: u64,
}

/// Parse the fields this domain needs out of one `/proc/<pid>/stat` line.
///
/// `comm` (field 2) is parenthesized and may itself contain spaces and
/// parentheses (`(Web Content)`, `(foo) bar)`), so the split point is the
/// **last** `)`, never whitespace tokenization of the whole line.
pub fn parse_proc_stat(raw: &str) -> Result<ProcStatFields, OsControlError> {
    const SOURCE: &str = "process stat reading";
    let line = raw.trim_end_matches('\n');
    let open = line.find('(').ok_or_else(|| unparseable_proc_reading(SOURCE))?;
    let close = line
        .rfind(')')
        .ok_or_else(|| unparseable_proc_reading(SOURCE))?;
    if close <= open {
        return Err(unparseable_proc_reading(SOURCE));
    }
    let pid: u32 = line[..open]
        .trim()
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    let comm = line[open + 1..close].to_string();
    let rest: Vec<&str> = line[close + 1..].split_whitespace().collect();
    // `rest[0]` is field 3 (`state`), so field N lives at `rest[N - 3]`.
    let field = |n: usize| -> Result<&str, OsControlError> {
        rest.get(n - 3)
            .copied()
            .ok_or_else(|| unparseable_proc_reading(SOURCE))
    };
    let state = parse_lifecycle_state(field(3)?);
    let nice: i32 = field(19)?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    let utime: u64 = field(14)?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    let stime: u64 = field(15)?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    let start_time_ticks: u64 = field(22)?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    let rss_pages: u64 = field(24)?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    Ok(ProcStatFields {
        pid,
        comm,
        state,
        nice,
        cpu_ticks: utime.saturating_add(stime),
        start_time_ticks,
        rss_pages,
    })
}

/// Normalize a `/proc/<pid>/stat` state token onto the frozen closed set.
///
/// `D` (uninterruptible sleep) maps to
/// [`ProcessLifecycleState::Sleeping`] because the manifest's member covers
/// "interruptible or uninterruptible"; `t` (tracing stop) maps to
/// [`ProcessLifecycleState::Stopped`]. Anything else — `I` (idle kernel
/// thread), `X`/`x` (dead), `W`, `P`, or a token a future kernel introduces —
/// maps to [`ProcessLifecycleState::Unknown`], which is the manifest's own
/// member for "provider-reported state outside the closed set" and therefore
/// a normalization rather than an invented default.
#[must_use]
pub fn parse_lifecycle_state(token: &str) -> ProcessLifecycleState {
    match token.chars().next() {
        Some('R') => ProcessLifecycleState::Running,
        Some('S') | Some('D') => ProcessLifecycleState::Sleeping,
        Some('T') | Some('t') => ProcessLifecycleState::Stopped,
        Some('Z') => ProcessLifecycleState::Zombie,
        _ => ProcessLifecycleState::Unknown,
    }
}

/// The `/proc/<pid>/status` fields this domain reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStatusFields {
    /// The `Pid:` line — cross-checked against the directory the file came
    /// from.
    pub pid: u32,
    /// The `Name:` line — the same `comm` `stat` reports, used only as a
    /// cross-file consistency check.
    pub name: String,
    /// The **real** uid from the `Uid:` line (first of real/effective/saved/fs).
    pub uid: u32,
}

/// Parse `Pid:`, `Name:` and the real uid out of `/proc/<pid>/status`.
///
/// A missing `Uid:` line is an error rather than an anonymous owner: `owner`
/// is a required field of every
/// [`crate::os_control::processes::ProcessObservation`], and an invented
/// owner is exactly the kind of fabricated fact a policy decision could later
/// be made against.
pub fn parse_proc_status(raw: &str) -> Result<ProcStatusFields, OsControlError> {
    const SOURCE: &str = "process status reading";
    let mut pid = None;
    let mut name = None;
    let mut uid = None;
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("Name:") {
            if name.is_none() {
                name = Some(value.trim().to_string());
            }
        } else if let Some(value) = line.strip_prefix("Pid:") {
            if pid.is_none() {
                pid = value.trim().parse::<u32>().ok();
            }
        } else if let Some(value) = line.strip_prefix("Uid:") {
            if uid.is_none() {
                uid = value
                    .split_whitespace()
                    .next()
                    .and_then(|real| real.parse::<u32>().ok());
            }
        }
    }
    match (pid, name, uid) {
        (Some(pid), Some(name), Some(uid)) => Ok(ProcStatusFields { pid, name, uid }),
        _ => Err(unparseable_proc_reading(SOURCE)),
    }
}

/// Parse the kernel boot time (`btime`, seconds since the epoch) out of
/// `/proc/stat`. This is the epoch base for every normalized start time.
pub fn parse_boot_time_seconds(raw: &str) -> Result<u64, OsControlError> {
    const SOURCE: &str = "kernel boot time reading";
    raw.lines()
        .find_map(|line| line.strip_prefix("btime "))
        .and_then(|value| value.trim().parse::<u64>().ok())
        .ok_or_else(|| unparseable_proc_reading(SOURCE))
}

/// Parse system uptime in seconds out of `/proc/uptime` (`"<up> <idle>"`).
pub fn parse_uptime_seconds(raw: &str) -> Result<f64, OsControlError> {
    const SOURCE: &str = "system uptime reading";
    let seconds: f64 = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| unparseable_proc_reading(SOURCE))?
        .parse()
        .map_err(|_| unparseable_proc_reading(SOURCE))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(unparseable_proc_reading(SOURCE));
    }
    Ok(seconds)
}

/// Normalize `/proc/<pid>/stat` field 22 into the domain's start time
/// (milliseconds since the epoch).
pub fn start_time_ms(
    boot_time_seconds: u64,
    start_time_ticks: u64,
    ticks_per_second: u64,
) -> Result<u64, OsControlError> {
    const SOURCE: &str = "process start time reading";
    if ticks_per_second == 0 {
        return Err(unparseable_proc_reading(SOURCE));
    }
    let since_boot_ms = start_time_ticks.saturating_mul(1_000) / ticks_per_second;
    Ok(boot_time_seconds
        .saturating_mul(1_000)
        .saturating_add(since_boot_ms))
}

/// Whether a freshly observed start time identifies the same process the
/// caller's [`crate::os_control::processes::ProcessIdentity`] was captured
/// from.
///
/// # Strength of the guarantee
///
/// A match means: same PID **and** a start time within
/// [`START_TIME_MATCH_TOLERANCE_MS`]. Because PID reuse requires the PID space
/// to wrap, an unrelated process that both reused the PID *and* started within
/// one second of the original's start time is not distinguishable by this
/// check alone. Distinguishing it would require binding the executable or argv
/// digest into the identity, and
/// [`crate::os_control::processes::ProcessIdentity`] has no field to carry
/// one — it is exactly `(pid, start_time)`. This function therefore states the
/// bound it actually enforces rather than claiming a stronger one.
#[must_use]
pub fn start_time_matches(expected_ms: u64, observed_ms: u64) -> bool {
    expected_ms.abs_diff(observed_ms) <= START_TIME_MATCH_TOLERANCE_MS
}

/// Average CPU usage over the process's lifetime, as a whole percentage.
///
/// A single `procfs` sample carries cumulative CPU ticks, not an instantaneous
/// rate, so this is `cpu_ticks / elapsed_ticks` — a real reading, explicitly
/// the lifetime average rather than a momentary one. The alternative (two
/// samples separated by a sleep) would make every process listing pay a
/// sampling delay, and reporting `0` from one sample would be a fabricated
/// value.
///
/// The elapsed window is floored at one tick so a process that started within
/// the current tick cannot divide by zero.
pub fn cpu_percent_since_start(
    cpu_ticks: u64,
    start_time_ticks: u64,
    uptime_seconds: f64,
    ticks_per_second: u64,
) -> Result<u8, OsControlError> {
    const SOURCE: &str = "process cpu reading";
    if ticks_per_second == 0 || !uptime_seconds.is_finite() || uptime_seconds < 0.0 {
        return Err(unparseable_proc_reading(SOURCE));
    }
    let uptime_ticks = (uptime_seconds * ticks_per_second as f64) as u64;
    let elapsed_ticks = uptime_ticks.saturating_sub(start_time_ticks).max(1);
    let percent = cpu_ticks.saturating_mul(100) / elapsed_ticks;
    Ok(percent.min(100) as u8)
}

/// Convert a resident-set page count into bytes. A zero page size is an error
/// rather than a zero-byte reading.
pub fn memory_bytes(rss_pages: u64, page_size_bytes: u64) -> Result<u64, OsControlError> {
    const SOURCE: &str = "process memory reading";
    if page_size_bytes == 0 {
        return Err(unparseable_proc_reading(SOURCE));
    }
    Ok(rss_pages.saturating_mul(page_size_bytes))
}

/// The stable local-identity reference for a uid, as carried by
/// [`crate::os_control::processes::ProcessObservation::owner`]. Uid-derived
/// only: no user name lookup, so no directory/NSS read happens on an
/// observation path.
#[must_use]
pub fn owner_label(uid: u32) -> String {
    format!("uid:{uid}")
}

/// Whether a [`crate::os_control::processes::ProcessFilter::owner`] request
/// selects this uid. Accepts both the rendered `uid:<n>` label and a bare
/// numeric uid, so a caller echoing back an observed `owner` and a caller
/// passing a raw uid both filter correctly instead of silently matching
/// nothing.
#[must_use]
pub fn owner_matches(requested: &str, uid: u32) -> bool {
    let requested = requested.trim();
    requested == owner_label(uid) || requested == uid.to_string()
}

/// The bounded, sanitized executable label for an observation.
///
/// Built from the kernel's `comm` — the process *name*, which structurally
/// cannot contain arguments — with control characters dropped (a `comm` may
/// contain a newline) and the result bounded at
/// [`MAX_EXECUTABLE_LABEL_BYTES`]. A `comm` that sanitizes to nothing is an
/// error rather than an empty label.
pub fn executable_label(comm: &str) -> Result<String, OsControlError> {
    const SOURCE: &str = "process name reading";
    let mut label = String::new();
    for ch in comm.chars().filter(|ch| !ch.is_control()) {
        if label.len() + ch.len_utf8() > MAX_EXECUTABLE_LABEL_BYTES {
            break;
        }
        label.push(ch);
    }
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err(unparseable_proc_reading(SOURCE));
    }
    Ok(label)
}

/// The digest binding an observed executable's identity.
///
/// `/proc/<pid>/exe` is the authoritative binding but is only readable for
/// processes the caller may inspect, so the digest names which of the two
/// domains it was computed in (`process-exe:` vs `process-comm:`). An auditor
/// can therefore tell a path-bound digest from a name-bound one instead of
/// having to assume the stronger of the two.
#[must_use]
pub fn executable_digest(exe_path: Option<&str>, comm: &str) -> Digest {
    match exe_path {
        Some(path) => Digest::of_str(&format!("process-exe:{path}")),
        None => Digest::of_str(&format!("process-comm:{comm}")),
    }
}

/// Split `/proc/<pid>/cmdline`'s NUL-separated argv.
///
/// Interior empty elements are preserved (an empty argument is a real
/// argument); only the file's trailing NUL terminator is trimmed. An entirely
/// empty file fails closed with [`no_command_line_error`].
pub fn parse_proc_cmdline(raw: &str) -> Result<Vec<String>, OsControlError> {
    let trimmed = raw.trim_end_matches('\0');
    if trimmed.is_empty() {
        return Err(no_command_line_error());
    }
    Ok(trimmed.split('\0').map(str::to_string).collect())
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    /// A real `/proc/<pid>/stat` line (fields past 24 truncated for width;
    /// the parser only reads up to field 24).
    const BASH_STAT: &str = "4242 (bash) S 4200 4242 4242 34816 4300 4194304 1583 1200 0 0 \
         37 12 3 4 20 0 1 0 987654 12345678 512 18446744073709551615";

    #[test]
    fn stat_fields_are_parsed() {
        let stat = parse_proc_stat(BASH_STAT).expect("real stat line parses");
        assert_eq!(stat.pid, 4242);
        assert_eq!(stat.comm, "bash");
        assert_eq!(stat.state, ProcessLifecycleState::Sleeping);
        assert_eq!(stat.nice, 0);
        assert_eq!(stat.cpu_ticks, 49, "utime 37 + stime 12");
        assert_eq!(stat.start_time_ticks, 987_654);
        assert_eq!(stat.rss_pages, 512);
    }

    #[test]
    fn comm_with_spaces_and_parentheses_is_parsed() {
        // Both shapes occur on a real desktop: Firefox content processes are
        // `(Web Content)`, and a program is free to put a `)` in its own name.
        let raw = "9001 (Web Content) R 1 9001 9001 0 -1 4194304 10 0 0 0 \
                   900 100 0 0 15 -5 24 0 555 999 4096 18446744073709551615";
        let stat = parse_proc_stat(raw).expect("parenthesized comm parses");
        assert_eq!(stat.comm, "Web Content");
        assert_eq!(stat.state, ProcessLifecycleState::Running);
        assert_eq!(stat.nice, -5, "a negative niceness is a real reading");
        assert_eq!(stat.rss_pages, 4096);

        let nested = "7 (foo) bar) Z 1 7 7 0 -1 0 0 0 0 0 \
                      0 0 0 0 20 0 1 0 42 0 0 18446744073709551615";
        let stat = parse_proc_stat(nested).expect("nested paren splits on the last one");
        assert_eq!(stat.comm, "foo) bar");
        assert_eq!(stat.state, ProcessLifecycleState::Zombie);
    }

    #[test]
    fn unrecognised_stat_is_an_error_never_a_default() {
        // The whole point: a layout change must not silently become "nice 0,
        // start time 0", which would let a mutation verify against nothing.
        for raw in [
            "",
            "not a stat line",
            "4242 bash S 4200",                    // no parens
            "4242 (bash) S 4200 4242",             // too few fields
            "abc (bash) S 4200 4242 4242 0 0 0 0 0 0 0 0 0 20 0 1 0 42 0 0 0", // bad pid
        ] {
            assert!(
                parse_proc_stat(raw).is_err(),
                "must refuse to guess for {raw:?}"
            );
        }
    }

    #[test]
    fn lifecycle_states_normalize_onto_the_closed_set() {
        assert_eq!(parse_lifecycle_state("R"), ProcessLifecycleState::Running);
        assert_eq!(parse_lifecycle_state("S"), ProcessLifecycleState::Sleeping);
        assert_eq!(
            parse_lifecycle_state("D"),
            ProcessLifecycleState::Sleeping,
            "uninterruptible sleep is still the manifest's Sleeping"
        );
        assert_eq!(parse_lifecycle_state("T"), ProcessLifecycleState::Stopped);
        assert_eq!(parse_lifecycle_state("t"), ProcessLifecycleState::Stopped);
        assert_eq!(parse_lifecycle_state("Z"), ProcessLifecycleState::Zombie);
        // `I` (idle kernel thread) is real and outside the closed set; the
        // manifest's own `Unknown` member is the normalization, not a default.
        assert_eq!(parse_lifecycle_state("I"), ProcessLifecycleState::Unknown);
        assert_eq!(parse_lifecycle_state(""), ProcessLifecycleState::Unknown);
    }

    #[test]
    fn status_pid_name_and_real_uid_are_parsed() {
        let raw = "Name:\tWeb Content\nUmask:\t0022\nState:\tS (sleeping)\n\
                   Tgid:\t9001\nPid:\t9001\nPPid:\t1\n\
                   Uid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n\
                   VmRSS:\t   16384 kB\n";
        let status = parse_proc_status(raw).expect("real status parses");
        assert_eq!(status.pid, 9001);
        assert_eq!(status.name, "Web Content");
        assert_eq!(status.uid, 1000, "the real uid, not effective/saved/fs");
    }

    #[test]
    fn status_without_a_uid_line_is_an_error_never_an_anonymous_owner() {
        let raw = "Name:\tbash\nPid:\t4242\nState:\tS (sleeping)\n";
        assert!(parse_proc_status(raw).is_err());
        assert!(parse_proc_status("").is_err());
        assert!(parse_proc_status("Uid:\tnotanumber\nPid:\t1\nName:\tx\n").is_err());
    }

    #[test]
    fn boot_time_is_parsed_from_a_full_proc_stat() {
        let raw = "cpu  1 2 3 4 5 6 7 8 9 0\ncpu0 1 2 3 4 5 6 7 8 9 0\n\
                   intr 12345 0 0\nctxt 987654\nbtime 1739000000\nprocesses 4242\n\
                   procs_running 2\nprocs_blocked 0\n";
        assert_eq!(parse_boot_time_seconds(raw).unwrap(), 1_739_000_000);
    }

    #[test]
    fn missing_boot_time_is_an_error_never_epoch_zero() {
        assert!(parse_boot_time_seconds("cpu 1 2 3\nctxt 9\n").is_err());
        assert!(parse_boot_time_seconds("btime notanumber").is_err());
        assert!(parse_boot_time_seconds("").is_err());
    }

    #[test]
    fn uptime_takes_the_first_field() {
        assert!((parse_uptime_seconds("123456.78 987654.32\n").unwrap() - 123_456.78).abs() < 1e-6);
    }

    #[test]
    fn unrecognised_uptime_is_an_error_never_zero() {
        assert!(parse_uptime_seconds("").is_err());
        assert!(parse_uptime_seconds("up 3 days").is_err());
        assert!(parse_uptime_seconds("-1.0 2.0").is_err());
        assert!(parse_uptime_seconds("NaN 2.0").is_err());
    }

    #[test]
    fn start_time_normalizes_ticks_against_boot_time() {
        // 987654 ticks at 100 Hz = 9876.54 s after boot.
        assert_eq!(
            start_time_ms(1_739_000_000, 987_654, 100).unwrap(),
            1_739_000_000_000 + 9_876_540
        );
        // A 1000 Hz kernel is the other real USER_HZ people ship.
        assert_eq!(
            start_time_ms(1_739_000_000, 987_654, 1_000).unwrap(),
            1_739_000_000_000 + 987_654
        );
    }

    #[test]
    fn zero_clock_ticks_is_an_error_never_a_start_time() {
        assert!(start_time_ms(1_739_000_000, 987_654, 0).is_err());
    }

    #[test]
    fn start_time_match_is_bounded_by_the_documented_tolerance() {
        assert!(start_time_matches(1_739_000_000_000, 1_739_000_000_000));
        assert!(
            start_time_matches(1_739_000_000_000, 1_739_000_000_999),
            "sub-second btime quantization must not read as a different process"
        );
        assert!(!start_time_matches(1_739_000_000_000, 1_739_000_002_000));
        assert!(
            !start_time_matches(1_739_000_002_000, 1_739_000_000_000),
            "the comparison is symmetric"
        );
    }

    #[test]
    fn cpu_percent_is_the_lifetime_average() {
        // 100 Hz; started 1000 ticks after boot; uptime 101 s = 10100 ticks;
        // elapsed 9100 ticks; 910 cpu ticks = 10%.
        assert_eq!(cpu_percent_since_start(910, 1_000, 101.0, 100).unwrap(), 10);
    }

    #[test]
    fn a_just_started_process_cannot_divide_by_zero() {
        // Real edge case: the process started in the current tick, so the
        // elapsed window is zero.
        assert_eq!(cpu_percent_since_start(0, 10_100, 101.0, 100).unwrap(), 0);
        assert_eq!(cpu_percent_since_start(5, 10_100, 101.0, 100).unwrap(), 100);
    }

    #[test]
    fn unusable_cpu_inputs_are_an_error_never_zero_percent() {
        assert!(cpu_percent_since_start(910, 1_000, 101.0, 0).is_err());
        assert!(cpu_percent_since_start(910, 1_000, f64::NAN, 100).is_err());
        assert!(cpu_percent_since_start(910, 1_000, -1.0, 100).is_err());
    }

    #[test]
    fn memory_bytes_scale_by_page_size() {
        assert_eq!(memory_bytes(512, 4_096).unwrap(), 2_097_152);
        // 16 KiB pages are real on aarch64, which is why the page size is
        // read from the host rather than assumed.
        assert_eq!(memory_bytes(512, 16_384).unwrap(), 8_388_608);
        assert_eq!(memory_bytes(0, 4_096).unwrap(), 0, "a zombie holds no rss");
    }

    #[test]
    fn zero_page_size_is_an_error_never_zero_bytes() {
        assert!(memory_bytes(512, 0).is_err());
    }

    #[test]
    fn owner_label_and_filter_matching() {
        assert_eq!(owner_label(1000), "uid:1000");
        assert!(owner_matches("uid:1000", 1000));
        assert!(owner_matches(" 1000 ", 1000), "a bare uid also selects");
        assert!(!owner_matches("uid:1001", 1000));
        assert!(!owner_matches("", 1000));
        assert!(!owner_matches("root", 0), "no name lookup on a read path");
    }

    #[test]
    fn executable_label_is_bounded_and_stripped_of_control_characters() {
        assert_eq!(executable_label("bash").unwrap(), "bash");
        assert_eq!(
            executable_label("kworker/0:1-events").unwrap(),
            "kworker/0:1-events"
        );
        assert_eq!(
            executable_label("weird\nname").unwrap(),
            "weirdname",
            "a comm may contain a newline"
        );
        let long = "x".repeat(MAX_EXECUTABLE_LABEL_BYTES + 10);
        assert_eq!(
            executable_label(&long).unwrap().len(),
            MAX_EXECUTABLE_LABEL_BYTES
        );
    }

    #[test]
    fn an_empty_label_is_an_error_never_an_empty_string() {
        assert!(executable_label("").is_err());
        assert!(executable_label("\n\t").is_err());
    }

    #[test]
    fn executable_digest_names_its_binding_domain() {
        let path_bound = executable_digest(Some("/usr/bin/bash"), "bash");
        let name_bound = executable_digest(None, "bash");
        assert_ne!(
            path_bound, name_bound,
            "a name-bound digest must never collide with a path-bound one"
        );
        assert_eq!(path_bound, executable_digest(Some("/usr/bin/bash"), "bash"));
    }

    #[test]
    fn cmdline_splits_on_nul_and_keeps_empty_arguments() {
        let raw = "/usr/bin/grep\u{0}-e\u{0}some pattern\u{0}";
        assert_eq!(
            parse_proc_cmdline(raw).unwrap(),
            vec!["/usr/bin/grep", "-e", "some pattern"]
        );
        // An empty interior argument is a real argument, not padding.
        let with_empty = "/bin/sh\u{0}\u{0}-c\u{0}";
        assert_eq!(
            parse_proc_cmdline(with_empty).unwrap(),
            vec!["/bin/sh", "", "-c"]
        );
        // Some processes rewrite argv without a trailing NUL.
        assert_eq!(parse_proc_cmdline("init").unwrap(), vec!["init"]);
    }

    #[test]
    fn an_empty_cmdline_is_an_error_never_zero_arguments() {
        // Kernel threads and exited processes both look like this; claiming
        // "zero arguments" would be a fabricated reading.
        assert!(parse_proc_cmdline("").is_err());
        assert!(parse_proc_cmdline("\u{0}\u{0}").is_err());
    }
}
