//! Clipboard domain: the `ClipboardControl` desired-state provider (design
//! §3, §9.10).
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-023).
//!
//! This module replaces the direct `arboard::Clipboard` calls that used to
//! live in `tools/interaction.rs` (`get_clipboard`/`set_clipboard`;
//! `transform_clipboard` composes the same read+write and is migrated
//! separately in the tool facade by calling through this same provider
//! twice). `arboard` talks to the X11/Wayland clipboard selection directly —
//! not a subprocess — so this domain, like [`crate::os_control::processes`],
//! has no [`crate::os_control::linux::structured_command::StructuredCommandRequest`]
//! seam; its raw transport is a device-class access
//! ([`crate::os_control::access::RawTransportKind::Device`]).
//!
//! * [`ClipboardState`] is a normalized observation
//!   ([`NormalizedObservation`]) binding a content digest + byte length, so
//!   `set_clipboard` idempotency/verification are real without ever
//!   surfacing the raw text in a digest-adjacent trace.
//! * [`ClipboardControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle for `set_clipboard`. `get_clipboard` is a pure read
//!   (`current()`) outside the mutation lifecycle, mirroring
//!   `ConnectivityControl::scan_wifi_networks`.
//! * `rollback` always reports the truthful "no inverse" fact: the frozen
//!   manifest declares `rollbackClaim: None` for `set_clipboard`.
//! * Content classification (OSC-023, OSC-029): clipboard payloads are
//!   `DataClass::Content` in the shared redaction registry
//!   ([`crate::os_control::redaction`]) — never raw text in audit/trace.
//!   Task 3.9 owns the mandatory RED intent-bound clipboard-read policy and
//!   content-free approval projection; this task wires the provider seam the
//!   later policy composes onto.
//! * The live transport
//!   ([`crate::os_control::linux::providers::clipboard::LiveClipboard`]) is a
//!   raw, deny-live-gated adapter; deny-live tests inject
//!   [`fake::FakeClipboardTransport`].

use async_trait::async_trait;
use std::time::SystemTime;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

pub mod selection;

/// Deny-live fake transport (Task 2.5 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

/// The stable provider identity for the native clipboard-selection backend.
pub const CLIPBOARD_PROVIDER_ID: &str = "clipboard-native-selection";

/// A normalized clipboard observation (design §5, §9.10). Never carries the
/// raw text: only its content digest and byte length, so this type is safe to
/// pass through the generic runtime's `RedactedObservation` wrapper without
/// any additional scrubbing step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardState {
    /// Digest of the current clipboard text content.
    pub content_digest: Digest,
    /// Byte length of the current clipboard text content.
    pub byte_len: usize,
}

impl ClipboardState {
    /// Construct from raw text (digest computed once here; the raw text is
    /// not retained on the returned value).
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            content_digest: Digest::of_str(text),
            byte_len: text.len(),
        }
    }

    /// Construct directly from an already-computed digest + length (used by
    /// [`ClipboardRequest::desired_state`], which must not need the plaintext
    /// again after the initial digest was taken).
    #[must_use]
    pub fn from_digest(content_digest: Digest, byte_len: usize) -> Self {
        Self {
            content_digest,
            byte_len,
        }
    }
}

impl NormalizedObservation for ClipboardState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "clipboard:{}:{}",
            self.content_digest, self.byte_len
        ))
    }
}

/// A fully-described `set_clipboard` request.
#[derive(Debug, Clone)]
pub struct ClipboardRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest). **Never** the raw text itself — the caller's tool handler
    /// binds only the content digest into the canonical params, per the
    /// shared `Content` redaction class (OSC-023, OSC-029).
    pub params: serde_json::Value,
    /// The text to write to the clipboard.
    pub text: String,
}

impl ClipboardRequest {
    /// The desired end state (the digest/length of `text`).
    #[must_use]
    pub fn desired_state(&self) -> ClipboardState {
        ClipboardState::from_text(&self.text)
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for `set_clipboard`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw clipboard transport seam. The live implementation
/// ([`crate::os_control::linux::providers::clipboard::LiveClipboard`]) is a
/// deny-live-gated adapter over the X11/Wayland clipboard selection (a device
/// access, never a subprocess); deny-live tests inject
/// [`fake::FakeClipboardTransport`].
#[async_trait]
pub trait ClipboardTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Read the current clipboard text.
    async fn read_text(&self, ctx: &HostExecutionContext) -> Result<String, OsControlError>;

    /// Write `text` to the clipboard. This is the only mutating clipboard
    /// operation; there is no structured command involved (no `xclip`/`xsel`
    /// subprocess).
    async fn write_text(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        text: &str,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Read the retained-history **state** (Task 4.9): how many entries the
    /// session's clipboard manager is holding.
    ///
    /// This is metadata only — no entry payload is read, digested or returned.
    /// A session with no clipboard manager reports `Unavailable`; it never
    /// reports zero entries, because "there is no history store" and "the
    /// history store is empty" are different facts.
    async fn read_history_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryState, OsControlError>;

    /// Read one bounded page of retained history **entries** (Task 4.9).
    ///
    /// The most privacy-sensitive read in the domain: the history is a rolling
    /// log of everything the user copied, which routinely includes passwords. A
    /// transport must therefore return entry *metadata*
    /// ([`ClipboardHistoryEntry`]) and must not populate any payload it cannot
    /// justify. A transport that cannot supply the frozen per-entry metadata
    /// returns [`OsControlError::Unsupported`] naming the missing facts — never
    /// an empty page, and never metadata reconstructed from a preview.
    async fn read_history(
        &self,
        ctx: &HostExecutionContext,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ClipboardHistoryPage, OsControlError>;

    /// Destroy the entire retained history. **Irreversible** — there is no
    /// inverse and no rollback token.
    async fn clear_history(
        &self,
        ctx: &AdmittedMutationContext<'_>,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Read the retained-history **retention configuration** (Task 4.9).
    async fn read_history_config(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryConfigState, OsControlError>;

    /// Apply a retention configuration to the history store.
    async fn write_history_config(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        config: &ClipboardHistoryConfigState,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The `ClipboardControl` desired-state provider (design §3, §4, §9.10).
/// Generic over the [`ClipboardTransport`] so the same governed logic runs
/// over the live X11/Wayland adapter and the deny-live fake.
pub struct ClipboardControl<T: ClipboardTransport> {
    transport: T,
}

impl<T: ClipboardTransport> ClipboardControl<T> {
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

    /// Read the current clipboard text (the read-only `get_clipboard` path;
    /// not part of the `DesiredStateControl` mutation lifecycle).
    pub async fn current_text(&self, ctx: &HostExecutionContext) -> Result<String, OsControlError> {
        self.transport.read_text(ctx).await
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::IndependentProviderQuery
    }

    fn satisfying(&self, observed: &ClipboardState) -> SatisfyingVerification<ClipboardState> {
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
impl<T: ClipboardTransport> DesiredStateControl<ClipboardRequest, ClipboardState>
    for ClipboardControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardRequest,
    ) -> Result<ClipboardState, OsControlError> {
        let text = self.transport.read_text(ctx).await?;
        Ok(ClipboardState::from_text(&text))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ClipboardRequest,
        _desired: &ClipboardState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport.write_text(ctx, &request.text).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardRequest,
        desired: &ClipboardState,
    ) -> Result<VerificationReport<ClipboardState>, OsControlError> {
        let text = self.transport.read_text(ctx).await?;
        let observed = ClipboardState::from_text(&text);

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
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
        // `rollbackClaim: None` in the frozen manifest — never actually invoked.
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

/// Map a governed [`MutationReceipt`] to the **existing** `set_clipboard`
/// result fields (`set`, `length`), plus additive `lifecycle`/`verified`
/// fields.
#[must_use]
pub fn set_clipboard_result(
    receipt: &MutationReceipt<ClipboardState>,
    text_len: usize,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "set": true,
        "length": text_len,
        "changed": receipt.changed(),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map the current clipboard text to the **existing** `get_clipboard` result
/// fields (`content`).
#[must_use]
pub fn get_clipboard_result(text: &str) -> serde_json::Value {
    serde_json::json!({ "content": text })
}

// ─────────────────────────────────────────────────────────────────────────────
// Clipboard history (Task 4.9, OSC-023) — the most privacy-sensitive read
// ─────────────────────────────────────────────────────────────────────────────
//
// The retained history is a rolling log of everything the user copied, which
// routinely includes passwords. Every type below is therefore **metadata**: an
// entry carries its identity, type, size, capture time and payload digest, and
// the payload itself is optional in the frozen contract and never populated
// here. No history value reaches a log line, an error message or a test fixture.

/// Hard cap on entries returned in one history page (frozen `maxItems: 256`).
pub const MAX_CLIPBOARD_HISTORY_PAGE: u32 = 256;

/// The page size used when the caller names no `limit`.
pub const DEFAULT_CLIPBOARD_HISTORY_PAGE: u32 = 32;

/// The frozen `AllowedMime` set — the only content types the clipboard history
/// surface recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedMime {
    /// `text/plain`.
    TextPlain,
    /// `text/html`.
    TextHtml,
    /// `image/png`.
    ImagePng,
    /// `image/jpeg`.
    ImageJpeg,
}

impl AllowedMime {
    /// The canonical MIME token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            AllowedMime::TextPlain => "text/plain",
            AllowedMime::TextHtml => "text/html",
            AllowedMime::ImagePng => "image/png",
            AllowedMime::ImageJpeg => "image/jpeg",
        }
    }

    /// Parse a caller-supplied MIME token. An unrecognized token is `None` —
    /// callers reject it rather than widening the allow-list.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text/plain" => Some(AllowedMime::TextPlain),
            "text/html" => Some(AllowedMime::TextHtml),
            "image/png" => Some(AllowedMime::ImagePng),
            "image/jpeg" => Some(AllowedMime::ImageJpeg),
            _ => None,
        }
    }
}

/// A stable clipboard-history entry identity (the store's own id, never a
/// preview of the copied text — a label is neither unique nor stable, and a
/// payload fragment is a secret).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClipboardHistoryItemId(String);

impl ClipboardHistoryItemId {
    /// Maximum length (chars) of an entry id (frozen `maxLength: 128`).
    pub const MAX_CHARS: usize = 128;

    /// Construct a bounded, control-char-free entry id.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self(
            raw.chars()
                .filter(|c| !c.is_control())
                .take(Self::MAX_CHARS)
                .collect(),
        )
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One retained history entry, as **metadata only**.
///
/// `captured_at_ms` is `None` when the composed store does not record a capture
/// time. That absence is reported as absence; it is never filled in with the
/// observation time, which would be a fabricated fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryEntry {
    /// The store's stable entry identity.
    pub item_id: ClipboardHistoryItemId,
    /// The entry's content type.
    pub mime: AllowedMime,
    /// The entry payload's size in bytes.
    pub byte_count: u64,
    /// When the entry was captured, if the store records it.
    pub captured_at_ms: Option<u64>,
    /// Digest of the entry payload. Binds the entry's identity to its content
    /// without surfacing the content.
    pub payload_digest: Digest,
}

/// One bounded page of retained history entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipboardHistoryPage {
    /// The entries in this page, newest first.
    pub entries: Vec<ClipboardHistoryEntry>,
    /// The opaque cursor for the next page, if more entries remain.
    pub next_cursor: Option<String>,
    /// Whether the store held more entries than this page could carry.
    pub truncated: bool,
}

/// A normalized observation of the retained-history **size**.
///
/// This is the postcondition surface for `clear_clipboard_history`: the only
/// fact the operation changes is how many entries remain. Nothing about entry
/// content is observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryState {
    /// How many entries the store is retaining.
    pub item_count: u64,
}

impl NormalizedObservation for ClipboardHistoryState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!("clipboard-history:{}", self.item_count))
    }
}

/// A normalized observation of the retained-history **retention
/// configuration** — the postcondition surface for
/// `configure_clipboard_history`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryConfigState {
    /// Whether the store is capturing new entries at all.
    pub enabled: bool,
    /// Entry lifetime in seconds, when the store supports one.
    pub ttl_seconds: Option<u64>,
    /// Maximum retained entries, when the store supports a bound.
    pub max_items: Option<u64>,
    /// The content types the store may retain. Empty means the store does not
    /// filter by type.
    pub allowed_mimes: Vec<AllowedMime>,
}

impl NormalizedObservation for ClipboardHistoryConfigState {
    fn observation_digest(&self) -> Digest {
        // Sorted so two equal configurations never differ by argument order.
        let mut mimes: Vec<&'static str> = self.allowed_mimes.iter().map(|m| m.as_str()).collect();
        mimes.sort_unstable();
        mimes.dedup();
        Digest::of_str(&format!(
            "clipboard-history-config:{}:{}:{}:{}",
            self.enabled,
            self.ttl_seconds
                .map_or_else(|| "unset".to_string(), |t| t.to_string()),
            self.max_items
                .map_or_else(|| "unset".to_string(), |m| m.to_string()),
            mimes.join(",")
        ))
    }
}

/// A fully-described `clear_clipboard_history` request.
#[derive(Debug, Clone)]
pub struct ClipboardHistoryClearRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    /// The operation takes no parameters, so this is an empty object — there is
    /// nothing about the history's contents to bind.
    pub params: serde_json::Value,
}

impl ClipboardHistoryClearRequest {
    /// The desired end state: nothing retained.
    #[must_use]
    pub fn desired_state(&self) -> ClipboardHistoryState {
        ClipboardHistoryState { item_count: 0 }
    }

    /// The frozen comparator (`ExactTypedPostcondition`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// A fully-described `configure_clipboard_history` request.
#[derive(Debug, Clone)]
pub struct ClipboardHistoryConfigRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The requested retention configuration.
    pub config: ClipboardHistoryConfigState,
}

impl ClipboardHistoryConfigRequest {
    /// The desired end state: the store reports exactly this configuration.
    #[must_use]
    pub fn desired_state(&self) -> ClipboardHistoryConfigState {
        self.config.clone()
    }

    /// The frozen comparator (`ExactTypedPostcondition`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

impl<T: ClipboardTransport> ClipboardControl<T> {
    fn satisfying_history(
        &self,
        observed: &ClipboardHistoryState,
    ) -> SatisfyingVerification<ClipboardHistoryState> {
        SatisfyingVerification::new(
            OsEvidenceSource::IndependentProviderQuery,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }

    fn satisfying_history_config(
        &self,
        observed: &ClipboardHistoryConfigState,
    ) -> SatisfyingVerification<ClipboardHistoryConfigState> {
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
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
impl<T: ClipboardTransport> DesiredStateControl<ClipboardHistoryClearRequest, ClipboardHistoryState>
    for ClipboardControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardHistoryClearRequest,
    ) -> Result<ClipboardHistoryState, OsControlError> {
        self.transport.read_history_state(ctx).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        _request: &ClipboardHistoryClearRequest,
        _desired: &ClipboardHistoryState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport.clear_history(ctx).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardHistoryClearRequest,
        desired: &ClipboardHistoryState,
    ) -> Result<VerificationReport<ClipboardHistoryState>, OsControlError> {
        // A real postcondition: re-count the store. A wipe that silently failed
        // must not be reported as a wipe that succeeded.
        let observed = self.transport.read_history_state(ctx).await?;
        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(
                self.satisfying_history(&observed),
            ))
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
        // Destroying a copy history is irreversible: the payloads are gone.
        // `rollbackClaim: None` — never actually invoked, and never advertised.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

#[async_trait]
impl<T: ClipboardTransport>
    DesiredStateControl<ClipboardHistoryConfigRequest, ClipboardHistoryConfigState>
    for ClipboardControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardHistoryConfigRequest,
    ) -> Result<ClipboardHistoryConfigState, OsControlError> {
        self.transport.read_history_config(ctx).await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ClipboardHistoryConfigRequest,
        _desired: &ClipboardHistoryConfigState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport
            .write_history_config(ctx, &request.config)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        _request: &ClipboardHistoryConfigRequest,
        desired: &ClipboardHistoryConfigState,
    ) -> Result<VerificationReport<ClipboardHistoryConfigState>, OsControlError> {
        let observed = self.transport.read_history_config(ctx).await?;
        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(
                self.satisfying_history_config(&observed),
            ))
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
        // `rollbackClaim: None`: entries already dropped by a narrower retention
        // policy cannot be brought back, so no inverse is advertised.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::clipboard()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible clipboard domain port design §4 names
/// `fn clipboard(&self) -> &dyn ClipboardControl` on `HostOsControl`. Because
/// the concrete [`ClipboardControl`] provider struct above is generic over
/// its [`ClipboardTransport`], `HostOsControl::clipboard()` returns this
/// object-safe supertrait instead so any transport (live X11/Wayland, or a
/// deny-live fake) can be composed behind one erased reference. Every
/// [`ClipboardControl<T>`] implements it automatically via the blanket impl
/// below.
///
/// The three `DesiredStateControl` supertraits are the domain's three distinct
/// postconditions: the selection's content (`set_clipboard`), the history's size
/// (`clear_clipboard_history`) and the history's retention configuration
/// (`configure_clipboard_history`). They are separate because each operation must
/// verify against the fact it actually changes — a shared observation would let
/// one operation "verify" against a field it never touched.
#[async_trait]
pub trait ClipboardControlPort:
    DesiredStateControl<ClipboardRequest, ClipboardState>
    + DesiredStateControl<ClipboardHistoryClearRequest, ClipboardHistoryState>
    + DesiredStateControl<ClipboardHistoryConfigRequest, ClipboardHistoryConfigState>
{
    /// Read the current clipboard text (erased passthrough for the read-only
    /// `get_clipboard` tool).
    async fn current_text(&self, ctx: &HostExecutionContext) -> Result<String, OsControlError>;

    /// Read one bounded page of retained history entry **metadata** (erased
    /// passthrough for `get_clipboard_history`).
    async fn history(
        &self,
        ctx: &HostExecutionContext,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ClipboardHistoryPage, OsControlError>;

    /// Read the retained-history size (erased passthrough used by the
    /// `clear_clipboard_history` pre-state report).
    async fn history_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryState, OsControlError>;
}

#[async_trait]
impl<T: ClipboardTransport> ClipboardControlPort for ClipboardControl<T> {
    async fn current_text(&self, ctx: &HostExecutionContext) -> Result<String, OsControlError> {
        ClipboardControl::current_text(self, ctx).await
    }

    async fn history(
        &self,
        ctx: &HostExecutionContext,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ClipboardHistoryPage, OsControlError> {
        self.transport.read_history(ctx, limit, cursor).await
    }

    async fn history_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryState, OsControlError> {
        self.transport.read_history_state(ctx).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_content_and_length() {
        let a = ClipboardState::from_text("hello");
        let b = ClipboardState::from_text("hello");
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = ClipboardState::from_text("world");
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn desired_state_matches_requested_text() {
        let req = ClipboardRequest {
            action: "set_clipboard".to_string(),
            params: serde_json::json!({}),
            text: "hello world".to_string(),
        };
        assert_eq!(
            req.desired_state(),
            ClipboardState::from_text("hello world")
        );
    }
}
