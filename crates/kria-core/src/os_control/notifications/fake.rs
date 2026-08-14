//! Deny-live fake [`NotificationTransport`] (OSC-023, OSC-033), Task 2.5.
//!
//! Compiled only under `os-control-test`. Nothing reaches the
//! `org.freedesktop.Notifications` D-Bus portal and no `notify-send`/`paplay`
//! process is spawned, so [`crate::os_control::access::deny_live_transport`] is
//! unreachable from here and the deny-live sentinel never trips. A send is
//! recorded as a [`SendCall`] — the suite's only evidence that a notification
//! was dispatched.
//!
//! # Do-not-disturb: "unknown" is not "off"
//!
//! [`super::DoNotDisturb`] has two variants on purpose — `On` and `Off` — so a
//! switch that could not be read has nowhere to hide. This fake keeps the two
//! situations strictly apart:
//!
//! | scripted with | models | returns |
//! |---|---|---|
//! | [`Self::dnd_ok`]`(false)` | the switch was read and alerts are delivered | `Ok(Off)` |
//! | [`Self::dnd_ok`]`(true)` | the switch was read and alerts are suppressed | `Ok(On)` |
//! | [`Self::dnd_unknown`] | the switch could not be located or parsed | [`OsControlError::Unavailable`] |
//! | *nothing scripted* | a test that never established a switch position | [`OsControlError::Unavailable`] |
//!
//! Collapsing "unknown" into `Off` would tell the user they will be alerted when
//! do-not-disturb may in fact be suppressing the alert they are relying on. That
//! is why an unknown switch is an error here rather than a third state some
//! caller would eventually treat as "off".
//!
//! Server availability is the mirror image: [`super::ServerAvailability::Unknown`]
//! **is** a real scriptable answer ([`Self::server_unknown`]), because a failed
//! bus round trip does not establish that nothing is serving notifications.
//!
//! # A read never sends
//!
//! `read_do_not_disturb` and `read_server_availability` are observations; if
//! either produced a notification, an assistant merely *reporting* the user's
//! notification state would interrupt them. Both read paths assert their own
//! send count is unchanged, and [`Self::send_count`] lets a suite check it too.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::ApplyOutcome;

use super::{DoNotDisturb, DoNotDisturbState, NotificationTransport, ServerAvailability};

/// Provider identity reported by the fake transport.
pub const FAKE_NOTIFICATION_PROVIDER_ID: &str = "fake-notifications";

/// Placeholder notification content for fixtures that do not assert on the text.
pub const PLACEHOLDER_NOTIFICATION_TITLE: &str = "PLACEHOLDER-NOTIFICATION-TITLE";

/// Placeholder notification body for fixtures that do not assert on the text.
pub const PLACEHOLDER_NOTIFICATION_BODY: &str = "PLACEHOLDER-NOTIFICATION-BODY";

/// One recorded `send` — the fake's whole evidence that a notification was
/// dispatched.
///
/// The suite asserts on these fields rather than on any host side effect,
/// because there is no host side effect: nothing is delivered anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendCall {
    /// The title handed to the transport.
    pub title: String,
    /// The body handed to the transport.
    pub body: String,
}

/// What the fake knows about the do-not-disturb switch.
enum ScriptedDnd {
    /// No test established a switch position → a read fails closed.
    Unscripted,
    /// The switch was read and holds this position.
    Known(DoNotDisturb),
    /// The switch could not be read. Distinct from `Known(Off)`.
    Unknown {
        /// Why the switch could not be read.
        reason: String,
    },
}

/// A scripted, in-memory notification transport.
///
/// Sends are recorded, never delivered. Do-not-disturb is a small self-applying
/// model: `write_do_not_disturb` moves the switch the fake reports, so an
/// observe → apply → re-observe → verify lifecycle exercises the real governed
/// path rather than a scripted sequence.
pub struct FakeNotificationTransport {
    /// Sends recorded instead of delivered, in order.
    sends: Mutex<Vec<SendCall>>,
    /// Scripted outcome for a mutating call.
    outcome: Mutex<Option<ApplyOutcome>>,
    /// Sticky: every send fails while set (models a refused portal call).
    send_failure: Option<String>,
    /// The do-not-disturb switch the fake reports.
    dnd: Mutex<ScriptedDnd>,
    /// Whether a notification server is answering. `None` means unscripted →
    /// fail closed (distinct from the scriptable `Unknown` answer).
    server: Mutex<Option<ServerAvailability>>,
    /// Mutating transport calls attempted (sends and switch writes).
    dispatches: Mutex<usize>,
    /// Reads served or refused.
    reads: Mutex<usize>,
}

impl Default for FakeNotificationTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeNotificationTransport {
    /// A fake with nothing scripted: a send fails closed until
    /// [`Self::dispatch_outcome`] scripts one, and a read fails closed until a
    /// `dnd_*`/`server_*` builder establishes a fact.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sends: Mutex::new(Vec::new()),
            outcome: Mutex::new(None),
            send_failure: None,
            dnd: Mutex::new(ScriptedDnd::Unscripted),
            server: Mutex::new(None),
            dispatches: Mutex::new(0),
            reads: Mutex::new(0),
        }
    }

    /// Builder: script the outcome a mutating call returns.
    ///
    /// Required before a send: an unscripted send is refused rather than
    /// reported delivered (see [`Self::send`]).
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// Builder: make every send fail, so the apply-failure path is testable.
    #[must_use]
    pub fn send_failure(mut self, reason: impl Into<String>) -> Self {
        self.send_failure = Some(reason.into());
        self
    }

    /// Builder: the switch was read and holds `enabled`.
    #[must_use]
    pub fn dnd_ok(self, enabled: bool) -> Self {
        *self.dnd.lock().expect("dnd mutex") = ScriptedDnd::Known(DoNotDisturb::from_bool(enabled));
        self
    }

    /// Builder: the switch **could not be read**.
    ///
    /// Deliberately not the same as `dnd_ok(false)`: reporting an unreadable
    /// switch as "off" would promise the user an alert that do-not-disturb may
    /// suppress.
    #[must_use]
    pub fn dnd_unknown(self, reason: impl Into<String>) -> Self {
        *self.dnd.lock().expect("dnd mutex") = ScriptedDnd::Unknown {
            reason: reason.into(),
        };
        self
    }

    /// Builder: a notification server answered an identity read.
    #[must_use]
    pub fn server_available(self) -> Self {
        *self.server.lock().expect("server mutex") = Some(ServerAvailability::Available);
        self
    }

    /// Builder: server availability could not be determined — a real answer,
    /// never rendered as "no server".
    #[must_use]
    pub fn server_unknown(self) -> Self {
        *self.server.lock().expect("server mutex") = Some(ServerAvailability::Unknown);
        self
    }

    /// The sends recorded instead of delivered, in order.
    #[must_use]
    pub fn send_calls(&self) -> Vec<SendCall> {
        self.sends.lock().expect("sends mutex").clone()
    }

    /// How many sends were attempted.
    #[must_use]
    pub fn send_count(&self) -> usize {
        self.sends.lock().expect("sends mutex").len()
    }

    /// How many mutating transport calls were attempted (sends + switch writes).
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        *self.dispatches.lock().expect("dispatch mutex")
    }

    /// How many reads were served or refused.
    #[must_use]
    pub fn read_count(&self) -> usize {
        *self.reads.lock().expect("reads mutex")
    }

    /// The switch position the fake currently models, or `None` when it is
    /// unscripted or explicitly unknown.
    ///
    /// Lets a test prove `write_do_not_disturb` applied its effect to the fake's
    /// own state without scripting a further read.
    #[must_use]
    pub fn modeled_do_not_disturb(&self) -> Option<DoNotDisturb> {
        match &*self.dnd.lock().expect("dnd mutex") {
            ScriptedDnd::Known(state) => Some(*state),
            ScriptedDnd::Unscripted | ScriptedDnd::Unknown { .. } => None,
        }
    }

    /// The error an unscripted read returns. Never a value: a fake that invented
    /// state would let a test prove a mutation verified against a fact nobody read.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_NOTIFICATION_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }
}

#[async_trait]
impl NotificationTransport for FakeNotificationTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_NOTIFICATION_PROVIDER_ID)
    }

    async fn send(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        title: &str,
        body: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        *self.dispatches.lock().expect("dispatch mutex") += 1;
        // Recorded, never delivered: no portal call, no notify-send, no sound.
        self.sends.lock().expect("sends mutex").push(SendCall {
            title: title.to_string(),
            body: body.to_string(),
        });

        if let Some(reason) = &self.send_failure {
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_NOTIFICATION_PROVIDER_ID)),
                reason: SafeText::new(format!("notification portal refused the send: {reason}")),
                retryable: true,
            });
        }

        // Fail closed with no scripted outcome. The portal reply *is* the
        // delivery evidence for this domain — `verify` synthesizes its
        // satisfying evidence from the dispatch rather than re-observing — so a
        // defaulted `Applied` here would certify a delivery no test ever
        // scripted.
        self.outcome
            .lock()
            .expect("outcome mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no dispatch outcome scripted on the fake transport"))
    }

    async fn read_do_not_disturb(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<DoNotDisturbState, OsControlError> {
        let sends_before = self.send_count();
        *self.reads.lock().expect("reads mutex") += 1;

        let result = match &*self.dnd.lock().expect("dnd mutex") {
            ScriptedDnd::Known(state) => Ok(DoNotDisturbState {
                do_not_disturb: *state,
            }),
            // An unreadable switch is an error, never `Off`.
            ScriptedDnd::Unknown { reason } => Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_NOTIFICATION_PROVIDER_ID)),
                reason: SafeText::new(format!(
                    "do-not-disturb switch could not be read: {reason}"
                )),
                retryable: true,
            }),
            ScriptedDnd::Unscripted => Err(
                self.unscripted("no do-not-disturb state scripted on the fake transport")
            ),
        };

        debug_assert_eq!(
            self.send_count(),
            sends_before,
            "reading do-not-disturb must never send a notification"
        );
        result
    }

    async fn write_do_not_disturb(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        *self.dispatches.lock().expect("dispatch mutex") += 1;
        // Self-applying: the switch the fake reports actually moves, so a
        // re-observation after apply sees the requested position. A write also
        // establishes a previously-unknown switch — writing is how it becomes
        // known.
        *self.dnd.lock().expect("dnd mutex") = ScriptedDnd::Known(DoNotDisturb::from_bool(enabled));

        self.outcome
            .lock()
            .expect("outcome mutex")
            .clone()
            .ok_or_else(|| self.unscripted("no dispatch outcome scripted on the fake transport"))
    }

    async fn read_server_availability(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<ServerAvailability, OsControlError> {
        let sends_before = self.send_count();
        *self.reads.lock().expect("reads mutex") += 1;

        // `Unknown` is a scripted answer, not a failure; an *unscripted* read is
        // a failure, because the test established nothing.
        let result = self
            .server
            .lock()
            .expect("server mutex")
            .ok_or_else(|| {
                self.unscripted("no notification server availability scripted on the fake transport")
            });

        debug_assert_eq!(
            self.send_count(),
            sends_before,
            "reading server availability must never send a notification"
        );
        result
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio_util::sync::CancellationToken;

    use crate::agent::execution_gate::OsActionGrant;
    use crate::agent::turn_memory::ExecutionTarget;
    use crate::os_control::context::{
        AuditAdmissionToken, MutationPermit, RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{
        ActionId, AuditAdmissionId, BoundedVec, CorrelationId, Digest, SessionId,
    };
    use crate::os_control::receipt::AppliedDispatch;
    use crate::os_control::resource::AcquiredResourceLeaseSet;
    use crate::safety::RiskLevel;

    use super::*;

    const SESSION: &str = "session-notification-fake";

    fn applied() -> ApplyOutcome {
        ApplyOutcome::Applied(AppliedDispatch::new(None, BoundedVec::new()))
    }

    /// Holds the sealed authorities so a mutation context can be handed to a
    /// dispatch without lifetime trouble.
    struct Fixture {
        grant: OsActionGrant,
        host_ctx: HostExecutionContext,
        lease_set: AcquiredResourceLeaseSet,
        audit_token: AuditAdmissionToken,
        resource_digest: Digest,
    }

    impl Fixture {
        fn build() -> Self {
            let params = serde_json::json!({});
            let grant = OsActionGrant::for_test(
                SESSION,
                "send_notification",
                &params,
                ExecutionTarget::Host,
                &[],
                RiskLevel::Yellow,
            );
            let resource_digest = Digest::of_str(grant.resource_set_digest());
            let audit_token = AuditAdmissionToken::for_test(
                AuditAdmissionId::new("adm-notification-fake"),
                resource_digest.clone(),
            );
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-notification-fake"),
                ActionId::new("act-notification-fake"),
                audit_token.observation_authority(),
                Arc::new(SessionContext::new(SessionId::new(SESSION))),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(30),
                RedactionPolicy::default(),
            );
            let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest.clone());
            Self {
                grant,
                host_ctx,
                lease_set,
                audit_token,
                resource_digest,
            }
        }

        fn host(&self) -> &HostExecutionContext {
            &self.host_ctx
        }

        fn admitted(&self) -> AdmittedMutationContext<'_> {
            let permit = MutationPermit::for_test(
                &self.lease_set,
                &self.audit_token,
                self.resource_digest.clone(),
            );
            AdmittedMutationContext::for_test(&self.host_ctx, &self.grant, permit)
        }
    }

    #[tokio::test]
    async fn unscripted_do_not_disturb_read_fails_closed() {
        let fx = Fixture::build();
        let fake = FakeNotificationTransport::new();

        let err = fake
            .read_do_not_disturb(fx.host())
            .await
            .expect_err("an unscripted switch must not read as off");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn an_unknown_switch_is_not_an_off_switch() {
        let fx = Fixture::build();

        // Off is a read fact: the user will be alerted.
        let off = FakeNotificationTransport::new().dnd_ok(false);
        assert_eq!(
            off.read_do_not_disturb(fx.host())
                .await
                .expect("scripted off")
                .do_not_disturb,
            DoNotDisturb::Off
        );

        // Unknown is not: reporting it as off would promise an alert that
        // do-not-disturb may be suppressing.
        let unknown = FakeNotificationTransport::new().dnd_unknown("no portal answered");
        let err = unknown
            .read_do_not_disturb(fx.host())
            .await
            .expect_err("an unreadable switch must not read as off");
        match err {
            OsControlError::Unavailable { retryable, .. } => assert!(retryable),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_read_never_sends_a_notification() {
        let fx = Fixture::build();
        let fake = FakeNotificationTransport::new()
            .dnd_ok(true)
            .server_available();

        fake.read_do_not_disturb(fx.host())
            .await
            .expect("scripted switch");
        fake.read_server_availability(fx.host())
            .await
            .expect("scripted availability");

        assert_eq!(
            fake.send_count(),
            0,
            "reporting notification state must never interrupt the user"
        );
        assert_eq!(fake.dispatch_count(), 0);
        assert_eq!(fake.read_count(), 2);
    }

    #[tokio::test]
    async fn unknown_server_availability_is_a_value_but_unscripted_is_an_error() {
        let fx = Fixture::build();

        // A failed bus round trip does not establish that nothing is serving
        // notifications, so Unknown is a real answer.
        let unknown = FakeNotificationTransport::new().server_unknown();
        assert_eq!(
            unknown
                .read_server_availability(fx.host())
                .await
                .expect("Unknown is a value"),
            ServerAvailability::Unknown
        );

        // Having scripted nothing at all is different: the test established no fact.
        let unscripted = FakeNotificationTransport::new();
        assert!(matches!(
            unscripted.read_server_availability(fx.host()).await,
            Err(OsControlError::Unavailable { .. })
        ));
    }

    #[tokio::test]
    async fn a_send_is_recorded_and_an_unscripted_outcome_fails_closed() {
        let fx = Fixture::build();

        // No scripted outcome: the fake must not certify a delivery.
        let unscripted = FakeNotificationTransport::new();
        unscripted
            .send(
                &fx.admitted(),
                PLACEHOLDER_NOTIFICATION_TITLE,
                PLACEHOLDER_NOTIFICATION_BODY,
            )
            .await
            .expect_err("an unscripted send must not report delivered");
        assert_eq!(
            unscripted.send_count(),
            1,
            "the attempt is still recorded as evidence"
        );

        let scripted = FakeNotificationTransport::new().dispatch_outcome(applied());
        scripted
            .send(
                &fx.admitted(),
                PLACEHOLDER_NOTIFICATION_TITLE,
                PLACEHOLDER_NOTIFICATION_BODY,
            )
            .await
            .expect("scripted send");
        let calls = scripted.send_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].title, PLACEHOLDER_NOTIFICATION_TITLE);
        assert_eq!(calls[0].body, PLACEHOLDER_NOTIFICATION_BODY);
        assert_eq!(scripted.dispatch_count(), 1);
    }

    #[tokio::test]
    async fn writing_the_switch_moves_the_fakes_own_state() {
        let fx = Fixture::build();
        // The switch starts unreadable; writing it is what establishes it.
        let fake = FakeNotificationTransport::new()
            .dnd_unknown("no portal answered")
            .dispatch_outcome(applied());

        fake.write_do_not_disturb(&fx.admitted(), true)
            .await
            .expect("switch write applies");

        assert_eq!(fake.modeled_do_not_disturb(), Some(DoNotDisturb::On));
        assert_eq!(
            fake.read_do_not_disturb(fx.host())
                .await
                .expect("switch now readable")
                .do_not_disturb,
            DoNotDisturb::On
        );
        assert_eq!(fake.send_count(), 0, "setting the switch sends nothing");
    }
}
