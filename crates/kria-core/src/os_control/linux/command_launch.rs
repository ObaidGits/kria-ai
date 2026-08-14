//! The governed child-process launcher — the **single** place in KRIA where a
//! mutating OS command is actually executed.
//!
//! linux-os-control-production design §4 and §8; OSC-005, OSC-007, OSC-016.
//!
//! Everything about *what* to run was already decided and sealed by
//! [`StructuredCommandRequest`]: the trusted executable, the exact argv and its
//! digest, the allowlisted environment, the fixed locale, the output bounds, the
//! deadline and the cancellation token. This module only *runs* it, and its whole
//! job is to preserve one invariant while doing so:
//!
//! # The moment of no return
//!
//! Every [`OsControlError`] variant is named `…BeforeMutation` on purpose: an
//! `Err` from this module is a claim that **the effect provably did not happen**.
//! `spawn()` returning `Ok` is the moment that claim stops being available —
//! after it, the child may already have changed the system.
//!
//! So the launcher is split in two halves:
//!
//! * **Before spawn succeeds** — any problem returns `Err(…BeforeMutation)`.
//!   Nothing ran; the caller may safely report "not applied".
//! * **After spawn succeeds** — a timeout, a cancellation or a lost pipe can
//!   never return `Err`. They return `Ok(ApplyOutcome::Uncertain)`, because the
//!   truth is unknown and the verifier must re-observe the system to settle it.
//!   The launcher never re-runs a mutator to "retry" (OSC-005).
//!
//! A non-zero exit is *also* `Uncertain`, not an error: a mutator that reports
//! failure may still have partially changed something, so only a fresh
//! observation may conclude otherwise.
//!
//! # Containment
//!
//! * `env_clear()` first, then only the validated allowlist — an inherited
//!   `LD_PRELOAD` or `PATH` could otherwise redirect a trusted executable to
//!   attacker-controlled code.
//! * `stdin` is `/dev/null`, so a child can never block waiting for input or
//!   consume the parent's console.
//! * Output is bounded **while streaming**, not after: a runaway child cannot
//!   exhaust memory even if it writes forever.
//! * The child gets its own process group, so a deadline kill reaches the whole
//!   tree rather than leaking orphaned grandchildren.
//! * `kill_on_drop` guarantees no child survives an early return.

use std::process::Stdio;
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::os_control::access::{deny_live_transport, RawTransportKind};
use crate::os_control::contract::{
    BoundedVec, ProviderId, SafeErrorCode, SafeOperation, SafeText, SafeWarning,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    classify_post_dispatch, PostDispatchInterruption, StructuredCommandRequest,
};
use crate::os_control::receipt::{
    ApplyOutcome, AppliedDispatch, UncertainDispatch, UncertainEffectCause,
};

/// The sealed inputs `run_child` needs, so it can serve a mutation request and a
/// read request without knowing which it is holding.
pub(crate) struct ChildSpec<'a> {
    /// Trusted absolute executable path.
    pub program: &'a str,
    /// Exact sealed argv (excluding the program).
    pub args: &'a [String],
    /// Validated environment allowlist.
    pub env: &'a std::collections::BTreeMap<String, String>,
    /// Pinned locale, so output parses deterministically.
    pub locale: &'a str,
    /// Byte bound, applied while streaming.
    pub max_output_bytes: usize,
    /// Absolute deadline.
    pub deadline: Instant,
    /// Cooperative cancellation.
    pub cancellation: &'a CancellationToken,
    /// Applies the caller's *line* bound; the byte bound is applied while
    /// streaming.
    pub bound: &'a (dyn Fn(&str) -> (String, bool) + Sync),
    /// An optional payload written to the child's stdin and then closed. Used for
    /// user content that must not appear in argv (see
    /// [`crate::os_control::linux::structured_command::SecretStdin`]).
    pub stdin: Option<&'a [u8]>,
}

/// What a finished child produced, bounded and lossy-decoded.
pub(crate) struct CapturedChild {
    /// Bounded stdout.
    pub stdout: String,
    /// Bounded stderr.
    pub stderr: String,
    /// Whether either stream exceeded its bound.
    pub truncated: bool,
    /// Whether the child exited zero.
    pub exit_ok: bool,
    /// Whether the exit status could not be read at all.
    pub status_lost: bool,
}

/// Why a child never started. Every variant means the effect provably did not
/// happen, so callers may safely map these to `…BeforeMutation` errors.
pub(crate) enum NotStarted {
    /// Already cancelled at entry.
    Cancelled,
    /// The deadline had already elapsed at entry.
    DeadlineElapsed,
    /// `spawn()` failed: missing, not executable, or fork failure.
    SpawnFailed,
}

/// The outcome of trying to run a child.
pub(crate) enum ChildRun {
    /// Never started; provably no effect.
    NotStarted(NotStarted),
    /// Started, then interrupted; the effect is unknown.
    Interrupted(PostDispatchInterruption),
    /// Ran to completion.
    Finished(CapturedChild),
}

/// Spawn and supervise one child under the caller's sealed bounds.
///
/// This is the **only** place in KRIA that spawns a process, shared by the
/// mutation launcher and the read/query path so both inherit the same
/// containment: hermetic environment, null stdin, bounded output, own process
/// group, deadline, and cancellation.
pub(crate) async fn run_child(spec: &ChildSpec<'_>) -> ChildRun {
    // A deny-live (`os-control-test`) build must never reach a real process.
    deny_live_transport(RawTransportKind::Process);

    // ── Pre-spawn gates: nothing has run, so each refusal is provably no-effect.
    if spec.cancellation.is_cancelled() {
        return ChildRun::NotStarted(NotStarted::Cancelled);
    }
    if spec.deadline <= Instant::now() {
        return ChildRun::NotStarted(NotStarted::DeadlineElapsed);
    }

    let mut command = Command::new(spec.program);
    command.args(spec.args);
    // Hermetic environment: inherit nothing, then re-add only what the policy
    // validated. An inherited LD_PRELOAD or PATH could otherwise redirect a
    // trusted executable to attacker-controlled code.
    command.env_clear();
    for (key, value) in spec.env {
        command.env(key, value);
    }
    command.env("LC_ALL", spec.locale);
    command
        // A child gets a pipe only when there is a payload for it; otherwise
        // stdin is /dev/null so it can never block waiting for input.
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // No child may outlive this future.
        .kill_on_drop(true);
    // Own process group, so a deadline kill reaches the whole tree instead of
    // leaking orphaned grandchildren.
    #[cfg(unix)]
    command.process_group(0);

    // ── The moment of no return ─────────────────────────────────────────────
    let Ok(mut child) = command.spawn() else {
        return ChildRun::NotStarted(NotStarted::SpawnFailed);
    };

    // Write the payload and close the pipe, so the child sees EOF and exits
    // rather than waiting forever. A write failure is not fatal here: the child
    // has already started, so the effect is decided by the exit status below.
    if let Some(payload) = spec.stdin {
        if let Some(mut sink) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = sink.write_all(payload).await;
            let _ = sink.shutdown().await;
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let max_bytes = spec.max_output_bytes;

    let collect = async {
        // Drain both pipes while waiting: a child writing more than one pipe
        // buffer would otherwise deadlock against a full pipe.
        let (out, err) = tokio::join!(
            read_bounded(stdout, max_bytes),
            read_bounded(stderr, max_bytes)
        );
        let status = child.wait().await;
        (out, err, status)
    };

    let ((raw_stdout, out_cut), (raw_stderr, err_cut), status) = tokio::select! {
        biased;

        // Cancellation and the deadline are checked before the completion arm so
        // a child finishing in the same instant as its deadline is not silently
        // preferred over the interruption the caller asked for.
        () = spec.cancellation.cancelled() => {
            return ChildRun::Interrupted(PostDispatchInterruption::Cancelled);
        }
        () = tokio::time::sleep_until(spec.deadline.into()) => {
            return ChildRun::Interrupted(PostDispatchInterruption::TimedOut);
        }
        result = collect => result,
    };

    // Child output is untrusted bytes; never assume valid UTF-8.
    let stdout_text = String::from_utf8_lossy(&raw_stdout).into_owned();
    let stderr_text = String::from_utf8_lossy(&raw_stderr).into_owned();
    let (stdout_bounded, stdout_lines_cut) = (spec.bound)(&stdout_text);
    let (stderr_bounded, stderr_lines_cut) = (spec.bound)(&stderr_text);

    ChildRun::Finished(CapturedChild {
        stdout: stdout_bounded,
        stderr: stderr_bounded,
        truncated: out_cut || err_cut || stdout_lines_cut || stderr_lines_cut,
        exit_ok: status.as_ref().is_ok_and(std::process::ExitStatus::success),
        status_lost: status.is_err(),
    })
}

/// Keep at most `max_bytes` from a child stream, reporting whether more arrived.
///
/// The stream is drained **to EOF** even after the cap is reached, and the excess
/// is discarded. Simply stopping the read would close the pipe and kill a chatty
/// child with `SIGPIPE`, which would then be reported as a failed mutation purely
/// because it talked too much. Draining keeps memory bounded (`max_bytes` plus one
/// chunk) *and* keeps the exit status honest.
async fn read_bounded<R>(reader: Option<R>, max_bytes: usize) -> (Vec<u8>, bool)
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            // EOF.
            Ok(0) => break,
            Ok(read) => {
                let room = max_bytes.saturating_sub(kept.len());
                if room == 0 {
                    truncated = true;
                    continue;
                }
                let take = room.min(read);
                kept.extend_from_slice(&chunk[..take]);
                if take < read {
                    truncated = true;
                }
            }
            // A broken pipe is not fatal — whatever was captured still stands.
            Err(_) => break,
        }
    }
    (kept, truncated)
}

/// Run the sealed request's mutating command.
///
/// See the module docs for the before/after-spawn contract; it is the reason this
/// function returns `Ok(Uncertain)` where a naive implementation would return an
/// error.
pub(crate) async fn launch(
    request: &StructuredCommandRequest,
) -> Result<ApplyOutcome, OsControlError> {
    let bound = |text: &str| request.enforce_output_bounds(text);
    let spec = ChildSpec {
        program: request.executable().path(),
        args: request.args(),
        env: request.env(),
        locale: request.locale(),
        max_output_bytes: request.max_output_bytes(),
        deadline: request.deadline(),
        cancellation: request.cancellation(),
        bound: &bound,
        stdin: request.stdin().map(
            crate::os_control::linux::structured_command::SecretStdin::expose,
        ),
    };

    match run_child(&spec).await {
        // ── Nothing ran: `Err` is safe, because no effect is possible. ───────
        ChildRun::NotStarted(NotStarted::Cancelled) => {
            Err(OsControlError::CancelledBeforeMutation)
        }
        ChildRun::NotStarted(NotStarted::DeadlineElapsed) => {
            Err(OsControlError::TimedOutBeforeMutation {
                operation: SafeOperation::new(request.action()),
                timeout_ms: 0,
            })
        }
        ChildRun::NotStarted(NotStarted::SpawnFailed) => {
            // Missing, not executable, or fork failure. The label never carries
            // the raw OS string.
            Err(OsControlError::ProtocolBeforeMutation {
                provider: ProviderId::new(request.executable().safe_label().as_str()),
                operation: SafeOperation::new(request.action()),
            })
        }

        // ── Started then interrupted: `Err` is now forbidden. ────────────────
        ChildRun::Interrupted(interruption) => Ok(classify_post_dispatch(interruption)),

        ChildRun::Finished(captured) => {
            let mut warnings: BoundedVec<SafeWarning> = BoundedVec::new();
            if captured.truncated {
                // Surfaced so a verifier never treats bounded output as complete.
                let _ = warnings.try_push(SafeWarning {
                    code: SafeErrorCode::from_static("output_truncated"),
                    detail: Some(SafeText::new(
                        "command output exceeded its bound and was truncated",
                    )),
                });
            }

            if captured.status_lost {
                // The child was lost before its status could be read.
                return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                    None,
                    UncertainEffectCause::TransportLostAfterDispatch,
                    warnings,
                )));
            }
            if captured.exit_ok {
                return Ok(ApplyOutcome::Applied(AppliedDispatch::new(None, warnings)));
            }
            // The mutator ran and reported failure. That is NOT proof of no
            // effect — it may have changed part of the system before failing — so
            // the honest answer is uncertain and the verifier decides.
            Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::ProviderReportedFailureAfterDispatch,
                warnings,
            )))
        }
    }
}
