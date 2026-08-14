//! Live freedesktop `org.freedesktop.Notifications` portal adapter (raw
//! transport seam).
//!
//! linux-os-control-production **Task 2.5** — "upgrade notification adapter"
//! (OSC-023), design §3, §9.10.
//!
//! Replaces the pre-migration `notify-send` subprocess spawn / `notify_rust`
//! library fallback with the freedesktop D-Bus session-bus portal. Like the
//! other `linux/providers/*` adapters, construction requires a
//! [`LiveHostAccessToken`] and every method trips the deny-live sentinel
//! before touching the host. Deny-live tests inject
//! [`crate::os_control::notifications::fake::FakeNotificationTransport`].
//!
//! # Reads
//!
//! [`LiveNotifications::read_capabilities`] and
//! [`LiveNotifications::read_server_identity`] are **pure reads** of the
//! portal's own `GetCapabilities` / `GetServerInformation` members over the
//! session-bus connection a live composition root opened with
//! [`LiveDbusTransport`] (see [`LiveNotifications::with_bus`]). Neither one
//! posts, replaces, or closes a notification: an observation must not have the
//! side effect it is supposed to be observing, so the portal's `Notify` member
//! is never used to probe availability.
//!
//! Sending remains unwired — the `Notify` mutation is a separate concern from
//! these reads, and there is no ungoverned `notify-send` fallback.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeOperation, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::notifications::selection::{
    parse_do_not_disturb, read_dnd_argv, select_dnd_backend, write_dnd_argv, DndBackend,
};
use crate::os_control::notifications::{
    DoNotDisturbState, NotificationTransport, ServerAvailability, NOTIFICATION_PROVIDER_ID,
};
use crate::os_control::receipt::ApplyOutcome;

/// The notification portal's bus name (session bus).
const NOTIFICATIONS_SERVICE: &str = "org.freedesktop.Notifications";
/// The portal's object.
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
/// The portal's interface (identical to its bus name).
const NOTIFICATIONS_IFACE: &str = "org.freedesktop.Notifications";

/// The live freedesktop-portal notification adapter. Constructible only in a
/// live composition; a value cannot exist under `os-control-test`.
pub struct LiveNotifications {
    /// The session-bus connection capability/state reads run over, when a live
    /// composition root handed this adapter one.
    session_bus: Option<zbus::Connection>,
    _seal: (),
}

impl LiveNotifications {
    /// Construct in a live composition root **without** a bus connection: the
    /// capability reads report `Unavailable`. Requires a
    /// [`LiveHostAccessToken`].
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self {
            session_bus: None,
            _seal: (),
        }
    }

    /// Construct over the session bus a live composition root already opened
    /// with [`LiveDbusTransport`]. This is the constructor that makes the
    /// portal's capability/state reads real; the transport was opened behind the
    /// deny-live sentinel and the live token, so this adapter never opens a bus
    /// of its own.
    #[must_use]
    pub fn with_bus(_token: &LiveHostAccessToken, transport: &LiveDbusTransport) -> Self {
        Self {
            session_bus: transport.connection(BusKind::Session).cloned(),
            _seal: (),
        }
    }


    /// Borrow the session bus, or fail closed.
    fn bus(&self) -> Result<&zbus::Connection, OsControlError> {
        self.session_bus
            .as_ref()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "no session-bus connection was composed for the notification portal adapter",
                ),
                retryable: false,
            })
    }

    /// The portal answered with something this adapter cannot interpret.
    fn protocol(&self, member: &str) -> OsControlError {
        OsControlError::ProtocolBeforeMutation {
            provider: self.provider_id(),
            operation: SafeOperation::new(member),
        }
    }

    /// One deadline- and cancellation-bounded **read** of a portal member.
    ///
    /// Only side-effect-free members belong here (`GetCapabilities`,
    /// `GetServerInformation`); posting a notification is a mutation and never a
    /// probe. The bound comes from the observation context, never from this
    /// provider.
    async fn read_member<R>(
        &self,
        ctx: &HostExecutionContext,
        member: &str,
    ) -> Result<R, OsControlError>
    where
        R: zbus::zvariant::Type + for<'d> serde::Deserialize<'d>,
    {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let conn = self.bus()?;
        let call = conn.call_method(
            Some(NOTIFICATIONS_SERVICE),
            NOTIFICATIONS_PATH,
            Some(NOTIFICATIONS_IFACE),
            member,
            &(),
        );
        let deadline = tokio::time::Instant::from_std(ctx.deadline);
        let reply = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                return Err(OsControlError::CancelledBeforeMutation);
            }
            outcome = tokio::time::timeout_at(deadline, call) => outcome,
        }
        .map_err(|_| OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new(member),
            timeout_ms: 0,
        })?
        .map_err(|_| OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new("the notification portal did not answer the capability read"),
            retryable: true,
        })?;
        reply
            .body()
            .deserialize::<R>()
            .map_err(|_| self.protocol(member))
    }

    /// Read the portal's advertised capability tokens (`GetCapabilities`, `as`),
    /// e.g. `body`, `actions`, `body-markup`.
    ///
    /// The list is returned exactly as advertised — including an empty list,
    /// which is a real answer from a minimal server and not something to pad
    /// with assumed capabilities.
    pub async fn read_capabilities(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Vec<String>, OsControlError> {
        // A capability read opens a session-bus round trip.
        deny_live_transport(RawTransportKind::SessionBus);
        self.read_member(ctx, "GetCapabilities").await
    }

    /// Read the portal's own identity (`GetServerInformation`, `(ssss)`) as
    /// `(name, vendor, version, spec_version)`.
    ///
    /// This is the side-effect-free way to confirm a *reachable, answering*
    /// notification server. Nothing is posted to the user's screen.
    pub async fn read_server_identity(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<(String, String, String, String), OsControlError> {
        // A server-information read opens a session-bus round trip.
        deny_live_transport(RawTransportKind::SessionBus);
        self.read_member(ctx, "GetServerInformation").await
    }

    /// Resolve the authority that owns this session's do-not-disturb switch.
    ///
    /// A session whose desktop family was not conclusively probed, or whose
    /// switch tool is not installed, has **no** readable switch and reports
    /// `Unavailable`. It never falls back to "not disturbed": that would tell the
    /// user their alerts will arrive when they may be silenced.
    fn dnd_backend(&self, ctx: &HostExecutionContext) -> Result<DndBackend, OsControlError> {
        let installed: Vec<DndBackend> = DndBackend::PREFERENCE
            .into_iter()
            .filter(|candidate| std::path::Path::new(candidate.executable_path()).is_file())
            .collect();

        select_dnd_backend(ctx.session.desktop_family, &installed).ok_or_else(|| {
            OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "no do-not-disturb switch is readable for this session's desktop family; the alert-suppression state is unknown, not off",
                ),
                retryable: false,
            }
        })
    }
}

/// Bound and sanitize notification text before it becomes an argv element.
///
/// Control characters are stripped rather than escaped: a newline or terminal
/// escape sequence in a notification body could otherwise corrupt a log line or
/// a terminal that renders it.
fn bounded_text(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect()
}

#[async_trait::async_trait]
impl NotificationTransport for LiveNotifications {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(NOTIFICATION_PROVIDER_ID)
    }

    async fn send(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        title: &str,
        body: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);
        let executable = TrustedExecutable::new(
            "/usr/bin/notify-send",
            crate::os_control::contract::Digest::of_str("notify-send-v1"),
        )?;
        // `--` ends option parsing, so a title or body beginning with `-` is
        // delivered as text instead of being read as a flag. The text is argv
        // rather than stdin because notify-send takes no stdin — acceptable here
        // because the content is about to be displayed on screen anyway.
        let plan = CommandPlan::new(
            CapabilityId::new("send_notification"),
            "send_notification",
            serde_json::Value::Null,
            executable,
            vec![
                "--".into(),
                bounded_text(title, 200),
                bounded_text(body, 1000),
            ],
        );
        let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
        request.dispatch().await
    }

    async fn read_do_not_disturb(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<DoNotDisturbState, OsControlError> {
        // Reading the switch runs a query child process.
        deny_live_transport(RawTransportKind::Process);

        let backend = self.dnd_backend(ctx)?;
        let plan = CommandPlan::new(
            CapabilityId::new("get_notification_state"),
            "get_notification_state",
            // A switch read takes no parameters.
            serde_json::Value::Null,
            backend.trusted_executable()?,
            read_dnd_argv(backend),
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "the do-not-disturb switch reading was truncated; refusing to interpret a partial answer",
                ),
                retryable: true,
            });
        }
        // Fail-closed parse: anything but the documented boolean is an error.
        let enabled = parse_do_not_disturb(backend, &output.stdout)?;
        Ok(DoNotDisturbState::from_bool(enabled))
    }

    async fn write_do_not_disturb(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        enabled: bool,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);

        let backend = self.dnd_backend(ctx.observation())?;
        // Every argv element is a compile-time constant selected by `enabled`, so
        // no caller-supplied value can be read as an option.
        let plan = CommandPlan::new(
            CapabilityId::new("set_do_not_disturb"),
            "set_do_not_disturb",
            serde_json::json!({ "enabled": enabled }),
            backend.trusted_executable()?,
            write_dnd_argv(backend, enabled),
        );
        let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
        request.dispatch().await
    }

    async fn read_server_availability(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ServerAvailability, OsControlError> {
        // Cancellation is the caller's decision, not a fact about the server, so
        // it propagates instead of collapsing into `Unknown`.
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        if self.session_bus.is_none() {
            // No bus was composed: nothing was asked, so nothing is known.
            return Ok(ServerAvailability::Unknown);
        }
        // A side-effect-free identity read. A failed round trip does not
        // distinguish "no server" from "no answer", so it reports `Unknown`
        // rather than asserting an absence.
        match self.read_server_identity(ctx).await {
            Ok(_identity) => Ok(ServerAvailability::Available),
            Err(OsControlError::CancelledBeforeMutation) => {
                Err(OsControlError::CancelledBeforeMutation)
            }
            Err(_) => Ok(ServerAvailability::Unknown),
        }
    }
}
