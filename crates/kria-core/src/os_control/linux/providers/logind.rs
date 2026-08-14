//! Live `org.freedesktop.login1` D-Bus / `loginctl` adapter (raw transport
//! seam).
//!
//! linux-os-control-production **Task 2.4** — "Migrate lock, suspend,
//! hibernate, shutdown and reboot" (OSC-004, OSC-005, OSC-020), design §3,
//! §9.7 (`linux/providers/logind.rs`).
//!
//! # Host safety
//!
//! Driving the session manager (`loginctl`, or a native
//! `org.freedesktop.login1` D-Bus call) is a **raw live transport**. Like
//! [`crate::os_control::linux::providers::power_profiles`] and
//! [`crate::os_control::linux::providers::gnome_display`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in a
//!    live composition root under `os-control-live`), so no completion test can
//!    build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    read, probe, or dispatch, so a deny-live (`os-control-test`) build that
//!    reached here would trip the sentinel and abort rather than run a child
//!    process or open a session-manager transport.
//!
//! # Reads
//!
//! Both reads are **live `org.freedesktop.login1` system-bus property/method
//! calls** over the connection a live composition root opened with
//! [`LiveDbusTransport`] (see [`LiveLogind::with_bus`]). Mutations continue to
//! dispatch through the governed [`StructuredCommandRequest`], and there is no
//! ungoverned subprocess or `sudo`/privilege-escalation fallback anywhere here
//! — D-Bus/Polkit denial for these operations stays denied (OSC-004).
//!
//! An adapter composed without a system bus ([`LiveLogind::new`]) reports
//! [`OsControlError::Unavailable`] for both reads. That distinction matters
//! most for the lock state: **"the session is not locked" and "the lock state
//! could not be read" are different facts**, so an unreachable bus never
//! collapses into `Ok(false)`.
//!
//! Deny-live tests inject
//! [`crate::os_control::power::session::fake::FakePowerSessionTransport`].
//!
//! # Hibernate availability
//!
//! [`LiveLogind::hibernate_available`] is a capability probe, not a literal
//! swap-presence check: it asks
//! `org.freedesktop.login1.Manager.CanHibernate` (which itself already accounts
//! for swap, firmware, and policy) and classifies the reply through
//! [`parse_can_hibernate`], so the provider never re-implements that
//! classification.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeOperation, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::linux::structured_command::StructuredCommandRequest;
use crate::os_control::power::session::selection::{
    parse_can_hibernate, parse_scheduled_shutdown, ScheduledShutdown,
};
use crate::os_control::power::session::{PowerSessionBackend, PowerSessionTransport};
use crate::os_control::receipt::ApplyOutcome;

/// The `logind` bus name (system bus).
const LOGIND_SERVICE: &str = "org.freedesktop.login1";
/// The `logind` manager object.
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
/// The `logind` manager interface.
const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
/// The `logind` per-session interface (owns `LockedHint`).
const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";
/// The standard freedesktop property interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

/// The live `logind`/`loginctl` adapter. Constructible only in a live
/// composition; a value cannot exist under `os-control-test`.
pub struct LiveLogind {
    backend: PowerSessionBackend,
    /// The system-bus connection reads run over, when a live composition root
    /// handed this adapter one. `None` makes every read `Unavailable`; it never
    /// makes a read answer from a guess.
    system_bus: Option<zbus::Connection>,
    _seal: (),
}

impl LiveLogind {
    /// Construct in a live composition root over a selected backend, **without**
    /// a bus connection: mutations dispatch through the governed structured
    /// command, and both reads report `Unavailable` because no authoritative
    /// source is reachable. Requires a [`LiveHostAccessToken`], so no completion
    /// test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, backend: PowerSessionBackend) -> Self {
        Self {
            backend,
            system_bus: None,
            _seal: (),
        }
    }

    /// Construct over the system bus a live composition root already opened
    /// with [`LiveDbusTransport`]. This is the constructor that makes the
    /// `LockedHint` and `CanHibernate` reads real; the transport itself was
    /// opened behind the deny-live sentinel and the live token, so this adapter
    /// never opens a bus of its own.
    #[must_use]
    pub fn with_bus(
        _token: &LiveHostAccessToken,
        backend: PowerSessionBackend,
        transport: &LiveDbusTransport,
    ) -> Self {
        Self {
            backend,
            system_bus: transport.connection(BusKind::System).cloned(),
            _seal: (),
        }
    }

    /// Borrow the system bus, or fail closed. The reason names the *unknown*,
    /// never a substituted state.
    fn bus(&self) -> Result<&zbus::Connection, OsControlError> {
        self.system_bus
            .as_ref()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "no system-bus connection was composed for the logind adapter; session state is unknown, not a default",
                ),
                retryable: false,
            })
    }

    /// The service answered with something this adapter cannot interpret.
    fn protocol(&self, member: &str) -> OsControlError {
        OsControlError::ProtocolBeforeMutation {
            provider: self.provider_id(),
            operation: SafeOperation::new(member),
        }
    }

    /// One deadline- and cancellation-bounded `org.freedesktop.login1` call.
    ///
    /// The bound comes from the observation context, never from this provider:
    /// an adapter cannot grant itself more time than the admitted action has.
    /// A read takes no grant because it changes nothing.
    async fn call<A, R>(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        interface: &str,
        member: &str,
        args: &A,
    ) -> Result<R, OsControlError>
    where
        A: serde::Serialize + zbus::zvariant::DynamicType,
        R: zbus::zvariant::Type + for<'d> serde::Deserialize<'d>,
    {
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        let conn = self.bus()?;
        let call = conn.call_method(Some(LOGIND_SERVICE), path, Some(interface), member, args);
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
            reason: SafeText::new("the logind service did not answer the state read"),
            retryable: true,
        })?;
        reply
            .body()
            .deserialize::<R>()
            .map_err(|_| self.protocol(member))
    }

    /// Read one `logind` property as a typed value.
    async fn property<R>(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        interface: &str,
        property: &str,
    ) -> Result<R, OsControlError>
    where
        R: TryFrom<zbus::zvariant::OwnedValue>,
    {
        let value: zbus::zvariant::OwnedValue = self
            .call(ctx, path, PROPERTIES_IFACE, "Get", &(interface, property))
            .await?;
        R::try_from(value).map_err(|_| self.protocol(property))
    }

    /// Resolve **this process's own** login session object.
    ///
    /// `logind` keys sessions by id, so the path has to come from the session
    /// manager: guessing an id (or reading another session's state) would report
    /// — or terminate — somebody else's session. Reading the process id is not a
    /// transport.
    async fn current_session_path(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<zbus::zvariant::OwnedObjectPath, OsControlError> {
        self.call(
            ctx,
            LOGIND_MANAGER_PATH,
            LOGIND_MANAGER_IFACE,
            "GetSessionByPID",
            &(std::process::id()),
        )
        .await
        .map_err(|error| match error {
            // A process outside any login session (a system unit, a container)
            // has no session to observe. Unknown, not unlocked, and certainly not
            // a different session to act on.
            OsControlError::Unavailable { retryable, .. } => OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "this process's login session could not be resolved; the session state is unknown, not a default",
                ),
                retryable,
            },
            other => other,
        })
    }

    /// Read logind's `ScheduledShutdown` property (`(st)`) as its two typed
    /// fields. A reply of any other shape is a protocol error, never an assumed
    /// "nothing scheduled".
    async fn scheduled_shutdown_fields(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<(String, u64), OsControlError> {
        let owned: zbus::zvariant::OwnedValue = self
            .call(
                ctx,
                LOGIND_MANAGER_PATH,
                PROPERTIES_IFACE,
                "Get",
                &(LOGIND_MANAGER_IFACE, "ScheduledShutdown"),
            )
            .await?;
        let value = zbus::zvariant::Value::from(owned);
        let zbus::zvariant::Value::Structure(fields) = value else {
            return Err(self.protocol("ScheduledShutdown"));
        };
        let fields = fields.fields();
        let action = match fields.first() {
            Some(zbus::zvariant::Value::Str(text)) => text.as_str().to_string(),
            _ => return Err(self.protocol("ScheduledShutdown")),
        };
        let usec = match fields.get(1) {
            Some(zbus::zvariant::Value::U64(usec)) => *usec,
            _ => return Err(self.protocol("ScheduledShutdown")),
        };
        Ok((action, usec))
    }
}

#[async_trait::async_trait]
impl PowerSessionTransport for LiveLogind {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(format!("power-session-{}", self.backend.as_str()))
    }

    fn selected_backend(&self) -> PowerSessionBackend {
        self.backend
    }

    async fn read_locked(&self, ctx: &HostExecutionContext) -> Result<bool, OsControlError> {
        // A `LockedHint` property read opens a system-bus round trip.
        deny_live_transport(RawTransportKind::SystemBus);

        let session_path = self.current_session_path(ctx).await?;
        self.property(ctx, session_path.as_str(), LOGIND_SESSION_IFACE, "LockedHint")
            .await
    }

    async fn hibernate_available(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<bool, OsControlError> {
        // A `CanHibernate` capability query opens a system-bus round trip.
        // Never fabricate availability when the bus cannot answer.
        deny_live_transport(RawTransportKind::SystemBus);

        let reply: String = self
            .call(
                ctx,
                LOGIND_MANAGER_PATH,
                LOGIND_MANAGER_IFACE,
                "CanHibernate",
                &(),
            )
            .await?;
        parse_can_hibernate(self.backend, &reply)
    }

    async fn dispatch(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        request: &StructuredCommandRequest,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The governed request's own launch trips the deny-live sentinel; keep
        // an explicit guard here too so the adapter is unreachable under test.
        deny_live_transport(RawTransportKind::Session);
        request.dispatch().await
    }

    async fn read_current_session_id(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<String, OsControlError> {
        // Resolving the session opens a system-bus round trip.
        deny_live_transport(RawTransportKind::SystemBus);

        let session_path = self.current_session_path(ctx).await?;
        let id: String = self
            .property(ctx, session_path.as_str(), LOGIND_SESSION_IFACE, "Id")
            .await?;
        // An empty id would go on to `loginctl terminate-session ''`, so refuse it
        // here rather than dispatching against nothing.
        if id.trim().is_empty() {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "logind reported an empty session id; the session to terminate is unknown",
                ),
                retryable: false,
            });
        }
        Ok(id)
    }

    async fn read_scheduled_shutdown(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<Option<ScheduledShutdown>, OsControlError> {
        // A `ScheduledShutdown` property read opens a system-bus round trip.
        deny_live_transport(RawTransportKind::SystemBus);

        // logind's `ScheduledShutdown` is `(st)`: the action token and the
        // scheduled time in microseconds since the epoch. `("", 0)` is its
        // answer for "nothing is scheduled".
        let (action, usec) = self.scheduled_shutdown_fields(ctx).await?;
        parse_scheduled_shutdown(self.backend, &action, usec)
    }
}
