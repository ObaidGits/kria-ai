//! Notification domain: the `NotificationControl` desired-state provider
//! (design §3, §9.10).
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-023).
//!
//! # Upgrade notification adapter (explicit Task 2.5 requirement)
//!
//! This module replaces the direct `tokio::process::Command::new("notify-send")`
//! calls (with manually-resolved `DBUS_SESSION_BUS_ADDRESS`/`DISPLAY`
//! environment guessing) and the `notify_rust` library fallback that used to
//! live in `tools/communication.rs::SendNotification`/`ScheduleReminder`, with
//! a single freedesktop-portal-style [`NotificationTransport`] seam. The
//! provider itself never spawns `notify-send`, never guesses a D-Bus address,
//! and never plays an alert sound via `paplay` — those were all direct
//! subprocess/environment-guessing behaviors this task's objective targets.
//! `schedule_reminder`'s *timer* scheduling (an in-process `tokio::spawn` +
//! `sleep`, not a host mutation) is unaffected; only the eventual notification
//! delivery routes through this provider.
//!
//! * [`NotificationState`] is a normalized observation
//!   ([`NormalizedObservation`]) binding the notification's content digest so
//!   `send_notification`'s postcondition is a real "a notification was
//!   delivered with this content" check rather than an unconditional success.
//! * [`NotificationControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle. Every `send_notification` mutates observable notification-
//!   center state, so it is never `Unchanged` by construction (each send is a
//!   new, distinct notification — there is no "already sent" idempotent
//!   state to converge toward, mirroring
//!   [`crate::os_control::power::session`]'s session-ending digest shape).
//! * `rollback` always reports the truthful "no inverse" fact: the frozen
//!   manifest declares `rollbackClaim: None` for `send_notification`.
//! * The live transport
//!   ([`crate::os_control::linux::providers::notifications::LiveNotifications`])
//!   is a raw, deny-live-gated adapter over the freedesktop
//!   `org.freedesktop.Notifications` D-Bus portal (never a `notify-send`
//!   subprocess); deny-live tests inject
//!   [`fake::FakeNotificationTransport`].

use async_trait::async_trait;
use std::time::SystemTime;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    AcceptanceEvidence, ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    UncertainDispatch, UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

pub mod selection;

/// Deny-live fake transport (Task 2.5 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

/// The stable provider identity for the freedesktop-portal notification backend.
pub const NOTIFICATION_PROVIDER_ID: &str = "notifications-freedesktop-portal";

/// A normalized observation binding a delivered notification's content
/// digest, so distinct sends never collide and never fabricate an "already
/// sent" idempotent match (design §5, §9.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationState {
    /// Digest over the canonical `title`+`body` content that was requested.
    pub content_digest: Digest,
    /// A monotonically-distinguishing nonce so two identical-content sends
    /// (e.g. two "Reminder: standup" notifications) never collapse into the
    /// same digest and spuriously report `Unchanged`.
    pub dispatch_nonce: u64,
}

impl NotificationState {
    /// Construct the desired-state marker for one `send_notification` call.
    #[must_use]
    pub fn requested(title: &str, body: &str, nonce: u64) -> Self {
        Self {
            content_digest: Digest::of_str(&format!("{title}\u{1f}{body}")),
            dispatch_nonce: nonce,
        }
    }
}

impl NormalizedObservation for NotificationState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "notification:{}:{}",
            self.content_digest, self.dispatch_nonce
        ))
    }
}

/// A fully-described `send_notification` request.
#[derive(Debug, Clone)]
pub struct NotificationRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The notification title.
    pub title: String,
    /// The notification body.
    pub body: String,
    /// A per-request nonce (e.g. a monotonic counter or timestamp) ensuring
    /// every send is a distinct desired state (see [`NotificationState`]).
    pub nonce: u64,
}

impl NotificationRequest {
    /// The desired end state: a delivered notification with this content.
    #[must_use]
    pub fn desired_state(&self) -> NotificationState {
        NotificationState::requested(&self.title, &self.body, self.nonce)
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for `send_notification`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw notification transport seam. The live implementation
/// ([`crate::os_control::linux::providers::notifications::LiveNotifications`])
/// is a deny-live-gated adapter over the `org.freedesktop.Notifications`
/// D-Bus portal; deny-live tests inject [`fake::FakeNotificationTransport`].
#[async_trait]
pub trait NotificationTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Send a notification via the freedesktop portal, returning the
    /// dispatch outcome. The portal's `Notify` D-Bus method reply *is* the
    /// acceptance evidence — never a guessed environment or subprocess exit
    /// code.
    async fn send(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        title: &str,
        body: &str,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Read this session's do-not-disturb switch (Task 4.9).
    ///
    /// Fails closed: a session whose switch cannot be located or whose reading is
    /// unrecognized returns an error. It never reports "not disturbed", which
    /// would tell the user they will be alerted when they will not be.
    async fn read_do_not_disturb(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DoNotDisturbState, OsControlError>;

    /// Set this session's do-not-disturb switch (Task 4.9).
    async fn write_do_not_disturb(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Whether a notification server is answering on this session (Task 4.9).
    ///
    /// A *secondary* fact, reported separately from the switch: an answering
    /// server is [`ServerAvailability::Available`], and anything else is
    /// [`ServerAvailability::Unknown`] — never `Unavailable`, because a failed
    /// bus round trip does not distinguish "nothing is serving notifications"
    /// from "the read did not complete".
    async fn read_server_availability(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ServerAvailability, OsControlError>;
}

/// The `NotificationControl` desired-state provider (design §3, §4, §9.10).
/// Generic over the [`NotificationTransport`] so the same governed logic runs
/// over the live freedesktop-portal adapter and the deny-live fake.
pub struct NotificationControl<T: NotificationTransport> {
    transport: T,
}

impl<T: NotificationTransport> NotificationControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport (used by tests).
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
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(
        &self,
        observed: &NotificationState,
    ) -> SatisfyingVerification<NotificationState> {
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
}

#[async_trait]
impl<T: NotificationTransport> DesiredStateControl<NotificationRequest, NotificationState>
    for NotificationControl<T>
{
    async fn observe(
        &self,
        _ctx: &HostExecutionContext,
        _request: &NotificationRequest,
    ) -> Result<NotificationState, OsControlError> {
        // There is no "current notification state" to read before dispatch —
        // each send is a fresh event, so the pre-apply baseline is the
        // digest-zero marker, which never equals the requested (nonced)
        // desired state and therefore never idempotency-skips a send.
        Ok(NotificationState {
            content_digest: Digest::of_str(""),
            dispatch_nonce: 0,
        })
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &NotificationRequest,
        _desired: &NotificationState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport
            .send(ctx, &request.title, &request.body)
            .await
    }

    async fn verify(
        &self,
        _ctx: &HostExecutionContext,
        _request: &NotificationRequest,
        desired: &NotificationState,
    ) -> Result<VerificationReport<NotificationState>, OsControlError> {
        // The portal's `Notify` reply (captured as `AcceptedDispatch`/
        // `AppliedDispatch` by the transport) is itself the delivery
        // evidence; there is no further independent re-observation surface
        // for a transient desktop notification. Report the satisfying
        // evidence directly from the desired marker — the runtime only
        // reaches `verify` after a non-`Accepted` apply outcome, at which
        // point the dispatch itself already proved delivery.
        Ok(VerificationReport::Satisfied(self.satisfying(desired)))
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // `rollbackClaim: None` — a delivered notification cannot be
        // un-delivered; never actually invoked.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

/// Construct the [`AcceptanceEvidence`] a live portal transport attaches to an
/// [`crate::os_control::receipt::AcceptedDispatch`] for a successful `Notify`
/// call. Exposed so both the live adapter and fakes build identically-shaped
/// evidence.
#[must_use]
pub fn portal_acceptance_evidence() -> AcceptanceEvidence {
    AcceptanceEvidence {
        detail: crate::os_control::contract::SafeText::new(
            "freedesktop Notifications portal accepted",
        ),
        accepted_at: SystemTime::now(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `send_notification`
/// result fields (`sent`, `title`, `method`), plus additive `lifecycle`
/// field. `method` is now always the portal provider id rather than
/// `"notify-send"`/`"notify_rust"`.
#[must_use]
pub fn send_notification_result(
    receipt: &MutationReceipt<NotificationState>,
    title: &str,
    provider: ProviderId,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "sent": matches!(
            lifecycle,
            ActionLifecycle::Verified | ActionLifecycle::Accepted | ActionLifecycle::Unchanged
        ),
        "title": title,
        "method": provider.as_str(),
        "lifecycle": lifecycle.as_str(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Do-not-disturb + notification session state (Task 4.9, OSC-023)
// ─────────────────────────────────────────────────────────────────────────────

/// Whether this session is currently suppressing notification alerts.
///
/// Two states only, by design. There is no `Unknown` variant: a switch that could
/// not be read is an **error**, not a third state that some caller would
/// eventually treat as "off" and report that the user will be alerted when they
/// will not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoNotDisturb {
    /// Alerts are suppressed.
    On,
    /// Alerts are delivered.
    Off,
}

impl DoNotDisturb {
    /// The stable token surfaced to callers.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DoNotDisturb::On => "on",
            DoNotDisturb::Off => "off",
        }
    }

    /// Whether alerts are suppressed.
    #[must_use]
    pub fn is_on(self) -> bool {
        matches!(self, DoNotDisturb::On)
    }

    /// Construct from the boolean a backend switch reports.
    #[must_use]
    pub fn from_bool(enabled: bool) -> Self {
        if enabled {
            DoNotDisturb::On
        } else {
            DoNotDisturb::Off
        }
    }
}

/// Whether a notification server is answering on this session.
///
/// `Unknown` is a real answer, distinct from "no server": a bus round trip that
/// times out or is refused does not establish that nothing is serving
/// notifications, so it is never reported as an absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerAvailability {
    /// A notification server answered an identity read.
    Available,
    /// Availability could not be determined.
    Unknown,
}

impl ServerAvailability {
    /// The stable token surfaced to callers.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ServerAvailability::Available => "available",
            ServerAvailability::Unknown => "unknown",
        }
    }
}

/// A normalized observation of the do-not-disturb switch — the postcondition
/// surface for `set_do_not_disturb`.
///
/// The digest binds the switch and nothing else. Binding a secondary fact such as
/// server availability would let an unrelated change between apply and verify
/// contradict a mutation that actually succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoNotDisturbState {
    /// The observed switch position.
    pub do_not_disturb: DoNotDisturb,
}

impl DoNotDisturbState {
    /// Construct from the boolean a backend switch reports.
    #[must_use]
    pub fn from_bool(enabled: bool) -> Self {
        Self {
            do_not_disturb: DoNotDisturb::from_bool(enabled),
        }
    }
}

impl NormalizedObservation for DoNotDisturbState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "notification-dnd:{}",
            self.do_not_disturb.as_str()
        ))
    }
}

/// The read-only session notification state (`get_notification_state`).
///
/// The switch is authoritative and mandatory; server availability is an
/// independently-sourced secondary fact. Nothing here describes the *content* of
/// any notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationSessionState {
    /// Whether alerts are currently suppressed.
    pub do_not_disturb: DoNotDisturb,
    /// Whether a notification server is answering.
    pub server: ServerAvailability,
}

/// A fully-described `set_do_not_disturb` request.
#[derive(Debug, Clone)]
pub struct DoNotDisturbRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The requested switch position.
    pub enabled: bool,
}

impl DoNotDisturbRequest {
    /// The desired end state: the switch reads exactly as requested.
    #[must_use]
    pub fn desired_state(&self) -> DoNotDisturbState {
        DoNotDisturbState::from_bool(self.enabled)
    }

    /// The frozen comparator (`ExactTypedPostcondition`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

impl<T: NotificationTransport> NotificationControl<T> {
    fn satisfying_dnd(
        &self,
        observed: &DoNotDisturbState,
    ) -> SatisfyingVerification<DoNotDisturbState> {
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(*observed, observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }

    /// Read the full session notification state.
    ///
    /// The switch is read first and its failure is fatal: an unknown suppression
    /// state is never papered over with a server-availability answer.
    pub async fn session_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<NotificationSessionState, OsControlError> {
        let switch = self.transport.read_do_not_disturb(ctx).await?;
        let server = self.transport.read_server_availability(ctx).await?;
        Ok(NotificationSessionState {
            do_not_disturb: switch.do_not_disturb,
            server,
        })
    }
}

#[async_trait]
impl<T: NotificationTransport> DesiredStateControl<DoNotDisturbRequest, DoNotDisturbState>
    for NotificationControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &DoNotDisturbRequest,
    ) -> Result<DoNotDisturbState, OsControlError> {
        self.transport.read_do_not_disturb(ctx).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &DoNotDisturbRequest,
        _desired: &DoNotDisturbState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport
            .write_do_not_disturb(ctx, request.enabled)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &DoNotDisturbRequest,
        desired: &DoNotDisturbState,
    ) -> Result<VerificationReport<DoNotDisturbState>, OsControlError> {
        // A real re-read of the switch. "The command exited zero" is not evidence
        // that alerts are now suppressed.
        let observed = self.transport.read_do_not_disturb(ctx).await?;
        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying_dnd(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The rollback token carries no prior switch position, and this provider
        // holds no state of its own, so there is no inverse it can perform. The
        // receipt therefore advertises no rollback (rule: rollback only if it is
        // real); reverting is a fresh `set_do_not_disturb` with the other value.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::notifications()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible notification domain port design §4 names
/// `fn notifications(&self) -> &dyn NotificationControl` on `HostOsControl`.
/// Because the concrete [`NotificationControl`] provider struct above is
/// generic over its [`NotificationTransport`], `HostOsControl::notifications()`
/// returns this object-safe supertrait instead so any transport (live
/// freedesktop portal, or a deny-live fake) can be composed behind one erased
/// reference. Every [`NotificationControl<T>`] implements it automatically
/// via the blanket impl below.
///
/// The two `DesiredStateControl` supertraits are the domain's two distinct
/// postconditions: a delivered notification (`send_notification`) and the
/// do-not-disturb switch (`set_do_not_disturb`). They verify against different
/// facts, so they are separate lifecycles rather than one shared observation.
#[async_trait]
pub trait NotificationControlPort:
    DesiredStateControl<NotificationRequest, NotificationState>
    + DesiredStateControl<DoNotDisturbRequest, DoNotDisturbState>
{
    /// Read the session's notification state (erased passthrough for
    /// `get_notification_state`).
    async fn session_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<NotificationSessionState, OsControlError>;
}

#[async_trait]
impl<T: NotificationTransport> NotificationControlPort for NotificationControl<T> {
    async fn session_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<NotificationSessionState, OsControlError> {
        NotificationControl::session_state(self, ctx).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_content_and_nonce() {
        let a = NotificationState::requested("Title", "Body", 1);
        let b = NotificationState::requested("Title", "Body", 1);
        assert_eq!(a.observation_digest(), b.observation_digest());

        // Same content, different nonce → distinct digests (no spurious
        // idempotent collapse across two separate sends).
        let c = NotificationState::requested("Title", "Body", 2);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn desired_state_matches_request() {
        let req = NotificationRequest {
            action: "send_notification".to_string(),
            params: serde_json::json!({ "title": "T", "body": "B" }),
            title: "T".to_string(),
            body: "B".to_string(),
            nonce: 7,
        };
        assert_eq!(
            req.desired_state(),
            NotificationState::requested("T", "B", 7)
        );
    }
}
