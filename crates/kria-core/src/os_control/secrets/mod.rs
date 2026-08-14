//! `os_control::secrets` — opaque credential references, the non-leaking secret
//! payload wrapper, and the provider-only [`CredentialStore`] contract.
//!
//! linux-os-control-production **Task 1.10** — "Implement Secret Service and
//! sandbox-grant foundation" (OSC-007, OSC-025, OSC-029), design §4, §9.11, §14
//! and Correctness Properties 25 / 29.
//!
//! # What this module owns
//!
//! KRIA never persists a plaintext credential. Instead it stores an **opaque
//! reference** ([`SecretRef`]) plus non-sensitive [`SecretMetadata`] (purpose,
//! scope, expiry) in the freedesktop Secret Service, and exchanges only those
//! references through tools and plans (OSC-025.2). The actual secret bytes are
//! obtained **only inside a bound provider**, for its bound purpose and scope,
//! through [`CredentialStore::resolve_for_operation`] under a sealed
//! [`AdmittedMutationContext`] (OSC-025.3). They are never returned to the
//! model, a tool result, a plan, an approval projection, or the audit log.
//!
//! # Leak-proofing (OSC-025.4, Property 25)
//!
//! [`SecretPayload`] is the sole carrier of secret bytes and is deliberately
//! hostile to leakage:
//!
//! * it does **not** implement `Serialize`, `Display`, or `Clone`, so it cannot
//!   be serialized into a DTO, formatted into a message, or duplicated into a
//!   log (compile-fail doctests below prove each of these);
//! * its manual [`std::fmt::Debug`] prints only a fixed redacted placeholder —
//!   never the value or even its length;
//! * it zeroizes its buffer on drop (via [`zeroize::Zeroizing`]), keeping the
//!   plaintext in memory for the minimum operation duration (OSC-025.5).
//!
//! # Fail-closed (OSC-025.1/.7)
//!
//! A locked or unavailable Secret Service produces an actionable
//! [`OsControlError::Unavailable`] with **no plaintext fallback** and no
//! password interception. A resolution whose purpose/scope does not match the
//! stored metadata fails closed with [`OsControlError::PolicyDenied`].

use std::time::{SystemTime, UNIX_EPOCH};

use zeroize::Zeroizing;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{BoundedVec, Digest, SafeField, SafeText};
use crate::os_control::error::OsControlError;

/// Maximum length (chars) of an opaque [`SecretRef`] / [`SecretScope`] token.
pub const SECRET_TOKEN_MAX_CHARS: usize = 160;
/// Maximum bytes a single secret payload may carry (bounds every buffer).
pub const SECRET_PAYLOAD_MAX_BYTES: usize = 8 * 1024;
/// Default page size for [`CredentialStore::list_metadata`].
pub const SECRET_METADATA_PAGE_CAP: usize = 128;

// ─────────────────────────────────────────────────────────────────────────────
// Non-leaking, zeroizing secret payload wrapper (OSC-025.4/.5, Property 25)
// ─────────────────────────────────────────────────────────────────────────────

/// The one carrier of raw secret bytes. **Never** serializes, displays, clones,
/// or debug-prints its value; zeroizes on drop.
///
/// The bytes are obtained **only** inside a bound provider through
/// [`CredentialStore::resolve_for_operation`] and are used for the minimum
/// operation duration. There is exactly one way to read them
/// ([`SecretPayload::expose_secret`]), which is documented as provider-only.
///
/// # Cannot serialize
///
/// ```compile_fail
/// use kria_core::os_control::secrets::SecretPayload;
/// fn leak(p: &SecretPayload) -> String {
///     serde_json::to_string(p).unwrap() // error: `SecretPayload: Serialize` is not satisfied
/// }
/// ```
///
/// # Cannot clone (never duplicated into a log)
///
/// ```compile_fail
/// use kria_core::os_control::secrets::SecretPayload;
/// fn dup(p: &SecretPayload) -> SecretPayload {
///     p.clone() // error: `SecretPayload` does not implement `Clone`
/// }
/// ```
///
/// # Cannot `Display`
///
/// ```compile_fail
/// use kria_core::os_control::secrets::SecretPayload;
/// fn show(p: &SecretPayload) -> String {
///     format!("{p}") // error: `SecretPayload` does not implement `Display`
/// }
/// ```
pub struct SecretPayload {
    /// Zeroized on drop. Bounded at construction to [`SECRET_PAYLOAD_MAX_BYTES`].
    bytes: Zeroizing<Vec<u8>>,
}

impl SecretPayload {
    /// Wrap raw secret bytes, truncating to [`SECRET_PAYLOAD_MAX_BYTES`]. This is
    /// the only constructor; callers obtain bytes from a protected input channel
    /// or the Secret Service transport, never from a plan/DTO.
    #[must_use]
    pub fn new(mut bytes: Vec<u8>) -> Self {
        if bytes.len() > SECRET_PAYLOAD_MAX_BYTES {
            bytes.truncate(SECRET_PAYLOAD_MAX_BYTES);
        }
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// **Provider-only** access to the raw secret bytes. Named `expose_secret`
    /// so any call site is auditable; must be used only inside the bound
    /// provider for the bound operation and never copied into a DTO/log.
    #[must_use]
    pub fn expose_secret(&self) -> &[u8] {
        &self.bytes
    }

    /// The length of the secret in bytes. Metadata only — callers must not treat
    /// this as, or surface it alongside, the value.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl std::fmt::Debug for SecretPayload {
    /// Prints a fixed redacted placeholder — never the value or its length, so a
    /// secret can never reach a trace/error/panic message via `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretPayload(<redacted>)")
    }
}

/// An ephemeral protected-input handle carrying a to-be-stored secret. Wraps a
/// [`SecretPayload`], so a store/replace call receives the value through the
/// protected channel rather than as a plan/DTO field (OSC-015.3, OSC-025.2). It
/// inherits the payload's non-`Serialize` / non-`Clone` / non-leaking `Debug`.
pub struct ProtectedInputHandle {
    payload: SecretPayload,
}

impl ProtectedInputHandle {
    /// Wrap an already-obtained payload from a protected input channel.
    #[must_use]
    pub fn new(payload: SecretPayload) -> Self {
        Self { payload }
    }

    /// Wrap raw protected-input bytes.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::new(SecretPayload::new(bytes))
    }

    /// Consume the handle, yielding the payload for the store transport. The
    /// handle is single-use: the value moves out and cannot be read twice.
    #[must_use]
    pub fn into_payload(self) -> SecretPayload {
        self.payload
    }

    /// The length of the protected input (metadata only).
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// Whether the protected input is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl std::fmt::Debug for ProtectedInputHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProtectedInputHandle(<redacted>)")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Opaque references, purpose, and scope (OSC-025.2)
// ─────────────────────────────────────────────────────────────────────────────

/// Sanitize + bound an opaque token: strip control characters, cap length.
fn sanitize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(SECRET_TOKEN_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= SECRET_TOKEN_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// An opaque identifier to a credential held by the system secret service
/// (OSC-025.2, the design `Secret_Reference`). It is *only* an identifier: it
/// carries no secret value and is safe to place in tools/plans/metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretRef(String);

impl SecretRef {
    /// Construct from a raw string (bounded, control-char stripped).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(sanitize_token(&raw.into()))
    }

    /// Borrow the opaque reference string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A correlation-safe digest of the reference for audit/trace (never the
    /// value; the reference itself is opaque).
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&self.0)
    }
}

impl std::fmt::Display for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for SecretRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SecretRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(d)?))
    }
}

/// The closed set of credential purposes a secret may serve (OSC-025.2). A
/// resolution must name the same purpose the secret was stored under, so a
/// credential cannot be used for an unrelated operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SecretPurpose {
    /// A Wi-Fi network profile password.
    WifiPassword,
    /// An existing VPN profile credential.
    VpnCredential,
    /// A recognized desktop proxy credential.
    ProxyCredential,
    /// A generated hotspot credential (v2).
    HotspotCredential,
    /// Any other bounded credential purpose not covered above.
    Other,
}

impl SecretPurpose {
    /// The stable snake_case token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WifiPassword => "wifi_password",
            Self::VpnCredential => "vpn_credential",
            Self::ProxyCredential => "proxy_credential",
            Self::HotspotCredential => "hotspot_credential",
            Self::Other => "other",
        }
    }
}

/// The scope a secret is bound to (e.g. a specific network profile). A
/// resolution must name the same scope the secret was stored under. The scope
/// string is opaque and bounded; equality is exact so a credential cannot be
/// resolved for a different profile/device.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretScope(String);

impl SecretScope {
    /// Construct a bounded, control-char-free scope token.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self(sanitize_token(&raw.into()))
    }

    /// Borrow the scope token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A correlation-safe digest of the scope (safe for audit/trace).
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&self.0)
    }
}

impl serde::Serialize for SecretScope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for SecretScope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(String::deserialize(d)?))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Metadata DTOs (opaque; value-free) — OSC-025.2
// ─────────────────────────────────────────────────────────────────────────────

/// Non-sensitive metadata about a stored secret. Contains **no** value: only an
/// opaque reference, its purpose, scope, a redacted label, and expiry. Safe to
/// return to tools/plans and to audit as digests (OSC-025.2, OSC-007.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SecretMetadata {
    /// The opaque reference identifying this secret.
    pub reference: SecretRef,
    /// The credential purpose.
    pub purpose: SecretPurpose,
    /// The scope the secret is bound to.
    pub scope: SecretScope,
    /// A redacted human-safe label (never the value).
    pub label: SafeText,
    /// Creation time (unix seconds).
    pub created_unix: u64,
    /// Optional expiry time (unix seconds); `None` means non-expiring.
    pub expires_unix: Option<u64>,
}

impl SecretMetadata {
    /// Whether the secret has expired at `now_unix`.
    #[must_use]
    pub fn is_expired(&self, now_unix: u64) -> bool {
        self.expires_unix.is_some_and(|e| now_unix >= e)
    }
}

/// A bounded page of secret metadata for [`CredentialStore::list_metadata`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SecretMetadataPage {
    /// The metadata items (value-free), bounded.
    pub items: BoundedVec<SecretMetadata>,
    /// Opaque cursor for the next page, when more remain.
    pub next_cursor: Option<SafeText>,
}

/// A provider's request to resolve a secret for a bound operation. It names the
/// exact reference, purpose, and scope; the store rejects any mismatch against
/// the stored metadata (OSC-025.3, Property 25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretResolutionRequest {
    /// The opaque reference to resolve.
    pub reference: SecretRef,
    /// The purpose the resolution is for; must match the stored purpose.
    pub purpose: SecretPurpose,
    /// The scope the resolution is for; must match the stored scope.
    pub scope: SecretScope,
}

/// The current lock/availability state of the secret service backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceState {
    /// Unlocked and reachable.
    Available,
    /// The collection is locked; resolution/mutation fail closed (OSC-025.7).
    Locked,
    /// The service is unreachable/absent; fail closed with no fallback.
    Unavailable,
}

// ─────────────────────────────────────────────────────────────────────────────
// The CredentialStore contract (design §9.11)
// ─────────────────────────────────────────────────────────────────────────────

/// The frozen secret-service port (design §9.11). Reads take a
/// [`HostExecutionContext`]; mutations and provider-only resolution take a
/// sealed [`AdmittedMutationContext`], so a secret can only be stored, replaced,
/// deleted, or resolved under an admitted, granted OS action.
///
/// A credential value never crosses this boundary except as a [`SecretPayload`]
/// returned by [`Self::resolve_for_operation`], which itself cannot serialize or
/// leak. Every failure is fail-closed: there is no plaintext fallback path.
#[async_trait::async_trait]
pub trait CredentialStore: Send + Sync {
    /// List value-free metadata, optionally filtered by purpose (a read).
    async fn list_metadata(
        &self,
        ctx: &HostExecutionContext,
        purpose: Option<SecretPurpose>,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<SecretMetadataPage, OsControlError>;

    /// Store a new secret from protected input, returning its metadata (mutation).
    async fn store(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        purpose: SecretPurpose,
        scope: SecretScope,
        label: SafeText,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError>;

    /// Replace an existing secret's value from protected input (mutation).
    async fn replace(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
        input: ProtectedInputHandle,
    ) -> Result<SecretMetadata, OsControlError>;

    /// Delete a secret by reference (mutation).
    async fn delete(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        reference: &SecretRef,
    ) -> Result<(), OsControlError>;

    /// **Provider-only** resolution: return the secret value for an admitted
    /// action whose bound purpose/scope match the stored metadata. Never
    /// returned to the model/tool/plan/audit (OSC-025.3).
    async fn resolve_for_operation(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &SecretResolutionRequest,
    ) -> Result<SecretPayload, OsControlError>;
}

/// Build the fail-closed error for a locked/unavailable secret service. There is
/// no plaintext fallback and no password interception (OSC-025.1/.7).
#[must_use]
pub fn service_unavailable(state: SecretServiceState) -> OsControlError {
    match state {
        SecretServiceState::Locked => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new(
                "the secret service collection is locked; unlock the keyring to continue",
            ),
            retryable: true,
        },
        SecretServiceState::Unavailable | SecretServiceState::Available => {
            OsControlError::Unavailable {
                provider: None,
                reason: SafeText::new(
                    "the freedesktop Secret Service is unavailable; no plaintext fallback exists",
                ),
                retryable: false,
            }
        }
    }
}

/// The current unix-seconds timestamp (monotonic-enough for expiry checks).
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Field label used when a resolution fails validation.
#[must_use]
fn secret_field() -> SafeField {
    SafeField::new("secret")
}

/// The pre-mutation error for an unknown/absent reference (fail closed).
#[must_use]
pub fn unknown_reference() -> OsControlError {
    OsControlError::InvalidRequest {
        field: secret_field(),
        reason: SafeText::new("unknown or absent secret reference"),
    }
}

/// The pre-mutation error for a purpose/scope mismatch (fail closed, OSC-025.3).
#[must_use]
pub fn purpose_scope_mismatch() -> OsControlError {
    OsControlError::PolicyDenied {
        reason: SafeText::new("secret resolution purpose/scope does not match the stored binding"),
    }
}

#[cfg(feature = "os-control-test")]
mod fake;
#[cfg(feature = "os-control-test")]
pub use fake::FakeCredentialStore;

