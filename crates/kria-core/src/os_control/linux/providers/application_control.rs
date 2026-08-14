//! Live application-control adapter (raw transport seam).
//!
//! linux-os-control-production **Task 2.5** (OSC-013), design §3, §9.3.
//!
//! # Host safety
//!
//! Constructible only with a [`LiveHostAccessToken`] (mintable solely in a live
//! composition root under `os-control-live`), and every method calls
//! [`deny_live_transport`] **before** touching the host, so a deny-live
//! (`os-control-test`) build that reached here would trip the sentinel and
//! abort. Deny-live tests inject
//! [`crate::os_control::applications::fake::FakeApplicationCloseTransport`].
//!
//! The two directions use different raw transports, and each declares the one
//! it actually opens:
//!
//! * the **observation** launches a governed `ps` child process
//!   ([`RawTransportKind::Process`]), through [`StructuredQueryRequest`] — a
//!   trusted absolute executable, an exact digested argv, a hermetic
//!   environment, a pinned `C` locale, bounded output, a deadline and
//!   cancellation;
//! * the **mutation** signals an existing process
//!   ([`RawTransportKind::ProcessSignal`]), sharing the `kill(2)` boundary with
//!   [`crate::os_control::linux::providers::process_control`].
//!
//! # Identity is a stable id, never a window title
//!
//! Matching runs over the kernel `comm` name and the `/proc/<pid>/exe`
//! basename — see [`crate::os_control::applications::selection`] for why both
//! are needed and why `ps -o args=` is never requested. A human-visible window
//! title is never used: it is neither unique nor stable, so matching on it
//! could send `SIGTERM` to an unrelated process.
//!
//! # Not running is a different fact from could not be determined
//!
//! `ps -e` lists every process and exits zero even when nothing matches, so a
//! successful listing with no match is a truthful `0`. Every failure — missing
//! tool, non-zero exit, truncated output, unparseable format — returns an
//! [`OsControlError`] instead, because reporting "zero processes alive" from a
//! failed read would make the governed pipeline treat an untouched application
//! as already closed.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::applications::selection::{
    count_matching_processes, matching_pids, parse_pid_comm_rows, parse_pid_exe_rows, query_process_executables_argv, query_process_names_argv, trusted_ps_executable, validate_matchable_name,
};
use crate::os_control::applications::{ApplicationCloseTransport, APPLICATION_CLOSE_PROVIDER_ID};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, CapabilityId, ProviderId, SafeErrorCode, SafeText, SafeWarning};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::signal::{
    read_start_time, send_signal as signal_process,
    GovernedSignal,
};
use crate::os_control::processes::ProcessIdentity;
use crate::os_control::receipt::AppliedDispatch;

use crate::os_control::linux::structured_command::{CommandPlan, CommandPolicy};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The canonical action the observation is taken for.
const OBSERVE_ACTION: &str = "graceful_close_application.observe";

/// The live application-control adapter. Constructible only in a live
/// composition; a value cannot exist under `os-control-test`.
pub struct LiveApplicationControl {
    _seal: (),
}

impl LiveApplicationControl {
    /// Construct in a live composition root. Requires a [`LiveHostAccessToken`].
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }


    /// Run one governed observation and return its bounded stdout.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(OBSERVE_ACTION),
            OBSERVE_ACTION,
            serde_json::Value::Null,
            trusted_ps_executable()?,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A partial process listing under-counts, which would report a
            // running application as already closed.
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(APPLICATION_CLOSE_PROVIDER_ID)),
                reason: SafeText::new(
                    "process listing was truncated; refusing a partial read",
                ),
                retryable: true,
            });
        }
        Ok(output.stdout)
    }
}

#[async_trait::async_trait]
impl ApplicationCloseTransport for LiveApplicationControl {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(APPLICATION_CLOSE_PROVIDER_ID)
    }

    async fn count_matching_alive(
        &self,
        ctx: &HostExecutionContext,
        name: &str,
    ) -> Result<u32, OsControlError> {
        // The observation launches a child process, not a signal syscall.
        deny_live_transport(RawTransportKind::Process);

        let name = validate_matchable_name(name)?;

        // Both identity fields are read: `comm` is truncated to 15 bytes by the
        // kernel, and `exe` is unreadable for some processes. Matches are then
        // counted per distinct pid, so a process matching both is counted once.
        let comm_rows = parse_pid_comm_rows(&self.query(ctx, query_process_names_argv()).await?)?;
        let exe_rows =
            parse_pid_exe_rows(&self.query(ctx, query_process_executables_argv()).await?)?;

        Ok(count_matching_processes(&name, &comm_rows, &exe_rows))
    }

    async fn terminate_matching(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        name: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        let name = validate_matchable_name(name)?;
        let observation = ctx.observation();

        // Resolve matches HERE, immediately before signalling, rather than reusing
        // an earlier listing: a process that started or exited in between would
        // otherwise be signalled or missed. Both identity fields are read because
        // `comm` is truncated to 15 bytes and `exe` is unreadable for some
        // processes.
        let comm_rows =
            parse_pid_comm_rows(&self.query(observation, query_process_names_argv()).await?)?;
        let exe_rows = parse_pid_exe_rows(
            &self
                .query(observation, query_process_executables_argv())
                .await?,
        )?;
        let pids = matching_pids(&name, &comm_rows, &exe_rows);

        if pids.is_empty() {
            // Nothing to close is not a failure; the desired state already holds.
            return Ok(ApplyOutcome::Applied(AppliedDispatch::new(
                None,
                BoundedVec::new(),
            )));
        }

        // Signal each match by verified identity. A pid whose start time no longer
        // matches has already exited and been replaced — skipping it is correct,
        // and signalling it would hit an unrelated process.
        let mut delivered = 0usize;
        let mut refused = 0usize;
        for pid in pids {
            let Ok(start_time) = read_start_time(pid) else {
                // Exited between resolution and signalling: already closed.
                continue;
            };
            match signal_process(ProcessIdentity::new(pid, start_time), GovernedSignal::Term) {
                Ok(()) => delivered += 1,
                // Our own process group and pid 1 are refused by the transport.
                Err(OsControlError::PolicyDenied { .. }) => refused += 1,
                Err(OsControlError::TargetChanged) => {}
                Err(error) => return Err(error),
            }
        }

        let mut warnings: BoundedVec<SafeWarning> = BoundedVec::new();
        if refused > 0 {
            let _ = warnings.try_push(SafeWarning {
                code: SafeErrorCode::from_static("protected_process_skipped"),
                detail: Some(SafeText::new(
                    "one or more matching processes were protected and not signalled",
                )),
            });
        }
        if delivered == 0 && refused > 0 {
            // Every match was protected, so nothing was closed. Reporting success
            // would tell the user an application closed when it did not.
            return Err(OsControlError::PolicyDenied {
                reason: SafeText::new(
                    "every matching process is protected from being signalled",
                ),
            });
        }
        Ok(ApplyOutcome::Applied(AppliedDispatch::new(None, warnings)))
    }
}
