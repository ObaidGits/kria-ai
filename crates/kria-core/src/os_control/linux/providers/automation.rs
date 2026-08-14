//! Live cron/systemd-timer adapter (raw transport seam).
//!
//! linux-os-control-production **Task 2.5** (`list_scheduled_tasks`) and
//! **Task 4.5** (`modify_scheduled_task`) — OSC-027, design §3, §9.13.
//!
//! Every host contact here is a [`StructuredQueryRequest`] (reads) or a
//! [`StructuredCommandRequest`] dispatch (mutations): trusted executable, exact
//! argv, hermetic environment, bounded output, deadline and cancellation. There
//! is no `Command`, no shell and no ungoverned fallback anywhere in this file,
//! and [`deny_live_transport`] runs before each one so no completion test can
//! reach a live `systemctl`/`crontab`/`stat`.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::automation::{
    selection, AutomationBackend, AutomationListing, AutomationTaskState, AutomationTransport,
    UnitEnablement, AUTOMATION_PROVIDER_ID,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The live cron/systemd-timer adapter. Constructible only in a live
/// composition; a value cannot exist under `os-control-test`.
pub struct LiveAutomation {
    backend: AutomationBackend,
    _seal: (),
}

impl LiveAutomation {
    /// Construct in a live composition root. Requires a [`LiveHostAccessToken`].
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self {
            backend: AutomationBackend::SystemdUserTimers,
            _seal: (),
        }
    }

    /// Construct for an explicitly selected backend.
    #[must_use]
    pub fn with_backend(_token: &LiveHostAccessToken, backend: AutomationBackend) -> Self {
        Self {
            backend,
            _seal: (),
        }
    }

    /// Run one governed observation and return its bounded stdout.
    ///
    /// A truncated observation is refused rather than parsed: half of a
    /// property list is not a smaller fact, it is an unread one.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        executable: TrustedExecutable,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            executable,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id_value()),
                reason: SafeText::new(
                    "automation state output was truncated; refusing a partial read",
                ),
                retryable: true,
            });
        }
        Ok(output.stdout)
    }

    fn provider_id_value(&self) -> ProviderId {
        ProviderId::new(format!("{AUTOMATION_PROVIDER_ID}-{}", self.backend.as_str()))
    }
}

#[async_trait::async_trait]
impl AutomationTransport for LiveAutomation {
    fn provider_id(&self) -> ProviderId {
        self.provider_id_value()
    }

    fn selected_backend(&self) -> AutomationBackend {
        self.backend
    }

    async fn read_listing(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<AutomationListing, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        Err(OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new(
                "live crontab/systemctl listing transport is not yet wired; no ungoverned fallback exists",
            ),
            retryable: false,
        })
    }

    async fn read_task(
        &self,
        ctx: &HostExecutionContext,
        unit: &str,
    ) -> Result<AutomationTaskState, OsControlError> {
        if !self.backend.supports_modification() {
            // A crontab entry has no stable identity, so there is nothing to
            // read *as a task*. Refusing beats returning a task-shaped guess.
            return Err(OsControlError::Unsupported {
                capability: CapabilityId::new("modify_scheduled_task"),
                reason: SafeText::new(
                    "the crontab backend has no stable per-entry identity to observe",
                ),
            });
        }

        let argv = selection::show_timer_argv(unit)?;
        let stdout = self
            .query(
                ctx,
                "modify_scheduled_task",
                self.backend.trusted_executable()?,
                argv,
            )
            .await?;
        let show = selection::parse_timer_show(&stdout)?;

        if !show.present {
            // The unambiguous absent fact.
            return Ok(AutomationTaskState {
                present: false,
                enabled: None,
                revision: None,
                next_run_ms: None,
            });
        }

        let enabled = match show.enablement {
            Some(UnitEnablement::Enabled) => Some(true),
            Some(UnitEnablement::Disabled) => Some(false),
            // A static/generated unit has no enablement state, and a unit whose
            // state we could not read has none either — both are "unknown", and
            // the domain refuses a compare-and-set against unknown.
            Some(UnitEnablement::NotApplicable) | None => None,
        };

        // The configuration revision is the unit fragment's modification time in
        // milliseconds: it changes on every configuration write and is readable
        // without introducing a second source of truth. A unit with no fragment
        // path has no readable revision, which fails the compare-and-set closed.
        let revision = match show.fragment_path.as_deref() {
            None => None,
            Some(path) => {
                let argv = selection::fragment_mtime_argv(path)?;
                let stdout = self
                    .query(
                        ctx,
                        "modify_scheduled_task",
                        selection::stat_executable()?,
                        argv,
                    )
                    .await?;
                Some(selection::parse_fragment_mtime_ms(&stdout)?)
            }
        };

        Ok(AutomationTaskState {
            present: true,
            enabled,
            revision,
            next_run_ms: show.next_run_ms,
        })
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The governed request's own launch trips the deny-live sentinel; keep an
        // explicit guard here too so the adapter is unreachable under test.
        deny_live_transport(RawTransportKind::Process);
        request.dispatch().await
    }
}
