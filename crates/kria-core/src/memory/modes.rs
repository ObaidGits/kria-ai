//! Memory modes: the deterministic mode gate (task **F1.4.4**; design §23,
//! MGR-035; glossary `Memory_Mode`).
//!
//! MGR-035 / the glossary define **five canonical `Memory_Mode` classes** and
//! their exact admission / read / session-purge behavior:
//!
//! | Class          | Admission (durable write)                        | Read                                   | Session purge |
//! |----------------|--------------------------------------------------|----------------------------------------|---------------|
//! | `Permanent`    | durable                                          | normal (any session)                   | none          |
//! | `Temporary`    | session-scoped with a bounded, expiring lifetime | owning session, until the record expires | purged at session end **or** expiry |
//! | `Session_Only` | session-scoped, bound to the current session     | owning session only                    | purged at session end |
//! | `Read_Only`    | **rejected** with a typed mode error             | normal (authorized reads preserved)    | n/a           |
//! | `Disabled`     | **rejected** with a typed mode error             | **rejected** with a typed mode error   | n/a           |
//!
//! Two invariants are load-bearing and property-tested below:
//!
//! * **Typed mode errors.** Every rejection is a typed [`ModeError`] naming the
//!   [`MemoryMode`], its [`ModeClass`], and the [`ModeErrorKind`] (write- or
//!   read-forbidden, or unknown mode). No rejection is a generic string or a
//!   silent drop.
//! * **No hidden durable fallback.** `Temporary`/`Session_Only` admission is
//!   *always* session-scoped, never durable — a session-scoped mode can never
//!   silently become `Permanent`. A `Read_Only`/`Disabled`/unknown mode can
//!   never admit a durable write. And a session-scoped record stops being
//!   readable the instant its session closes **regardless of whether the
//!   physical purge succeeded** ([`SessionScopeLedger`]) — a failed
//!   `Session_Only` purge never leaves data readable as `Permanent`.
//!
//! ## Reconciliation with the historical mode gate
//!
//! The pre-redesign write gate ([`evaluate`], used by
//! [`crate::memory::authority::validation`] F1.3.2) recognized finer-grained
//! product modes (`Developer`, `Workspace`, `LibraryOnly`, `Incognito`,
//! `Guest`, …). Rather than fork a parallel gate, every [`MemoryMode`] maps onto
//! exactly one canonical [`ModeClass`] via [`MemoryMode::class`], and both
//! [`evaluate`] (the write-decision gate) and [`admit`] (the canonical
//! admission function) derive from that single mapping so they can never
//! disagree. [`evaluate`] additionally layers the namespace/ingest context gates
//! that the `Workspace`/`LibraryOnly` product modes carry.
//!
//! ## Governed-path / F2 binding
//!
//! Concrete cognitive-record tables (and the `memory_mode_sessions` /
//! `deletion_jobs` authority tables of design §19.2) land in **F2**. This module
//! implements the mode admission decision, the per-record session/expiry scoping
//! metadata, and the purge *mechanism* at the policy layer: [`SessionScopeLedger`]
//! is the F1 in-RAM authority for session scope and purge state. F2 persists the
//! same state to `memory_mode_sessions(session_id, mode, …, purge_state, closed_at)`
//! and routes the actual record deletions through the governed
//! `AuthorityTransaction` (they are never durable authority truth beyond the
//! session). Readability is decided from this scoping metadata, never from
//! whether a physical delete has run — which is what makes a failed purge safe.

use std::collections::HashMap;

use chrono::Duration;
use dashmap::DashMap;
use uuid::Uuid;

use crate::memory::error::{MemoryError, PermissionError};
use crate::memory::ids::Timestamp;
use crate::memory::types::{MemoryMode, RejectReason};

// ─────────────────────────────────────────────────────────────────────────
// Canonical mode class
// ─────────────────────────────────────────────────────────────────────────

/// One of the five canonical `Memory_Mode` classes (MGR-035, glossary). Every
/// [`MemoryMode`] variant maps onto exactly one of these (or none, for an
/// unknown forward-compat value which fails closed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeClass {
    /// Durable admission; normal reads.
    Permanent,
    /// Session-scoped admission with a bounded, expiring lifetime; reads until
    /// expiry within the owning session.
    Temporary,
    /// Session-scoped admission; reads limited to the owning session; purged at
    /// session end.
    SessionOnly,
    /// Durable writes rejected (typed error); authorized reads preserved.
    ReadOnly,
    /// Durable writes and reads both rejected (typed error); honest degraded
    /// surface.
    Disabled,
}

impl ModeClass {
    /// The canonical snake_case text (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            ModeClass::Permanent => "permanent",
            ModeClass::Temporary => "temporary",
            ModeClass::SessionOnly => "session_only",
            ModeClass::ReadOnly => "read_only",
            ModeClass::Disabled => "disabled",
        }
    }

    /// Whether admission under this class produces a durable write.
    pub fn admits_durable(self) -> bool {
        matches!(self, ModeClass::Permanent)
    }

    /// Whether admission under this class produces a session-scoped write.
    pub fn admits_session_scoped(self) -> bool {
        matches!(self, ModeClass::Temporary | ModeClass::SessionOnly)
    }

    /// Whether any durable write is admissible under this class.
    pub fn allows_writes(self) -> bool {
        self.admits_durable() || self.admits_session_scoped()
    }

    /// Whether reads are permitted under this class (all but `Disabled`).
    pub fn allows_reads(self) -> bool {
        !matches!(self, ModeClass::Disabled)
    }
}

impl std::fmt::Display for ModeClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl MemoryMode {
    /// Map this mode onto its canonical [`ModeClass`]. Returns `None` for an
    /// unknown/forward-compat [`MemoryMode::Other`] value, which the gate treats
    /// as fail-closed (no durable fallback).
    ///
    /// The mapping (documented on [`MemoryMode`]):
    ///
    /// * `Permanent`, `Developer`, `Research`, `Benchmark`, `Safe`, `Workspace`,
    ///   `LibraryOnly` → [`ModeClass::Permanent`] (durable-class; `Workspace`/
    ///   `LibraryOnly` additionally carry namespace/ingest context gates applied
    ///   by [`evaluate`]).
    /// * `Temporary` → [`ModeClass::Temporary`].
    /// * `SessionOnly` → [`ModeClass::SessionOnly`].
    /// * `ReadOnly`, `Incognito`, `Guest` → [`ModeClass::ReadOnly`] (no durable
    ///   write, reads preserved).
    /// * `Disabled` → [`ModeClass::Disabled`].
    /// * `Other(_)` → `None` (fail closed).
    pub fn class(&self) -> Option<ModeClass> {
        use MemoryMode::*;
        Some(match self {
            Permanent | Developer | Research | Benchmark | Safe | Workspace | LibraryOnly => {
                ModeClass::Permanent
            }
            Temporary => ModeClass::Temporary,
            SessionOnly => ModeClass::SessionOnly,
            ReadOnly | Incognito | Guest => ModeClass::ReadOnly,
            Disabled => ModeClass::Disabled,
            Other(_) => return None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Typed mode errors
// ─────────────────────────────────────────────────────────────────────────

/// Why a mode rejected an operation. A rejection is always one of these — never
/// a generic string or a silent drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeErrorKind {
    /// The mode forbids durable/session writes (`Read_Only`, `Disabled`).
    WriteForbidden,
    /// The mode forbids reads (`Disabled`).
    ReadForbidden,
    /// The mode value is unknown/forward-compat and has no canonical class, so
    /// it fails closed for every operation.
    UnknownMode,
}

impl ModeErrorKind {
    /// The canonical snake_case text.
    pub fn as_str(self) -> &'static str {
        match self {
            ModeErrorKind::WriteForbidden => "write_forbidden",
            ModeErrorKind::ReadForbidden => "read_forbidden",
            ModeErrorKind::UnknownMode => "unknown_mode",
        }
    }
}

/// A typed mode-gate rejection naming the [`MemoryMode`], its canonical
/// [`ModeClass`] (absent for an unknown mode), and the [`ModeErrorKind`]. This
/// is the single typed error every mode rejection returns (MGR-035 AC6/AC7).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ModeError {
    /// The offending mode.
    pub mode: MemoryMode,
    /// Its canonical class, or `None` if the mode is unknown/forward-compat.
    pub class: Option<ModeClass>,
    /// What was forbidden.
    pub kind: ModeErrorKind,
}

impl ModeError {
    fn write_forbidden(mode: &MemoryMode) -> Self {
        Self {
            mode: mode.clone(),
            class: mode.class(),
            kind: ModeErrorKind::WriteForbidden,
        }
    }

    fn read_forbidden(mode: &MemoryMode) -> Self {
        Self {
            mode: mode.clone(),
            class: mode.class(),
            kind: ModeErrorKind::ReadForbidden,
        }
    }

    fn unknown(mode: &MemoryMode) -> Self {
        Self {
            mode: mode.clone(),
            class: None,
            kind: ModeErrorKind::UnknownMode,
        }
    }
}

impl std::fmt::Display for ModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.class {
            Some(class) => write!(
                f,
                "memory mode `{}` (class {}) {}",
                self.mode,
                class,
                self.kind.as_str()
            ),
            None => write!(
                f,
                "memory mode `{}` is unknown ({})",
                self.mode,
                self.kind.as_str()
            ),
        }
    }
}

impl From<ModeError> for MemoryError {
    /// Surface a mode rejection through the canonical error taxonomy
    /// ([`PermissionError::Mode`]). The typed [`ModeError`] is preserved for
    /// callers that match on it directly; this conversion is for boundaries that
    /// only speak [`MemoryError`].
    fn from(e: ModeError) -> Self {
        MemoryError::Permission(PermissionError::Mode(e.mode))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Session binding + admission
// ─────────────────────────────────────────────────────────────────────────

/// The default bounded lifetime of a `Temporary`-mode record: it is purged at
/// this age even if its session never explicitly closes (design §23 temporary
/// retention; the lifecycle sweep uses [`SessionScopeLedger::expired`]).
pub const DEFAULT_TEMPORARY_RETENTION: Duration = Duration::hours(24);

/// How a session-scoped admitted record is bound to its session. A durable
/// (`Permanent`-class) admission carries **no** binding and is never tracked
/// here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionBinding {
    /// `Temporary`: readable within the owning session until `expires_at`, and
    /// purged at session end or expiry (whichever comes first).
    Temporary { expires_at: Timestamp },
    /// `Session_Only`: readable only within the owning session; purged at
    /// session end.
    SessionOnly,
}

impl SessionBinding {
    /// A `Temporary` binding expiring [`DEFAULT_TEMPORARY_RETENTION`] after
    /// `now`.
    pub fn temporary(now: Timestamp) -> Self {
        SessionBinding::Temporary {
            expires_at: now + DEFAULT_TEMPORARY_RETENTION,
        }
    }
}

/// The canonical admission decision for a mode. Never carries a durable result
/// for a session-scoped class, so a session-scoped mode can never silently
/// become `Permanent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// A durable write (`Permanent`-class).
    Durable,
    /// A session-scoped write bound as described (`Temporary`/`Session_Only`).
    SessionScoped(SessionBinding),
}

impl Admission {
    /// Whether this admission is durable.
    pub fn is_durable(self) -> bool {
        matches!(self, Admission::Durable)
    }

    /// The session binding, if the admission is session-scoped.
    pub fn session_binding(self) -> Option<SessionBinding> {
        match self {
            Admission::SessionScoped(b) => Some(b),
            Admission::Durable => None,
        }
    }
}

/// The canonical **admission** decision for `mode` at time `now`
/// (MGR-035 AC4/AC5/AC6/AC7). This is the mode-class gate:
///
/// * `Permanent`-class → `Ok(Admission::Durable)`.
/// * `Temporary` → `Ok(Admission::SessionScoped(Temporary{expires_at}))`.
/// * `Session_Only` → `Ok(Admission::SessionScoped(SessionOnly))`.
/// * `Read_Only` / `Disabled` → `Err(ModeError { WriteForbidden })`.
/// * unknown mode → `Err(ModeError { UnknownMode })` (fail closed).
///
/// Namespace/scope gating (e.g. `Workspace` personal scope, `LibraryOnly`
/// ingest) is not a mode-class concern; it is applied by [`evaluate`] and the
/// Effective-Policy meet (F1.4.2).
pub fn admit(mode: &MemoryMode, now: Timestamp) -> Result<Admission, ModeError> {
    match mode.class() {
        Some(ModeClass::Permanent) => Ok(Admission::Durable),
        Some(ModeClass::Temporary) => Ok(Admission::SessionScoped(SessionBinding::temporary(now))),
        Some(ModeClass::SessionOnly) => Ok(Admission::SessionScoped(SessionBinding::SessionOnly)),
        Some(ModeClass::ReadOnly) | Some(ModeClass::Disabled) => {
            Err(ModeError::write_forbidden(mode))
        }
        None => Err(ModeError::unknown(mode)),
    }
}

/// The canonical **read** gate for a *reading* session's mode (MGR-035 AC6/AC7).
/// `Disabled` forbids reads with a typed error; an unknown mode fails closed;
/// every other class permits reads (per-record session/expiry scoping is then
/// enforced by [`SessionScopeLedger`]).
pub fn read_permitted(mode: &MemoryMode) -> Result<(), ModeError> {
    match mode.class() {
        Some(class) if class.allows_reads() => Ok(()),
        Some(_) => Err(ModeError::read_forbidden(mode)), // Disabled
        None => Err(ModeError::unknown(mode)),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Write-decision gate (historical F1.3.2 surface)
// ─────────────────────────────────────────────────────────────────────────

/// Context the mode gate needs to decide a write.
#[derive(Clone, Copy, Debug)]
pub struct ModeWriteContext {
    /// Whether the candidate is personal-scoped (rejected in Workspace mode).
    pub is_personal_scope: bool,
    /// Whether the candidate is a library-ingestion write (the only kind
    /// allowed in Library-only mode).
    pub is_library_ingest: bool,
}

/// The fast-path decision for a write under a given mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeWriteDecision {
    /// Persist normally (policy-governed).
    Allow,
    /// Persist but tag session-scoped (purged at session end — Temporary /
    /// Session_Only modes).
    AllowSessionScoped,
    /// Do not persist; return this reject reason.
    Reject(RejectReason),
}

/// Evaluate the mode decision table (design §23, MGR-035 AC4–AC7). Deterministic
/// and LLM-free. Derives the class decision from [`admit`] (single source of
/// truth) and layers the `Workspace`/`LibraryOnly` namespace/ingest context
/// gates so it can never disagree with the canonical admission function.
pub fn evaluate(mode: &MemoryMode, ctx: &ModeWriteContext) -> ModeWriteDecision {
    // Context gates carried by the finer-grained product modes take precedence
    // over the shared Permanent-class admission they map to.
    match mode {
        MemoryMode::Workspace if ctx.is_personal_scope => {
            return ModeWriteDecision::Reject(RejectReason::NamespaceViolation);
        }
        MemoryMode::LibraryOnly if !ctx.is_library_ingest => {
            return ModeWriteDecision::Reject(RejectReason::Mode(MemoryMode::LibraryOnly));
        }
        _ => {}
    }

    // The mode-class admission decision is authoritative. `now` only affects a
    // Temporary binding's expiry, which the write-decision surface discards, so
    // any instant is fine here.
    match admit(mode, chrono::Utc::now()) {
        Ok(Admission::Durable) => ModeWriteDecision::Allow,
        Ok(Admission::SessionScoped(_)) => ModeWriteDecision::AllowSessionScoped,
        Err(e) => ModeWriteDecision::Reject(RejectReason::Mode(e.mode)),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Per-session mode registry
// ─────────────────────────────────────────────────────────────────────────

/// Per-session mode registry. In-RAM authoritative state (design §27 caching):
/// the current mode is transient and re-derived from the session row on resume.
#[derive(Debug)]
pub struct ModeManager {
    modes: DashMap<Uuid, MemoryMode>,
    default: MemoryMode,
}

impl ModeManager {
    pub fn new(default: MemoryMode) -> Self {
        Self {
            modes: DashMap::new(),
            default,
        }
    }

    /// Current mode for a session (defaults to the configured default).
    pub fn current(&self, session_id: Uuid) -> MemoryMode {
        self.modes
            .get(&session_id)
            .map(|m| m.clone())
            .unwrap_or_else(|| self.default.clone())
    }

    /// Switch a session's mode. Returns the previous mode (for the boundary
    /// event the caller emits).
    pub fn set_mode(&self, session_id: Uuid, mode: MemoryMode) -> MemoryMode {
        let prev = self.current(session_id);
        self.modes.insert(session_id, mode);
        prev
    }

    /// Whether writes in this session's mode are session-scoped
    /// (`Temporary`/`Session_Only`) so they can be purged at session end.
    pub fn is_session_scoped(&self, session_id: Uuid) -> bool {
        self.current(session_id)
            .class()
            .is_some_and(ModeClass::admits_session_scoped)
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Session-scope + purge ledger
// ─────────────────────────────────────────────────────────────────────────

/// The purge lifecycle of a session that admitted session-scoped records
/// (mirrors `memory_mode_sessions.purge_state`, design §19.2). Once a session
/// leaves [`PurgeState::Open`] its scoped records are unreadable forever —
/// including when the physical purge later fails — so no session-scoped record
/// can ever be read as if it were `Permanent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PurgeState {
    /// The session is live; its scoped records are readable per their binding.
    Open,
    /// The session has closed and its records have been handed to the governed
    /// purge but the delete has not yet been confirmed.
    Purging,
    /// The governed purge confirmed deletion of the session's records.
    Purged,
    /// The governed purge failed. Records remain excluded from every read (they
    /// are *not* readable as durable truth) and are retried by the lifecycle.
    PurgeFailed,
}

impl PurgeState {
    /// The canonical snake_case text (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            PurgeState::Open => "open",
            PurgeState::Purging => "purging",
            PurgeState::Purged => "purged",
            PurgeState::PurgeFailed => "purge_failed",
        }
    }
}

/// The outcome of a per-record readability check against the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopedRead {
    /// The record is readable by the requesting session at the given time.
    Readable,
    /// The record's session has closed (purged, purging, or purge-failed): not
    /// readable, regardless of physical-delete success.
    SessionClosed,
    /// The requesting session is not the record's owning session.
    WrongSession,
    /// A `Temporary` record whose expiry has passed.
    Expired,
    /// The record is not a tracked session-scoped record.
    NotTracked,
}

impl ScopedRead {
    /// Whether this outcome permits the read.
    pub fn is_readable(self) -> bool {
        matches!(self, ScopedRead::Readable)
    }
}

/// A batch of records a closing/expiring session hands to the governed purge
/// path. In F2 these ids drive `AuthorityTransaction` deletions and
/// `deletion_jobs`; here they are the mechanism's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeBatch {
    /// The session whose records are being purged.
    pub session_id: Uuid,
    /// The record ids to purge.
    pub record_ids: Vec<Uuid>,
}

#[derive(Debug)]
struct SessionEntry {
    mode: MemoryMode,
    purge_state: PurgeState,
    /// record id → binding for every session-scoped record admitted here.
    scoped: HashMap<Uuid, SessionBinding>,
}

/// In-RAM authority for session scope and purge state (task F1.4.4). It tracks
/// every session-scoped (`Temporary`/`Session_Only`) admitted record, decides
/// per-record readability from that scoping metadata (never from physical
/// existence), and produces the [`PurgeBatch`] the governed path deletes at
/// session end or expiry.
///
/// F2 persists the same state to `memory_mode_sessions` and executes the purge
/// through the `AuthorityTransaction`; this type is the F1 mechanism and the
/// authority for *visibility*.
#[derive(Debug, Default)]
pub struct SessionScopeLedger {
    sessions: DashMap<Uuid, SessionEntry>,
}

impl SessionScopeLedger {
    /// A fresh, empty ledger.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Open a session under `mode`. Idempotent for an already-open session;
    /// re-opening a closed session is refused (its scoped records are gone).
    pub fn open_session(&self, session_id: Uuid, mode: MemoryMode) {
        self.sessions
            .entry(session_id)
            .or_insert_with(|| SessionEntry {
                mode,
                purge_state: PurgeState::Open,
                scoped: HashMap::new(),
            });
    }

    /// Record a session-scoped admission so it can be scoped for reads and
    /// purged later. The session is auto-opened if not already tracked. Returns
    /// `false` if the session is not `Open` (a closed session never admits new
    /// records — no durable fallback).
    pub fn admit_record(&self, session_id: Uuid, record_id: Uuid, binding: SessionBinding) -> bool {
        let mut entry = self
            .sessions
            .entry(session_id)
            .or_insert_with(|| SessionEntry {
                mode: MemoryMode::SessionOnly,
                purge_state: PurgeState::Open,
                scoped: HashMap::new(),
            });
        if entry.purge_state != PurgeState::Open {
            return false;
        }
        entry.scoped.insert(record_id, binding);
        true
    }

    /// The current purge state of a session, if tracked.
    pub fn purge_state(&self, session_id: Uuid) -> Option<PurgeState> {
        self.sessions.get(&session_id).map(|e| e.purge_state)
    }

    /// The mode a tracked session was opened under (mirrors
    /// `memory_mode_sessions.mode`, design §19.2), if tracked.
    pub fn session_mode(&self, session_id: Uuid) -> Option<MemoryMode> {
        self.sessions.get(&session_id).map(|e| e.mode.clone())
    }

    /// Decide whether `record_id` is readable by `reading_session` at `now`.
    ///
    /// A session-scoped record is readable only when **all** hold: its owning
    /// session is still `Open`, the reader is the owning session, and (for
    /// `Temporary`) its expiry has not passed. The owning-session-`Open` check
    /// is what guarantees a failed purge never leaves data readable.
    pub fn read_decision(
        &self,
        record_id: Uuid,
        reading_session: Uuid,
        now: Timestamp,
    ) -> ScopedRead {
        // The record lives under its owning session.
        let entry = match self.sessions.get(&reading_session) {
            Some(e) if e.scoped.contains_key(&record_id) => e,
            // Not owned by the reading session: either owned elsewhere or not
            // tracked at all. Distinguish so callers can tell "wrong session"
            // from "not a scoped record".
            _ => {
                let owned_elsewhere = self
                    .sessions
                    .iter()
                    .any(|e| e.scoped.contains_key(&record_id));
                return if owned_elsewhere {
                    ScopedRead::WrongSession
                } else {
                    ScopedRead::NotTracked
                };
            }
        };

        if entry.purge_state != PurgeState::Open {
            return ScopedRead::SessionClosed;
        }
        match entry.scoped.get(&record_id) {
            Some(SessionBinding::Temporary { expires_at }) if now >= *expires_at => {
                ScopedRead::Expired
            }
            Some(_) => ScopedRead::Readable,
            None => ScopedRead::NotTracked,
        }
    }

    /// Close a session at end-of-session: transition `Open → Purging` and return
    /// the [`PurgeBatch`] the governed path must delete. From this point the
    /// session's scoped records are unreadable ([`ScopedRead::SessionClosed`]).
    /// A no-op (empty batch) for an unknown or already-closed session.
    pub fn close_session(&self, session_id: Uuid) -> PurgeBatch {
        match self.sessions.get_mut(&session_id) {
            Some(mut entry) if entry.purge_state == PurgeState::Open => {
                entry.purge_state = PurgeState::Purging;
                PurgeBatch {
                    session_id,
                    record_ids: entry.scoped.keys().copied().collect(),
                }
            }
            _ => PurgeBatch {
                session_id,
                record_ids: Vec::new(),
            },
        }
    }

    /// Confirm the governed purge succeeded: `Purging → Purged`. The scoped
    /// record set is cleared (they are gone). Ignored unless the session is
    /// `Purging`.
    pub fn mark_purged(&self, session_id: Uuid) {
        if let Some(mut entry) = self.sessions.get_mut(&session_id) {
            if entry.purge_state == PurgeState::Purging {
                entry.purge_state = PurgeState::Purged;
                entry.scoped.clear();
            }
        }
    }

    /// Record that the governed purge failed: `Purging → PurgeFailed`. The
    /// scoped set is **retained** for retry, but the records stay unreadable
    /// (the session is not `Open`) — a failed purge never restores visibility.
    pub fn mark_purge_failed(&self, session_id: Uuid) {
        if let Some(mut entry) = self.sessions.get_mut(&session_id) {
            if entry.purge_state == PurgeState::Purging {
                entry.purge_state = PurgeState::PurgeFailed;
            }
        }
    }

    /// The `(session_id, record_id)` pairs of `Temporary` records whose expiry
    /// has passed as of `now`, across all open sessions — the lifecycle sweep's
    /// input. Expired records are already excluded from reads by
    /// [`Self::read_decision`]; this drives their physical purge.
    pub fn expired(&self, now: Timestamp) -> Vec<(Uuid, Uuid)> {
        let mut out = Vec::new();
        for entry in self.sessions.iter() {
            if entry.purge_state != PurgeState::Open {
                continue;
            }
            let session_id = *entry.key();
            for (record_id, binding) in &entry.scoped {
                if let SessionBinding::Temporary { expires_at } = binding {
                    if now >= *expires_at {
                        out.push((session_id, *record_id));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use proptest::prelude::*;

    fn ctx(personal: bool, library: bool) -> ModeWriteContext {
        ModeWriteContext {
            is_personal_scope: personal,
            is_library_ingest: library,
        }
    }

    // ── Historical write-decision gate (F1.3.2 invariants preserved) ─────

    #[test]
    fn incognito_and_readonly_reject_all() {
        assert!(matches!(
            evaluate(&MemoryMode::Incognito, &ctx(false, false)),
            ModeWriteDecision::Reject(RejectReason::Mode(MemoryMode::Incognito))
        ));
        assert!(matches!(
            evaluate(&MemoryMode::ReadOnly, &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn workspace_rejects_personal_scope() {
        assert_eq!(
            evaluate(&MemoryMode::Workspace, &ctx(true, false)),
            ModeWriteDecision::Reject(RejectReason::NamespaceViolation)
        );
        assert_eq!(
            evaluate(&MemoryMode::Workspace, &ctx(false, false)),
            ModeWriteDecision::Allow
        );
    }

    #[test]
    fn library_only_allows_only_ingest() {
        assert_eq!(
            evaluate(&MemoryMode::LibraryOnly, &ctx(false, true)),
            ModeWriteDecision::Allow
        );
        assert!(matches!(
            evaluate(&MemoryMode::LibraryOnly, &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn temporary_and_session_only_are_session_scoped() {
        assert_eq!(
            evaluate(&MemoryMode::Temporary, &ctx(false, false)),
            ModeWriteDecision::AllowSessionScoped
        );
        assert_eq!(
            evaluate(&MemoryMode::SessionOnly, &ctx(false, false)),
            ModeWriteDecision::AllowSessionScoped
        );
    }

    #[test]
    fn disabled_rejects_writes() {
        assert_eq!(
            evaluate(&MemoryMode::Disabled, &ctx(false, false)),
            ModeWriteDecision::Reject(RejectReason::Mode(MemoryMode::Disabled))
        );
    }

    #[test]
    fn unknown_mode_fails_safe() {
        assert!(matches!(
            evaluate(&MemoryMode::Other("weird".into()), &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn manager_tracks_and_switches() {
        let mgr = ModeManager::new(MemoryMode::Permanent);
        let s = Uuid::now_v7();
        assert_eq!(mgr.current(s), MemoryMode::Permanent);
        let prev = mgr.set_mode(s, MemoryMode::Incognito);
        assert_eq!(prev, MemoryMode::Permanent);
        assert_eq!(mgr.current(s), MemoryMode::Incognito);
        assert!(!mgr.is_session_scoped(s));
        mgr.set_mode(s, MemoryMode::Temporary);
        assert!(mgr.is_session_scoped(s));
        mgr.set_mode(s, MemoryMode::SessionOnly);
        assert!(mgr.is_session_scoped(s));
    }

    // ── Canonical class mapping ──────────────────────────────────────────

    #[test]
    fn class_mapping_covers_the_five_canonical_classes() {
        assert_eq!(MemoryMode::Permanent.class(), Some(ModeClass::Permanent));
        assert_eq!(MemoryMode::Developer.class(), Some(ModeClass::Permanent));
        assert_eq!(MemoryMode::Workspace.class(), Some(ModeClass::Permanent));
        assert_eq!(MemoryMode::LibraryOnly.class(), Some(ModeClass::Permanent));
        assert_eq!(MemoryMode::Temporary.class(), Some(ModeClass::Temporary));
        assert_eq!(
            MemoryMode::SessionOnly.class(),
            Some(ModeClass::SessionOnly)
        );
        assert_eq!(MemoryMode::ReadOnly.class(), Some(ModeClass::ReadOnly));
        assert_eq!(MemoryMode::Incognito.class(), Some(ModeClass::ReadOnly));
        assert_eq!(MemoryMode::Guest.class(), Some(ModeClass::ReadOnly));
        assert_eq!(MemoryMode::Disabled.class(), Some(ModeClass::Disabled));
        assert_eq!(MemoryMode::Other("x".into()).class(), None);
    }

    // ── Admission: typed errors + no hidden durable fallback ─────────────

    #[test]
    fn permanent_admits_durable() {
        assert_eq!(
            admit(&MemoryMode::Permanent, Utc::now()),
            Ok(Admission::Durable)
        );
    }

    #[test]
    fn temporary_admits_expiring_session_scope_never_durable() {
        let now = Utc::now();
        let a = admit(&MemoryMode::Temporary, now).unwrap();
        assert!(!a.is_durable(), "Temporary must never admit durably");
        match a.session_binding().unwrap() {
            SessionBinding::Temporary { expires_at } => {
                assert_eq!(expires_at, now + DEFAULT_TEMPORARY_RETENTION);
            }
            other => panic!("expected Temporary binding, got {other:?}"),
        }
    }

    #[test]
    fn session_only_admits_session_scope_never_durable() {
        let a = admit(&MemoryMode::SessionOnly, Utc::now()).unwrap();
        assert_eq!(a, Admission::SessionScoped(SessionBinding::SessionOnly));
        assert!(!a.is_durable());
    }

    #[test]
    fn read_only_and_disabled_reject_writes_with_typed_error() {
        let e = admit(&MemoryMode::ReadOnly, Utc::now()).unwrap_err();
        assert_eq!(e.mode, MemoryMode::ReadOnly);
        assert_eq!(e.class, Some(ModeClass::ReadOnly));
        assert_eq!(e.kind, ModeErrorKind::WriteForbidden);

        let e = admit(&MemoryMode::Disabled, Utc::now()).unwrap_err();
        assert_eq!(e.class, Some(ModeClass::Disabled));
        assert_eq!(e.kind, ModeErrorKind::WriteForbidden);
    }

    #[test]
    fn unknown_mode_admission_fails_closed_with_typed_error() {
        let e = admit(&MemoryMode::Other("weird".into()), Utc::now()).unwrap_err();
        assert_eq!(e.class, None);
        assert_eq!(e.kind, ModeErrorKind::UnknownMode);
    }

    // ── Read gate ────────────────────────────────────────────────────────

    #[test]
    fn reads_permitted_except_disabled_and_unknown() {
        assert!(read_permitted(&MemoryMode::Permanent).is_ok());
        assert!(read_permitted(&MemoryMode::ReadOnly).is_ok());
        assert!(read_permitted(&MemoryMode::Temporary).is_ok());
        assert!(read_permitted(&MemoryMode::SessionOnly).is_ok());

        let e = read_permitted(&MemoryMode::Disabled).unwrap_err();
        assert_eq!(e.kind, ModeErrorKind::ReadForbidden);

        let e = read_permitted(&MemoryMode::Other("weird".into())).unwrap_err();
        assert_eq!(e.kind, ModeErrorKind::UnknownMode);
    }

    // ── Session-scope ledger ─────────────────────────────────────────────

    #[test]
    fn session_only_record_is_readable_only_by_owning_open_session() {
        let ledger = SessionScopeLedger::new();
        let owner = Uuid::now_v7();
        let other = Uuid::now_v7();
        let rec = Uuid::now_v7();
        let now = Utc::now();

        ledger.open_session(owner, MemoryMode::SessionOnly);
        assert!(ledger.admit_record(owner, rec, SessionBinding::SessionOnly));

        assert_eq!(ledger.read_decision(rec, owner, now), ScopedRead::Readable);
        assert_eq!(
            ledger.read_decision(rec, other, now),
            ScopedRead::WrongSession
        );
    }

    #[test]
    fn closing_session_purges_and_hides_records() {
        let ledger = SessionScopeLedger::new();
        let owner = Uuid::now_v7();
        let rec = Uuid::now_v7();
        let now = Utc::now();

        ledger.open_session(owner, MemoryMode::SessionOnly);
        ledger.admit_record(owner, rec, SessionBinding::SessionOnly);

        let batch = ledger.close_session(owner);
        assert_eq!(batch.record_ids, vec![rec]);
        assert_eq!(ledger.purge_state(owner), Some(PurgeState::Purging));
        // Unreadable immediately on close, before the physical delete confirms.
        assert_eq!(
            ledger.read_decision(rec, owner, now),
            ScopedRead::SessionClosed
        );

        ledger.mark_purged(owner);
        assert_eq!(ledger.purge_state(owner), Some(PurgeState::Purged));
        assert_eq!(
            ledger.read_decision(rec, owner, now),
            ScopedRead::NotTracked
        );
    }

    #[test]
    fn failed_purge_never_leaves_data_readable_as_permanent() {
        // The load-bearing no-hidden-fallback case: a failed Session_Only purge
        // must not restore visibility.
        let ledger = SessionScopeLedger::new();
        let owner = Uuid::now_v7();
        let rec = Uuid::now_v7();
        let now = Utc::now();

        ledger.open_session(owner, MemoryMode::SessionOnly);
        ledger.admit_record(owner, rec, SessionBinding::SessionOnly);
        ledger.close_session(owner);
        ledger.mark_purge_failed(owner);

        assert_eq!(ledger.purge_state(owner), Some(PurgeState::PurgeFailed));
        assert_eq!(
            ledger.read_decision(rec, owner, now),
            ScopedRead::SessionClosed,
            "a failed purge must never make a session-scoped record readable"
        );
    }

    #[test]
    fn closed_session_admits_no_new_records() {
        let ledger = SessionScopeLedger::new();
        let owner = Uuid::now_v7();
        ledger.open_session(owner, MemoryMode::SessionOnly);
        ledger.close_session(owner);
        // No durable fallback: a closed session cannot admit.
        assert!(!ledger.admit_record(owner, Uuid::now_v7(), SessionBinding::SessionOnly));
    }

    #[test]
    fn temporary_record_expires_and_is_swept() {
        let ledger = SessionScopeLedger::new();
        let owner = Uuid::now_v7();
        let rec = Uuid::now_v7();
        let t0 = Utc::now();
        let binding = SessionBinding::temporary(t0);

        ledger.open_session(owner, MemoryMode::Temporary);
        ledger.admit_record(owner, rec, binding);

        // Before expiry: readable by the owner.
        assert_eq!(ledger.read_decision(rec, owner, t0), ScopedRead::Readable);
        // After expiry: excluded and swept.
        let after = t0 + DEFAULT_TEMPORARY_RETENTION + Duration::seconds(1);
        assert_eq!(ledger.read_decision(rec, owner, after), ScopedRead::Expired);
        assert_eq!(ledger.expired(after), vec![(owner, rec)]);
        assert!(ledger.expired(t0).is_empty());
    }

    // ── Property tests ───────────────────────────────────────────────────

    fn any_mode() -> impl Strategy<Value = MemoryMode> {
        prop_oneof![
            Just(MemoryMode::Permanent),
            Just(MemoryMode::Temporary),
            Just(MemoryMode::SessionOnly),
            Just(MemoryMode::Incognito),
            Just(MemoryMode::Workspace),
            Just(MemoryMode::LibraryOnly),
            Just(MemoryMode::ReadOnly),
            Just(MemoryMode::Disabled),
            Just(MemoryMode::Guest),
            Just(MemoryMode::Developer),
            Just(MemoryMode::Benchmark),
            Just(MemoryMode::Safe),
            Just(MemoryMode::Research),
            "[a-z]{1,8}".prop_map(MemoryMode::Other),
        ]
    }

    proptest! {
        /// No hidden durable fallback: a session-scoped class never admits
        /// durably, and a non-writing class (Read_Only/Disabled/unknown) never
        /// admits at all.
        #[test]
        fn admit_never_falls_back_to_durable(mode in any_mode()) {
            let result = admit(&mode, Utc::now());
            match mode.class() {
                Some(ModeClass::Permanent) => prop_assert!(matches!(result, Ok(Admission::Durable))),
                Some(ModeClass::Temporary) | Some(ModeClass::SessionOnly) => {
                    let a = result.expect("session-scoped class admits");
                    prop_assert!(!a.is_durable(), "session-scoped mode must not admit durably");
                }
                Some(ModeClass::ReadOnly) | Some(ModeClass::Disabled) => {
                    let e = result.expect_err("non-writing class must reject");
                    prop_assert_eq!(e.kind, ModeErrorKind::WriteForbidden);
                    prop_assert_eq!(e.mode, mode.clone());
                }
                None => {
                    let e = result.expect_err("unknown mode fails closed");
                    prop_assert_eq!(e.kind, ModeErrorKind::UnknownMode);
                }
            }
        }

        /// The canonical `admit` and the historical `evaluate` gate never
        /// disagree on whether a write is admitted (single source of truth),
        /// once the Workspace/LibraryOnly namespace/ingest gates are satisfied.
        #[test]
        fn evaluate_agrees_with_admit(mode in any_mode()) {
            // Neutral context: not personal scope, and library-ingest true so the
            // LibraryOnly context gate passes and only the class decision remains.
            let decision = evaluate(&mode, &ctx(false, true));
            match admit(&mode, Utc::now()) {
                Ok(Admission::Durable) => prop_assert_eq!(decision, ModeWriteDecision::Allow),
                Ok(Admission::SessionScoped(_)) => {
                    prop_assert_eq!(decision, ModeWriteDecision::AllowSessionScoped)
                }
                Err(_) => prop_assert!(matches!(decision, ModeWriteDecision::Reject(_))),
            }
        }

        /// A session-scoped record is never readable once its session leaves the
        /// Open state — regardless of whether the purge succeeded or failed.
        #[test]
        fn closed_session_records_never_readable(purge_ok in any::<bool>()) {
            let ledger = SessionScopeLedger::new();
            let owner = Uuid::now_v7();
            let rec = Uuid::now_v7();
            let now = Utc::now();
            ledger.open_session(owner, MemoryMode::SessionOnly);
            ledger.admit_record(owner, rec, SessionBinding::SessionOnly);
            ledger.close_session(owner);
            if purge_ok {
                ledger.mark_purged(owner);
            } else {
                ledger.mark_purge_failed(owner);
            }
            prop_assert!(!ledger.read_decision(rec, owner, now).is_readable());
        }
    }
}
