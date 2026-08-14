//! Live X11/Wayland clipboard adapter (raw transport seam).
//!
//! linux-os-control-production **Task 2.5** (OSC-023), design §3, §9.10.
//!
//! # Host safety
//!
//! Reading the desktop clipboard selection is a **raw live transport**. Like
//! the other `linux/providers/*` adapters, this one:
//!
//! 1. can be constructed **only** with a [`LiveHostAccessToken`] (mintable
//!    solely in a live composition root under `os-control-live`), so no
//!    completion test can build it; and
//! 2. calls [`deny_live_transport`] **before** any read or dispatch, so a
//!    deny-live (`os-control-test`) build that reached here would trip the
//!    sentinel and abort rather than touch the host selection.
//!
//! The read runs through [`StructuredQueryRequest`] — a trusted absolute
//! executable, an exact digested argv, a hermetic environment, a pinned `C`
//! locale, bounded output, a deadline and cancellation. There is no ungoverned
//! subprocess or `arboard` fallback anywhere in this file.
//!
//! # The clipboard is privacy-critical
//!
//! The selection routinely holds a password the user copied seconds ago
//! (`DataClass::Content`, OSC-023/OSC-029). Therefore:
//!
//! * the payload is transferred by a query whose stdout is handed straight to
//!   the domain, which reduces it to a digest + byte length
//!   ([`crate::os_control::clipboard::ClipboardState`]);
//! * the payload is **never** parsed, logged, traced, included in an error, or
//!   used to pick a code path. Only the selection's offered **types** —
//!   metadata — are parsed, in
//!   [`crate::os_control::clipboard::selection::parse_offered_types`];
//! * only a compile-time constant type token from
//!   [`crate::os_control::clipboard::selection::TEXT_TYPE_PREFERENCE`] can
//!   reach an argv position, so nothing captured from the host is ever
//!   re-executed;
//! * a truncated payload read fails rather than returning the fragment, which
//!   would otherwise be digested as if it were the whole clipboard.
//!
//! # Empty is a different fact from unreadable
//!
//! An empty clipboard resolves to `Ok("")`; a clipboard whose state could not
//! be determined resolves to [`OsControlError::Unavailable`]. The two are never
//! conflated. Because neither `wl-paste` nor `xclip` exits zero when the
//! selection has no owner, and the governed read path reports a non-zero exit
//! as a failed observation, "no selection owner" reaches the caller as
//! *unknown* rather than as *empty*. Emptiness is only ever asserted from a
//! **successful** type listing that named no content type.
//!
//! # Known normalization limit (affects a future `set_clipboard` verify)
//!
//! [`StructuredQueryRequest`] bounds output by lines and rejoins them with
//! `\n`, so a payload's trailing newline and any CRLF line endings are
//! normalized away on the way back. Reads are unaffected in practice, but once
//! `write_text` is wired, exact-digest verification of content ending in a
//! newline would contradict against a truthful write. The mutation seam is
//! unwired today (see [`LiveClipboard::write_text`]), and closing that gap
//! needs a byte-exact read on the governed path, not a change here.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::clipboard::selection::{ClipboardBackend, ClipboardHistoryBackend, ClipboardOffer, history_list_argv, history_wipe_argv, parse_history_item_count, parse_offered_types, query_offered_types_argv, query_text_argv, select_backend, select_history_backend, write_text_argv};
use crate::os_control::clipboard::{ClipboardHistoryConfigState, ClipboardHistoryPage, ClipboardHistoryState, ClipboardTransport, CLIPBOARD_PROVIDER_ID};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{CommandPlan, CommandPolicy, SecretStdin, StructuredCommandRequest};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// The live clipboard adapter. Constructible only in a live composition; a
/// value cannot exist under `os-control-test`.
pub struct LiveClipboard {
    _seal: (),
}

impl LiveClipboard {
    /// Construct in a live composition root. Requires a [`LiveHostAccessToken`].
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }


    fn unavailable(reason: &'static str, retryable: bool) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(CLIPBOARD_PROVIDER_ID)),
            reason: SafeText::new(reason),
            retryable,
        }
    }

    /// Resolve the backend for **this** session.
    ///
    /// The composition root cannot pass one in (the port takes only the access
    /// token), so eligibility is resolved from the probe-confirmed display
    /// server on the observation context, intersected with the read tools
    /// actually installed. A session whose display server was not conclusively
    /// probed selects nothing: reading the X11 selection in a Wayland session
    /// (or the reverse) would return a foreign observation, which is worse than
    /// an honest `Unavailable`.
    fn backend(&self, ctx: &HostExecutionContext) -> Result<ClipboardBackend, OsControlError> {
        let installed: Vec<ClipboardBackend> = ClipboardBackend::PREFERENCE
            .into_iter()
            .filter(|candidate| {
                std::path::Path::new(candidate.read_executable_path()).is_file()
            })
            .collect();

        select_backend(ctx.session.display_server, &installed).ok_or_else(|| {
            Self::unavailable(
                "no clipboard read backend is installed for this session's display server",
                false,
            )
        })
    }

    /// Run one governed observation and return its bounded stdout.
    ///
    /// The returned string may be the clipboard payload itself, so no caller
    /// may log, trace or embed it in an error.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        backend: ClipboardBackend,
        action: &str,
        argv: Vec<String>,
        truncated_reason: &'static str,
    ) -> Result<String, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            // Never the clipboard payload: a read takes no parameters at all.
            serde_json::Value::Null,
            backend.trusted_read_executable()?,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A partial payload would be digested as if it were the whole
            // clipboard, and a partial type listing could look empty.
            return Err(Self::unavailable(truncated_reason, true));
        }
        Ok(output.stdout)
    }

    /// Resolve the clipboard **history** backend for this session.
    ///
    /// A history store is a separate program from the selection tools: it exists
    /// only when the user runs a clipboard manager. A session without one reports
    /// `Unavailable` — never an empty history, because "no store" and "an empty
    /// store" are different facts and only one of them means nothing was copied.
    fn history_backend(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryBackend, OsControlError> {
        let installed: Vec<ClipboardHistoryBackend> = ClipboardHistoryBackend::PREFERENCE
            .into_iter()
            .filter(|candidate| std::path::Path::new(candidate.executable_path()).is_file())
            .collect();

        select_history_backend(ctx.session.display_server, &installed).ok_or_else(|| {
            Self::unavailable(
                "no clipboard history store is installed for this session; the history state is unknown, not empty",
                false,
            )
        })
    }

    /// Run one governed observation against the history store.
    ///
    /// The returned string contains previews of copied payloads, so no caller may
    /// log, trace or embed it in an error; only the entry-id column is parsed.
    async fn history_query(
        &self,
        ctx: &HostExecutionContext,
        backend: ClipboardHistoryBackend,
        action: &str,
        argv: Vec<String>,
        truncated_reason: &'static str,
    ) -> Result<String, OsControlError> {
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            // A history read takes no parameters, so nothing about the retained
            // content can reach the audit record.
            serde_json::Value::Null,
            backend.trusted_executable()?,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A partial listing would undercount, and an undercount is the one
            // error that would make a failed wipe look successful.
            return Err(Self::unavailable(truncated_reason, true));
        }
        Ok(output.stdout)
    }
}

#[async_trait::async_trait]
impl ClipboardTransport for LiveClipboard {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(CLIPBOARD_PROVIDER_ID)
    }

    async fn read_text(&self, ctx: &HostExecutionContext) -> Result<String, OsControlError> {
        // A clipboard read runs a query child process.
        deny_live_transport(RawTransportKind::Process);

        let backend = self.backend(ctx)?;

        // 1. Observe what the selection offers. Metadata only — this call never
        //    transfers the payload, so an empty or non-text selection is
        //    resolved without ever touching content.
        let offered = self
            .query(
                ctx,
                backend,
                "get_clipboard.offered_types",
                query_offered_types_argv(backend),
                "clipboard type listing was truncated; refusing a partial read",
            )
            .await?;

        match parse_offered_types(backend, &offered)? {
            // A live owner offering no content type: positively empty. This is
            // reached only from a successful listing; a failed listing already
            // returned `Unavailable` above.
            ClipboardOffer::Empty => Ok(String::new()),

            // Refuse what cannot be addressed rather than returning the wrong
            // facts: an image-only selection is not empty and is not text.
            ClipboardOffer::NonTextOnly => Err(OsControlError::Unsupported {
                capability: CapabilityId::new("get_clipboard.non_text"),
                reason: SafeText::new(
                    "the clipboard holds only non-text content; reporting it as text would be a false observation",
                ),
            }),

            // 2. Transfer the payload. `mime` is a compile-time constant, so no
            //    captured host output reaches the argv.
            ClipboardOffer::Text { mime } => {
                self.query(
                    ctx,
                    backend,
                    "get_clipboard",
                    query_text_argv(backend, mime),
                    "clipboard content exceeded the observation bound; refusing a partial read",
                )
                .await
            }
        }
    }

    async fn write_text(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        text: &str,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);

        let backend = self.backend(ctx.observation())?;
        // The payload travels on **stdin**, never in argv: `/proc/<pid>/cmdline`
        // is world-readable and the argv digest lands in the audit record, so an
        // argv transfer would publish a password the user had just copied. Only
        // the byte length is recorded.
        let plan = CommandPlan::new(
            CapabilityId::new("set_clipboard"),
            "set_clipboard",
            serde_json::json!({ "length": text.len() }),
            backend.trusted_write_executable()?,
            write_text_argv(backend),
        )
        .with_secret_stdin(SecretStdin::new(text.as_bytes().to_vec()));

        let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
        request.dispatch().await
    }

    async fn read_history_state(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryState, OsControlError> {
        // Counting the history runs a query child process.
        deny_live_transport(RawTransportKind::Process);

        let backend = self.history_backend(ctx)?;
        let listing = self
            .history_query(
                ctx,
                backend,
                "get_clipboard_history.count",
                history_list_argv(backend),
                "the clipboard history listing was truncated; refusing to report a partial count as the history size",
            )
            .await?;

        // Only the entry-id column is examined. The rest of each line is a
        // preview of copied content and is never parsed, digested or logged.
        let item_count = parse_history_item_count(backend, &listing)?;
        Ok(ClipboardHistoryState { item_count })
    }

    async fn read_history(
        &self,
        ctx: &HostExecutionContext,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<ClipboardHistoryPage, OsControlError> {
        // Fail closed *before* touching the host: confirm a history store exists
        // at all, so "no clipboard manager in this session" stays distinguishable
        // from "this store cannot describe its entries".
        let backend = self.history_backend(ctx)?;

        // The frozen `ClipboardHistoryPage` entry requires `mime`, `byte_count`,
        // `captured_at_ms` and `payload_digest`. `cliphist list` supplies only an
        // entry id and a preview of the copied text: it records no capture time,
        // no declared type and no payload size. Those facts could only be
        // reconstructed by decoding every entry — i.e. by transferring the user's
        // entire copy history, passwords included — and the capture time would
        // still be unavailable.
        //
        // So this reports what is true: the store exists but cannot describe its
        // entries. It does not return an empty page (which would say the history
        // is empty), and it does not synthesize metadata from a preview.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("get_clipboard_history"),
            reason: SafeText::new(format!(
                "the {} history store records no per-entry type, size or capture time, and enumerating entries would require transferring every retained payload; entry metadata cannot be reported without fabricating it",
                backend.as_str()
            )),
        })
    }

    async fn clear_history(
        &self,
        ctx: &AdmittedMutationContext<'_>,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);

        let backend = self.history_backend(ctx.observation())?;
        // Irreversible: every retained payload is destroyed and there is no
        // inverse, so the domain advertises no rollback. The argv carries no
        // clipboard content — the operation takes no parameters at all.
        let plan = CommandPlan::new(
            CapabilityId::new("clear_clipboard_history"),
            "clear_clipboard_history",
            serde_json::Value::Null,
            backend.trusted_executable()?,
            history_wipe_argv(backend),
        );
        let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
        request.dispatch().await
    }

    async fn read_history_config(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<ClipboardHistoryConfigState, OsControlError> {
        let backend = self.history_backend(ctx)?;
        // `cliphist` has no retention configuration: no enable switch, no entry
        // lifetime, no entry cap and no type filter. Reporting invented defaults
        // here would let `configure_clipboard_history` "verify" against settings
        // that do not exist.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("configure_clipboard_history"),
            reason: SafeText::new(format!(
                "the {} history store exposes no retention configuration to read",
                backend.as_str()
            )),
        })
    }

    async fn write_history_config(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        _config: &ClipboardHistoryConfigState,
    ) -> Result<ApplyOutcome, OsControlError> {
        let backend = self.history_backend(ctx.observation())?;
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("configure_clipboard_history"),
            reason: SafeText::new(format!(
                "the {} history store exposes no retention configuration to set",
                backend.as_str()
            )),
        })
    }
}
