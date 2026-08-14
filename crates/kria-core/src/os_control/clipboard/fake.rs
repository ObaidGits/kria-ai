//! Deny-live fake [`ClipboardTransport`] (OSC-023, OSC-033), Task 2.5.
//!
//! Compiled only under `os-control-test`. No X11/Wayland selection is opened
//! and no `xclip`/`wl-copy` process is spawned, so
//! [`crate::os_control::access::deny_live_transport`] is unreachable from here
//! and the deny-live sentinel never trips.
//!
//! # Privacy posture — read this before adding a fixture
//!
//! The clipboard is the single most credential-dense surface in the domain:
//! users copy passwords into it constantly. Two rules follow, and they are not
//! stylistic:
//!
//! 1. **Fixtures use obvious placeholders only.** [`PLACEHOLDER_CLIPBOARD_TEXT`]
//!    and [`PLACEHOLDER_HISTORY_PAYLOAD`] exist so no test ever commits a string
//!    that could be mistaken for — or copy-pasted into — a real secret. A
//!    fixture that *looks* like a credential trains readers to accept
//!    credentials in test data.
//! 2. **The raw text this fake retains is for assertion only.** [`Self::write_calls`]
//!    keeps the exact text handed to `write_text` so a suite can prove the
//!    provider passed it through unchanged. Production audit never records this
//!    value: it binds the content digest under the shared `Content` redaction
//!    class ([`super::ClipboardState`] carries a digest and a length, never the
//!    text).
//!
//! # The three "no text came back" facts are kept apart
//!
//! Conflating them would make the assistant tell the user something false about
//! their own clipboard, so the fake scripts each distinctly:
//!
//! | scripted with | models | returns |
//! |---|---|---|
//! | [`Self::read_empty`] | a live selection owner offering no content type | `Ok("")` — positively empty |
//! | [`Self::read_non_text`] | an image-only copy (`image/png`, …) | [`OsControlError::Unsupported`] — present, but not text |
//! | [`Self::read_unreadable`] / [`Self::read_failure`] | the read itself did not complete | [`OsControlError::Unavailable`] — unknown |
//! | *nothing scripted* | a test that never established a state | [`OsControlError::Unavailable`] — never a default |
//!
//! Reporting an unreadable or image-only clipboard as empty would tell the user
//! their clipboard holds nothing while it holds their password. That is the
//! failure this fake exists to make testable.
//!
//! # Self-applying model
//!
//! `write_text` and `clear_history` mutate the fake's own state rather than
//! returning a canned value, so an observe → apply → re-observe → verify
//! lifecycle exercises the real governed path.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, CapabilityId, Digest, ProviderId, SafeField, SafeText,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{AppliedDispatch, ApplyOutcome};

use super::{
    AllowedMime, ClipboardHistoryConfigState, ClipboardHistoryEntry, ClipboardHistoryItemId,
    ClipboardHistoryPage, ClipboardHistoryState, ClipboardTransport,
    DEFAULT_CLIPBOARD_HISTORY_PAGE, MAX_CLIPBOARD_HISTORY_PAGE,
};

/// Provider identity reported by the fake transport.
pub const FAKE_CLIPBOARD_PROVIDER_ID: &str = "fake-clipboard";

/// The placeholder text fixtures put on the fake clipboard.
///
/// Deliberately unmistakable for a credential. Never replace it with something
/// that reads like a real password, token or key.
pub const PLACEHOLDER_CLIPBOARD_TEXT: &str = "PLACEHOLDER-CLIPBOARD-TEXT";

/// The placeholder a fake history entry's payload digest is taken over.
///
/// A history entry is metadata only ([`ClipboardHistoryEntry`]); no payload is
/// stored here, and the digest is taken over this constant so nothing
/// secret-shaped exists to leak.
pub const PLACEHOLDER_HISTORY_PAYLOAD: &str = "PLACEHOLDER-NOT-A-REAL-SECRET";

/// One scripted clipboard read.
///
/// Private on purpose: tests script through the `read_*` builders so every
/// scripted fact carries the builder's documented meaning.
enum ScriptedRead {
    /// A selection owner offering text with exactly this payload.
    Text(String),
    /// A live owner offering no content type at all: positively empty.
    Empty,
    /// Content is offered, but none of it is text (e.g. an image-only copy).
    NonTextOnly,
    /// The read itself did not complete; nothing was learned about the content.
    Unreadable {
        /// Why the read did not complete.
        reason: String,
    },
}

/// Build a history entry whose payload digest is a known placeholder.
///
/// Use this instead of hand-rolling a [`ClipboardHistoryEntry`], so no fixture
/// ever digests something that resembles copied content.
#[must_use]
pub fn placeholder_history_entry(
    item_id: &str,
    mime: AllowedMime,
    byte_count: u64,
    captured_at_ms: Option<u64>,
) -> ClipboardHistoryEntry {
    ClipboardHistoryEntry {
        item_id: ClipboardHistoryItemId::new(item_id),
        mime,
        byte_count,
        captured_at_ms,
        payload_digest: Digest::of_str(PLACEHOLDER_HISTORY_PAYLOAD),
    }
}

/// A scripted, in-memory clipboard transport.
///
/// Reads are a FIFO queue because one governed mutation performs several in a
/// fixed order (pre-observation → under-lease re-observation → post-apply
/// re-observation → verify). Script them with successive [`Self::read_ok`]
/// calls. When the queue is exhausted the last **established** value is held —
/// a steady state — so a test that only cares about one state scripts once.
/// When nothing was ever established, a read fails closed.
pub struct FakeClipboardTransport {
    /// Reads still to be served, in order.
    scripted: Mutex<VecDeque<ScriptedRead>>,
    /// The clipboard content the fake currently models. `None` until a read or
    /// a write establishes one — never defaulted to empty.
    current: Mutex<Option<String>>,
    /// Sticky: every read fails while set (models e.g. no reachable display).
    read_failure: Option<String>,
    /// Sticky: every write fails while set (models e.g. a refused selection).
    write_failure: Option<String>,
    /// Scripted outcome for a mutating call.
    outcome: Mutex<Option<ApplyOutcome>>,
    /// The exact texts handed to `write_text`, in order. Assertion-only; see
    /// the module's privacy posture.
    write_calls: Mutex<Vec<String>>,
    /// Mutating transport calls attempted (writes, history clears, config writes).
    dispatches: Mutex<usize>,
    /// Reads served or refused.
    reads: Mutex<usize>,
    /// The retained-history table. `None` models a session with **no clipboard
    /// manager**, which is a different fact from an empty history.
    history: Mutex<Option<Vec<ClipboardHistoryEntry>>>,
    /// The retention configuration the history store reports. `None` means
    /// unscripted → fail closed.
    history_config: Mutex<Option<ClipboardHistoryConfigState>>,
    /// Configurations applied through `write_history_config`, in order.
    config_writes: Mutex<Vec<ClipboardHistoryConfigState>>,
}

impl Default for FakeClipboardTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeClipboardTransport {
    /// A fake with nothing scripted; every read fails closed until a `read_*`
    /// builder establishes a state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(VecDeque::new()),
            current: Mutex::new(None),
            read_failure: None,
            write_failure: None,
            outcome: Mutex::new(None),
            write_calls: Mutex::new(Vec::new()),
            dispatches: Mutex::new(0),
            reads: Mutex::new(0),
            history: Mutex::new(None),
            history_config: Mutex::new(None),
            config_writes: Mutex::new(Vec::new()),
        }
    }

    /// Builder: queue the next read as a selection holding `text`.
    ///
    /// Pass [`PLACEHOLDER_CLIPBOARD_TEXT`] unless the test asserts on a specific
    /// string; never pass anything credential-shaped.
    #[must_use]
    pub fn read_ok(self, text: impl Into<String>) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::Text(text.into()));
        self
    }

    /// Builder: queue the next read as a **positively empty** clipboard — a live
    /// owner offering no content type.
    ///
    /// Distinct from [`Self::read_unreadable`] by design: this one returns
    /// `Ok("")`, because "the clipboard is empty" is a fact the fake actually
    /// established.
    #[must_use]
    pub fn read_empty(self) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::Empty);
        self
    }

    /// Builder: queue the next read as a **non-text-only** offer (an image was
    /// copied).
    ///
    /// Returns [`OsControlError::Unsupported`] naming `get_clipboard.non_text`,
    /// exactly as the live selection provider does. Reporting an image as text —
    /// or as an empty clipboard — would be a false observation.
    #[must_use]
    pub fn read_non_text(self) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::NonTextOnly);
        self
    }

    /// Builder: queue the next read as one that **did not complete**.
    ///
    /// Returns a retryable [`OsControlError::Unavailable`]: nothing is known
    /// about the content. Use this, not [`Self::read_empty`], to model a failed
    /// read.
    #[must_use]
    pub fn read_unreadable(self, reason: impl Into<String>) -> Self {
        self.scripted
            .lock()
            .expect("scripted mutex")
            .push_back(ScriptedRead::Unreadable {
                reason: reason.into(),
            });
        self
    }

    /// Builder: make **every** read fail, proving an unreadable clipboard never
    /// becomes a fabricated state.
    #[must_use]
    pub fn read_failure(mut self, reason: impl Into<String>) -> Self {
        self.read_failure = Some(reason.into());
        self
    }

    /// Builder: make every write fail. A refused write leaves the modeled
    /// clipboard untouched, so a following read still reports the old content.
    #[must_use]
    pub fn write_failure(mut self, reason: impl Into<String>) -> Self {
        self.write_failure = Some(reason.into());
        self
    }

    /// Builder: script the outcome a mutating call returns.
    #[must_use]
    pub fn dispatch_outcome(self, outcome: ApplyOutcome) -> Self {
        *self.outcome.lock().expect("outcome mutex") = Some(outcome);
        self
    }

    /// Builder: script the retained-history table.
    ///
    /// An empty `Vec` is a valid answer ("the store is present and holds
    /// nothing"); leaving this unscripted models "there is no history store",
    /// which reports `Unavailable` rather than zero entries.
    #[must_use]
    pub fn history_ok(self, entries: Vec<ClipboardHistoryEntry>) -> Self {
        *self.history.lock().expect("history mutex") = Some(entries);
        self
    }

    /// Builder: script the history store's retention configuration.
    #[must_use]
    pub fn history_config_ok(self, config: ClipboardHistoryConfigState) -> Self {
        *self.history_config.lock().expect("history config mutex") = Some(config);
        self
    }

    /// The exact texts handed to `write_text`, in order.
    ///
    /// Assertion-only (see the module's privacy posture): production audit binds
    /// the content digest, never this value.
    #[must_use]
    pub fn write_calls(&self) -> Vec<String> {
        self.write_calls.lock().expect("write calls mutex").clone()
    }

    /// How many mutating transport calls were attempted.
    #[must_use]
    pub fn dispatch_count(&self) -> usize {
        *self.dispatches.lock().expect("dispatch mutex")
    }

    /// How many reads were served or refused.
    #[must_use]
    pub fn read_count(&self) -> usize {
        *self.reads.lock().expect("reads mutex")
    }

    /// The clipboard content the fake currently models, if any read or write has
    /// established one.
    ///
    /// Lets a test prove `write_text` applied its effect to the fake's own state
    /// without scripting a further read.
    #[must_use]
    pub fn modeled_text(&self) -> Option<String> {
        self.current.lock().expect("current mutex").clone()
    }

    /// The retention configurations applied through `write_history_config`.
    #[must_use]
    pub fn history_config_writes(&self) -> Vec<ClipboardHistoryConfigState> {
        self.config_writes
            .lock()
            .expect("config writes mutex")
            .clone()
    }

    /// The error an unscripted read returns. Never a value: a fake that invented
    /// state would let a test prove a mutation verified against a fact nobody read.
    fn unscripted(&self, reason: &str) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_CLIPBOARD_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable: false,
        }
    }

    /// The error a read that did not complete returns — "unknown", never "empty".
    fn unreadable(&self, reason: impl Into<String>) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(FAKE_CLIPBOARD_PROVIDER_ID)),
            reason: SafeText::new(format!(
                "clipboard read did not complete: {}",
                reason.into()
            )),
            retryable: true,
        }
    }

    /// The scripted dispatch outcome, or a recorded-not-executed `Applied`.
    fn scripted_outcome(&self) -> ApplyOutcome {
        self.outcome
            .lock()
            .expect("outcome mutex")
            .clone()
            .unwrap_or_else(|| {
                ApplyOutcome::Applied(AppliedDispatch::new(
                    Some(Digest::of_str(crate::os_control::testing::FAKE_RECEIPT_TAG)),
                    BoundedVec::new(),
                ))
            })
    }
}

#[async_trait]
impl ClipboardTransport for FakeClipboardTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FAKE_CLIPBOARD_PROVIDER_ID)
    }

    async fn read_text(&self, _ctx: &HostExecutionContext) -> Result<String, OsControlError> {
        *self.reads.lock().expect("reads mutex") += 1;

        if let Some(reason) = &self.read_failure {
            return Err(self.unreadable(reason.clone()));
        }

        let next = self.scripted.lock().expect("scripted mutex").pop_front();
        let mut current = self.current.lock().expect("current mutex");
        match next {
            // A selection owner holding text.
            Some(ScriptedRead::Text(text)) => {
                *current = Some(text.clone());
                Ok(text)
            }
            // Positively empty: established by a successful listing, so it is a
            // value, not an error. `""` is a real clipboard state.
            Some(ScriptedRead::Empty) => {
                *current = Some(String::new());
                Ok(String::new())
            }
            // Present but not addressable as text. Deliberately NOT `Ok("")`:
            // telling the user their clipboard is empty when it holds an image
            // is a false observation. `current` is left untouched — nothing was
            // learned about any text content.
            Some(ScriptedRead::NonTextOnly) => Err(OsControlError::Unsupported {
                capability: CapabilityId::new("get_clipboard.non_text"),
                reason: SafeText::new(
                    "the clipboard holds only non-text content; reporting it as text would be a false observation",
                ),
            }),
            // The read did not complete: unknown, not empty. `current` keeps
            // whatever was last established rather than being invalidated.
            Some(ScriptedRead::Unreadable { reason }) => Err(self.unreadable(reason)),
            // Queue drained: hold the last established value (a steady state).
            // With nothing ever established, fail closed — never invent a state.
            None => current.clone().ok_or_else(|| {
                self.unscripted("no clipboard state scripted on the fake transport")
            }),
        }
    }

    async fn write_text(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        text: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        *self.dispatches.lock().expect("dispatch mutex") += 1;
        // Recorded, never written to a real selection: no `xclip`/`wl-copy` runs
        // and no host clipboard changes. The raw text is retained for assertion
        // only (module privacy posture).
        self.write_calls
            .lock()
            .expect("write calls mutex")
            .push(text.to_string());

        if let Some(reason) = &self.write_failure {
            // A refused write changed nothing: the modeled content is not
            // advanced, so a following read still reports the old text.
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(FAKE_CLIPBOARD_PROVIDER_ID)),
                reason: SafeText::new(format!("clipboard write refused: {reason}")),
                retryable: true,
            });
        }

        // Apply the effect to the fake's own state, so an observe → apply →
        // re-observe lifecycle sees the change even with no further scripted read.
        *self.current.lock().expect("current mutex") = Some(text.to_string());
        Ok(self.scripted_outcome())
    }

    async fn read_history_state(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryState, OsControlError> {
        *self.reads.lock().expect("reads mutex") += 1;
        // No scripted store means there is no clipboard manager at all — which
        // is reported as absence, never as zero retained entries.
        let entries = self
            .history
            .lock()
            .expect("history mutex")
            .clone()
            .ok_or_else(|| {
                self.unscripted("no clipboard history store scripted on the fake transport")
            })?;
        Ok(ClipboardHistoryState {
            item_count: u64::try_from(entries.len()).unwrap_or(u64::MAX),
        })
    }

    async fn read_history(
        &self,
        _ctx: &HostExecutionContext,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ClipboardHistoryPage, OsControlError> {
        *self.reads.lock().expect("reads mutex") += 1;
        let entries = self
            .history
            .lock()
            .expect("history mutex")
            .clone()
            .ok_or_else(|| {
                self.unscripted("no clipboard history store scripted on the fake transport")
            })?;

        let bound = if limit == 0 {
            DEFAULT_CLIPBOARD_HISTORY_PAGE
        } else {
            limit.min(MAX_CLIPBOARD_HISTORY_PAGE)
        } as usize;

        // The fake's cursor is a decimal entry offset. An unparsable cursor is a
        // bad request, not an unavailable provider.
        let offset = match cursor {
            None => 0_usize,
            Some(raw) => raw
                .parse::<usize>()
                .map_err(|_| OsControlError::InvalidRequest {
                    field: SafeField::new("cursor"),
                    reason: SafeText::new(
                        "the fake clipboard history cursor is a decimal entry offset",
                    ),
                })?,
        };

        let total = entries.len();
        let page: Vec<ClipboardHistoryEntry> =
            entries.into_iter().skip(offset).take(bound).collect();
        let consumed = offset.saturating_add(page.len());
        let truncated = consumed < total;

        Ok(ClipboardHistoryPage {
            entries: page,
            next_cursor: if truncated {
                Some(consumed.to_string())
            } else {
                None
            },
            truncated,
        })
    }

    async fn clear_history(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
    ) -> Result<ApplyOutcome, OsControlError> {
        *self.dispatches.lock().expect("dispatch mutex") += 1;
        let mut history = self.history.lock().expect("history mutex");
        if history.is_none() {
            // Nothing to clear, and no success may be reported for a store that
            // was never there.
            return Err(self.unscripted(
                "no clipboard history store scripted on the fake transport",
            ));
        }
        // Irreversible in the real subsystem and irreversible here: the fake
        // keeps no shadow copy to "restore" from.
        *history = Some(Vec::new());
        drop(history);
        Ok(self.scripted_outcome())
    }

    async fn read_history_config(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryConfigState, OsControlError> {
        *self.reads.lock().expect("reads mutex") += 1;
        self.history_config
            .lock()
            .expect("history config mutex")
            .clone()
            .ok_or_else(|| {
                self.unscripted("no clipboard history configuration scripted on the fake transport")
            })
    }

    async fn write_history_config(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        config: &ClipboardHistoryConfigState,
    ) -> Result<ApplyOutcome, OsControlError> {
        *self.dispatches.lock().expect("dispatch mutex") += 1;
        self.config_writes
            .lock()
            .expect("config writes mutex")
            .push(config.clone());

        let mut current = self.history_config.lock().expect("history config mutex");
        if current.is_none() {
            // Applying a retention policy to a store nobody scripted would
            // invent the store itself.
            return Err(self.unscripted(
                "no clipboard history configuration scripted on the fake transport",
            ));
        }
        // Self-applying: a following read reports exactly what was written.
        *current = Some(config.clone());
        drop(current);
        Ok(self.scripted_outcome())
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
    use crate::os_control::contract::{ActionId, AuditAdmissionId, CorrelationId, SessionId};
    use crate::os_control::resource::AcquiredResourceLeaseSet;
    use crate::safety::RiskLevel;

    use super::*;

    const SESSION: &str = "session-clipboard-fake";

    /// A second placeholder, for asserting a write replaced the first.
    const PLACEHOLDER_CLIPBOARD_TEXT_2: &str = "PLACEHOLDER-CLIPBOARD-TEXT-2";

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
                "set_clipboard",
                &params,
                ExecutionTarget::Host,
                &[],
                RiskLevel::Red,
            );
            let resource_digest = Digest::of_str(grant.resource_set_digest());
            let audit_token = AuditAdmissionToken::for_test(
                AuditAdmissionId::new("adm-clipboard-fake"),
                resource_digest.clone(),
            );
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-clipboard-fake"),
                ActionId::new("act-clipboard-fake"),
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
    async fn unscripted_read_fails_closed_and_never_reports_an_empty_clipboard() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new();

        let err = fake
            .read_text(fx.host())
            .await
            .expect_err("an unscripted clipboard read must not produce a value");
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[tokio::test]
    async fn empty_and_unreadable_are_distinct_facts() {
        let fx = Fixture::build();
        // Queued in order: a positively-empty clipboard, then a read that did
        // not complete.
        let fake = FakeClipboardTransport::new()
            .read_empty()
            .read_unreadable("no selection owner answered");

        // An empty clipboard is a value: the user really has nothing copied.
        assert_eq!(
            fake.read_text(fx.host()).await.expect("empty is a value"),
            ""
        );

        // A failed read is not: claiming "empty" here would tell the user their
        // clipboard is empty while it may hold their password.
        let err = fake
            .read_text(fx.host())
            .await
            .expect_err("an unreadable clipboard is not an empty clipboard");
        match err {
            OsControlError::Unavailable { retryable, .. } => assert!(retryable),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_text_offer_is_unsupported_never_text_and_never_empty() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new().read_non_text();

        let err = fake
            .read_text(fx.host())
            .await
            .expect_err("an image-only clipboard cannot be reported as text");
        match err {
            OsControlError::Unsupported { capability, .. } => {
                assert_eq!(capability.as_str(), "get_clipboard.non_text");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_read_never_dispatches() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new().read_ok(PLACEHOLDER_CLIPBOARD_TEXT);

        let text = fake.read_text(fx.host()).await.expect("scripted read");
        assert_eq!(text, PLACEHOLDER_CLIPBOARD_TEXT);
        assert_eq!(fake.dispatch_count(), 0, "observing must never mutate");
        assert_eq!(fake.read_count(), 1);
    }

    #[tokio::test]
    async fn write_applies_to_the_fakes_own_state() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new().read_ok(PLACEHOLDER_CLIPBOARD_TEXT);

        assert_eq!(
            fake.read_text(fx.host()).await.expect("pre-observation"),
            PLACEHOLDER_CLIPBOARD_TEXT
        );

        fake.write_text(&fx.admitted(), PLACEHOLDER_CLIPBOARD_TEXT_2)
            .await
            .expect("write applies");

        // No further read was scripted: the value comes from the fake's own
        // model, which the write moved.
        assert_eq!(
            fake.read_text(fx.host()).await.expect("re-observation"),
            PLACEHOLDER_CLIPBOARD_TEXT_2
        );
        assert_eq!(fake.modeled_text().as_deref(), Some(PLACEHOLDER_CLIPBOARD_TEXT_2));
        assert_eq!(fake.write_calls(), vec![PLACEHOLDER_CLIPBOARD_TEXT_2.to_string()]);
        assert_eq!(fake.dispatch_count(), 1);
    }

    #[tokio::test]
    async fn a_refused_write_leaves_the_modeled_clipboard_untouched() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new()
            .read_ok(PLACEHOLDER_CLIPBOARD_TEXT)
            .write_failure("selection owner refused the transfer");

        assert_eq!(
            fake.read_text(fx.host()).await.expect("pre-observation"),
            PLACEHOLDER_CLIPBOARD_TEXT
        );
        fake.write_text(&fx.admitted(), PLACEHOLDER_CLIPBOARD_TEXT_2)
            .await
            .expect_err("scripted write failure");

        // The write changed nothing, so the old content is still what is there.
        assert_eq!(
            fake.read_text(fx.host()).await.expect("re-observation"),
            PLACEHOLDER_CLIPBOARD_TEXT
        );
    }

    #[tokio::test]
    async fn an_absent_history_store_is_not_an_empty_history() {
        let fx = Fixture::build();

        // Nothing scripted: there is no clipboard manager at all.
        let absent = FakeClipboardTransport::new();
        let err = absent
            .read_history_state(fx.host())
            .await
            .expect_err("no history store must not report zero entries");
        assert!(matches!(err, OsControlError::Unavailable { .. }));

        // A present-but-empty store reports zero, which is a different fact.
        let empty = FakeClipboardTransport::new().history_ok(Vec::new());
        assert_eq!(
            empty
                .read_history_state(fx.host())
                .await
                .expect("present store")
                .item_count,
            0
        );
    }

    #[tokio::test]
    async fn clearing_history_applies_to_the_fakes_own_state() {
        let fx = Fixture::build();
        let fake = FakeClipboardTransport::new().history_ok(vec![
            placeholder_history_entry("item-1", AllowedMime::TextPlain, 12, Some(1_000)),
            placeholder_history_entry("item-2", AllowedMime::ImagePng, 2_048, None),
        ]);

        assert_eq!(
            fake.read_history_state(fx.host())
                .await
                .expect("scripted store")
                .item_count,
            2
        );

        fake.clear_history(&fx.admitted())
            .await
            .expect("clear applies");

        assert_eq!(
            fake.read_history_state(fx.host())
                .await
                .expect("store still present")
                .item_count,
            0,
            "the store survives; only its entries are destroyed"
        );
    }
}
