//! Live freedesktop Secret Service adapter (raw transport seam).
//!
//! linux-os-control-production **Task 1.10** (OSC-025, OSC-033) and **Task 2/§5**
//! (live reads), design §3, §9.11.
//!
//! # Host safety
//!
//! Opening the Secret Service is a **raw live transport**. Like
//! [`crate::os_control::linux::dbus`] and the broker's live native seam, this
//! adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable only in a live
//!    composition root under `os-control-live`), so no completion test can build
//!    it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before** any
//!    secret operation *and* again before every individual bus call, so if a
//!    deny-live (`os-control-test`) build ever reached here it would trip the
//!    sentinel and abort rather than touch the keyring.
//!
//! Deny-live tests use [`crate::os_control::secrets::FakeCredentialStore`]; this
//! type is unreachable there.
//!
//! # What is live
//!
//! **Reads are live** over `org.freedesktop.secrets` on the **session** bus,
//! through the already-open [`LiveDbusTransport`] connection:
//!
//! * [`CredentialStore::list_metadata`] — `Service.SearchItems` plus
//!   `Properties.GetAll` per item. Value-free: it answers *existence and
//!   metadata* questions only and **never** calls `GetSecret`.
//! * [`CredentialStore::resolve_for_operation`] — the one operation whose
//!   contract returns a value. It decides purpose/scope/expiry from value-free
//!   attributes *first*, so a mismatched or expired reference never reaches
//!   `Item.GetSecret` at all.
//!
//! **Mutations (`store`/`replace`/`delete`) are not wired yet** and fail closed
//! with [`OsControlError::Unavailable`]; there is never a plaintext fallback
//! (OSC-025.1).
//!
//! # Leak discipline (OSC-025.4, Property 25)
//!
//! A secret value is carried only by [`SecretPayload`] and only out of
//! `resolve_for_operation`. This file deliberately contains **no logging of any
//! kind**, never formats a bus reply or a D-Bus error string into an
//! [`OsControlError`] (error text is a fixed code-owned label), never digests a
//! value, and never launches a process — so no argv can carry a credential.
//!
//! # Locking
//!
//! A locked collection is a **fact to report, never something to route around**:
//! this adapter never calls `Service.Unlock`, `Collection.Unlock`, or triggers a
//! `Prompt`. Locked items are still *listed* (their attributes and label are
//! value-free metadata that a keyring exposes while locked), and a *resolution*
//! of a locked item fails closed with the actionable
//! [`crate::os_control::secrets::service_unavailable`] "unlock the keyring"
//! error.
//!
//! # Adapter conventions (chosen here, consumed by the future mutation wiring)
//!
//! * A [`SecretRef`] **is** the item's D-Bus object path under
//!   `/org/freedesktop/secrets/collection/…`. It is opaque to callers, carries no
//!   value, is stable while the item exists, and lets a resolution address the
//!   item directly instead of searching. Every reference is re-validated with
//!   [`parse_item_path`] before it is used as a path, so a caller cannot address
//!   an arbitrary object on the bus.
//! * KRIA-owned items are identified by the item attribute `xdg:schema` =
//!   [`SCHEMA_VALUE`], and carry [`ATTR_PURPOSE`], [`ATTR_SCOPE`] and the
//!   optional [`ATTR_EXPIRES`]. These are searchable, value-free attributes.
//!
//! # Residual risks (accepted, documented)
//!
//! * `Service.OpenSession` is opened with the `plain` algorithm, so a retrieved
//!   value crosses the local `AF_UNIX` session bus unencrypted. The spec's
//!   alternative (`dh-ietf1024-sha256-aes128-cbc-pkcs7`) needs a DH + AES
//!   implementation, and this task may not add dependencies. The session is
//!   closed immediately after the single `GetSecret`.
//! * The decoded bus reply holding the plaintext lives in the `zbus` message
//!   buffer until that message drops; only KRIA's own copy is zeroized (by
//!   [`SecretPayload`]).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::capability::BusKind;
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, ProviderId, SafeField, SafeOperation, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::dbus::LiveDbusTransport;
use crate::os_control::secrets::{
    now_unix, purpose_scope_mismatch, service_unavailable, unknown_reference, CredentialStore,
    ProtectedInputHandle, SecretMetadata, SecretMetadataPage, SecretPayload, SecretPurpose,
    SecretRef, SecretResolutionRequest, SecretScope, SecretServiceState, SECRET_METADATA_PAGE_CAP,
    SECRET_TOKEN_MAX_CHARS,
};

// ─────────────────────────────────────────────────────────────────────────────
// Bus vocabulary
// ─────────────────────────────────────────────────────────────────────────────

/// The well-known Secret Service bus name (session bus).
const SECRETS_SERVICE: &str = "org.freedesktop.secrets";
/// The service object path.
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
/// The service interface.
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
/// The item interface (properties + `GetSecret`).
const ITEM_IFACE: &str = "org.freedesktop.Secret.Item";
/// The session interface (`Close`).
const SESSION_IFACE: &str = "org.freedesktop.Secret.Session";
/// The standard properties interface.
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// Every addressable item path starts here.
const ITEM_PATH_PREFIX: &str = "/org/freedesktop/secrets/collection/";
/// The only session algorithm available without new dependencies (see module docs).
const SESSION_ALGORITHM: &str = "plain";
/// The collection interface, which owns item creation.
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
/// The default collection alias. Writing through the alias rather than a hardcoded
/// collection name means KRIA stores into whichever keyring the user actually uses.
const DEFAULT_COLLECTION_PATH: &str = "/org/freedesktop/secrets/aliases/default";
/// Hard per-call bound, further clamped by the observation deadline.
const MAX_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// The attribute naming the schema an item belongs to (libsecret convention).
const ATTR_SCHEMA: &str = "xdg:schema";
/// The schema value marking an item as KRIA-owned.
const SCHEMA_VALUE: &str = "dev.kria.os_control.Secret";
/// The attribute carrying the [`SecretPurpose`] token.
const ATTR_PURPOSE: &str = "kria:purpose";
/// The attribute carrying the opaque [`SecretScope`] token.
const ATTR_SCOPE: &str = "kria:scope";
/// The optional attribute carrying an expiry in unix seconds.
const ATTR_EXPIRES: &str = "kria:expires_unix";

/// Every purpose token this adapter recognises. A token outside this set is a
/// malformed item, never a silently substituted [`SecretPurpose::Other`].
const KNOWN_PURPOSES: [SecretPurpose; 5] = [
    SecretPurpose::WifiPassword,
    SecretPurpose::VpnCredential,
    SecretPurpose::ProxyCredential,
    SecretPurpose::HotspotCredential,
    SecretPurpose::Other,
];

// ─────────────────────────────────────────────────────────────────────────────
// Parsers (pure, unit-tested; `secrets/selection.rs` does not exist)
// ─────────────────────────────────────────────────────────────────────────────

/// Why a piece of Secret Service data could not be understood. Every variant is
/// a refusal: nothing is ever defaulted or substituted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretParseError {
    /// The object path is not an addressable Secret Service item path.
    ItemPath,
    /// The purpose attribute is absent or not a recognised token.
    Purpose,
    /// The scope attribute is absent, empty, or over-long.
    Scope,
    /// The expiry attribute is present but not a unix-second count.
    Expiry,
}

impl SecretParseError {
    /// A fixed, code-owned field label. Never host data.
    const fn field(self) -> &'static str {
        match self {
            Self::ItemPath => "item_path",
            Self::Purpose => "purpose",
            Self::Scope => "scope",
            Self::Expiry => "expires_unix",
        }
    }
}

/// Validate a Secret Service **item** object path and return it unchanged.
///
/// Accepts only `/org/freedesktop/secrets/collection/<collection>/<item>` (or
/// deeper), with D-Bus object-path elements (`[A-Za-z0-9_]+`). This is what makes
/// a caller-supplied [`SecretRef`] unable to address a different object on the
/// bus (a service root, a session, another service's path, or a `..` traversal),
/// and it bounds the path so a reference always round-trips through
/// [`SecretRef::new`] unchanged.
fn parse_item_path(raw: &str) -> Result<&str, SecretParseError> {
    if raw.chars().count() > SECRET_TOKEN_MAX_CHARS {
        return Err(SecretParseError::ItemPath);
    }
    let rest = raw
        .strip_prefix(ITEM_PATH_PREFIX)
        .ok_or(SecretParseError::ItemPath)?;
    let elements: Vec<&str> = rest.split('/').collect();
    // A collection root is not an item: require at least `<collection>/<item>`.
    if elements.len() < 2 {
        return Err(SecretParseError::ItemPath);
    }
    for element in elements {
        if element.is_empty()
            || !element
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(SecretParseError::ItemPath);
        }
    }
    Ok(raw)
}

/// Parse the stored purpose token. An unrecognised token fails closed.
fn parse_purpose(raw: Option<&str>) -> Result<SecretPurpose, SecretParseError> {
    let token = raw.ok_or(SecretParseError::Purpose)?;
    KNOWN_PURPOSES
        .iter()
        .copied()
        .find(|purpose| purpose.as_str() == token)
        .ok_or(SecretParseError::Purpose)
}

/// Parse the stored scope token, rejecting absent/empty/over-long values so the
/// [`SecretScope`] built from it is never a truncated near-match of the stored
/// binding.
fn parse_scope(raw: Option<&str>) -> Result<SecretScope, SecretParseError> {
    let token = raw.ok_or(SecretParseError::Scope)?.trim();
    if token.is_empty() || token.chars().count() > SECRET_TOKEN_MAX_CHARS {
        return Err(SecretParseError::Scope);
    }
    let scope = SecretScope::new(token);
    // `SecretScope::new` sanitizes; refuse anything that did not survive intact
    // rather than binding to a value the store never held.
    if scope.as_str() != token {
        return Err(SecretParseError::Scope);
    }
    Ok(scope)
}

/// Parse the optional expiry attribute.
///
/// Absent means non-expiring. **Present but unparseable is an error**, never a
/// substituted `None` — treating a malformed expiry as "never expires" would
/// resolve a credential the user believed had lapsed.
fn parse_expires(raw: Option<&str>) -> Result<Option<u64>, SecretParseError> {
    match raw {
        None => Ok(None),
        Some(text) => text
            .trim()
            .parse::<u64>()
            .map(Some)
            .map_err(|_| SecretParseError::Expiry),
    }
}

/// The `SearchItems` attribute filter: KRIA-owned items, optionally narrowed to
/// one purpose. Server-side filtering keeps the listing bounded.
fn search_attributes(purpose: Option<SecretPurpose>) -> HashMap<String, String> {
    let mut attributes = HashMap::with_capacity(2);
    attributes.insert(ATTR_SCHEMA.to_string(), SCHEMA_VALUE.to_string());
    if let Some(purpose) = purpose {
        attributes.insert(ATTR_PURPOSE.to_string(), purpose.as_str().to_string());
    }
    attributes
}

/// How a D-Bus method error reply is classified. The wire error *name* is used
/// only to choose one of these fixed outcomes; it never reaches an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DbusFailure {
    /// The collection/item is locked. Report it; never unlock as a side effect.
    Locked,
    /// The addressed object does not exist (deleted or never existed).
    NoSuchObject,
    /// The bus or the service refused the call.
    Denied,
    /// No Secret Service is running or activatable.
    ServiceAbsent,
    /// The service does not speak the expected interface.
    Protocol,
    /// Anything else: treat as a transient transport failure.
    Transport,
}

/// Classify a D-Bus error name into a fixed outcome.
fn classify_dbus_error(name: &str) -> DbusFailure {
    match name {
        "org.freedesktop.Secret.Error.IsLocked" => DbusFailure::Locked,
        "org.freedesktop.Secret.Error.NoSuchObject"
        | "org.freedesktop.DBus.Error.UnknownObject" => DbusFailure::NoSuchObject,
        "org.freedesktop.DBus.Error.AccessDenied"
        | "org.freedesktop.DBus.Error.AuthFailed"
        | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => DbusFailure::Denied,
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.Spawn.ServiceNotFound" => DbusFailure::ServiceAbsent,
        "org.freedesktop.DBus.Error.UnknownMethod"
        | "org.freedesktop.DBus.Error.UnknownInterface"
        | "org.freedesktop.DBus.Error.UnknownProperty"
        | "org.freedesktop.DBus.Error.InvalidArgs" => DbusFailure::Protocol,
        _ => DbusFailure::Transport,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal call/read error shapes
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of one bounded bus call that did not succeed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallError {
    /// The observation was cancelled before the call completed.
    Cancelled,
    /// The observation deadline elapsed.
    TimedOut,
    /// The call failed or was refused.
    Failure(DbusFailure),
}

/// The outcome of reading one item's facts, distinguishing "gone" (which each
/// caller reports differently) from every other refusal.
enum ItemReadError {
    /// The item does not exist.
    Missing,
    /// Any other fail-closed refusal, already shaped for the caller.
    Refused(OsControlError),
}

/// One item's value-free facts.
struct ItemFacts {
    /// Value-free metadata safe to return to tools/plans.
    metadata: SecretMetadata,
    /// Whether the item is currently locked (a fact, not an error).
    locked: bool,
}

/// The live Secret Service adapter. Constructible only in a live composition; a
/// value cannot exist under `os-control-test`. Every operation trips the
/// deny-live sentinel first, so it is unreachable in completion tests.
pub struct LiveSecretService {
    /// The session-bus connection from the composition root's transport, if the
    /// session bus was reachable. `None` fails every operation closed.
    session_bus: Option<zbus::Connection>,
    _seal: (),
}

impl LiveSecretService {
    /// Construct in a live composition root over the already-open D-Bus
    /// transport. Requires a [`LiveHostAccessToken`], so no completion test can
    /// build one; the session bus is borrowed from the shared transport rather
    /// than opened again here.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken, transport: &LiveDbusTransport) -> Self {
        Self {
            session_bus: transport.connection(BusKind::Session).cloned(),
            _seal: (),
        }
    }

    /// This adapter's provider identity for error attribution.
    fn provider(&self) -> ProviderId {
        ProviderId::new("secret-service")
    }


    /// The session bus, or a fail-closed refusal. Never a fallback store.
    fn bus(&self) -> Result<&zbus::Connection, OsControlError> {
        self.session_bus
            .as_ref()
            .ok_or_else(|| OsControlError::Unavailable {
                provider: Some(self.provider()),
                reason: SafeText::new(
                    "the session D-Bus is unavailable, so the Secret Service cannot be reached; no plaintext fallback exists",
                ),
                retryable: false,
            })
    }

    /// Map a classified call failure onto the closed error taxonomy.
    fn call_error(&self, error: CallError, operation: &str) -> OsControlError {
        match error {
            CallError::Cancelled => OsControlError::CancelledBeforeMutation,
            CallError::TimedOut => OsControlError::TimedOutBeforeMutation {
                operation: SafeOperation::new(operation),
                timeout_ms: u64::try_from(MAX_CALL_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
            },
            CallError::Failure(DbusFailure::Locked) => {
                service_unavailable(SecretServiceState::Locked)
            }
            CallError::Failure(DbusFailure::ServiceAbsent) => {
                service_unavailable(SecretServiceState::Unavailable)
            }
            CallError::Failure(DbusFailure::NoSuchObject) => OsControlError::Unavailable {
                provider: Some(self.provider()),
                reason: SafeText::new(
                    "the addressed secret item no longer exists; refusing to report it either way",
                ),
                retryable: true,
            },
            CallError::Failure(DbusFailure::Denied) => OsControlError::PermissionDenied {
                authority: SafeText::new("freedesktop Secret Service"),
                remediation: SafeText::new(
                    "the secret service refused this read; grant KRIA access to the keyring",
                ),
            },
            CallError::Failure(DbusFailure::Protocol) => OsControlError::ProtocolBeforeMutation {
                provider: self.provider(),
                operation: SafeOperation::new(operation),
            },
            CallError::Failure(DbusFailure::Transport) => OsControlError::Unavailable {
                provider: Some(self.provider()),
                reason: SafeText::new("the Secret Service call failed on the session bus"),
                retryable: true,
            },
        }
    }

    /// A malformed reply: fail closed rather than parse a shape we do not know.
    fn malformed_reply(&self, operation: &str) -> OsControlError {
        OsControlError::ProtocolBeforeMutation {
            provider: self.provider(),
            operation: SafeOperation::new(operation),
        }
    }

    /// A stored item whose value-free metadata cannot be understood. The field
    /// label is a fixed code-owned string, never host data.
    fn malformed_item(&self, error: SecretParseError) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(self.provider()),
            reason: SafeText::new(format!(
                "a secret item's '{}' metadata is malformed; refusing a partial or guessed reading",
                error.field()
            )),
            retryable: false,
        }
    }

    /// Run one bounded, cancellable Secret Service call on the session bus.
    ///
    /// The deadline and cancellation come from the live observation context, so a
    /// provider cannot grant itself more time, and the per-call bound is clamped
    /// to [`MAX_CALL_TIMEOUT`]. A D-Bus error reply is classified by name only;
    /// neither the name nor the description is ever surfaced.
    async fn call<B>(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message, CallError>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType + Sync,
    {
        // Every individual bus call re-arms the guard, so no path into this
        // adapter can reach the session bus under a deny-live build.
        deny_live_transport(RawTransportKind::SessionBus);

        if ctx.cancellation.is_cancelled() {
            return Err(CallError::Cancelled);
        }
        let remaining = ctx.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CallError::TimedOut);
        }
        let bound = remaining.min(MAX_CALL_TIMEOUT);

        let connection = self.session_bus.as_ref().ok_or(CallError::Failure(
            DbusFailure::ServiceAbsent,
        ))?;
        let call = connection.call_method(
            Some(SECRETS_SERVICE),
            path,
            Some(interface),
            method,
            body,
        );

        tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => Err(CallError::Cancelled),
            result = tokio::time::timeout(bound, call) => match result {
                Err(_elapsed) => Err(CallError::TimedOut),
                Ok(Err(zbus::Error::MethodError(name, _description, _reply))) => {
                    Err(CallError::Failure(classify_dbus_error(name.as_str())))
                }
                Ok(Err(_other)) => Err(CallError::Failure(DbusFailure::Transport)),
                Ok(Ok(reply)) => Ok(reply),
            },
        }
    }

    /// Read one item's value-free facts with a single `Properties.GetAll`.
    ///
    /// This is the only metadata read in the adapter and it never calls
    /// `GetSecret`: existence, purpose, scope, label, creation time and lock
    /// state are all answerable without retrieving a value.
    async fn read_item(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
    ) -> Result<ItemFacts, ItemReadError> {
        const OPERATION: &str = "read_secret_metadata";

        let reply = self
            .call(ctx, path, PROPERTIES_IFACE, "GetAll", &ITEM_IFACE)
            .await
            .map_err(|error| match error {
                CallError::Failure(DbusFailure::NoSuchObject) => ItemReadError::Missing,
                other => ItemReadError::Refused(self.call_error(other, OPERATION)),
            })?;

        let mut properties = reply
            .body()
            .deserialize::<HashMap<String, OwnedValue>>()
            .map_err(|_| ItemReadError::Refused(self.malformed_reply(OPERATION)))?;

        let attributes: HashMap<String, String> = properties
            .remove("Attributes")
            .and_then(|value| HashMap::try_from(value).ok())
            .ok_or_else(|| {
                ItemReadError::Refused(self.malformed_item(SecretParseError::Purpose))
            })?;
        let label: String = properties
            .remove("Label")
            .and_then(|value| String::try_from(value).ok())
            .ok_or_else(|| ItemReadError::Refused(self.malformed_reply(OPERATION)))?;
        let created_unix: u64 = properties
            .remove("Created")
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| ItemReadError::Refused(self.malformed_reply(OPERATION)))?;
        let locked: bool = properties
            .remove("Locked")
            .and_then(|value| bool::try_from(value).ok())
            .ok_or_else(|| ItemReadError::Refused(self.malformed_reply(OPERATION)))?;

        let purpose = parse_purpose(attributes.get(ATTR_PURPOSE).map(String::as_str))
            .map_err(|error| ItemReadError::Refused(self.malformed_item(error)))?;
        let scope = parse_scope(attributes.get(ATTR_SCOPE).map(String::as_str))
            .map_err(|error| ItemReadError::Refused(self.malformed_item(error)))?;
        let expires_unix = parse_expires(attributes.get(ATTR_EXPIRES).map(String::as_str))
            .map_err(|error| ItemReadError::Refused(self.malformed_item(error)))?;

        // The reference is the validated path; refuse if it would not round-trip,
        // because a truncated reference could never be resolved again.
        let reference = SecretRef::new(path);
        if reference.as_str() != path {
            return Err(ItemReadError::Refused(
                self.malformed_item(SecretParseError::ItemPath),
            ));
        }

        Ok(ItemFacts {
            metadata: SecretMetadata {
                reference,
                purpose,
                scope,
                label: SafeText::new(label),
                created_unix,
                expires_unix,
            },
            locked,
        })
    }

    /// Open a `plain` session for exactly one retrieval (see module docs).
    async fn open_session(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<OwnedObjectPath, OsControlError> {
        const OPERATION: &str = "open_secret_session";

        let reply = self
            .call(
                ctx,
                SERVICE_PATH,
                SERVICE_IFACE,
                "OpenSession",
                &(SESSION_ALGORITHM, Value::new("")),
            )
            .await
            .map_err(|error| self.call_error(error, OPERATION))?;
        let (_output, session) = reply
            .body()
            .deserialize::<(OwnedValue, OwnedObjectPath)>()
            .map_err(|_| self.malformed_reply(OPERATION))?;
        Ok(session)
    }

    /// Close the retrieval session immediately. Best effort: the value has
    /// already been taken, and a failure to close changes no host state.
    async fn close_session(&self, ctx: &HostExecutionContext, session: &OwnedObjectPath) {
        let _ = self
            .call(ctx, session.as_str(), SESSION_IFACE, "Close", &())
            .await;
    }

    /// Retrieve the value for one already-validated item over `session`.
    ///
    /// The reply's session path is checked against ours: a value returned for a
    /// different session could be encrypted for someone else's parameters, so it
    /// is refused rather than interpreted.
    async fn get_secret(
        &self,
        ctx: &HostExecutionContext,
        path: &str,
        session: &OwnedObjectPath,
    ) -> Result<SecretPayload, OsControlError> {
        const OPERATION: &str = "read_secret_value";

        let reply = self
            .call(ctx, path, ITEM_IFACE, "GetSecret", session)
            .await
            .map_err(|error| self.call_error(error, OPERATION))?;
        let (reply_session, _parameters, value, _content_type) = reply
            .body()
            .deserialize::<(OwnedObjectPath, Vec<u8>, Vec<u8>, String)>()
            .map_err(|_| self.malformed_reply(OPERATION))?;
        if reply_session != *session {
            return Err(self.malformed_reply(OPERATION));
        }
        Ok(SecretPayload::new(value))
    }
    /// Create or overwrite one item in the default collection.
    ///
    /// # Why the value goes straight into the call body
    ///
    /// The Secret Service API takes the value inline in `CreateItem`, so it never
    /// becomes an argv element and never appears in `/proc/<pid>/cmdline`. The
    /// plaintext exists only inside the call body for the duration of the call:
    /// it is not logged, not digested, and not retained in the returned metadata.
    ///
    /// # Prompts
    ///
    /// A non-root `prompt` path in the reply means the backend wants user
    /// interaction (typically an unlock). That is **reported**, never auto-driven:
    /// silently unlocking a keyring on the user's behalf would defeat the point of
    /// it being locked.
    async fn create_item(
        &self,
        ctx: &HostExecutionContext,
        attributes: HashMap<String, String>,
        label: &str,
        value: &SecretPayload,
        replace: bool,
    ) -> Result<OwnedObjectPath, OsControlError> {
        const OPERATION: &str = "write_secret_item";

        // A write needs its own session, because the value is transported inside
        // it. `plain` matches the read path; both are local-socket only.
        let session = self.open_session(ctx).await?;

        let mut properties: HashMap<&str, Value<'_>> = HashMap::with_capacity(2);
        properties.insert("org.freedesktop.Secret.Item.Label", Value::new(label));
        properties.insert(
            "org.freedesktop.Secret.Item.Attributes",
            Value::new(attributes),
        );
        // `(session, parameters, value, content_type)` — the wire shape of a
        // Secret. `parameters` is empty for the `plain` algorithm.
        let secret = (
            session.clone(),
            Vec::<u8>::new(),
            value.expose_secret().to_vec(),
            "text/plain".to_string(),
        );

        let reply = self
            .call(
                ctx,
                DEFAULT_COLLECTION_PATH,
                COLLECTION_IFACE,
                "CreateItem",
                &(properties, secret, replace),
            )
            .await;
        // Close the session whatever happened: leaving it open would leak a
        // transport handle for the lifetime of the connection.
        self.close_session(ctx, &session).await;

        let reply = reply.map_err(|error| self.call_error(error, OPERATION))?;
        let (item, prompt) = reply
            .body()
            .deserialize::<(OwnedObjectPath, OwnedObjectPath)>()
            .map_err(|_| self.malformed_reply(OPERATION))?;

        if prompt.as_str() != "/" {
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider()),
                reason: SafeText::new(
                    "the keyring requires an interactive unlock; KRIA will not unlock it on your behalf",
                ),
                retryable: true,
            });
        }
        // An empty item path means the backend accepted the call but created
        // nothing — reporting success would claim a credential is stored when it
        // is not.
        if item.as_str() == "/" {
            return Err(self.malformed_reply(OPERATION));
        }
        Ok(item)
    }
}

#[async_trait::async_trait]
impl CredentialStore for LiveSecretService {
    async fn list_metadata(
        &self,
        ctx: &HostExecutionContext,
        purpose: Option<SecretPurpose>,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<SecretMetadataPage, OsControlError> {
        deny_live_transport(RawTransportKind::Secret);
        const OPERATION: &str = "list_secret_metadata";

        // Fail closed before any bus traffic if the session bus never opened.
        self.bus()?;

        let cap = (limit as usize).clamp(1, SECRET_METADATA_PAGE_CAP);
        let after = match cursor {
            Some(raw) => Some(
                parse_item_path(raw)
                    .map_err(|_| OsControlError::InvalidRequest {
                        field: SafeField::new("cursor"),
                        reason: SafeText::new("the page cursor is not a valid secret item cursor"),
                    })?
                    .to_string(),
            ),
            None => None,
        };

        let reply = self
            .call(
                ctx,
                SERVICE_PATH,
                SERVICE_IFACE,
                "SearchItems",
                &search_attributes(purpose),
            )
            .await
            .map_err(|error| self.call_error(error, OPERATION))?;
        let (unlocked, locked) = reply
            .body()
            .deserialize::<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)>()
            .map_err(|_| self.malformed_reply(OPERATION))?;

        // A locked item still exists, and its attributes/label are value-free, so
        // it belongs in the listing. Nothing here unlocks anything.
        let mut paths: Vec<String> = Vec::with_capacity(unlocked.len() + locked.len());
        for path in unlocked.iter().chain(locked.iter()) {
            let validated = parse_item_path(path.as_str())
                .map_err(|error| self.malformed_item(error))?;
            paths.push(validated.to_string());
        }
        // Deterministic order so the cursor is stable across pages.
        paths.sort();
        paths.dedup();

        let mut window: Vec<String> = paths
            .into_iter()
            .filter(|path| {
                after
                    .as_ref()
                    .is_none_or(|cursor| path.as_str() > cursor.as_str())
            })
            .collect();
        let has_more = window.len() > cap;
        window.truncate(cap);

        let mut items = Vec::with_capacity(window.len());
        for path in &window {
            let facts = self.read_item(ctx, path).await.map_err(|error| match error {
                // Losing a race with a concurrent delete is a fact about the
                // read, not a reason to silently shorten the page.
                ItemReadError::Missing => OsControlError::Unavailable {
                    provider: Some(self.provider()),
                    reason: SafeText::new(
                        "a secret item was removed while listing; retry for a consistent page",
                    ),
                    retryable: true,
                },
                ItemReadError::Refused(error) => error,
            })?;
            items.push(facts.metadata);
        }

        // The purpose filter is applied by the service's own search. Verify it
        // held: a backend that ignored the filter must fail rather than widen the
        // page beyond what was asked for.
        if let Some(requested) = purpose {
            if items.iter().any(|item| item.purpose != requested) {
                return Err(OsControlError::Unavailable {
                    provider: Some(self.provider()),
                    reason: SafeText::new(
                        "the secret service ignored the purpose filter; refusing an unfiltered listing",
                    ),
                    retryable: false,
                });
            }
        }

        let next_cursor = if has_more {
            window.last().map(SafeText::new)
        } else {
            None
        };

        Ok(SecretMetadataPage {
            items: BoundedVec::from_iter_capped(items, cap),
            next_cursor,
        })
    }

    async fn store(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        purpose: SecretPurpose,
        scope: SecretScope,
        label: SafeText,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError> {
        deny_live_transport(RawTransportKind::Secret);
        self.bus()?;

        if input.len() == 0 {
            // An empty secret is never stored: it would later read back as a
            // present-but-useless credential.
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("secret"),
                reason: SafeText::new("an empty secret is never stored"),
            });
        }

        let observation = ctx.observation();
        // Attributes are the same value-free set the read path searches on, so an
        // item written here is findable by `list_metadata` and can never collide
        // with another application's items.
        let mut attributes = search_attributes(Some(purpose));
        attributes.insert(ATTR_SCOPE.to_string(), scope.as_str().to_string());

        let payload = input.into_payload();
        let item = self
            .create_item(
                observation,
                attributes,
                label.as_str(),
                &payload,
                // `replace: true` — purpose+scope identify one logical credential,
                // so storing again updates it rather than accumulating duplicates
                // that a later read would have to disambiguate.
                true,
            )
            .await?;

        Ok(SecretMetadata {
            reference: SecretRef::new(item.as_str()),
            purpose,
            scope,
            label,
            created_unix: now_unix(),
            expires_unix: None,
        })
    }

    async fn replace(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError> {
        deny_live_transport(RawTransportKind::Secret);
        self.bus()?;

        if input.len() == 0 {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("secret"),
                reason: SafeText::new("an empty secret is never stored"),
            });
        }

        let observation = ctx.observation();
        // The path is validated before use: a reference outside the item tree
        // could otherwise address an arbitrary bus object.
        let path = parse_item_path(reference.as_str()).map_err(|error| self.malformed_item(error))?;

        // Read the existing item's own attributes first. Rewriting it with
        // freshly-invented attributes would silently re-key the credential, so a
        // later read would no longer find it.
        let facts = self
            .read_item(observation, path)
            .await
            .map_err(|error| match error {
                // A reference that no longer resolves is not "replaced with
                // nothing" — it is a missing target, and must be reported.
                ItemReadError::Missing => OsControlError::InvalidRequest {
                    field: SafeField::new("reference"),
                    reason: SafeText::new("the secret to replace no longer exists"),
                },
                ItemReadError::Refused(error) => error,
            })?;

        let mut attributes = search_attributes(Some(facts.metadata.purpose));
        attributes.insert(ATTR_SCOPE.to_string(), facts.metadata.scope.as_str().to_string());
        if let Some(expires) = facts.metadata.expires_unix {
            attributes.insert(ATTR_EXPIRES.to_string(), expires.to_string());
        }

        let payload = input.into_payload();
        let item = self
            .create_item(
                observation,
                attributes,
                facts.metadata.label.as_str(),
                &payload,
                true,
            )
            .await?;

        Ok(SecretMetadata {
            reference: SecretRef::new(item.as_str()),
            purpose: facts.metadata.purpose,
            scope: facts.metadata.scope,
            label: facts.metadata.label,
            created_unix: now_unix(),
            expires_unix: facts.metadata.expires_unix,
        })
    }

    async fn delete(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
    ) -> Result<(), OsControlError> {
        const OPERATION: &str = "delete_secret";

        deny_live_transport(RawTransportKind::Secret);
        self.bus()?;

        let observation = ctx.observation();
        let path = parse_item_path(reference.as_str()).map_err(|error| self.malformed_item(error))?;

        let reply = self
            .call(observation, path, ITEM_IFACE, "Delete", &())
            .await
            .map_err(|error| self.call_error(error, OPERATION))?;
        let prompt = reply
            .body()
            .deserialize::<OwnedObjectPath>()
            .map_err(|_| self.malformed_reply(OPERATION))?;
        if prompt.as_str() != "/" {
            // Deleting needs an unlock. Report it rather than driving the prompt.
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider()),
                reason: SafeText::new(
                    "the keyring requires an interactive unlock before the item can be deleted",
                ),
                retryable: true,
            });
        }
        Ok(())
    }

    async fn resolve_for_operation(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError> {
        deny_live_transport(RawTransportKind::Secret);

        let observation = ctx.observation();
        self.bus()?;

        // 1. A reference is only ever used as a path after validation, so a
        //    resolution cannot address an arbitrary object on the bus.
        let path = parse_item_path(request.reference.as_str()).map_err(|_| {
            OsControlError::InvalidRequest {
                field: SafeField::new("secret.reference"),
                reason: SafeText::new("the secret reference is not a valid secret item reference"),
            }
        })?;

        // 2. Decide from value-free metadata first: a mismatched, expired, or
        //    locked item never reaches `GetSecret` at all.
        let facts = self
            .read_item(observation, path)
            .await
            .map_err(|error| match error {
                ItemReadError::Missing => unknown_reference(),
                ItemReadError::Refused(error) => error,
            })?;
        if facts.metadata.purpose != request.purpose || facts.metadata.scope != request.scope {
            return Err(purpose_scope_mismatch());
        }
        if facts.metadata.is_expired(now_unix()) {
            // Indistinguishable from absent on purpose: an expired binding is not
            // resolvable, and the caller learns nothing else about it.
            return Err(unknown_reference());
        }
        if facts.locked {
            // A locked keyring is a fact to report. Never unlock as a side effect
            // of a read, and never prompt.
            return Err(service_unavailable(SecretServiceState::Locked));
        }

        // 3. Retrieve for the minimum duration: open, read, close.
        let session = self.open_session(observation).await?;
        let payload = self.get_secret(observation, path, &session).await;
        self.close_session(observation, &session).await;
        payload
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    // Every fixture below is an obviously fake placeholder. No parser in this
    // module ever handles a secret value, so no test needs one.
    const ITEM: &str = "/org/freedesktop/secrets/collection/login/1";

    #[test]
    fn item_path_accepts_a_normal_item() {
        assert_eq!(parse_item_path(ITEM), Ok(ITEM));
    }

    #[test]
    fn item_path_accepts_underscored_collection_and_deeper_layouts() {
        // Real backends differ in item id shape (numeric in gnome-keyring,
        // name-like elsewhere) and may nest deeper.
        let underscored = "/org/freedesktop/secrets/collection/kria_store/item_7";
        assert_eq!(parse_item_path(underscored), Ok(underscored));
        let nested = "/org/freedesktop/secrets/collection/login/group/9";
        assert_eq!(parse_item_path(nested), Ok(nested));
    }

    #[test]
    fn item_path_rejects_a_collection_root() {
        // A collection is not an item; addressing one would read the wrong object.
        assert_eq!(
            parse_item_path("/org/freedesktop/secrets/collection/login"),
            Err(SecretParseError::ItemPath)
        );
    }

    #[test]
    fn item_path_rejects_paths_outside_the_item_tree() {
        for raw in [
            "",
            "/",
            "/org/freedesktop/secrets",
            "/org/freedesktop/secrets/session/s1",
            "/org/freedesktop/secrets/aliases/default",
            "/org/freedesktop/NetworkManager/Settings/1",
            "org/freedesktop/secrets/collection/login/1",
        ] {
            assert_eq!(
                parse_item_path(raw),
                Err(SecretParseError::ItemPath),
                "must refuse {raw}"
            );
        }
    }

    #[test]
    fn item_path_rejects_traversal_and_invalid_elements() {
        for raw in [
            "/org/freedesktop/secrets/collection/login/../../evil",
            "/org/freedesktop/secrets/collection//1",
            "/org/freedesktop/secrets/collection/login/1/",
            "/org/freedesktop/secrets/collection/log in/1",
            "/org/freedesktop/secrets/collection/login/1-2",
        ] {
            assert_eq!(
                parse_item_path(raw),
                Err(SecretParseError::ItemPath),
                "must refuse {raw}"
            );
        }
    }

    #[test]
    fn item_path_rejects_an_unbounded_path() {
        let long = format!("{ITEM_PATH_PREFIX}login/{}", "1".repeat(SECRET_TOKEN_MAX_CHARS));
        assert_eq!(parse_item_path(&long), Err(SecretParseError::ItemPath));
    }

    #[test]
    fn a_validated_path_round_trips_through_secret_ref() {
        // Resolution depends on this: a reference that does not survive
        // sanitization could never address its item again.
        let validated = parse_item_path(ITEM).expect("valid item path");
        assert_eq!(SecretRef::new(validated).as_str(), ITEM);
    }

    #[test]
    fn purpose_parses_every_known_token() {
        for purpose in KNOWN_PURPOSES {
            assert_eq!(parse_purpose(Some(purpose.as_str())), Ok(purpose));
        }
    }

    #[test]
    fn purpose_refuses_an_unrecognised_token_instead_of_defaulting() {
        // Mandatory: an unknown purpose must never fall back to `Other`, which
        // would let a credential be used for an unrelated operation.
        for raw in ["WIFI_PASSWORD", "wifi password", "root_password", "", "othe"] {
            assert_eq!(parse_purpose(Some(raw)), Err(SecretParseError::Purpose));
        }
        assert_eq!(parse_purpose(None), Err(SecretParseError::Purpose));
    }

    #[test]
    fn scope_parses_a_normal_token() {
        let scope = parse_scope(Some("wifi:profile-placeholder")).expect("valid scope");
        assert_eq!(scope.as_str(), "wifi:profile-placeholder");
    }

    #[test]
    fn scope_refuses_absent_empty_and_unbounded_tokens() {
        assert_eq!(parse_scope(None), Err(SecretParseError::Scope));
        assert_eq!(parse_scope(Some("   ")), Err(SecretParseError::Scope));
        let long = "s".repeat(SECRET_TOKEN_MAX_CHARS + 1);
        assert_eq!(parse_scope(Some(&long)), Err(SecretParseError::Scope));
    }

    #[test]
    fn scope_refuses_a_token_that_would_not_survive_sanitization() {
        // A control character would be stripped, leaving a near-match that could
        // satisfy the exact scope comparison against the wrong binding.
        assert_eq!(
            parse_scope(Some("wifi:pro\u{7}file")),
            Err(SecretParseError::Scope)
        );
    }

    #[test]
    fn expiry_absent_means_non_expiring() {
        assert_eq!(parse_expires(None), Ok(None));
    }

    #[test]
    fn expiry_parses_a_unix_second_count() {
        assert_eq!(parse_expires(Some("1893456000")), Ok(Some(1_893_456_000)));
        // Real writers pad values; a trimmed count is still an exact reading.
        assert_eq!(parse_expires(Some("  1893456000 ")), Ok(Some(1_893_456_000)));
        assert_eq!(parse_expires(Some("0")), Ok(Some(0)));
    }

    #[test]
    fn expiry_refuses_unrecognised_output_instead_of_defaulting() {
        // Mandatory: a malformed expiry must not be read as "never expires".
        for raw in ["", "never", "-1", "1893456000.0", "1893456000s", "0x10"] {
            assert_eq!(
                parse_expires(Some(raw)),
                Err(SecretParseError::Expiry),
                "must refuse {raw}"
            );
        }
    }

    #[test]
    fn search_attributes_scope_the_listing_to_kria_items() {
        let all = search_attributes(None);
        assert_eq!(all.get(ATTR_SCHEMA).map(String::as_str), Some(SCHEMA_VALUE));
        assert_eq!(all.len(), 1);

        let filtered = search_attributes(Some(SecretPurpose::WifiPassword));
        assert_eq!(
            filtered.get(ATTR_PURPOSE).map(String::as_str),
            Some("wifi_password")
        );
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn dbus_errors_classify_locked_absent_and_missing_distinctly() {
        assert_eq!(
            classify_dbus_error("org.freedesktop.Secret.Error.IsLocked"),
            DbusFailure::Locked
        );
        assert_eq!(
            classify_dbus_error("org.freedesktop.Secret.Error.NoSuchObject"),
            DbusFailure::NoSuchObject
        );
        assert_eq!(
            classify_dbus_error("org.freedesktop.DBus.Error.ServiceUnknown"),
            DbusFailure::ServiceAbsent
        );
        assert_eq!(
            classify_dbus_error("org.freedesktop.DBus.Error.AccessDenied"),
            DbusFailure::Denied
        );
        assert_eq!(
            classify_dbus_error("org.freedesktop.DBus.Error.UnknownMethod"),
            DbusFailure::Protocol
        );
    }

    #[test]
    fn an_unrecognised_dbus_error_is_still_a_failure() {
        // Unknown names stay failures — never success, never a substituted value.
        assert_eq!(
            classify_dbus_error("com.example.Whatever"),
            DbusFailure::Transport
        );
        assert_eq!(classify_dbus_error(""), DbusFailure::Transport);
    }
}
