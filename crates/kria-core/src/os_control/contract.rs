//! Provider-independent contract primitives for the OS-control runtime.
//!
//! linux-os-control-production **Task 1.1** — "Create base `os_control`
//! contracts, grants, receipt sums, and canonical errors" (OSC-001, OSC-005,
//! OSC-006, OSC-029), design §4.
//!
//! This module owns the *provider-independent* DTO foundations every later
//! task builds on:
//!
//! * bounded, typed identifier newtypes ([`GrantId`], [`SessionId`], …) so no
//!   handler passes a bare `String` where a bound identity is required;
//! * the [`Digest`] binding primitive used across grants, receipts, and audit;
//! * bounded collections ([`BoundedVec`], [`NonEmptyBoundedVec`]) so no list is
//!   unbounded (design §2 invariant 12);
//! * redacted "safe" value newtypes ([`SafeText`], [`SafeField`], …) that strip
//!   untrusted control characters and bound length at construction, so a DTO
//!   **cannot** carry raw stderr / object paths / model prose (design §5, §13,
//!   OSC-029);
//! * the generic [`DesiredStateControl`] mutation lifecycle every domain
//!   provider maps onto (design §4).
//!
//! Per design §4 the DTOs here contain *no model prose*: free text only enters
//! through a [`SafeText`]-family newtype that redacts at construction.

use std::fmt;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{ApplyOutcome, RollbackToken, VerificationReport};

/// Maximum length (in `char`s) any redacted safe-text value may retain. Longer
/// input is truncated at construction so no DTO carries an unbounded blob.
pub const SAFE_TEXT_MAX_CHARS: usize = 512;

/// Default hard cap for a [`BoundedVec`] when a caller does not specify one.
pub const DEFAULT_BOUNDED_VEC_CAP: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Typed bounded identifiers
// ─────────────────────────────────────────────────────────────────────────────

/// Declare an opaque, bounded identifier newtype over `String`.
///
/// Identifiers are validated at construction: bounded length and no control
/// characters, so an id can never smuggle untrusted content into a trace.
macro_rules! bounded_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Maximum length (chars) of this identifier.
            pub const MAX_CHARS: usize = 128;

            /// Construct from a raw string, bounding length and stripping control
            /// characters. Never fails: an id is always representable.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_bounded(&raw.into(), Self::MAX_CHARS))
            }

            /// Borrow the underlying string.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume into the owned string.
            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self::new(String::deserialize(d)?))
            }
        }
    };
}

bounded_id!(
    /// Opaque short-lived execution-grant identity (design §4 `ExecutionGrant`).
    GrantId
);
bounded_id!(
    /// User session identity binding a grant/admission to one login session.
    SessionId
);
bounded_id!(
    /// Durable `InteractionDecision` identity (SQLite authority).
    DecisionId
);
bounded_id!(
    /// Per-grant single-use nonce (replay defence for grant sealing).
    GrantNonce
);
bounded_id!(
    /// Correlation identity spanning one logical request/turn.
    CorrelationId
);
bounded_id!(
    /// One host action's identity within a correlation.
    ActionId
);
bounded_id!(
    /// Provider identity (e.g. `logind`, `network_manager`). Never model prose.
    ProviderId
);
bounded_id!(
    /// Capability identity from the frozen manifest (e.g. `set_volume`).
    CapabilityId
);
bounded_id!(
    /// Opaque receipt identity.
    ReceiptId
);
bounded_id!(
    /// Durable audit terminal-record identity.
    AuditRecordId
);
bounded_id!(
    /// Durable audit admission identity (owned in full by Task 1.8).
    AuditAdmissionId
);
bounded_id!(
    /// Idempotent audit recovery key committed before dispatch (Task 1.8).
    AuditRecoveryKey
);

/// Monotonic revision of a capability snapshot. A grant binds the revision it
/// was issued under so a stale-snapshot resume is detectable (OSC-001 crit. 5).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct SnapshotRevision(pub u64);

impl SnapshotRevision {
    /// The revision used before any capability probe has run (Task 1.3 fills
    /// this with real probe revisions).
    pub const UNPROBED: Self = Self(0);
}

impl fmt::Display for SnapshotRevision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Digest
// ─────────────────────────────────────────────────────────────────────────────

/// A binding digest: a lower-case hex SHA-256 over canonical bytes.
///
/// Used to bind grants, audit records, and receipts to exact action /
/// parameter / target / resource / observation values without carrying the raw
/// (possibly sensitive) content.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(String);

impl Digest {
    /// Compute a digest over arbitrary bytes.
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        use sha2::{Digest as _, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(hex::encode(hasher.finalize()))
    }

    /// Compute a digest over a string's UTF-8 bytes.
    #[must_use]
    pub fn of_str(text: &str) -> Self {
        Self::of_bytes(text.as_bytes())
    }

    /// Wrap a pre-computed lower-case hex digest string (bounded, sanitized).
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Self {
        Self(sanitize_bounded(&hex.into(), 64))
    }

    /// The lower-case hex representation.
    #[must_use]
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for Digest {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_hex(String::deserialize(d)?))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded collections
// ─────────────────────────────────────────────────────────────────────────────

/// A vector with a construction-enforced maximum length. Additional pushes past
/// the cap are rejected, so no list in a DTO can grow unbounded (design §2.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVec<T> {
    items: Vec<T>,
    cap: usize,
}

impl<T> BoundedVec<T> {
    /// Create an empty bounded vector with the given hard cap.
    #[must_use]
    pub fn with_cap(cap: usize) -> Self {
        Self {
            items: Vec::new(),
            cap: cap.max(1),
        }
    }

    /// Create an empty bounded vector with the default cap.
    #[must_use]
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_BOUNDED_VEC_CAP)
    }

    /// Build from an iterator, truncating at the cap.
    #[must_use]
    pub fn from_iter_capped(iter: impl IntoIterator<Item = T>, cap: usize) -> Self {
        let cap = cap.max(1);
        let items: Vec<T> = iter.into_iter().take(cap).collect();
        Self { items, cap }
    }

    /// Try to push; returns `false` (and drops `value`) when already at the cap.
    pub fn try_push(&mut self, value: T) -> bool {
        if self.items.len() >= self.cap {
            false
        } else {
            self.items.push(value);
            true
        }
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The hard cap.
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Borrow the elements.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }
}

impl<T> Default for BoundedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: serde::Serialize> serde::Serialize for BoundedVec<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.items.serialize(s)
    }
}

/// A [`BoundedVec`] guaranteed to hold at least one element. Used where a state
/// is only meaningful with content, e.g. a partial dispatch's completed steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyBoundedVec<T> {
    head: T,
    tail: BoundedVec<T>,
}

impl<T> NonEmptyBoundedVec<T> {
    /// Construct from a mandatory head element and a bounded tail.
    #[must_use]
    pub fn new(head: T, tail: BoundedVec<T>) -> Self {
        Self { head, tail }
    }

    /// Construct from a single element.
    #[must_use]
    pub fn single(head: T) -> Self {
        Self {
            head,
            tail: BoundedVec::new(),
        }
    }

    /// Borrow the guaranteed-present head element.
    #[must_use]
    pub fn head(&self) -> &T {
        &self.head
    }

    /// Borrow the (possibly empty) tail elements.
    #[must_use]
    pub fn tail(&self) -> &[T] {
        self.tail.as_slice()
    }

    /// Total element count (always >= 1).
    #[must_use]
    pub fn len(&self) -> usize {
        1 + self.tail.len()
    }

    /// Always false — kept for API symmetry / clippy.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl<T: Clone + serde::Serialize> serde::Serialize for NonEmptyBoundedVec<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.len()))?;
        seq.serialize_element(&self.head)?;
        for item in self.tail.as_slice() {
            seq.serialize_element(item)?;
        }
        seq.end()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Redacted "safe" value newtypes
// ─────────────────────────────────────────────────────────────────────────────

/// Strip control characters (except that all whitespace collapses to a single
/// space) and bound the length. This is the single sanitizer every safe-text
/// newtype uses so no DTO can carry raw stderr, object paths, or control chars.
#[must_use]
fn sanitize_bounded(raw: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(max_chars));
    for ch in raw.chars() {
        if out.chars().count() >= max_chars {
            break;
        }
        if ch.is_control() {
            // Collapse any control char (incl. newlines/tabs) to a space; drop
            // leading duplicates so we never emit runs of whitespace noise.
            if !out.ends_with(' ') && !out.is_empty() {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// Declare a bounded, control-char-free "safe" text newtype.
macro_rules! safe_text {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Construct, sanitizing and bounding at [`SAFE_TEXT_MAX_CHARS`].
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_bounded(&raw.into(), SAFE_TEXT_MAX_CHARS))
            }

            /// Construct with an explicit char bound.
            #[must_use]
            pub fn new_bounded(raw: impl Into<String>, max_chars: usize) -> Self {
                Self(sanitize_bounded(&raw.into(), max_chars))
            }

            /// Borrow the sanitized text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self::new(String::deserialize(d)?))
            }
        }
    };
}

safe_text!(
    /// Bounded, redacted human-safe message text (remediation, reasons).
    SafeText
);
safe_text!(
    /// Bounded, redacted field name for error attribution.
    SafeField
);
safe_text!(
    /// Bounded, redacted operation label.
    SafeOperation
);
safe_text!(
    /// Bounded, redacted resource label.
    SafeResource
);
safe_text!(
    /// Bounded, redacted provider revision string.
    SafeRevision
);
safe_text!(
    /// Bounded, redacted multi-step identifier.
    SafeStepId
);

/// A closed, versioned, redacted error/incident code (design §5). The exact set
/// is anchored by the frozen manifest (Task 0.1) / error taxonomy (design §5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SafeErrorCode(String);

impl SafeErrorCode {
    /// Construct from a static, code-owned string (never user/provider input).
    /// Static codes are authored in-tree and known control-char-free.
    #[must_use]
    pub fn from_static(code: &'static str) -> Self {
        Self(code.to_string())
    }

    /// Construct from a code string (bounded, sanitized).
    #[must_use]
    pub fn from_code(code: impl Into<String>) -> Self {
        Self(sanitize_bounded(&code.into(), 96))
    }

    /// Borrow the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for SafeErrorCode {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

/// A redacted disambiguation candidate for [`OsControlError::AmbiguousTarget`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SafeCandidate {
    /// Redacted label the user can recognize.
    pub label: SafeText,
    /// Stable opaque identity digest for selection.
    pub identity: Digest,
}

/// A redacted provider warning attached to a dispatch fact.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SafeWarning {
    /// Closed warning code.
    pub code: SafeErrorCode,
    /// Optional redacted detail.
    pub detail: Option<SafeText>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Grant decision
// ─────────────────────────────────────────────────────────────────────────────

/// How a grant's admission was decided. Bound into [`ExecutionGrant`] so a
/// receipt/audit can attribute the authority basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantDecision {
    /// Admitted without user confirmation (e.g. GREEN read/idempotent).
    NoConfirmationRequired,
    /// Admitted after a durable approved `InteractionDecision`.
    Approved,
}

// ─────────────────────────────────────────────────────────────────────────────
// Verification / availability classification
// ─────────────────────────────────────────────────────────────────────────────

/// Comparator used to decide whether an observation satisfies a postcondition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparatorKind {
    /// Exact normalized equality.
    Exact,
    /// Equality within a numeric tolerance.
    WithinTolerance,
    /// Boolean/state membership.
    Membership,
}

/// Numeric tolerance for [`ComparatorKind::WithinTolerance`] (e.g. audio %).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Tolerance {
    /// Allowed absolute delta.
    pub abs: f64,
}

/// Ranked authority of a verification evidence source (design §13). Higher
/// variants outrank lower ones; generic shell output is never authoritative.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum OsEvidenceSource {
    /// User attestation (subjective outcomes only).
    UserAttestation,
    /// Structured-command query with unambiguous parse.
    StructuredCommandQuery,
    /// Independent provider query for the same normalized state.
    IndependentProviderQuery,
    /// Authoritative service state/property or filesystem metadata.
    AuthoritativeServiceState,
}

/// Reliability class of a verification observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationReliability {
    /// Strong, directly authoritative evidence.
    Strong,
    /// Independent but indirect evidence.
    Moderate,
    /// Weak / best-effort evidence.
    Weak,
}

/// Whether an operation can be verified, and how strongly (design §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationClass {
    /// Fully verifiable through fresh independent evidence.
    Verifiable,
    /// Only acceptance is observable (session-ending / async).
    AcceptedOnly,
    /// Not observable after dispatch.
    Unverifiable,
}

/// Whether a probed capability is usable (design §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    /// Fully available.
    Available,
    /// Available with reduced fidelity.
    Degraded,
    /// Not available.
    Unavailable,
}

// ─────────────────────────────────────────────────────────────────────────────
// Desired-state lifecycle (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The generic mutation lifecycle every domain provider maps onto (design §4).
///
/// The signature rule is normative and compile-time enforced:
///
/// * reads (`observe`, `verify`) take `&HostExecutionContext` — observation
///   only, safe to create after read-policy/admission;
/// * mutators (`apply`, `rollback`) take `&AdmittedMutationContext<'_>` — which
///   cannot be constructed until [`crate::os_control::runtime`] (Task 1.7) seals
///   a grant with held leases + committed audit admission.
///
/// Providers return [`ApplyOutcome`] (a narrow dispatch fact), **never** a
/// [`crate::os_control::receipt::MutationReceipt`]; only the runtime constructs
/// receipts. `Err(OsControlError)` from a mutator is legal only when the
/// provider proves dispatch/effect did not start (design §4, §5).
#[async_trait::async_trait]
pub trait DesiredStateControl<R, O>: Send + Sync
where
    R: Send + Sync,
    O: Send + Sync,
{
    /// Observe current normalized state (read-only).
    async fn observe(&self, ctx: &HostExecutionContext, request: &R) -> Result<O, OsControlError>;

    /// Apply the desired state exactly once. Requires a sealed mutation context.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &R,
        desired: &O,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Verify the postcondition through fresh independent evidence (read-only).
    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &R,
        desired: &O,
    ) -> Result<VerificationReport<O>, OsControlError>;

    /// Roll back a prior mutation. Requires a sealed mutation context.
    async fn rollback(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError>;
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn safe_text_strips_control_chars_and_bounds_length() {
        let dirty = "line1\nline2\t\u{7}bell\r\n";
        let safe = SafeText::new(dirty);
        assert!(!safe.as_str().contains('\n'));
        assert!(!safe.as_str().contains('\t'));
        assert!(!safe.as_str().contains('\u{7}'));
        assert!(safe.as_str().contains("line1 line2"));

        let long = "x".repeat(SAFE_TEXT_MAX_CHARS + 100);
        let bounded = SafeText::new(long);
        assert!(bounded.as_str().chars().count() <= SAFE_TEXT_MAX_CHARS);
    }

    #[test]
    fn bounded_id_sanitizes_and_bounds() {
        let id = GrantId::new("grant\n\u{1b}[31m".to_string() + &"z".repeat(500));
        assert!(!id.as_str().contains('\n'));
        assert!(!id.as_str().contains('\u{1b}'));
        assert!(id.as_str().chars().count() <= GrantId::MAX_CHARS);
    }

    #[test]
    fn digest_is_deterministic_and_hex() {
        let a = Digest::of_str("hello");
        let b = Digest::of_str("hello");
        let c = Digest::of_str("world");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.as_hex().len(), 64);
        assert!(a.as_hex().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn bounded_vec_rejects_past_cap() {
        let mut v: BoundedVec<u32> = BoundedVec::with_cap(2);
        assert!(v.try_push(1));
        assert!(v.try_push(2));
        assert!(!v.try_push(3));
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn bounded_vec_from_iter_truncates() {
        let v: BoundedVec<u32> = BoundedVec::from_iter_capped(0..100, 4);
        assert_eq!(v.len(), 4);
        assert_eq!(v.as_slice(), &[0, 1, 2, 3]);
    }

    #[test]
    fn non_empty_bounded_vec_always_has_head() {
        let v = NonEmptyBoundedVec::single(SafeStepId::new("step-1"));
        assert_eq!(v.len(), 1);
        assert!(!v.is_empty());
        assert_eq!(v.head().as_str(), "step-1");
    }

    #[test]
    fn evidence_source_ranking_is_ordered() {
        assert!(
            OsEvidenceSource::AuthoritativeServiceState
                > OsEvidenceSource::IndependentProviderQuery
        );
        assert!(
            OsEvidenceSource::IndependentProviderQuery > OsEvidenceSource::StructuredCommandQuery
        );
        assert!(OsEvidenceSource::StructuredCommandQuery > OsEvidenceSource::UserAttestation);
    }

    #[test]
    fn snapshot_revision_default_is_unprobed() {
        assert_eq!(SnapshotRevision::UNPROBED, SnapshotRevision(0));
    }
}
