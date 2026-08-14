//! Clipboard backend selection, captured-argv construction, and **offer**
//! (MIME-type) parsing.
//!
//! linux-os-control-production **Task 2/§5** (OSC-023, OSC-031, OSC-032),
//! design §9.10.
//!
//! # Why this module parses metadata and never content
//!
//! Clipboard payloads are `DataClass::Content` (OSC-023, OSC-029): the
//! selection routinely holds a password the user copied seconds ago. So every
//! parser here operates on the selection's **offered type list** — pure
//! metadata — and never on the payload. The payload is read by a second
//! governed query whose stdout is returned to the domain untouched and is
//! never parsed, logged, quoted in an error, or written into a test fixture.
//!
//! # Empty is not unreadable
//!
//! An empty clipboard and a clipboard that could not be read are different
//! facts and are never conflated:
//!
//! * the offer query **succeeded** and named no content type → the selection
//!   positively holds nothing → [`ClipboardOffer::Empty`];
//! * the offer query **failed** (no selection owner, missing backend, timeout)
//!   → the state is unknown → the provider returns
//!   [`OsControlError::Unavailable`] and never an empty string.
//!
//! Neither `wl-paste` nor `xclip` exits zero when the selection has no owner,
//! and the governed read path deliberately surfaces a non-zero exit as a
//! failed observation rather than as data, so "no owner" reaches the caller as
//! *unknown*, never as *empty*. Only a live owner that offers no content type
//! is reported as empty.

use crate::os_control::capability::DisplayServer;
use crate::os_control::contract::{Digest, ProviderId, SafeText};
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::OsControlError;

use super::CLIPBOARD_PROVIDER_ID;

/// The concrete host clipboard backend a provider selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardBackend {
    /// The `wl-clipboard` suite (`wl-paste`). Native Wayland selections only.
    WlClipboard,
    /// `xclip`. X11 selections only.
    Xclip,
}

impl ClipboardBackend {
    /// The full, ordered preference list (most preferred first). Callers must
    /// still filter through [`select_backend`], which applies the
    /// display-server eligibility guard.
    pub const PREFERENCE: [ClipboardBackend; 2] =
        [ClipboardBackend::WlClipboard, ClipboardBackend::Xclip];

    /// The stable label used in traces (never model prose).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ClipboardBackend::WlClipboard => "wl-clipboard",
            ClipboardBackend::Xclip => "xclip",
        }
    }

    /// Display-server eligibility. Each backend speaks to exactly one
    /// selection store: `wl-paste` to the Wayland compositor's, `xclip` to the
    /// X11 server's. Reading the *other* session's selection would return a
    /// stale or foreign observation, so eligibility is exclusive rather than a
    /// preference — and an unprobed (`Unknown`) or headless session is
    /// eligible for neither, because guessing which store to read is exactly
    /// the fabricated observation this architecture exists to prevent.
    #[must_use]
    pub fn eligible_for(self, display_server: DisplayServer) -> bool {
        match self {
            ClipboardBackend::WlClipboard => display_server == DisplayServer::Wayland,
            ClipboardBackend::Xclip => display_server == DisplayServer::X11,
        }
    }

    /// The trusted absolute path of this backend's **read** tool. Exposed so
    /// the live adapter can confirm the tool is installed before selecting it.
    #[must_use]
    pub fn read_executable_path(self) -> &'static str {
        match self {
            ClipboardBackend::WlClipboard => "/usr/bin/wl-paste",
            ClipboardBackend::Xclip => "/usr/bin/xclip",
        }
    }

    /// The trusted absolute path of this backend's **write** tool.
    ///
    /// Reading and writing are different binaries under Wayland (`wl-paste` vs
    /// `wl-copy`), so they cannot share one path. Under X11 `xclip` serves both
    /// and the direction is chosen by argv (`-o` versus `-i`).
    #[must_use]
    pub fn write_executable_path(self) -> &'static str {
        match self {
            ClipboardBackend::WlClipboard => "/usr/bin/wl-copy",
            ClipboardBackend::Xclip => "/usr/bin/xclip",
        }
    }

    /// A stable trusted-executable identity for the read tool.
    pub fn trusted_read_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.read_executable_path(),
            Digest::of_str(&format!("{}-read-fallback-v1", self.as_str())),
        )
    }

    /// A stable trusted-executable identity for the write tool.
    pub fn trusted_write_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.write_executable_path(),
            Digest::of_str(&format!("{}-write-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred **eligible** backend that is also present in
/// `available`, or `None` when this session has no readable clipboard backend
/// (→ the provider reports `Unavailable`, never an empty clipboard).
#[must_use]
pub fn select_backend(
    display_server: DisplayServer,
    available: &[ClipboardBackend],
) -> Option<ClipboardBackend> {
    ClipboardBackend::PREFERENCE
        .into_iter()
        .filter(|candidate| candidate.eligible_for(display_server))
        .find(|candidate| available.contains(candidate))
}

/// The argv that lists the types the current selection offers. Metadata only —
/// this call never transfers the payload.
#[must_use]
pub fn query_offered_types_argv(backend: ClipboardBackend) -> Vec<String> {
    match backend {
        ClipboardBackend::WlClipboard => vec!["--list-types".into()],
        ClipboardBackend::Xclip => vec![
            "-selection".into(),
            "clipboard".into(),
            "-t".into(),
            "TARGETS".into(),
            "-o".into(),
        ],
    }
}

/// The argv that transfers the payload as `mime`.
///
/// `mime` is a `&'static str` on purpose: it can only ever be one of
/// [`TEXT_TYPE_PREFERENCE`], so a type token captured from tool output can
/// never itself flow into an argv position.
#[must_use]
pub fn query_text_argv(backend: ClipboardBackend, mime: &'static str) -> Vec<String> {
    match backend {
        // `--no-newline` suppresses the newline `wl-paste` would otherwise
        // append, which would corrupt the content digest.
        ClipboardBackend::WlClipboard => {
            vec!["--no-newline".into(), "--type".into(), mime.into()]
        }
        ClipboardBackend::Xclip => vec![
            "-selection".into(),
            "clipboard".into(),
            "-t".into(),
            mime.into(),
            "-o".into(),
        ],
    }
}

/// What the current selection offers, in metadata terms only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardOffer {
    /// A live selection owner that offers no content type: positively empty.
    Empty,
    /// A text type is offered; the token to request it with.
    Text {
        /// The text type to transfer, always one of [`TEXT_TYPE_PREFERENCE`].
        mime: &'static str,
    },
    /// Content is offered, but none of it is text (e.g. an image-only copy).
    NonTextOnly,
}

/// The text types this domain can transfer, most preferred first. The UTF-8
/// MIME type is preferred because it fixes the encoding; the bare X11 atoms
/// are the legacy fallbacks.
pub const TEXT_TYPE_PREFERENCE: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// X11 meta-targets that describe the selection rather than carry content.
/// Their presence says nothing about whether the selection holds data.
const META_TARGETS: [&str; 7] = [
    "TARGETS",
    "TIMESTAMP",
    "MULTIPLE",
    "SAVE_TARGETS",
    "DELETE",
    "INSERT_SELECTION",
    "INSERT_PROPERTY",
];

fn unparseable(backend: ClipboardBackend) -> OsControlError {
    OsControlError::Unavailable {
        // The backend label is a trace field on the provider identity; the
        // reason stays a fixed label and never carries captured output.
        provider: Some(ProviderId::new(format!(
            "{CLIPBOARD_PROVIDER_ID}-{}",
            backend.as_str()
        ))),
        reason: SafeText::new(
            "clipboard offered-type output could not be parsed; refusing to assume the selection is empty",
        ),
        retryable: true,
    }
}

/// Whether a token looks like a selection type at all: either a MIME type
/// (`type/subtype`) or an X11 target atom (`UTF8_STRING`).
fn looks_like_type(token: &str) -> bool {
    if token.contains('/') {
        // A MIME type may carry parameters (`text/plain; charset=UTF-8`), so an
        // interior space is legal here — but a control character never is.
        return token
            .chars()
            .all(|c| !c.is_control() && c != '\0' && c != '\n');
    }
    // X11 atom targets (`UTF8_STRING`, `TARGETS`) carry no parameters.
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The argv for **writing** the clipboard. The payload itself is never an argv
/// element: it is delivered on the child's stdin via
/// [`crate::os_control::linux::structured_command::SecretStdin`], because argv is
/// world-readable through `/proc/<pid>/cmdline` and clipboard content routinely
/// holds a password the user just copied.
#[must_use]
pub fn write_text_argv(backend: ClipboardBackend) -> Vec<String> {
    match backend {
        // `--foreground` would keep wl-copy alive owning the selection; the
        // default forks a background owner and exits, which is what a bounded
        // governed child needs.
        ClipboardBackend::WlClipboard => vec!["--type".into(), "text/plain".into()],
        // `-i` reads the selection from stdin; `-selection clipboard` targets the
        // normal clipboard rather than the X11 PRIMARY selection.
        ClipboardBackend::Xclip => vec![
            "-selection".into(),
            "clipboard".into(),
            "-i".into(),
        ],
    }
}

/// Parse a backend's offered-type listing.
///
/// **Fail-closed:** output that does not look like a type listing is an error,
/// never [`ClipboardOffer::Empty`]. Reporting "the clipboard is empty" because
/// `wl-paste` changed its output format would let `set_clipboard` verify
/// against a fact that was never observed, and would tell the user their
/// clipboard is empty when it is not.
pub fn parse_offered_types(
    backend: ClipboardBackend,
    stdout: &str,
) -> Result<ClipboardOffer, OsControlError> {
    let mut content_types: Vec<&str> = Vec::new();
    // One type per LINE, not per whitespace-separated word: a MIME type may carry
    // a parameter (`text/plain; charset=UTF-8`) and splitting on whitespace would
    // tear it into two tokens, the second of which is not a type at all.
    for line in stdout.lines() {
        let token = line.trim();
        if token.is_empty() {
            continue;
        }
        if !looks_like_type(token) {
            return Err(unparseable(backend));
        }
        if META_TARGETS
            .iter()
            .any(|meta| meta.eq_ignore_ascii_case(token))
        {
            continue;
        }
        content_types.push(token);
    }

    if content_types.is_empty() {
        // A successful listing that names no content type is a positively
        // empty selection. A *failed* listing never reaches this function.
        return Ok(ClipboardOffer::Empty);
    }

    for candidate in TEXT_TYPE_PREFERENCE {
        if content_types
            .iter()
            .any(|offered| normalized_type(offered) == normalized_type(candidate))
        {
            return Ok(ClipboardOffer::Text { mime: candidate });
        }
    }

    Ok(ClipboardOffer::NonTextOnly)
}

/// Normalize a type token for comparison: case-insensitive, and with the
/// optional whitespace real tools emit around MIME parameters removed
/// (`text/plain; charset=UTF-8` and `text/plain;charset=utf-8` are the same
/// offer).
fn normalized_type(token: &str) -> String {
    token
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    // Every fixture below is a *type listing*. No clipboard payload appears in
    // any fixture in this file, by design.

    #[test]
    fn selection_is_exclusive_per_display_server() {
        use ClipboardBackend::*;
        let both = [WlClipboard, Xclip];
        assert_eq!(
            select_backend(DisplayServer::Wayland, &both),
            Some(WlClipboard)
        );
        assert_eq!(select_backend(DisplayServer::X11, &both), Some(Xclip));
        // Installed but not eligible for this session.
        assert_eq!(select_backend(DisplayServer::X11, &[WlClipboard]), None);
        assert_eq!(select_backend(DisplayServer::Wayland, &[Xclip]), None);
        // Unprobed or headless: refuse rather than read the wrong store.
        assert_eq!(select_backend(DisplayServer::Unknown, &both), None);
        assert_eq!(select_backend(DisplayServer::Headless, &both), None);
        assert_eq!(select_backend(DisplayServer::Wayland, &[]), None);
    }

    #[test]
    fn captured_query_argv_golden() {
        assert_eq!(
            query_offered_types_argv(ClipboardBackend::WlClipboard),
            vec!["--list-types"]
        );
        assert_eq!(
            query_offered_types_argv(ClipboardBackend::Xclip),
            vec!["-selection", "clipboard", "-t", "TARGETS", "-o"]
        );
        assert_eq!(
            query_text_argv(ClipboardBackend::WlClipboard, "text/plain;charset=utf-8"),
            vec!["--no-newline", "--type", "text/plain;charset=utf-8"]
        );
        assert_eq!(
            query_text_argv(ClipboardBackend::Xclip, "UTF8_STRING"),
            vec!["-selection", "clipboard", "-t", "UTF8_STRING", "-o"]
        );
    }

    #[test]
    fn trusted_read_executables_are_absolute() {
        for backend in ClipboardBackend::PREFERENCE {
            let exe = backend
                .trusted_read_executable()
                .expect("valid trusted executable");
            assert!(exe.path().starts_with('/'));
        }
    }

    #[test]
    fn wl_paste_type_list_prefers_utf8_mime() {
        let out = "text/plain;charset=utf-8\ntext/plain\nTEXT\nSTRING\nUTF8_STRING\n";
        assert_eq!(
            parse_offered_types(ClipboardBackend::WlClipboard, out).unwrap(),
            ClipboardOffer::Text {
                mime: "text/plain;charset=utf-8"
            }
        );
    }

    #[test]
    fn xclip_targets_meta_atoms_are_ignored() {
        let out = "TIMESTAMP\nTARGETS\nMULTIPLE\nSAVE_TARGETS\nUTF8_STRING\nSTRING\nTEXT\n";
        assert_eq!(
            parse_offered_types(ClipboardBackend::Xclip, out).unwrap(),
            ClipboardOffer::Text {
                mime: "UTF8_STRING"
            }
        );
    }

    #[test]
    fn mime_parameter_spacing_and_case_are_normalized() {
        // Real toolkits emit `text/plain; charset=UTF-8`.
        let out = "text/plain; charset=UTF-8\n";
        assert_eq!(
            parse_offered_types(ClipboardBackend::WlClipboard, out).unwrap(),
            ClipboardOffer::Text {
                mime: "text/plain;charset=utf-8"
            }
        );
    }

    #[test]
    fn image_only_offer_is_not_text_and_not_empty() {
        let out = "image/png\nimage/jpeg\napplication/x-kde-cutselection\n";
        assert_eq!(
            parse_offered_types(ClipboardBackend::WlClipboard, out).unwrap(),
            ClipboardOffer::NonTextOnly
        );
    }

    #[test]
    fn owner_offering_only_meta_targets_is_positively_empty() {
        let out = "TIMESTAMP\nTARGETS\nMULTIPLE\n";
        assert_eq!(
            parse_offered_types(ClipboardBackend::Xclip, out).unwrap(),
            ClipboardOffer::Empty
        );
        // A successful listing with no tokens at all is the same fact.
        assert_eq!(
            parse_offered_types(ClipboardBackend::WlClipboard, "").unwrap(),
            ClipboardOffer::Empty
        );
        assert_eq!(
            parse_offered_types(ClipboardBackend::WlClipboard, "   \n\n").unwrap(),
            ClipboardOffer::Empty
        );
    }

    #[test]
    fn unrecognised_output_is_an_error_never_an_empty_clipboard() {
        // The whole point: a tool that starts printing prose must not become
        // "your clipboard is empty".
        for backend in ClipboardBackend::PREFERENCE {
            assert!(parse_offered_types(backend, "Nothing is copied").is_err());
            assert!(parse_offered_types(backend, "Error: target TARGETS not available").is_err());
            assert!(parse_offered_types(backend, "usage: xclip [OPTION]").is_err());
        }
    }

    #[test]
    fn parse_error_text_never_quotes_tool_output() {
        let err = parse_offered_types(ClipboardBackend::WlClipboard, "Nothing is copied")
            .expect_err("must fail closed");
        let rendered = format!("{err:?}");
        assert!(!rendered.contains("Nothing is copied"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Clipboard *history* backend selection (Task 4.9, OSC-023)
// ─────────────────────────────────────────────────────────────────────────────
//
// The clipboard *selection* (above) and the clipboard *history* are two
// different stores. A selection is owned by the display server and always
// exists; a history exists only when the user runs a clipboard manager, and it
// is a rolling log of everything they ever copied — passwords included. Nothing
// in this section ever parses, digests, logs or returns a history payload: the
// only facts derived here are the number of retained entries and whether a
// backend exists at all.

/// The concrete clipboard-**history** backend a provider selected.
///
/// A history manager is a separate program from the selection tools above: this
/// is `cliphist`, the store `wl-paste --watch cliphist store` feeds under
/// Wayland. GNOME and plain X11 sessions have no clipboard history at all, and
/// that absence is reported as `Unavailable` rather than as an empty history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardHistoryBackend {
    /// `cliphist` — the wlroots/Wayland clipboard history store.
    Cliphist,
}

impl ClipboardHistoryBackend {
    /// The full, ordered preference list (most preferred first).
    pub const PREFERENCE: [ClipboardHistoryBackend; 1] = [ClipboardHistoryBackend::Cliphist];

    /// The stable label used in traces (never model prose).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ClipboardHistoryBackend::Cliphist => "cliphist",
        }
    }

    /// Display-server eligibility.
    ///
    /// `cliphist` is fed by `wl-paste --watch`, so it only ever holds a Wayland
    /// session's history. An X11, headless or unprobed session is eligible for
    /// nothing: guessing that some other manager holds the history would be a
    /// fabricated observation.
    #[must_use]
    pub fn eligible_for(self, display_server: DisplayServer) -> bool {
        match self {
            ClipboardHistoryBackend::Cliphist => display_server == DisplayServer::Wayland,
        }
    }

    /// The trusted absolute path of this backend's tool.
    #[must_use]
    pub fn executable_path(self) -> &'static str {
        match self {
            ClipboardHistoryBackend::Cliphist => "/usr/bin/cliphist",
        }
    }

    /// A stable trusted-executable identity.
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            Digest::of_str(&format!("{}-history-fallback-v1", self.as_str())),
        )
    }
}

/// Select the most-preferred **eligible** history backend that is also present
/// in `available`, or `None` when this session has no clipboard history store
/// (→ the provider reports `Unavailable`, never an empty history).
#[must_use]
pub fn select_history_backend(
    display_server: DisplayServer,
    available: &[ClipboardHistoryBackend],
) -> Option<ClipboardHistoryBackend> {
    ClipboardHistoryBackend::PREFERENCE
        .into_iter()
        .filter(|candidate| candidate.eligible_for(display_server))
        .find(|candidate| available.contains(candidate))
}

/// The argv that lists the retained history entries.
///
/// The listing is used **only** to count entries. Each line carries a preview of
/// the copied payload, which is why the parser below reads the leading id and
/// never looks at the rest of the line.
#[must_use]
pub fn history_list_argv(backend: ClipboardHistoryBackend) -> Vec<String> {
    match backend {
        ClipboardHistoryBackend::Cliphist => vec!["list".into()],
    }
}

/// The argv that destroys the entire retained history. Irreversible.
#[must_use]
pub fn history_wipe_argv(backend: ClipboardHistoryBackend) -> Vec<String> {
    match backend {
        ClipboardHistoryBackend::Cliphist => vec!["wipe".into()],
    }
}

/// Count the retained history entries from a backend's listing.
///
/// **Privacy:** every line is `<id>\t<preview>`, where the preview is a fragment
/// of something the user copied. Only the id prefix is examined; the remainder of
/// the line is never inspected, digested, logged or returned. No fixture in this
/// file contains a preview.
///
/// **Fail-closed:** a line that is not an id-prefixed entry is an error, never a
/// skipped line and never a zero count. Reporting "the history is empty" because
/// the tool changed its output would let `clear_clipboard_history` verify against
/// a fact nobody observed, and would tell the user their copy history was
/// destroyed when it was not.
pub fn parse_history_item_count(
    backend: ClipboardHistoryBackend,
    stdout: &str,
) -> Result<u64, OsControlError> {
    let mut count: u64 = 0;
    for line in stdout.lines() {
        // A trailing empty line is a line terminator, not an entry.
        if line.trim().is_empty() {
            continue;
        }
        if !has_entry_id_prefix(backend, line) {
            return Err(unparseable_history(backend));
        }
        count += 1;
    }
    // A *successful* listing with no entries is a positively empty history. A
    // failed listing never reaches this function.
    Ok(count)
}

/// Whether `line` starts with this backend's entry-id column.
///
/// Reads the id prefix only, and stops at the separator — the preview after it is
/// deliberately never examined.
fn has_entry_id_prefix(backend: ClipboardHistoryBackend, line: &str) -> bool {
    match backend {
        // `cliphist list` emits `<decimal id>\t<preview>`.
        ClipboardHistoryBackend::Cliphist => match line.split_once('\t') {
            Some((id, _preview)) => {
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
            }
            None => false,
        },
    }
}

/// The history listing could not be interpreted. Names the tool, never any
/// clipboard content.
fn unparseable_history(backend: ClipboardHistoryBackend) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new(CLIPBOARD_PROVIDER_ID)),
        reason: SafeText::new(format!(
            "the {} history listing was not in the expected form; refusing to report a history state that was not observed",
            backend.as_str()
        )),
        retryable: false,
    }
}

#[cfg(test)]
mod history_parse_tests {
    use super::*;

    // No fixture below contains a clipboard payload or a payload preview: the
    // parser never looks past the id column, so the tests do not need one.

    #[test]
    fn history_backend_is_wayland_only() {
        use ClipboardHistoryBackend::Cliphist;
        let installed = [Cliphist];
        assert_eq!(
            select_history_backend(DisplayServer::Wayland, &installed),
            Some(Cliphist)
        );
        // Installed but not eligible: cliphist only ever holds a Wayland history.
        assert_eq!(select_history_backend(DisplayServer::X11, &installed), None);
        // Unprobed or headless: refuse rather than claim a history store.
        assert_eq!(
            select_history_backend(DisplayServer::Unknown, &installed),
            None
        );
        assert_eq!(
            select_history_backend(DisplayServer::Headless, &installed),
            None
        );
        // Nothing installed: no history store, which is not an empty history.
        assert_eq!(select_history_backend(DisplayServer::Wayland, &[]), None);
    }

    #[test]
    fn captured_history_argv_golden() {
        assert_eq!(
            history_list_argv(ClipboardHistoryBackend::Cliphist),
            vec!["list".to_string()]
        );
        assert_eq!(
            history_wipe_argv(ClipboardHistoryBackend::Cliphist),
            vec!["wipe".to_string()]
        );
    }

    #[test]
    fn empty_listing_is_zero_entries() {
        assert_eq!(
            parse_history_item_count(ClipboardHistoryBackend::Cliphist, "").unwrap(),
            0
        );
        assert_eq!(
            parse_history_item_count(ClipboardHistoryBackend::Cliphist, "\n").unwrap(),
            0
        );
    }

    #[test]
    fn counts_id_prefixed_entries() {
        // Tab-separated `<id>\t<preview>`; the preview column is a single
        // placeholder character because the parser must never read it.
        let listing = "3\tx\n2\tx\n1\tx\n";
        assert_eq!(
            parse_history_item_count(ClipboardHistoryBackend::Cliphist, listing).unwrap(),
            3
        );
    }

    #[test]
    fn unrecognised_output_is_an_error_not_a_default() {
        // The single most important test in this file: an unreadable history must
        // never degrade into "the history is empty".
        for hostile in [
            "cliphist: command not found",
            "usage: cliphist [list|decode|wipe]",
            "no-tab-separator",
            "\tmissing id",
            "abc\tnon numeric id",
        ] {
            let err = parse_history_item_count(ClipboardHistoryBackend::Cliphist, hostile);
            assert!(
                err.is_err(),
                "unrecognised history listing must be an error, not a count: {hostile}"
            );
        }
    }
}
