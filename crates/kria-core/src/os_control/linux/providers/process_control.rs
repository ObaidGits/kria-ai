//! Live native-syscall process adapter (raw transport seam).
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-013), design §3, §9.5 (`linux/providers/process_control.rs`), plus
//! **Task 2/§5** (live provider reads).
//!
//! # Host safety
//!
//! Signaling an existing process (`kill(2)`) or renicing it (`setpriority(2)`)
//! is a **raw live syscall** against host state outside KRIA's own process.
//! Like the other `linux/providers/*` adapters, this one:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test
//!    can build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read or dispatch, so a deny-live (`os-control-test`) build that reached
//!    here would trip the sentinel and abort rather than touch a real
//!    process.
//!
//! # Why the reads do not use the governed query path
//!
//! Every read here is a `procfs` **filesystem** read — `/proc/<pid>/stat`,
//! `/proc/<pid>/status`, `/proc/<pid>/cmdline`, `/proc/stat`, `/proc/uptime`.
//! There is no tool to shell out to, so
//! [`crate::os_control::linux::structured_query::StructuredQueryRequest`] would
//! have nothing to launch: forcing a child process (`ps`, `nice`) in front of a
//! file the kernel already exposes would *add* a raw transport instead of
//! removing one. The containment the governed query path supplies is therefore
//! reproduced directly: the deny-live sentinel guards every method, each file
//! is read with an explicit byte bound (a read that hits the bound is a failed
//! read, never a partially parsed one), the caller's deadline and cancellation
//! are honoured, and every byte is parsed by a fail-closed pure function in
//! [`crate::os_control::processes::selection`]. There is no ungoverned
//! subprocess fallback anywhere in this file.
//!
//! # PID reuse
//!
//! A PID is not an identity: a process that exited and whose PID was reused is
//! a different process. Every read that is handed a
//! [`ProcessIdentity`] with a non-zero `start_time` re-derives the live start
//! time from `/proc/<pid>/stat` field 22 and compares it
//! ([`start_time_matches`]). A mismatch is never a silent match — it is
//! [`unknown_process_identity_error`] for every read that returns process
//! facts, and `Ok(false)` for [`ProcessTransport::read_alive`] only because the
//! trait contract explicitly specifies "report the *original* process absent
//! rather than conflating it". `get_process_command_metadata` re-verifies the
//! start time *after* reading the argv as well, so a reuse that lands between
//! the two reads cannot hand back another process's arguments.
//!
//! # Still unwired
//!
//! The mutation side (`libc::kill`/`libc::setpriority`) is composed by the
//! desktop startup root; until then [`ProcessTransport::send_signal`] and
//! [`ProcessTransport::set_priority`] fail closed with
//! [`OsControlError::Unavailable`]. Deny-live tests inject
//! [`crate::os_control::processes::fake::FakeProcessTransport`].

use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, CapabilityId, ProviderId, SafeOperation, SafeText,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::signal::{
    send_signal as signal_process, set_priority as set_process_priority,
    GovernedSignal,
};
use crate::os_control::receipt::AppliedDispatch;

use crate::os_control::processes::selection::{
    cpu_percent_since_start, executable_digest, executable_label, memory_bytes, owner_label,
    owner_matches, parse_boot_time_seconds, parse_proc_cmdline, parse_proc_stat, parse_proc_status,
    parse_uptime_seconds, start_time_matches, start_time_ms, ProcStatFields,
};
use crate::os_control::processes::{
    process_permission_denied_error, unknown_process_identity_error, BoundedCommandMetadata,
    ProcessFilter, ProcessIdentity, ProcessLifecycleState, ProcessObservation, ProcessPage,
    ProcessTransport, PROCESS_PROVIDER_ID,
};
use crate::os_control::receipt::ApplyOutcome;

/// The `procfs` mount point every reading comes from.
const PROC_ROOT: &str = "/proc";

/// Byte bound for one `/proc/<pid>/stat` read (a real line is ~300 bytes).
const MAX_STAT_BYTES: usize = 4 * 1024;

/// Byte bound for one `/proc/<pid>/status` read.
const MAX_STATUS_BYTES: usize = 16 * 1024;

/// Byte bound for `/proc/stat` (grows with CPU count, hence the wider bound).
const MAX_SYSTEM_STAT_BYTES: usize = 256 * 1024;

/// Byte bound for `/proc/uptime`.
const MAX_UPTIME_BYTES: usize = 256;

/// Byte bound for one `/proc/<pid>/cmdline` read. Exceeding it is a failed
/// read rather than a truncated argv, because
/// [`BoundedCommandMetadata::from_raw_argv`] digests what it is given and a
/// digest over a partially read argv would misrepresent which command line it
/// summarizes.
const MAX_CMDLINE_BYTES: usize = 128 * 1024;

/// How many PIDs a table scan walks between deadline/cancellation checks.
const BUDGET_CHECK_STRIDE: usize = 64;

/// The live native-syscall process adapter. Constructible only in a live
/// composition; a value cannot exist under `os-control-test`.
pub struct LiveProcessControl {
    _seal: (),
}

impl LiveProcessControl {
    /// Construct in a live composition root. Requires a [`LiveHostAccessToken`],
    /// so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }

}

/// The stable provider identity, used in every read error.
fn provider() -> ProviderId {
    ProviderId::new(PROCESS_PROVIDER_ID)
}

/// A fail-closed read error. `reason` is always a fixed label — never a raw OS
/// error string and never a captured line of a `procfs` file.
fn unavailable(reason: &'static str, retryable: bool) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(provider()),
        reason: SafeText::new(reason),
        retryable,
    }
}

/// Honour the caller's observation budget before (and during) a read.
fn budget(
    cancellation: &CancellationToken,
    deadline: Instant,
    operation: &'static str,
) -> Result<(), OsControlError> {
    if cancellation.is_cancelled() {
        return Err(OsControlError::CancelledBeforeMutation);
    }
    if Instant::now() >= deadline {
        return Err(OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new(operation),
            timeout_ms: 0,
        });
    }
    Ok(())
}

/// Read one `procfs` file with an explicit byte bound.
///
/// `Ok(None)` means the entry does not exist — for a per-PID file that is the
/// real observation "this process is gone", not an error. A read that reaches
/// `max_bytes` is a **failed** read: parsing a partial file would fabricate a
/// reading out of half a record.
fn read_proc_bytes(
    path: &str,
    source: &'static str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, OsControlError> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(OsControlError::Unavailable {
                provider: Some(provider()),
                reason: SafeText::new(format!("{source} could not be opened")),
                retryable: false,
            })
        }
    };
    let mut buffer = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| OsControlError::Unavailable {
            provider: Some(provider()),
            reason: SafeText::new(format!("{source} could not be read")),
            retryable: true,
        })?;
    if buffer.len() > max_bytes {
        return Err(OsControlError::Unavailable {
            provider: Some(provider()),
            reason: SafeText::new(format!(
                "{source} exceeded its bounded read; refusing a partial reading"
            )),
            retryable: false,
        });
    }
    Ok(Some(buffer))
}

/// Host constants a `procfs` reading must be normalized against:
/// `(clock ticks per second, page size in bytes)`.
#[cfg(unix)]
fn machine_constants() -> Result<(u64, u64), OsControlError> {
    // SAFETY: `sysconf` reads a static, process-independent system parameter.
    // It takes no pointers, has no side effects, and cannot fail other than by
    // returning a non-positive value, which is checked below.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    // SAFETY: as above.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if ticks <= 0 || page <= 0 {
        return Err(unavailable(
            "the host did not report its clock tick or page size; refusing to assume either",
            false,
        ));
    }
    Ok((ticks as u64, page as u64))
}

#[cfg(not(unix))]
fn machine_constants() -> Result<(u64, u64), OsControlError> {
    Err(unavailable(
        "procfs process readings require a Unix host",
        false,
    ))
}

/// The host normalization basis shared by every reading in one call.
#[derive(Debug, Clone, Copy)]
struct ProcClock {
    ticks_per_second: u64,
    page_size_bytes: u64,
    boot_time_seconds: u64,
    uptime_seconds: f64,
}

/// Read the normalization basis once per call, so every observation in a page
/// is normalized against the same boot time and uptime.
fn read_clock() -> Result<ProcClock, OsControlError> {
    let (ticks_per_second, page_size_bytes) = machine_constants()?;
    let system_stat = read_proc_bytes(
        &format!("{PROC_ROOT}/stat"),
        "kernel boot time reading",
        MAX_SYSTEM_STAT_BYTES,
    )?
    .ok_or_else(|| unavailable("procfs is not available on this host", false))?;
    let boot_time_seconds = parse_boot_time_seconds(&String::from_utf8_lossy(&system_stat))?;
    let uptime = read_proc_bytes(
        &format!("{PROC_ROOT}/uptime"),
        "system uptime reading",
        MAX_UPTIME_BYTES,
    )?
    .ok_or_else(|| unavailable("procfs is not available on this host", false))?;
    let uptime_seconds = parse_uptime_seconds(&String::from_utf8_lossy(&uptime))?;
    Ok(ProcClock {
        ticks_per_second,
        page_size_bytes,
        boot_time_seconds,
        uptime_seconds,
    })
}

/// Read and parse one `/proc/<pid>/stat`. `Ok(None)` means the process is gone
/// (or the entry the kernel returned belongs to a different PID, which means it
/// was replaced under us).
fn read_pid_stat(pid: u32) -> Result<Option<ProcStatFields>, OsControlError> {
    let Some(bytes) = read_proc_bytes(
        &format!("{PROC_ROOT}/{pid}/stat"),
        "process stat reading",
        MAX_STAT_BYTES,
    )?
    else {
        return Ok(None);
    };
    let stat = parse_proc_stat(&String::from_utf8_lossy(&bytes))?;
    if stat.pid != pid {
        return Ok(None);
    }
    Ok(Some(stat))
}

/// The normalized start time of an already-read `stat`.
fn observed_start_time_ms(clock: &ProcClock, stat: &ProcStatFields) -> Result<u64, OsControlError> {
    start_time_ms(
        clock.boot_time_seconds,
        stat.start_time_ticks,
        clock.ticks_per_second,
    )
}

/// Whether a caller-supplied identity still names the process `stat` describes.
/// A `start_time` of `0` means the caller never captured one, so there is
/// nothing to compare (see [`ProcessIdentity`]'s documented narrower
/// guarantee).
fn identity_still_matches(
    clock: &ProcClock,
    identity: ProcessIdentity,
    stat: &ProcStatFields,
) -> Result<bool, OsControlError> {
    if identity.start_time == 0 {
        return Ok(true);
    }
    Ok(start_time_matches(
        identity.start_time,
        observed_start_time_ms(clock, stat)?,
    ))
}

/// The bounded, absolute executable path of a process, when the caller is
/// allowed to resolve it. `None` for kernel threads and for processes the
/// caller may not inspect — never an error, because the digest domain records
/// which of the two was observed.
fn executable_path(pid: u32) -> Option<String> {
    std::fs::read_link(format!("{PROC_ROOT}/{pid}/exe"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

/// One observed process plus the uid the owner filter compares against.
struct ObservedProcess {
    observation: ProcessObservation,
    uid: u32,
}

/// Build one content-free observation from `procfs`.
///
/// `Ok(None)` means the process is not observable as a single coherent record:
/// it exited, or its PID was reused between the `stat` and `status` reads (the
/// two files would then describe two different processes, and merging them
/// would invent a record that never existed).
fn observe_pid(clock: &ProcClock, pid: u32) -> Result<Option<ObservedProcess>, OsControlError> {
    let Some(stat) = read_pid_stat(pid)? else {
        return Ok(None);
    };
    let Some(status_bytes) = read_proc_bytes(
        &format!("{PROC_ROOT}/{pid}/status"),
        "process status reading",
        MAX_STATUS_BYTES,
    )?
    else {
        return Ok(None);
    };
    let status = parse_proc_status(&String::from_utf8_lossy(&status_bytes))?;
    // Cross-file consistency: both files must still describe the same task.
    if status.pid != pid || status.name != stat.comm.lines().next().unwrap_or_default() {
        return Ok(None);
    }
    let start_time = observed_start_time_ms(clock, &stat)?;
    let cpu_percent = cpu_percent_since_start(
        stat.cpu_ticks,
        stat.start_time_ticks,
        clock.uptime_seconds,
        clock.ticks_per_second,
    )?;
    let memory = memory_bytes(stat.rss_pages, clock.page_size_bytes)?;
    let label = executable_label(&stat.comm)?;
    let digest = executable_digest(executable_path(pid).as_deref(), &stat.comm);
    let observation = ProcessObservation::new(
        ProcessIdentity::new(pid, start_time),
        label,
        digest,
        owner_label(status.uid),
        stat.state,
        cpu_percent,
        memory,
    );
    Ok(Some(ObservedProcess {
        observation,
        uid: status.uid,
    }))
}

/// Apply the content-free filter to one observation. `app_id` is rejected
/// before the scan starts (this provider cannot observe app association), so
/// it is not considered here.
fn matches_filter(filter: &ProcessFilter, observed: &ObservedProcess) -> bool {
    if let Some(state) = filter.state {
        if observed.observation.state != state {
            return false;
        }
    }
    if let Some(owner) = filter.owner.as_deref() {
        if !owner_matches(owner, observed.uid) {
            return false;
        }
    }
    if let Some(min_cpu) = filter.min_cpu_percent {
        if observed.observation.cpu_percent < min_cpu {
            return false;
        }
    }
    if let Some(min_memory) = filter.min_memory_bytes {
        if observed.observation.memory_bytes < min_memory {
            return false;
        }
    }
    true
}

/// Walk the whole process table once, in ascending PID order, and return the
/// requested page of matching observations.
///
/// A PID that vanishes or cannot be observed mid-scan is **omitted** rather
/// than fabricated — the same semantics `ps` has, and unavoidable for any
/// listing of a table that mutates while it is read. If *nothing* could be
/// observed while entries existed (a `hidepid` mount, or a `procfs` layout
/// change), that is reported as an error instead of an empty table.
fn scan_process_table(
    clock: &ProcClock,
    filter: &ProcessFilter,
    cancellation: &CancellationToken,
    deadline: Instant,
    cursor: usize,
    limit: usize,
) -> Result<ProcessPage, OsControlError> {
    let entries = std::fs::read_dir(PROC_ROOT)
        .map_err(|_| unavailable("the process table could not be enumerated", false))?;
    let mut pids: Vec<u32> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()))
        .collect();
    pids.sort_unstable();

    let mut matching: Vec<ProcessObservation> = Vec::new();
    let mut observed = 0usize;
    let mut unobservable = 0usize;
    for (index, pid) in pids.iter().copied().enumerate() {
        if index % BUDGET_CHECK_STRIDE == 0 {
            budget(cancellation, deadline, "list_processes")?;
        }
        match observe_pid(clock, pid) {
            Ok(Some(process)) => {
                observed += 1;
                if matches_filter(filter, &process) {
                    matching.push(process.observation);
                }
            }
            Ok(None) => {}
            Err(_) => unobservable += 1,
        }
    }
    if observed == 0 && unobservable > 0 {
        return Err(unavailable(
            "no process in the table could be observed; refusing to report an empty process table",
            true,
        ));
    }

    let start = cursor.min(matching.len());
    let end = start.saturating_add(limit).min(matching.len());
    Ok(ProcessPage {
        items: matching[start..end].to_vec(),
        truncated: end < matching.len(),
    })
}

#[async_trait::async_trait]
impl ProcessTransport for LiveProcessControl {
    fn provider_id(&self) -> ProviderId {
        provider()
    }

    async fn read_alive(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<bool, OsControlError> {
        deny_live_transport(RawTransportKind::ProcessSignal);
        budget(&ctx.cancellation, ctx.deadline, "kill_process")?;

        let Some(stat) = read_pid_stat(identity.pid)? else {
            return Ok(false);
        };
        // A zombie has already terminated and is only awaiting a reap, so
        // reporting it alive would let a completed termination never verify.
        if stat.state == ProcessLifecycleState::Zombie {
            return Ok(false);
        }
        // PID reuse: the trait contract requires the *original* process be
        // reported absent rather than conflated with whatever reused its PID.
        if !identity_still_matches(&read_clock()?, identity, &stat)? {
            return Ok(false);
        }
        Ok(true)
    }

    async fn read_priority(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<i32, OsControlError> {
        deny_live_transport(RawTransportKind::ProcessSignal);
        budget(&ctx.cancellation, ctx.deadline, "set_process_priority")?;

        let Some(stat) = read_pid_stat(identity.pid)? else {
            return Err(unknown_process_identity_error());
        };
        // Returning an unrelated process's niceness would let a priority
        // mutation verify against the wrong process, so a reuse is an error.
        if !identity_still_matches(&read_clock()?, identity, &stat)? {
            return Err(unknown_process_identity_error());
        }
        Ok(stat.nice)
    }

    async fn send_signal(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        force: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The identity pairs pid with start time, and the transport re-verifies it
        // immediately before the syscall: a reused pid is refused, never signalled.
        let signal = if force {
            GovernedSignal::Kill
        } else {
            GovernedSignal::Term
        };
        signal_process(identity, signal)?;
        // A delivered signal is not a completed exit — SIGTERM may be caught and
        // ignored — so the effect is Applied and the verifier decides whether the
        // process actually went away.
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            BoundedVec::new(),
        )))
    }

    async fn set_priority(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        nice: i32,
    ) -> Result<ApplyOutcome, OsControlError> {
        set_process_priority(identity, nice)?;
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            BoundedVec::new(),
        )))
    }

    async fn list_observations(
        &self,
        ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError> {
        deny_live_transport(RawTransportKind::ProcessSignal);

        // `procfs` carries no application association. Ignoring the filter
        // would report unrelated processes as if they matched, so refuse.
        if filter.app_id.is_some() {
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new("list_processes.app_id"),
                reason: SafeText::new(
                    "this backend observes procfs, which exposes no application association; \
                     returning unfiltered processes for an app_id query would be a false observation",
                ),
            });
        }
        budget(&ctx.cancellation, ctx.deadline, "list_processes")?;

        let clock = read_clock()?;
        // A whole-table walk is thousands of small synchronous file reads;
        // keep it off the async worker rather than stalling the runtime.
        let filter = filter.clone();
        let cancellation = ctx.cancellation.clone();
        let deadline = ctx.deadline;
        tokio::task::spawn_blocking(move || {
            scan_process_table(&clock, &filter, &cancellation, deadline, cursor, limit)
        })
        .await
        .map_err(|_| unavailable("the process table scan did not complete", true))?
    }

    async fn read_observation(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError> {
        deny_live_transport(RawTransportKind::ProcessSignal);
        budget(&ctx.cancellation, ctx.deadline, "get_process_info")?;

        let clock = read_clock()?;
        let Some(process) = observe_pid(&clock, identity.pid)? else {
            return Err(unknown_process_identity_error());
        };
        if identity.start_time != 0
            && !start_time_matches(identity.start_time, process.observation.start_time_ms)
        {
            return Err(unknown_process_identity_error());
        }
        Ok(process.observation)
    }

    async fn read_command_metadata(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError> {
        deny_live_transport(RawTransportKind::ProcessSignal);

        // Provider policy: the RED, mandatory-approval read requires an
        // admitted purpose. An absent one fails closed as a permission denial
        // rather than being downgraded to "metadata unavailable".
        if purpose.trim().is_empty() {
            return Err(process_permission_denied_error());
        }
        budget(
            &ctx.cancellation,
            ctx.deadline,
            "get_process_command_metadata",
        )?;

        let clock = read_clock()?;
        let Some(before) = read_pid_stat(identity.pid)? else {
            return Err(unknown_process_identity_error());
        };
        if !identity_still_matches(&clock, identity, &before)? {
            return Err(unknown_process_identity_error());
        }

        let Some(bytes) = read_proc_bytes(
            &format!("{PROC_ROOT}/{}/cmdline", identity.pid),
            "process command line reading",
            MAX_CMDLINE_BYTES,
        )?
        else {
            return Err(unknown_process_identity_error());
        };
        // Arguments are the payload of this read, so a lossy decode would
        // hand back arguments the process does not actually have.
        let raw = String::from_utf8(bytes).map_err(|_| {
            unavailable(
                "the command line is not valid UTF-8; refusing a lossy reading of process arguments",
                false,
            )
        })?;
        let argv = parse_proc_cmdline(&raw)?;

        // Re-verify after the read: a PID reused between the two reads would
        // otherwise return a different process's arguments under this identity.
        let Some(after) = read_pid_stat(identity.pid)? else {
            return Err(unknown_process_identity_error());
        };
        if after.start_time_ticks != before.start_time_ticks {
            return Err(unknown_process_identity_error());
        }

        Ok(BoundedCommandMetadata::from_raw_argv(
            executable_digest(executable_path(identity.pid).as_deref(), &before.comm),
            &argv,
        ))
    }
}
