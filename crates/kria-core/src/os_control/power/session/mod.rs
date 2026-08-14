//! Power domain: the session/lifecycle slice of `PowerControl` — lock,
//! suspend, hibernate, shutdown, and reboot (design §3, §9.7).
//!
//! linux-os-control-production **Task 2.4** — "Migrate lock, suspend,
//! hibernate, shutdown and reboot" (OSC-004, OSC-005, OSC-020).
//!
//! This module replaces the `sh -c`, direct `tokio::process::Command`, and
//! `vm_dispatch_command_with_sudo` handling that used to live in
//! `tools/power.rs` for `lock_screen`, `sleep`, `hibernate`,
//! `shutdown_system`, and `reboot_system`. It composes the F1 runtime,
//! mirroring [`super::PowerControl`] (the profile slice) and
//! `os_control::connectivity`'s shape:
//!
//! * [`PowerSessionState`] is a normalized observation
//!   ([`NormalizedObservation`]). Its `Lock` variant binds the observable
//!   `LockedHint` state so `lock_screen` idempotency/verification are real;
//!   its `Running` / `SessionEndingRequested` variants give suspend/
//!   hibernate/shutdown/reboot a well-defined (always-distinct) desired-vs-
//!   observed pair, because there is no meaningful "already suspended" state
//!   for a system that is, by definition, currently running.
//! * [`PowerSessionControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle (observe → apply → verify → rollback) for all five operations.
//!   Its `apply` builds a governed [`StructuredCommandRequest`] from the
//!   borrowed [`AdmittedMutationContext`] — the only sanctioned path to a
//!   child process — so no session code touches `ExecWrapper`/
//!   `tokio::process`/`vm_dispatch_command_with_sudo` directly.
//! * The live transport ([`crate::os_control::linux::providers::logind`]) is a
//!   raw, deny-live-gated adapter; deny-live tests inject
//!   [`fake::FakePowerSessionTransport`].
//!
//! # Accepted semantics (OSC-005, OSC-020)
//!
//! Suspend, hibernate, shutdown, and reboot are session-ending or
//! observability-interrupting: this module never claims `Verified` for them.
//! The *transport* (live or fake) is the sole source of the
//! [`crate::os_control::receipt::AcceptedDispatch`] acceptance evidence — the
//! provider here only forwards the governed dispatch and, if a transport ever
//! (incorrectly) returned an `Applied`/`Uncertain` fact for one of these four
//! operations, [`PowerSessionControl::verify`] still refuses to fabricate a
//! satisfying verification for them (it reports
//! [`VerificationReport::Inconclusive`] instead).
//!
//! # No rollback claim (OSC-006, design §13.1)
//!
//! The frozen manifest declares `rollbackClaim: "None"` for all five
//! operations in this slice — including `lock_screen` (screen lock is a
//! privacy/session-security action KRIA does not programmatically reverse).
//! [`PowerSessionControl::rollback`] therefore never dispatches a compensating
//! command; it always reports the same unobservable-uncertain fact.
//!
//! # Hibernate availability (OSC-020)
//!
//! Hibernate is often unsupported (no swap, disabled in firmware, …).
//! [`PowerSessionTransport::hibernate_available`] is a capability probe the
//! transport surfaces (scripted in fakes; a real capability query — never a
//! literal swap-presence check — on the live adapter). When it reports
//! unavailable, [`PowerSessionControl::apply`] fails **before** dispatch with
//! [`OsControlError::Unsupported`] — never a fabricated acceptance.
//!
//! # Delayed shutdown (Task 3.8 scope)
//!
//! `shutdown_system`'s `delay_minutes` parameter is accepted and threaded
//! through the canonical request/action parameters, but this slice always
//! dispatches an **immediate** `loginctl poweroff`/logind `PowerOff` call.
//! KRIA-owned cancellable delayed-shutdown scheduling and
//! `cancel_scheduled_shutdown` are explicitly **Task 3.8's** job (design
//! §9.7); this task does not build a scheduler.

/// Deny-live scriptable session transport for the completion suites.
#[cfg(feature = "os-control-test")]
pub mod fake;

use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest,
};
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

pub mod selection;


pub use selection::PowerSessionBackend;

/// A normalized power-session observation (design §5, §9.7).
///
/// `Lock` is the only variant with real observable state (`LockedHint`).
/// `Running` / `SessionEndingRequested` exist so suspend/hibernate/shutdown/
/// reboot have a well-typed desired-vs-observed pair without inventing a
/// fictitious "already suspended" state: their digests are always distinct,
/// so idempotency never reports a zero-dispatch skip for a session-ending
/// action, and verification never claims a satisfying observation for them.
#[derive(Debug, Clone, PartialEq)]
pub enum PowerSessionState {
    /// The observed/desired screen-lock state (`org.freedesktop.login1.Session`
    /// `LockedHint` property).
    Lock {
        /// Whether the session is currently locked.
        locked: bool,
    },
    /// Baseline "the session is currently running" observation used as the
    /// pre-apply state for suspend/hibernate/shutdown/reboot.
    Running,
    /// The desired end state for a session-ending action, named by its
    /// canonical action so distinct operations never collide.
    SessionEndingRequested {
        /// The canonical action name (`sleep`, `hibernate`, `shutdown_system`,
        /// `reboot_system`).
        action: String,
    },
    /// Whether one specific pending shutdown schedule is still present
    /// (`cancel_scheduled_shutdown`, Task 3.8).
    ///
    /// Keyed by the *requested* schedule identity, so cancelling schedule `A`
    /// is never satisfied — nor triggered — by the presence of an unrelated
    /// schedule `B`. When the requested schedule is not pending, the desired and
    /// observed digests match and the runtime reports `Unchanged`: "already in
    /// the desired state" rather than a failure.
    ShutdownSchedule {
        /// The schedule identity this action targets.
        schedule_id: String,
        /// Whether that exact schedule is currently pending.
        pending: bool,
    },
}

impl PowerSessionState {
    /// Construct a lock-state observation.
    #[must_use]
    pub fn locked(locked: bool) -> Self {
        Self::Lock { locked }
    }

    /// Construct the "session running" baseline observation.
    #[must_use]
    pub fn running() -> Self {
        Self::Running
    }

    /// Construct the desired end state for a named session-ending action.
    #[must_use]
    pub fn session_ending_requested(action: impl Into<String>) -> Self {
        Self::SessionEndingRequested {
            action: action.into(),
        }
    }

    /// Construct a pending/absent observation for one shutdown schedule.
    #[must_use]
    pub fn shutdown_schedule(schedule_id: impl Into<String>, pending: bool) -> Self {
        Self::ShutdownSchedule {
            schedule_id: schedule_id.into(),
            pending,
        }
    }
}

impl NormalizedObservation for PowerSessionState {
    fn observation_digest(&self) -> Digest {
        match self {
            PowerSessionState::Lock { locked } => {
                Digest::of_str(&format!("power-session:lock:{locked}"))
            }
            PowerSessionState::Running => Digest::of_str("power-session:running"),
            PowerSessionState::SessionEndingRequested { action } => {
                Digest::of_str(&format!("power-session:requested:{action}"))
            }
            PowerSessionState::ShutdownSchedule {
                schedule_id,
                pending,
            } => Digest::of_str(&format!(
                "power-session:shutdown-schedule:{schedule_id}:pending={pending}"
            )),
        }
    }
}

/// The concrete session/lifecycle operation this task migrates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerSessionOp {
    /// `lock_screen`.
    Lock,
    /// `sleep`.
    Suspend,
    /// `hibernate`.
    Hibernate,
    /// `shutdown_system`. The delay is threaded through but not yet scheduled
    /// (Task 3.8 owns delayed-shutdown scheduling).
    Shutdown {
        /// Requested delay in minutes (0 = immediate); accepted and reported,
        /// never turned into a shell `shutdown +N` string.
        delay_minutes: u64,
    },
    /// `reboot_system`.
    Reboot,
    /// `logout_session` — terminate the caller's **own** login session
    /// (Task 3.8, OSC-020).
    ///
    /// Session-ending and irreversible in the way that matters most: every open
    /// application is torn down, so unsaved work is destroyed. The frozen
    /// manifest fixes it at RED with `rollbackClaim: None`, and this slice never
    /// treats it as a routine session action.
    Logout {
        /// The caller-supplied session id, when one was given. It must name the
        /// caller's *current* session (the contract's type is
        /// `CurrentSessionId`); the provider resolves and cross-checks it
        /// against the live session manager rather than trusting it, so this can
        /// never terminate a different session.
        session: Option<String>,
    },
    /// `cancel_scheduled_shutdown` — cancel one pending shutdown schedule
    /// (Task 3.8, OSC-020).
    CancelScheduledShutdown {
        /// The schedule identity to cancel, as derived by
        /// [`selection::derive_schedule_id`] from authoritative state.
        schedule_id: String,
    },
}

impl PowerSessionOp {
    /// The canonical tool/action name this operation maps to.
    #[must_use]
    pub fn action_name(&self) -> &'static str {
        match self {
            PowerSessionOp::Lock => "lock_screen",
            PowerSessionOp::Suspend => "sleep",
            PowerSessionOp::Hibernate => "hibernate",
            PowerSessionOp::Shutdown { .. } => "shutdown_system",
            PowerSessionOp::Reboot => "reboot_system",
            PowerSessionOp::Logout { .. } => "logout_session",
            PowerSessionOp::CancelScheduledShutdown { .. } => "cancel_scheduled_shutdown",
        }
    }

    /// Whether this operation is session-ending/observability-interrupting
    /// (suspend/hibernate/shutdown/reboot/logout) as opposed to `Lock` and
    /// `CancelScheduledShutdown`, which stay observable within the same session.
    #[must_use]
    pub fn is_session_ending(&self) -> bool {
        matches!(
            self,
            PowerSessionOp::Suspend
                | PowerSessionOp::Hibernate
                | PowerSessionOp::Shutdown { .. }
                | PowerSessionOp::Reboot
                | PowerSessionOp::Logout { .. }
        )
    }
}

/// A fully-described power-session request. Carries the canonical `action`/
/// `params` so the governed [`StructuredCommandRequest`] can bind them against
/// the grant.
#[derive(Debug, Clone)]
pub struct PowerSessionRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: PowerSessionOp,
}

impl PowerSessionRequest {
    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> PowerSessionState {
        match &self.op {
            PowerSessionOp::Lock => PowerSessionState::locked(true),
            // Cancelling a schedule converges on "that schedule is not pending",
            // so a schedule that is already gone is `Unchanged`, not a failure.
            PowerSessionOp::CancelScheduledShutdown { schedule_id } => {
                PowerSessionState::shutdown_schedule(schedule_id.clone(), false)
            }
            other => PowerSessionState::session_ending_requested(other.action_name()),
        }
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for `lock_screen`; the other four operations
    /// never reach a comparator-driven `Verified` state at all).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw power-session transport seam. The live implementation
/// ([`crate::os_control::linux::providers::logind::LiveLogind`]) is a
/// deny-live-gated adapter over `org.freedesktop.login1` D-Bus (structured
/// `loginctl` fallback until wired); deny-live tests inject
/// [`fake::FakePowerSessionTransport`]. Reads run a query/parse; `dispatch`
/// runs a governed [`StructuredCommandRequest`].
#[async_trait]
pub trait PowerSessionTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// The selected backend.
    fn selected_backend(&self) -> PowerSessionBackend;

    /// Read the current screen-lock state (`LockedHint`). A parse ambiguity
    /// must surface as an error, never a fabricated state.
    async fn read_locked(&self, ctx: &HostExecutionContext) -> Result<bool, OsControlError>;

    /// A capability probe for hibernate support (never a literal swap-presence
    /// check here — that classification belongs to the transport). `Ok(false)`
    /// and `Err(_)` both mean "do not attempt hibernate"; the caller maps both
    /// to [`OsControlError::Unsupported`] before any dispatch.
    async fn hibernate_available(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<bool, OsControlError>;

    /// Resolve the **caller's own** logind session id (Task 3.8).
    ///
    /// `logout_session` needs a session id in argv, and logind keys sessions by
    /// id: guessing one (or reading an environment hint) could terminate someone
    /// else's session, so the id must come from the live session manager.
    ///
    /// The default refuses rather than inventing an id.
    async fn read_current_session_id(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<String, OsControlError> {
        Err(OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "this session transport cannot resolve the current session id; refusing to guess which session to terminate",
            ),
            retryable: false,
        })
    }

    /// Read the currently pending system shutdown, if any (Task 3.8).
    ///
    /// `Ok(None)` is the positive fact "nothing is scheduled". A read that cannot
    /// be interpreted is an `Err`, so `cancel_scheduled_shutdown` never reports
    /// success against an unknown state.
    ///
    /// The default refuses rather than reporting a fabricated "nothing pending".
    async fn read_scheduled_shutdown(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<Option<selection::ScheduledShutdown>, OsControlError> {
        Err(OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "this session transport cannot read the pending shutdown schedule; it is unknown, not absent",
            ),
            retryable: false,
        })
    }

    /// Dispatch a governed structured command (the only path to a process).
    /// The transport itself decides whether the resulting fact is
    /// `Applied`/`Accepted`/`Uncertain` — session-ending operations are
    /// expected to return `Accepted` backed by real acceptance evidence.
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The `PowerControl` session/lifecycle-slice provider (design §3, §4, §9.7).
/// Generic over the [`PowerSessionTransport`] so the same governed logic runs
/// over the live `logind`/`loginctl` adapter and the deny-live fake.
pub struct PowerSessionControl<T: PowerSessionTransport> {
    transport: T,
    policy: CommandPolicy,
}

impl<T: PowerSessionTransport> PowerSessionControl<T> {
    /// Compose a `PowerSessionControl` over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            policy: CommandPolicy::new(),
        }
    }

    /// The selected backend (for the `backend` result field).
    #[must_use]
    pub fn backend(&self) -> PowerSessionBackend {
        self.transport.selected_backend()
    }

    /// Borrow the underlying transport (used by tests to inspect captured argv).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        match self.transport.selected_backend() {
            PowerSessionBackend::LogindDbus => OsEvidenceSource::AuthoritativeServiceState,
            PowerSessionBackend::Loginctl => OsEvidenceSource::StructuredCommandQuery,
        }
    }

    /// Build the governed structured command for a mutating operation.
    fn build_command(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        self.build_command_with(
            ctx,
            action,
            params,
            args,
            self.transport.selected_backend().trusted_executable()?,
        )
    }

    /// Build the governed structured command against an explicit trusted
    /// executable.
    ///
    /// `cancel_scheduled_shutdown` needs systemd's `shutdown` front-end because
    /// `loginctl` has no cancel subcommand, so the executable is a property of
    /// the *operation*, not only of the backend.
    fn build_command_with(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        action: &str,
        params: &serde_json::Value,
        args: Vec<String>,
        executable: crate::os_control::linux::structured_command::TrustedExecutable,
    ) -> Result<StructuredCommandRequest, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action.to_string(),
            params.clone(),
            executable,
            args,
        );
        StructuredCommandRequest::from_admitted(ctx, plan, &self.policy)
    }

    fn satisfying(&self, observed: &PowerSessionState) -> SatisfyingVerification<PowerSessionState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }

    fn unsupported(&self, reason: &str) -> OsControlError {
        OsControlError::Unsupported {
            capability: CapabilityId::new("hibernate"),
            reason: SafeText::new(reason),
        }
    }
}

#[async_trait]
impl<T: PowerSessionTransport> DesiredStateControl<PowerSessionRequest, PowerSessionState>
    for PowerSessionControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &PowerSessionRequest,
    ) -> Result<PowerSessionState, OsControlError> {
        match &request.op {
            PowerSessionOp::Lock => {
                let locked = self.transport.read_locked(ctx).await?;
                Ok(PowerSessionState::locked(locked))
            }
            // A fresh authoritative read of the pending schedule, keyed by the
            // requested identity. "Nothing pending" is a fact the transport
            // reports, never a substitute for a failed read.
            PowerSessionOp::CancelScheduledShutdown { schedule_id } => {
                let pending = self
                    .transport
                    .read_scheduled_shutdown(ctx)
                    .await?
                    .is_some_and(|scheduled| &scheduled.schedule_id == schedule_id);
                Ok(PowerSessionState::shutdown_schedule(
                    schedule_id.clone(),
                    pending,
                ))
            }
            // Session-ending ops: the pre-apply baseline is simply "running".
            // Its digest is always distinct from the desired
            // `SessionEndingRequested` digest, so idempotency never skips a
            // session-ending dispatch (there is no "already suspended" state
            // to converge toward).
            _ => Ok(PowerSessionState::running()),
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &PowerSessionRequest,
        _desired: &PowerSessionState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            PowerSessionOp::Lock => {
                let args = selection::lock_argv();
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::Suspend => {
                let args = selection::suspend_argv();
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::Hibernate => {
                // Capability probe *before* dispatch (OSC-020): unavailable
                // hibernate must never be fabricated as accepted.
                let available = self
                    .transport
                    .hibernate_available(ctx.observation())
                    .await
                    .unwrap_or(false);
                if !available {
                    return Err(self.unsupported("hibernate is not available on this host"));
                }
                let args = selection::hibernate_argv();
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::Shutdown { .. } => {
                // Delay scheduling is Task 3.8's scope; this dispatches an
                // immediate poweroff. `delay_minutes` is preserved in
                // `request.params` (and therefore the grant/audit bindings)
                // but never becomes a shell `shutdown +N` string.
                let args = selection::shutdown_argv();
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::Reboot => {
                let args = selection::reboot_argv();
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::Logout { session } => {
                // The session id always comes from the live session manager. A
                // caller-supplied id is *cross-checked* against it rather than
                // trusted: the frozen input type is `CurrentSessionId`, and
                // terminating any other session would destroy another user's
                // unsaved work.
                let current = self
                    .transport
                    .read_current_session_id(ctx.observation())
                    .await?;
                selection::validate_session_id(&current)?;
                if let Some(requested) = session {
                    selection::validate_session_id(requested)?;
                    if requested != &current {
                        return Err(OsControlError::InvalidRequest {
                            field: crate::os_control::contract::SafeField::new("session"),
                            reason: SafeText::new(
                                "logout_session only terminates the caller's current session; the requested session id is a different session",
                            ),
                        });
                    }
                }
                let args = selection::logout_argv(&current);
                let command = self.build_command(ctx, &request.action, &request.params, args)?;
                self.transport.dispatch(ctx, &command).await
            }
            PowerSessionOp::CancelScheduledShutdown { schedule_id } => {
                // Re-read under the held lease and cancel only when *this*
                // schedule is the pending one. `shutdown -c` cancels whatever is
                // pending, so dispatching against a schedule we did not observe
                // would cancel an unrelated shutdown.
                let pending = self
                    .transport
                    .read_scheduled_shutdown(ctx.observation())
                    .await?;
                let Some(scheduled) = pending else {
                    return Err(OsControlError::InvalidRequest {
                        field: crate::os_control::contract::SafeField::new("schedule_id"),
                        reason: SafeText::new(
                            "no shutdown is scheduled; nothing to cancel (the runtime reports this as already in the desired state before apply)",
                        ),
                    });
                };
                if &scheduled.schedule_id != schedule_id {
                    return Err(OsControlError::InvalidRequest {
                        field: crate::os_control::contract::SafeField::new("schedule_id"),
                        reason: SafeText::new(
                            "the pending shutdown is a different schedule than the one requested; refusing to cancel an unrelated shutdown",
                        ),
                    });
                }
                let args = selection::cancel_scheduled_shutdown_argv();
                let command = self.build_command_with(
                    ctx,
                    &request.action,
                    &request.params,
                    args,
                    selection::shutdown_schedule_executable()?,
                )?;
                self.transport.dispatch(ctx, &command).await
            }
        }
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &PowerSessionRequest,
        desired: &PowerSessionState,
    ) -> Result<VerificationReport<PowerSessionState>, OsControlError> {
        match &request.op {
            PowerSessionOp::Lock => {
                let locked = self.transport.read_locked(ctx).await?;
                let observed = PowerSessionState::locked(locked);
                if observed.observation_digest() == desired.observation_digest() {
                    Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
                } else {
                    Ok(VerificationReport::Contradicted(
                        crate::os_control::receipt::VerificationContradiction::new(
                            desired.observation_digest(),
                            Some(observed.observation_digest()),
                            SafeErrorCode::from_static("os_control.incident.contradicted"),
                        ),
                    ))
                }
            }
            // A cancelled schedule is verifiable by a fresh authoritative read:
            // the frozen contract names `FreshAuthoritativeObservation` with an
            // `ExactTypedPostcondition` for `cancel_scheduled_shutdown`.
            PowerSessionOp::CancelScheduledShutdown { schedule_id } => {
                let pending = self
                    .transport
                    .read_scheduled_shutdown(ctx)
                    .await?
                    .is_some_and(|scheduled| &scheduled.schedule_id == schedule_id);
                let observed =
                    PowerSessionState::shutdown_schedule(schedule_id.clone(), pending);
                if observed.observation_digest() == desired.observation_digest() {
                    Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
                } else {
                    Ok(VerificationReport::Contradicted(
                        crate::os_control::receipt::VerificationContradiction::new(
                            desired.observation_digest(),
                            Some(observed.observation_digest()),
                            SafeErrorCode::from_static("os_control.incident.contradicted"),
                        ),
                    ))
                }
            }
            // Session-ending operations never claim a satisfying
            // verification (OSC-005.4): observability terminates/suspends, so
            // a decisive fresh observation is never available here even if a
            // transport mistakenly returned `Applied`/`Uncertain` instead of
            // `Accepted`.
            _ => Ok(VerificationReport::Inconclusive {
                reason: SafeText::new(
                    "session-ending action; acceptance is the only observable outcome",
                ),
            }),
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The frozen manifest declares `rollbackClaim: None` for every
        // operation in this slice (including `lock_screen`): none of them are
        // ever advertised with a rollback token, so this is never actually
        // invoked by the runtime. It exists only to satisfy the
        // `DesiredStateControl` trait and reports the truthful "no inverse"
        // fact if it ever were.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `lock_screen` result
/// fields (`action`), plus additive `backend`/`lifecycle`/`verified` fields.
#[must_use]
pub fn lock_screen_result(
    receipt: &MutationReceipt<PowerSessionState>,
    backend: PowerSessionBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "action": "lock_screen",
        "backend": backend.as_str(),
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the **existing** session-ending
/// result fields (`action`), plus additive `backend`/`lifecycle`/`accepted`
/// fields. Never claims `verified`/`completed` (OSC-005.4).
#[must_use]
pub fn session_ending_result(
    receipt: &MutationReceipt<PowerSessionState>,
    action: &str,
    backend: PowerSessionBackend,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "action": action,
        "backend": backend.as_str(),
        "lifecycle": lifecycle.as_str(),
        "accepted": matches!(lifecycle, ActionLifecycle::Accepted),
    })
}

/// Map a governed [`MutationReceipt`] to the `shutdown_system` result fields,
/// additionally reporting the requested (not-yet-scheduled) delay.
#[must_use]
pub fn shutdown_result(
    receipt: &MutationReceipt<PowerSessionState>,
    delay_minutes: u64,
    backend: PowerSessionBackend,
) -> serde_json::Value {
    let mut value = session_ending_result(receipt, "shutdown_system", backend);
    value["delay_minutes"] = serde_json::json!(delay_minutes);
    value
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::power_session()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible power-session domain port. Because the concrete
/// [`PowerSessionControl`] provider struct above is generic over its
/// [`PowerSessionTransport`], `HostOsControl::power_session()` returns this
/// object-safe supertrait instead so any transport (live `logind`/`loginctl`,
/// or a deny-live fake) can be composed behind one erased reference. Every
/// [`PowerSessionControl<T>`] implements it automatically via the blanket impl
/// below. This is a sibling of [`super::PowerControlPort`] (the profile
/// slice): both live under the same `power` domain but bind different
/// request/observation types to `DesiredStateControl`, so they cannot share
/// one dyn port.
pub trait PowerSessionControlPort:
    DesiredStateControl<PowerSessionRequest, PowerSessionState>
{
    /// The composed backend label (for the `backend` result field).
    fn backend(&self) -> PowerSessionBackend;
}

impl<T: PowerSessionTransport> PowerSessionControlPort for PowerSessionControl<T> {
    fn backend(&self) -> PowerSessionBackend {
        PowerSessionControl::backend(self)
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn lock_digest_binds_exact_state() {
        let a = PowerSessionState::locked(true);
        let b = PowerSessionState::locked(true);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = PowerSessionState::locked(false);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn session_ending_digest_always_differs_from_running_baseline() {
        for op in [
            PowerSessionOp::Suspend,
            PowerSessionOp::Hibernate,
            PowerSessionOp::Shutdown { delay_minutes: 0 },
            PowerSessionOp::Reboot,
        ] {
            let desired = PowerSessionState::session_ending_requested(op.action_name());
            let observed = PowerSessionState::running();
            assert_ne!(
                desired.observation_digest(),
                observed.observation_digest(),
                "session-ending op `{}` must never idempotency-skip",
                op.action_name()
            );
        }
    }

    #[test]
    fn desired_state_matches_action() {
        let lock = PowerSessionRequest {
            action: "lock_screen".to_string(),
            params: serde_json::json!({}),
            op: PowerSessionOp::Lock,
        };
        assert_eq!(lock.desired_state(), PowerSessionState::locked(true));

        let reboot = PowerSessionRequest {
            action: "reboot_system".to_string(),
            params: serde_json::json!({}),
            op: PowerSessionOp::Reboot,
        };
        assert_eq!(
            reboot.desired_state(),
            PowerSessionState::session_ending_requested("reboot_system")
        );
    }

    #[test]
    fn action_names_match_the_frozen_manifest() {
        assert_eq!(PowerSessionOp::Lock.action_name(), "lock_screen");
        assert_eq!(PowerSessionOp::Suspend.action_name(), "sleep");
        assert_eq!(PowerSessionOp::Hibernate.action_name(), "hibernate");
        assert_eq!(
            PowerSessionOp::Shutdown { delay_minutes: 0 }.action_name(),
            "shutdown_system"
        );
        assert_eq!(PowerSessionOp::Reboot.action_name(), "reboot_system");
    }

    #[test]
    fn is_session_ending_excludes_only_lock() {
        assert!(!PowerSessionOp::Lock.is_session_ending());
        assert!(PowerSessionOp::Suspend.is_session_ending());
        assert!(PowerSessionOp::Hibernate.is_session_ending());
        assert!(PowerSessionOp::Shutdown { delay_minutes: 5 }.is_session_ending());
        assert!(PowerSessionOp::Reboot.is_session_ending());
    }

    // ── Task 3.8 ────────────────────────────────────────────────────────────

    #[test]
    fn logout_is_session_ending_and_cancel_is_not() {
        assert!(PowerSessionOp::Logout { session: None }.is_session_ending());
        assert!(!PowerSessionOp::CancelScheduledShutdown {
            schedule_id: "abc".to_string()
        }
        .is_session_ending());
    }

    #[test]
    fn task_38_action_names_match_the_frozen_manifest() {
        assert_eq!(
            PowerSessionOp::Logout { session: None }.action_name(),
            "logout_session"
        );
        assert_eq!(
            PowerSessionOp::CancelScheduledShutdown {
                schedule_id: "abc".to_string()
            }
            .action_name(),
            "cancel_scheduled_shutdown"
        );
    }

    #[test]
    fn logout_never_idempotency_skips() {
        let request = PowerSessionRequest {
            action: "logout_session".to_string(),
            params: serde_json::json!({}),
            op: PowerSessionOp::Logout { session: None },
        };
        assert_eq!(
            request.desired_state(),
            PowerSessionState::session_ending_requested("logout_session")
        );
        assert_ne!(
            request.desired_state().observation_digest(),
            PowerSessionState::running().observation_digest()
        );
    }

    #[test]
    fn cancelling_an_absent_schedule_is_unchanged_not_a_failure() {
        let request = PowerSessionRequest {
            action: "cancel_scheduled_shutdown".to_string(),
            params: serde_json::json!({ "schedule_id": "sched-1" }),
            op: PowerSessionOp::CancelScheduledShutdown {
                schedule_id: "sched-1".to_string(),
            },
        };
        let desired = request.desired_state();
        // Observing "not pending" equals the desired state, so the runtime's
        // idempotency check reports Unchanged and never dispatches.
        let observed_absent = PowerSessionState::shutdown_schedule("sched-1", false);
        assert_eq!(
            desired.observation_digest(),
            observed_absent.observation_digest()
        );
        // Observing it as pending must differ, or the cancel would be skipped.
        let observed_pending = PowerSessionState::shutdown_schedule("sched-1", true);
        assert_ne!(
            desired.observation_digest(),
            observed_pending.observation_digest()
        );
    }

    #[test]
    fn schedule_identity_is_part_of_the_digest() {
        // Cancelling schedule A must never be satisfied by the state of B.
        assert_ne!(
            PowerSessionState::shutdown_schedule("a", false).observation_digest(),
            PowerSessionState::shutdown_schedule("b", false).observation_digest()
        );
    }
}
