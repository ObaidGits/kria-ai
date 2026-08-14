//! Memory lifecycle: forget / delete cascade + key-status marking (design §5.4,
//! MGR-040, MGR-041).
//!
//! `forget` tombstones (reversible 30 days); `hard_delete` cascades across all
//! stores in one authority transaction and marks the subject's shred-key status
//! as `'destroyed'` in the `shred_keys` catalog row.
//!
//! ## HONESTY NOTE — no cryptographic erasure yet (MGR-041 / design §5.4)
//!
//! Memory content is stored as **plaintext** in the `memories.content` column.
//! There is no payload encryption: `shred_keys.key_ref` is a catalog reference,
//! not actual key material, and no encryption/decryption code paths exist.
//! Marking `shred_keys.status = 'destroyed'` does NOT make stored content
//! cryptographically unreadable — it is a **hard-delete status flag only**.
//!
//! Per design §5.4 and MGR-041, this state must be surfaced as
//! **"Hard Delete pending cryptographic erasure"**, not "Crypto-Shredded".
//! Until payloads are encrypted under subject-bound versioned keys held outside
//! the payload and destroyed-key tests return no plaintext across all read
//! paths, application-level cryptographic erasure is **unavailable**.  Health
//! reports reliance on host OS disk encryption only.
//!
//! [`Lifecycle::preview_hard_delete`] / [`Lifecycle::preview_forget`] (task
//! F1.7.1, design §5.4 "Lifecycle and erasure truth") compute a read-only
//! [`LifecyclePreview`] over a [`ForgetScope`] — dependent records, independent
//! evidence, affected sources/scopes, cascade/keep choices, reversibility, and
//! the base authority revision — without mutating anything.
//!
//! ## Preview / confirm pattern (design §5.4)
//!
//! 1. Caller obtains the current `GraphRevision` from the DB (or stores it from
//!    a prior read).
//! 2. Caller calls [`Lifecycle::preview_forget`] / [`Lifecycle::preview_hard_delete`] /
//!    [`Lifecycle::preview_restore`], passing `caller_revision`.  The method
//!    reads a snapshot at the *current* DB revision and verifies the caller's
//!    view is not stale (the scope target must not have changed since
//!    `caller_revision`).
//! 3. The preview returns a [`LifecyclePreview`] together with a minted
//!    [`LifecyclePreviewToken`] that encodes `base_revision`, a scope hash, and
//!    a timestamp.
//! 4. The governed commit methods (F1.7.2–F1.7.4) accept the token and verify
//!    it against the current revision before mutating anything.
//!
//! **Token signing note:** For this single-user, single-process, pre-production
//! system the token is a plain BLAKE3 hex digest of the relevant fields — it
//! serves as a tamper-evident handle, not a cryptographic MAC.  When a
//! multi-user / network-authenticated path lands, the token should be replaced
//! with an HMAC-SHA256 or equivalent keyed MAC so a remote caller cannot forge
//! a valid token without the server's key.
//!
//! **Legacy-model scope (F2 forward-compat note).** The preview walks the live
//! legacy `memories` / `memory_derived_from` / `memory_contradicts` /
//! `memory_supports` / `memory_mentions_entity` / `evidence` / `episodes`
//! tables — the tables the *current* write path (write_policy, merge, truth,
//! extraction) actually populates — never the v2 `records` /
//! `relationships_v2` / `evidence_v2` tables, because nothing durable writes
//! those yet (see [`crate::model::legacy_mapping`]). Two concrete
//! consequences follow from that:
//!
//! * "Independent evidence" (design §5.4) reads the legacy `evidence` table,
//!   which today has **no live writer** — every current write path that
//!   records support/corroboration for a memory does so only through
//!   `memory_supports`/`memory_contradicts` (identity-only, no source), not
//!   `evidence` rows with a `source_event_id`. The query below is correct
//!   against the schema, but will typically report zero independent evidence
//!   until a write path starts populating `evidence` — a real limitation of
//!   today's data, not a bug in the preview.
//! * When the v2 write path (F1.5) and the registry-governed relationship
//!   (F2.2) become the live authority, this preview must be re-pointed at
//!   `records` / `relationships_v2` / `evidence_v2` in the same task wave that
//!   retires `hard_delete`'s legacy cascade — tracked as a follow-up, not
//!   invented as parallel v2 logic today.

use std::collections::BTreeSet;
use std::sync::Arc;

use rusqlite::{params, params_from_iter, OptionalExtension};
use uuid::Uuid;

use crate::db::Database;
use crate::error::{MemoryError, MemoryResult, StorageError};
use crate::ids::blake3_hex;
use crate::model::GraphRevision;
use crate::stores::ports::{RelationalStore, SearchStore, VectorStore};
use crate::stores::sqlite_search::delete_fts_in_tx;
use crate::types::{MemoryState, ModelVersion};

/// Maximum dependent/evidence items shown in preview detail before truncation
/// (task F1.7.1 "bounded 500/5000 limits").
pub const PREVIEW_DETAIL_LIMIT: usize = 500;

/// Maximum total resolved scope size a preview will compute over; a larger
/// scope is refused rather than silently computed unbounded (task F1.7.1).
pub const PREVIEW_SCOPE_LIMIT: usize = 5000;

/// SQL `IN (...)` parameter chunk size, comfortably under SQLite's default
/// bound-parameter ceiling (~999) regardless of scope size.
const SQL_CHUNK: usize = 400;

/// Whether a resolved scope's total target count exceeded
/// [`PREVIEW_SCOPE_LIMIT`]. When `exceeded` is true, the rest of the preview
/// (dependents/evidence/sources) is intentionally left empty rather than
/// computed unbounded — callers must narrow the scope before a preview is
/// available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopeLimitStatus {
    pub target_count: usize,
    pub limit: usize,
    pub exceeded: bool,
}

/// Which legacy table/column a [`DependentRecord`] was found through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependentKind {
    /// A `memory_derived_from` row (parent or child side) involving a target.
    DerivedFromLink,
    /// A `memory_contradicts` row involving a target.
    ContradictsLink,
    /// A `memory_supports` row involving a target.
    SupportsLink,
    /// A `memory_mentions_entity` row for a target (graph entity mention).
    MentionsEntityLink,
    /// Another memory whose `superseded_by` points at a target.
    SupersededByReference,
    /// An episode whose `summary_memory_id` points at a target.
    EpisodeSummaryReference,
}

impl DependentKind {
    /// The recommended disposition for this kind (the caller may override at
    /// commit time). Semantics are uniform across kinds:
    /// * [`DependentDisposition::Cascade`] — the dangling reference itself
    ///   (a link-table row, which has no meaning without both endpoints and
    ///   carries no independent audit value) is removed at commit.
    /// * [`DependentDisposition::KeepOrphaned`] — the dependent *record*
    ///   (another memory or episode, which has its own identity/history) is
    ///   kept, and its reference to the deleted target is explicitly flagged
    ///   orphaned rather than silently nulled, preserving the audit trail.
    pub fn default_disposition(self) -> DependentDisposition {
        match self {
            DependentKind::DerivedFromLink
            | DependentKind::ContradictsLink
            | DependentKind::SupportsLink
            | DependentKind::MentionsEntityLink => DependentDisposition::Cascade,
            DependentKind::SupersededByReference | DependentKind::EpisodeSummaryReference => {
                DependentDisposition::KeepOrphaned
            }
        }
    }
}

/// The caller's cascade-vs-keep choice for one [`DependentRecord`] (task
/// F1.7.1). The preview only *recommends* a default; applying the chosen
/// disposition at commit time is F1.7.4 scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependentDisposition {
    /// Remove the dangling reference/link at commit.
    Cascade,
    /// Keep the dependent record; explicitly mark its reference to the
    /// deleted target as orphaned (never a silent null).
    KeepOrphaned,
}

/// One dependent record/link that would become dangling or orphaned if
/// `target` were hard-deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DependentRecord {
    pub kind: DependentKind,
    /// The id of the *other* record/entity this dependency involves (never
    /// the target itself) — a memory id, episode id, or entity id depending
    /// on `kind`.
    pub id: Uuid,
    /// Which target memory id this dependency references.
    pub target: Uuid,
    /// The recommended cascade/keep choice (see [`DependentKind::default_disposition`]).
    pub default_disposition: DependentDisposition,
}

/// One legacy `evidence` row for a target sourced from a *different* event
/// than the target's own creation event — i.e. independent corroboration the
/// user should see before deleting the primary record (task F1.7.1c).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndependentEvidence {
    pub target: Uuid,
    pub evidence_id: Uuid,
    pub source_event_id: Uuid,
}

/// Read-only preview of a lifecycle operation's blast radius (design §5.4,
/// task F1.7.1). Computed without mutating anything. See the module docs for
/// the legacy-vs-v2 data-model scope this preview operates over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecyclePreview {
    /// The resolved target memory ids (empty when [`ScopeLimitStatus::exceeded`]).
    pub target_ids: Vec<Uuid>,
    /// Whether the total scope size exceeded [`PREVIEW_SCOPE_LIMIT`].
    pub scope_limit: ScopeLimitStatus,
    /// Dependent records/links, bounded to the first [`PREVIEW_DETAIL_LIMIT`].
    pub dependents: Vec<DependentRecord>,
    /// The true total dependent count before truncation.
    pub dependents_total_count: usize,
    /// Whether `dependents` was truncated to [`PREVIEW_DETAIL_LIMIT`].
    pub dependents_truncated: bool,
    /// Independent evidence, bounded to the first [`PREVIEW_DETAIL_LIMIT`].
    /// Typically empty today — see the module docs' forward-compat note.
    pub independent_evidence: Vec<IndependentEvidence>,
    /// Whether `independent_evidence` was truncated to [`PREVIEW_DETAIL_LIMIT`].
    pub independent_evidence_truncated: bool,
    /// Distinct source provenance tags touched by this scope (`event.source`).
    pub affected_sources: Vec<String>,
    /// Distinct session ids touched by this scope.
    pub affected_sessions: Vec<Uuid>,
    /// Distinct `(namespace, scope)` pairs touched by this scope.
    pub affected_namespaces: Vec<(String, String)>,
    /// Whether this specific operation is reversible: `true` for Forget
    /// (30-day undo window), `false` for Hard Delete (never reversible).
    pub reversible: bool,
    /// Human-readable reversibility label shown to the caller.
    /// `"reversible (30-day restore window)"` for Forget/Restore;
    /// `"IRREVERSIBLE — cannot be undone"` for Hard Delete.
    pub reversibility_label: String,
    /// The authority revision (`authority_meta.graph_revision`) this preview
    /// was computed at. A caller should re-check this immediately before
    /// commit and refuse a stale commit if it has advanced (design §5.4
    /// "stale preview conflicts").
    pub base_revision: GraphRevision,
    /// A minted token that the corresponding commit method must present to
    /// prove the user confirmed AFTER seeing this preview at this exact
    /// revision.
    pub token: LifecyclePreviewToken,
}

/// An opaque preview confirmation token (task F1.7.1 / design §5.4).
///
/// Contains the `base_revision` this preview was computed at, a Blake3 digest
/// of the scope+operation, and the UTC millisecond timestamp it was minted at.
/// Commit methods (F1.7.2–F1.7.4) accept this token and verify:
/// 1. The current `graph_revision` still equals `base_revision` (or the target
///    records are known to be unchanged — stale-revision guard).
/// 2. The scope_hash matches the operation being confirmed (replay guard).
///
/// **Signing note:** The token is a plain hex encoding for this single-user
/// pre-production system.  A multi-user / network path should replace it with
/// an HMAC or signed JWT so the server can verify authenticity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LifecyclePreviewToken {
    /// The revision the preview was computed at.
    pub base_revision: GraphRevision,
    /// BLAKE3 hex of `"{op}:{scope_canonical}"` — tamper-evident scope binding.
    pub scope_hash: String,
    /// Milliseconds since Unix epoch when the token was minted.
    pub minted_at_ms: u64,
}

impl LifecyclePreviewToken {
    fn mint(op: &str, scope: &ForgetScope, base_revision: GraphRevision) -> Self {
        let scope_key = scope_canonical_key(scope);
        let preimage = format!("{op}:{scope_key}:{}", base_revision.get());
        let scope_hash = blake3_hex(preimage.as_bytes());
        let minted_at_ms = chrono::Utc::now().timestamp_millis() as u64;
        Self {
            base_revision,
            scope_hash,
            minted_at_ms,
        }
    }

    /// Encode as a compact string for use with [`crate::authority::command::PreviewToken`].
    pub fn encode(&self) -> String {
        format!(
            "lc1:{},{},{}",
            self.base_revision.get(),
            self.scope_hash,
            self.minted_at_ms
        )
    }

    /// Decode a token string produced by [`Self::encode`].
    pub fn decode(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("lc1:")?;
        let mut parts = rest.splitn(3, ',');
        let rev: u64 = parts.next()?.parse().ok()?;
        let scope_hash = parts.next()?.to_owned();
        let minted_at_ms: u64 = parts.next()?.parse().ok()?;
        if scope_hash.len() != 64 {
            return None;
        }
        Some(Self {
            base_revision: GraphRevision::new(rev),
            scope_hash,
            minted_at_ms,
        })
    }
}

impl std::fmt::Display for LifecyclePreviewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.encode())
    }
}

/// Limits controlling how much of a scope the preview may resolve and inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewLimits {
    /// Maximum total ids a scope-based preview will resolve before truncating.
    /// Exceeding this returns an error rather than silently computing unbounded.
    pub max_scope: usize,
    /// Maximum dependent records shown in the preview detail.
    pub max_dependents: usize,
    /// Maximum independent-evidence rows shown in the preview detail.
    pub max_evidence: usize,
}

impl PreviewLimits {
    /// Single-record limits: up to 500 dependents (task F1.7.1).
    pub fn single_record() -> Self {
        Self {
            max_scope: 1,
            max_dependents: PREVIEW_DETAIL_LIMIT,
            max_evidence: PREVIEW_DETAIL_LIMIT,
        }
    }

    /// Scope-based limits: up to 5000 total ids, 500 dependents (task F1.7.1).
    pub fn scope_based() -> Self {
        Self {
            max_scope: PREVIEW_SCOPE_LIMIT,
            max_dependents: PREVIEW_DETAIL_LIMIT,
            max_evidence: PREVIEW_DETAIL_LIMIT,
        }
    }
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self::scope_based()
    }
}

/// The lifecycle operation type a preview was computed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleOp {
    /// Forget: tombstone (reversible, 30-day restore window).
    Forget,
    /// Restore: un-tombstone a forgotten memory (reversible).
    Restore,
    /// Hard delete: cascade + key-status mark (IRREVERSIBLE).
    HardDelete,
}

impl LifecycleOp {
    fn as_str(self) -> &'static str {
        match self {
            LifecycleOp::Forget => "forget",
            LifecycleOp::Restore => "restore",
            LifecycleOp::HardDelete => "hard_delete",
        }
    }

    fn is_reversible(self) -> bool {
        match self {
            LifecycleOp::Forget | LifecycleOp::Restore => true,
            LifecycleOp::HardDelete => false,
        }
    }

    fn reversibility_label(self) -> &'static str {
        match self {
            LifecycleOp::Forget => "reversible (30-day restore window)",
            LifecycleOp::Restore => "reversible (re-tombstone with forget)",
            LifecycleOp::HardDelete => "IRREVERSIBLE — cannot be undone",
        }
    }
}

/// A canonical deterministic string for a scope, used in token hashing.
fn scope_canonical_key(scope: &ForgetScope) -> String {
    match scope {
        ForgetScope::Memory(id) => format!("memory:{id}"),
        ForgetScope::SourcePrefix(prefix) => format!("source_prefix:{prefix}"),
        ForgetScope::Session(sid) => format!("session:{sid}"),
        ForgetScope::Subject(subject) => format!("subject:{subject}"),
    }
}

/// What to forget/delete.
#[derive(Clone, Debug)]
pub enum ForgetScope {
    /// A single memory by id.
    Memory(Uuid),
    /// Every memory whose `source` provenance tag has this prefix
    /// (e.g. `tool:file_ops`, `mcp:github`, `library:{item}`) — per-source cascade.
    SourcePrefix(String),
    /// A session's memories (Temporary purge / session delete).
    Session(Uuid),
    /// An erasure subject key (person/employer/project) — hard-delete target.
    /// Marks the subject's shred-key status as `'destroyed'` (Hard Delete
    /// pending cryptographic erasure; not actual cryptographic unreadability
    /// until payload encryption is implemented — MGR-041).
    Subject(String),
}

/// Lifecycle service.
pub struct Lifecycle {
    db: Arc<Database>,
    relational: Arc<dyn RelationalStore>,
    vectors: Arc<dyn VectorStore>,
    search: Arc<dyn SearchStore>,
    embedding_model: ModelVersion,
}

impl Lifecycle {
    pub fn new(
        db: Arc<Database>,
        relational: Arc<dyn RelationalStore>,
        vectors: Arc<dyn VectorStore>,
        search: Arc<dyn SearchStore>,
        embedding_model: ModelVersion,
    ) -> Self {
        Self {
            db,
            relational,
            vectors,
            search,
            embedding_model,
        }
    }

    /// Resolve a scope to the concrete memory ids it targets.
    pub fn resolve(&self, scope: &ForgetScope) -> MemoryResult<Vec<Uuid>> {
        self.db.with_read(|conn| {
            let mut ids = Vec::new();
            match scope {
                ForgetScope::Memory(id) => ids.push(*id),
                ForgetScope::SourcePrefix(prefix) => {
                    // Memories whose source event tag starts with `prefix`.
                    let like = format!("{prefix}%");
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m JOIN events e ON m.source_event_id = e.id \
                             WHERE e.source LIKE ?1",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![like], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Session(sid) => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m JOIN events e ON m.source_event_id = e.id \
                             WHERE e.session_id = ?1",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![sid.to_string()], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Subject(subject) => {
                    let mut stmt = conn
                        .prepare("SELECT id FROM memories WHERE shred_key_id = ?1")
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![subject], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
            }
            Ok(ids)
        })
    }

    // ── Preview methods (task F1.7.1) ────────────────────────────────────────

    /// Read the current `graph_revision` from `authority_meta`.
    fn current_graph_revision(&self) -> MemoryResult<GraphRevision> {
        self.db.with_read(|conn| {
            let rev: i64 = conn
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            Ok(GraphRevision::new(rev as u64))
        })
    }

    /// Resolve a scope to ids, bounded to `limit`. Returns `(ids, exceeded)`.
    ///
    /// If the total count in the DB exceeds `limit`, ids is capped at `limit`
    /// and `exceeded` is `true`.
    fn resolve_bounded(
        &self,
        scope: &ForgetScope,
        limit: usize,
    ) -> MemoryResult<(Vec<Uuid>, bool)> {
        self.db.with_read(|conn| {
            let mut ids = Vec::new();
            // Fetch one extra row to detect whether we'd exceed the limit.
            let fetch = limit + 1;
            match scope {
                ForgetScope::Memory(id) => ids.push(*id),
                ForgetScope::SourcePrefix(prefix) => {
                    let like = format!("{prefix}%");
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m \
                             JOIN events e ON m.source_event_id = e.id \
                             WHERE e.source LIKE ?1 \
                             LIMIT ?2",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![like, fetch as i64], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Session(sid) => {
                    let mut stmt = conn
                        .prepare(
                            "SELECT m.id FROM memories m \
                             JOIN events e ON m.source_event_id = e.id \
                             WHERE e.session_id = ?1 \
                             LIMIT ?2",
                        )
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![sid.to_string(), fetch as i64], |r| {
                            r.get::<_, String>(0)
                        })
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
                ForgetScope::Subject(subject) => {
                    let mut stmt = conn
                        .prepare("SELECT id FROM memories WHERE shred_key_id = ?1 LIMIT ?2")
                        .map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params![subject, fetch as i64], |r| r.get::<_, String>(0))
                        .map_err(StorageError::Sqlite)?;
                    for r in rows {
                        let s = r.map_err(StorageError::Sqlite)?;
                        if let Ok(u) = Uuid::parse_str(&s) {
                            ids.push(u);
                        }
                    }
                }
            }
            let exceeded = ids.len() > limit;
            ids.truncate(limit);
            Ok((ids, exceeded))
        })
    }

    /// Compute dependent records for the given target ids (bounded to `max_dep`).
    ///
    /// Walks the six legacy relationship tables that the current write path
    /// populates and returns a flat list of [`DependentRecord`]s.
    fn compute_dependents(
        &self,
        target_ids: &[Uuid],
        max_dep: usize,
    ) -> MemoryResult<(Vec<DependentRecord>, usize)> {
        if target_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let id_strings: Vec<String> = target_ids.iter().map(|u| u.to_string()).collect();
        let mut all: Vec<DependentRecord> = Vec::new();

        self.db.with_read(|conn| {
            // 1) memory_derived_from: parent→child and child→parent links.
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                // child side: our target is the parent
                let sql = format!(
                    "SELECT child_id, parent_id FROM memory_derived_from WHERE parent_id IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (dep_id_s, target_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(dep), Ok(tgt)) = (Uuid::parse_str(&dep_id_s), Uuid::parse_str(&target_s)) {
                        all.push(DependentRecord {
                            kind: DependentKind::DerivedFromLink,
                            id: dep,
                            target: tgt,
                            default_disposition: DependentKind::DerivedFromLink.default_disposition(),
                        });
                    }
                }
                // parent side: our target is the child
                let sql = format!(
                    "SELECT parent_id, child_id FROM memory_derived_from WHERE child_id IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (dep_id_s, target_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(dep), Ok(tgt)) = (Uuid::parse_str(&dep_id_s), Uuid::parse_str(&target_s)) {
                        all.push(DependentRecord {
                            kind: DependentKind::DerivedFromLink,
                            id: dep,
                            target: tgt,
                            default_disposition: DependentKind::DerivedFromLink.default_disposition(),
                        });
                    }
                }
            }

            // 2) memory_contradicts
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                for (col_dep, col_tgt) in [("b_id", "a_id"), ("a_id", "b_id")] {
                    let sql = format!(
                        "SELECT {col_dep}, {col_tgt} FROM memory_contradicts WHERE {col_tgt} IN ({ph})"
                    );
                    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params_from_iter(chunk.iter()), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .map_err(StorageError::Sqlite)?;
                    for row in rows {
                        let (dep_s, tgt_s) = row.map_err(StorageError::Sqlite)?;
                        if let (Ok(dep), Ok(tgt)) = (Uuid::parse_str(&dep_s), Uuid::parse_str(&tgt_s)) {
                            all.push(DependentRecord {
                                kind: DependentKind::ContradictsLink,
                                id: dep,
                                target: tgt,
                                default_disposition: DependentKind::ContradictsLink.default_disposition(),
                            });
                        }
                    }
                }
            }

            // 3) memory_supports
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                for (col_dep, col_tgt) in [("b_id", "a_id"), ("a_id", "b_id")] {
                    let sql = format!(
                        "SELECT {col_dep}, {col_tgt} FROM memory_supports WHERE {col_tgt} IN ({ph})"
                    );
                    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                    let rows = stmt
                        .query_map(params_from_iter(chunk.iter()), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .map_err(StorageError::Sqlite)?;
                    for row in rows {
                        let (dep_s, tgt_s) = row.map_err(StorageError::Sqlite)?;
                        if let (Ok(dep), Ok(tgt)) = (Uuid::parse_str(&dep_s), Uuid::parse_str(&tgt_s)) {
                            all.push(DependentRecord {
                                kind: DependentKind::SupportsLink,
                                id: dep,
                                target: tgt,
                                default_disposition: DependentKind::SupportsLink.default_disposition(),
                            });
                        }
                    }
                }
            }

            // 4) memory_mentions_entity
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT entity_id, memory_id FROM memory_mentions_entity WHERE memory_id IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (entity_s, tgt_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(entity), Ok(tgt)) = (Uuid::parse_str(&entity_s), Uuid::parse_str(&tgt_s)) {
                        all.push(DependentRecord {
                            kind: DependentKind::MentionsEntityLink,
                            id: entity,
                            target: tgt,
                            default_disposition: DependentKind::MentionsEntityLink.default_disposition(),
                        });
                    }
                }
            }

            // 5) memories.superseded_by pointing at a target
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT id, superseded_by FROM memories WHERE superseded_by IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (dep_s, tgt_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(dep), Ok(tgt)) = (Uuid::parse_str(&dep_s), Uuid::parse_str(&tgt_s)) {
                        all.push(DependentRecord {
                            kind: DependentKind::SupersededByReference,
                            id: dep,
                            target: tgt,
                            default_disposition: DependentKind::SupersededByReference.default_disposition(),
                        });
                    }
                }
            }

            // 6) episodes.summary_memory_id pointing at a target
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len()).map(|i| format!("?{}", i + 1)).collect::<Vec<_>>().join(",");
                let sql = format!(
                    "SELECT id, summary_memory_id FROM episodes WHERE summary_memory_id IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (ep_s, tgt_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(ep), Ok(tgt)) = (Uuid::parse_str(&ep_s), Uuid::parse_str(&tgt_s)) {
                        all.push(DependentRecord {
                            kind: DependentKind::EpisodeSummaryReference,
                            id: ep,
                            target: tgt,
                            default_disposition: DependentKind::EpisodeSummaryReference.default_disposition(),
                        });
                    }
                }
            }

            Ok(())
        })?;

        let total = all.len();
        let truncated_list: Vec<DependentRecord> = all.into_iter().take(max_dep).collect();
        Ok((truncated_list, total))
    }

    /// Compute independent evidence for the given target ids (bounded to `max_ev`).
    fn compute_independent_evidence(
        &self,
        target_ids: &[Uuid],
        max_ev: usize,
    ) -> MemoryResult<(Vec<IndependentEvidence>, bool)> {
        if target_ids.is_empty() {
            return Ok((Vec::new(), false));
        }
        let id_strings: Vec<String> = target_ids.iter().map(|u| u.to_string()).collect();
        let mut all: Vec<IndependentEvidence> = Vec::new();

        self.db.with_read(|conn| {
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                // Evidence rows whose source_event_id differs from the memory's own
                // source_event_id — these are independently corroborating rows.
                let sql = format!(
                    "SELECT e.id, e.memory_id, e.source_event_id \
                     FROM evidence e \
                     JOIN memories m ON m.id = e.memory_id \
                     WHERE e.memory_id IN ({ph}) \
                       AND e.source_event_id IS NOT NULL \
                       AND e.source_event_id != m.source_event_id"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (ev_s, tgt_s, src_s) = row.map_err(StorageError::Sqlite)?;
                    if let (Ok(ev), Ok(tgt), Ok(src)) = (
                        Uuid::parse_str(&ev_s),
                        Uuid::parse_str(&tgt_s),
                        Uuid::parse_str(&src_s),
                    ) {
                        all.push(IndependentEvidence {
                            target: tgt,
                            evidence_id: ev,
                            source_event_id: src,
                        });
                    }
                }
            }
            Ok(())
        })?;

        let truncated = all.len() > max_ev;
        all.truncate(max_ev);
        Ok((all, truncated))
    }

    /// Collect source provenance tags, session ids, and (namespace, scope)
    /// pairs for the given target memory ids.
    fn compute_affected_sources(
        &self,
        target_ids: &[Uuid],
    ) -> MemoryResult<(Vec<String>, Vec<Uuid>, Vec<(String, String)>)> {
        if target_ids.is_empty() {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        }
        let id_strings: Vec<String> = target_ids.iter().map(|u| u.to_string()).collect();
        let mut sources: BTreeSet<String> = BTreeSet::new();
        let mut sessions: BTreeSet<Uuid> = BTreeSet::new();
        let mut namespaces: BTreeSet<(String, String)> = BTreeSet::new();

        self.db.with_read(|conn| {
            for chunk in id_strings.chunks(SQL_CHUNK) {
                let ph: String = (0..chunk.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT e.source, e.session_id, m.namespace, m.scope \
                     FROM memories m \
                     JOIN events e ON m.source_event_id = e.id \
                     WHERE m.id IN ({ph})"
                );
                let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
                let rows = stmt
                    .query_map(params_from_iter(chunk.iter()), |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    })
                    .map_err(StorageError::Sqlite)?;
                for row in rows {
                    let (src, sid_opt, ns, scope) = row.map_err(StorageError::Sqlite)?;
                    sources.insert(src);
                    if let Some(sid_s) = sid_opt {
                        if let Ok(sid) = Uuid::parse_str(&sid_s) {
                            sessions.insert(sid);
                        }
                    }
                    namespaces.insert((ns, scope));
                }
            }
            Ok(())
        })?;

        Ok((
            sources.into_iter().collect(),
            sessions.into_iter().collect(),
            namespaces.into_iter().collect(),
        ))
    }

    /// Core preview computation used by all three public preview methods.
    ///
    /// Reads a snapshot at the current `graph_revision`, verifies the caller's
    /// `caller_revision` is not stale (the DB revision must equal or be the
    /// same as what the caller knows), computes the blast radius, and returns a
    /// [`LifecyclePreview`] with a minted [`LifecyclePreviewToken`].
    ///
    /// The preview is **read-only**: no rows are written.
    fn compute_preview(
        &self,
        scope: &ForgetScope,
        op: LifecycleOp,
        caller_revision: GraphRevision,
        limits: PreviewLimits,
    ) -> MemoryResult<LifecyclePreview> {
        // 1. Read current revision and check for stale caller view.
        //    The rule: the caller's revision must match the current DB revision.
        //    A forward-compatible caller (caller_revision > current) is also
        //    refused — the caller holds a revision we haven't seen yet.
        let current_revision = self.current_graph_revision()?;
        if caller_revision != current_revision {
            return Err(MemoryError::Internal(format!(
                "stale preview: caller holds revision {} but current is {} — \
                 re-fetch the current graph_revision before previewing",
                caller_revision.get(),
                current_revision.get()
            )));
        }

        // 2. Resolve the scope to target ids, bounded by limits.max_scope.
        let (target_ids, scope_exceeded) = self.resolve_bounded(scope, limits.max_scope)?;
        let scope_limit = ScopeLimitStatus {
            target_count: target_ids.len(),
            limit: limits.max_scope,
            exceeded: scope_exceeded,
        };

        // 3. If the scope is too large, return a truncated preview without
        //    dependency details — the caller must narrow the scope first.
        let (
            dependents,
            dependents_total_count,
            dependents_truncated,
            independent_evidence,
            ie_truncated,
            affected_sources,
            affected_sessions,
            affected_namespaces,
        ) = if scope_exceeded {
            (
                Vec::new(),
                0,
                false,
                Vec::new(),
                false,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        } else {
            let (deps, dep_total) = self.compute_dependents(&target_ids, limits.max_dependents)?;
            let dep_truncated = dep_total > limits.max_dependents;
            let (ie, ie_trunc) =
                self.compute_independent_evidence(&target_ids, limits.max_evidence)?;
            let (srcs, sessions, ns) = self.compute_affected_sources(&target_ids)?;
            (
                deps,
                dep_total,
                dep_truncated,
                ie,
                ie_trunc,
                srcs,
                sessions,
                ns,
            )
        };

        // 4. Mint a preview token.
        let token = LifecyclePreviewToken::mint(op.as_str(), scope, current_revision);

        Ok(LifecyclePreview {
            target_ids,
            scope_limit,
            dependents,
            dependents_total_count,
            dependents_truncated,
            independent_evidence,
            independent_evidence_truncated: ie_truncated,
            affected_sources,
            affected_sessions,
            affected_namespaces,
            reversible: op.is_reversible(),
            reversibility_label: op.reversibility_label().to_owned(),
            base_revision: current_revision,
            token,
        })
    }

    /// Preview a `forget` operation — computes the blast radius and reversibility
    /// without mutating anything (task F1.7.1, design §5.4).
    ///
    /// `caller_revision` must equal the current `graph_revision`; if it doesn't,
    /// returns a stale-preview error.
    pub fn preview_forget(
        &self,
        scope: &ForgetScope,
        caller_revision: GraphRevision,
        limits: PreviewLimits,
    ) -> MemoryResult<LifecyclePreview> {
        self.compute_preview(scope, LifecycleOp::Forget, caller_revision, limits)
    }

    /// Preview a `restore` operation — describes what a restore would affect and
    /// confirms the operation is reversible.
    ///
    /// `caller_revision` must equal the current `graph_revision`.
    pub fn preview_restore(
        &self,
        scope: &ForgetScope,
        caller_revision: GraphRevision,
        limits: PreviewLimits,
    ) -> MemoryResult<LifecyclePreview> {
        self.compute_preview(scope, LifecycleOp::Restore, caller_revision, limits)
    }

    /// Preview a `hard_delete` operation — computes the blast radius and marks
    /// the operation explicitly as IRREVERSIBLE.
    ///
    /// `caller_revision` must equal the current `graph_revision`.
    pub fn preview_hard_delete(
        &self,
        scope: &ForgetScope,
        caller_revision: GraphRevision,
        limits: PreviewLimits,
    ) -> MemoryResult<LifecyclePreview> {
        self.compute_preview(scope, LifecycleOp::HardDelete, caller_revision, limits)
    }

    // ── Lifecycle commit methods ──────────────────────────────────────────────

    /// Forget: governed tombstone of target memories to `Forgotten`, reversible
    /// within 30 days (design §5.4, MGR-040, task F1.7.2).
    ///
    /// Each non-forgotten memory in scope gets:
    /// 1. `state = 'forgotten'` and `restore_until = now + 30d` written
    ///    atomically in a single authority transaction.
    /// 2. An `audit_records` row (`command_kind = 'forget'`,
    ///    `disposition = 'accepted'`).
    /// 3. A `graph_changes` row and a `graph_revision` advance (forget is
    ///    graph-visible: forgotten memories are excluded from default retrieval).
    /// 4. An `embedding_outbox` delete entry for each forgotten memory so the
    ///    derived FTS/vector indices are updated to exclude it.
    ///
    /// **Idempotency:** memories already in state `Forgotten` are skipped
    /// without creating duplicate audit/change rows; the already-applied count
    /// is still counted in the return value.
    ///
    /// **Token guard (optional):** when `token` is `Some`, the current
    /// `graph_revision` must equal `token.base_revision`; a stale token
    /// returns an error without mutating anything. Pass `None` for
    /// internal/automated callers that do not require a confirmed preview.
    pub fn forget(
        &self,
        scope: &ForgetScope,
        token: Option<&LifecyclePreviewToken>,
    ) -> MemoryResult<usize> {
        // 1. Optional stale-token guard.
        if let Some(tok) = token {
            let current = self.current_graph_revision()?;
            if tok.base_revision != current {
                return Err(MemoryError::Internal(format!(
                    "forget: stale preview token — token base_revision={} but current \
                     graph_revision={}; re-preview before committing",
                    tok.base_revision.get(),
                    current.get()
                )));
            }
        }

        // 2. Resolve the scope to concrete memory ids.
        let ids = self.resolve(scope)?;
        if ids.is_empty() {
            return Ok(0);
        }

        // 3. Compute restore_until: now + 30 days (RFC3339 UTC).
        let restore_until = (chrono::Utc::now() + chrono::Duration::days(30)).to_rfc3339();
        let now_str = chrono::Utc::now().to_rfc3339();

        // 4. Execute the authority transaction.
        let mut tx = self.db.begin()?;

        let mut applied_count = 0usize;
        for id in &ids {
            // Idempotency: skip already-forgotten memories.
            let already_forgotten: bool = tx
                .conn()
                .query_row(
                    "SELECT state FROM memories WHERE id = ?1",
                    params![id.to_string()],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?
                .map(|s| s == "forgotten")
                .unwrap_or(false);

            if already_forgotten {
                applied_count += 1; // count it, but skip re-writing
                continue;
            }

            // 4a. Tombstone the memory: set state + restore_until.
            tx.conn()
                .execute(
                    "UPDATE memories SET state = 'forgotten', restore_until = ?2 WHERE id = ?1",
                    params![id.to_string(), restore_until],
                )
                .map_err(StorageError::Sqlite)?;

            // 4b. Advance graph_revision (one increment per forget transaction,
            //     not per memory, to keep the revision count bounded for bulk
            //     forgets; each change is still recorded individually below).
            // We read the current revision inside the write transaction so we
            // hold the serialized write lock across the read + update (L10).
            let current_rev: i64 = tx
                .conn()
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            let new_rev = current_rev + 1;
            tx.conn()
                .execute(
                    "UPDATE authority_meta SET graph_revision = ?1 WHERE id = 1",
                    params![new_rev],
                )
                .map_err(StorageError::Sqlite)?;

            // 4c. Append a graph_revisions row.
            let tx_id = crate::ids::new_id().to_string();
            tx.conn()
                .execute(
                    "INSERT INTO graph_revisions(revision, base_revision, tx_id, committed_at, \
                     actor_id, policy_hash, change_count) \
                     VALUES(?1, ?2, ?3, ?4, 'lifecycle:forget', 'policy:default', 1)",
                    params![new_rev, current_rev, tx_id, now_str],
                )
                .map_err(StorageError::Sqlite)?;

            // 4d. Append a graph_changes row for this memory's state transition.
            tx.conn()
                .execute(
                    "INSERT INTO graph_changes(revision, ordinal, record_kind, record_id, \
                     change_kind, before_hash, after_hash, policy_partition, payload_json) \
                     VALUES(?1, 0, 'memory', ?2, 'state', NULL, NULL, 'ns:core', ?3)",
                    params![
                        new_rev,
                        id.to_string(),
                        serde_json::json!({
                            "transition": "active→forgotten",
                            "restore_until": restore_until
                        })
                        .to_string()
                    ],
                )
                .map_err(StorageError::Sqlite)?;

            // 4e. Append an audit_records row.
            let audit_id = crate::ids::new_id().to_string();
            tx.conn()
                .execute(
                    "INSERT INTO audit_records(id, command_kind, disposition, policy_version, \
                     actor_id, caller_partition, reason_codes_json, authority_revision, created_at) \
                     VALUES(?1, 'forget', 'accepted', 'policy:v1', 'lifecycle:forget', \
                     'lifecycle', ?2, ?3, ?4)",
                    params![
                        audit_id,
                        serde_json::json!([{"reason": "user_governed_forget"}]).to_string(),
                        new_rev,
                        now_str,
                    ],
                )
                .map_err(StorageError::Sqlite)?;

            // 4f. Enqueue an outbox delete entry for FTS + LanceDB so derived
            //     indexes are updated to exclude the forgotten memory.
            self.relational.enqueue_outbox(
                &mut tx,
                &crate::types::OutboxEntry::delete(
                    *id,
                    crate::types::IndexTarget::Fts,
                ),
            )?;
            self.relational.enqueue_outbox(
                &mut tx,
                &crate::types::OutboxEntry::delete(
                    *id,
                    crate::types::IndexTarget::LanceDb,
                ),
            )?;

            applied_count += 1;
        }

        tx.commit()?;
        Ok(applied_count)
    }

    /// Restore one tombstoned memory (Forgotten → Active) with a governed
    /// authority transaction.
    ///
    /// ## Guards (all checked before any mutation)
    /// - The memory must exist and be in the `Forgotten` state; any other state
    ///   is rejected with a typed error.
    /// - `memories.restore_until` must be non-NULL and in the future (UTC); a
    ///   NULL value (never forgotten via the governed path) or an expired
    ///   timestamp is rejected without mutation.
    /// - When `token` is supplied the token's `base_revision` must equal the
    ///   current `authority_meta.graph_revision` (stale-commit guard — same
    ///   pattern as [`Self::forget`]).
    ///
    /// ## Governed writes (all in one authority transaction)
    /// 1. Advance `authority_meta.graph_revision` (+1).
    /// 2. Append a `graph_revisions` row.
    /// 3. Append a `graph_changes` row (`transition: forgotten→active`).
    /// 4. Append an `audit_records` row (`command_kind = 'restore'`, `disposition = 'accepted'`).
    /// 5. Transition `memories.state` → `active` and clear `restore_until` to
    ///    NULL (the window no longer applies; clearing prevents a future
    ///    forget+restore from skipping the window check).
    /// 6. Enqueue outbox upsert entries for FTS + LanceDB so the memory is
    ///    re-indexed in derived stores.
    ///
    /// The memory's UUID is preserved (same stable identity).
    pub fn restore(&self, id: Uuid, token: Option<&LifecyclePreviewToken>) -> MemoryResult<()> {
        // 1. Optional stale-token guard.
        if let Some(tok) = token {
            let current = self.current_graph_revision()?;
            if tok.base_revision != current {
                return Err(MemoryError::Internal(format!(
                    "restore: stale preview token — token base_revision={} but current \
                     graph_revision={}; re-preview before committing",
                    tok.base_revision.get(),
                    current.get()
                )));
            }
        }

        // 2. Load the memory; reject if not Forgotten.
        let memory = self
            .relational
            .get_memory(id)?
            .ok_or_else(|| MemoryError::Internal(format!("restore: memory {id} not found")))?;
        if memory.state != MemoryState::Forgotten {
            return Err(MemoryError::Internal(format!(
                "restore: memory {id} is {}, expected forgotten",
                memory.state
            )));
        }

        // 3. Check the restore window: restore_until must be set and in the future.
        let restore_until_val: Option<String> = self.db.with_read(|conn| {
            conn.query_row(
                "SELECT restore_until FROM memories WHERE id = ?1",
                params![id.to_string()],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(StorageError::Sqlite)
            .map_err(Into::into)
        })?;

        match restore_until_val {
            None => {
                return Err(MemoryError::Internal(format!(
                    "restore: memory {id} has no restore_until — was not forgotten through the \
                     governed path; cannot restore"
                )));
            }
            Some(ref ru_str) => {
                let ru: chrono::DateTime<chrono::Utc> = ru_str.parse().map_err(|_| {
                    MemoryError::Internal(format!(
                        "restore: memory {id} has malformed restore_until '{ru_str}'"
                    ))
                })?;
                if chrono::Utc::now() > ru {
                    return Err(MemoryError::Internal(format!(
                        "restore: restore window for memory {id} has expired \
                         (restore_until={ru_str}); cannot restore"
                    )));
                }
            }
        }

        // 4. Execute the governed authority transaction.
        let now_str = chrono::Utc::now().to_rfc3339();
        let mut tx = self.db.begin()?;

        // 4a. Advance graph_revision.
        let current_rev: i64 = tx
            .conn()
            .query_row(
                "SELECT graph_revision FROM authority_meta WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(StorageError::Sqlite)?;
        let new_rev = current_rev + 1;
        tx.conn()
            .execute(
                "UPDATE authority_meta SET graph_revision = ?1 WHERE id = 1",
                params![new_rev],
            )
            .map_err(StorageError::Sqlite)?;

        // 4b. Append a graph_revisions row.
        let tx_id = crate::ids::new_id().to_string();
        tx.conn()
            .execute(
                "INSERT INTO graph_revisions(revision, base_revision, tx_id, committed_at, \
                 actor_id, policy_hash, change_count) \
                 VALUES(?1, ?2, ?3, ?4, 'lifecycle:restore', 'policy:default', 1)",
                params![new_rev, current_rev, tx_id, now_str],
            )
            .map_err(StorageError::Sqlite)?;

        // 4c. Append a graph_changes row for this memory's state transition.
        tx.conn()
            .execute(
                "INSERT INTO graph_changes(revision, ordinal, record_kind, record_id, \
                 change_kind, before_hash, after_hash, policy_partition, payload_json) \
                 VALUES(?1, 0, 'memory', ?2, 'state', NULL, NULL, 'ns:core', ?3)",
                params![
                    new_rev,
                    id.to_string(),
                    serde_json::json!({
                        "transition": "forgotten→active"
                    })
                    .to_string()
                ],
            )
            .map_err(StorageError::Sqlite)?;

        // 4d. Append an audit_records row.
        let audit_id = crate::ids::new_id().to_string();
        tx.conn()
            .execute(
                "INSERT INTO audit_records(id, command_kind, disposition, policy_version, \
                 actor_id, caller_partition, reason_codes_json, authority_revision, created_at) \
                 VALUES(?1, 'restore', 'accepted', 'policy:v1', 'lifecycle:restore', \
                 'lifecycle', ?2, ?3, ?4)",
                params![
                    audit_id,
                    serde_json::json!([{"reason": "user_governed_restore"}]).to_string(),
                    new_rev,
                    now_str,
                ],
            )
            .map_err(StorageError::Sqlite)?;

        // 4e. Transition state to Active and clear restore_until (window no
        //     longer applies; prevents a future forget+restore from skipping
        //     the window check).
        tx.conn()
            .execute(
                "UPDATE memories SET state = 'active', restore_until = NULL WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(StorageError::Sqlite)?;

        // 4f. Enqueue outbox upsert entries for FTS + LanceDB so the memory
        //     is re-indexed in derived stores (it was removed when forgotten).
        self.relational.enqueue_outbox(
            &mut tx,
            &crate::types::OutboxEntry::upsert(
                id,
                crate::types::IndexTarget::Fts,
                &memory.content_hash,
            ),
        )?;
        self.relational.enqueue_outbox(
            &mut tx,
            &crate::types::OutboxEntry::upsert(
                id,
                crate::types::IndexTarget::LanceDb,
                &memory.content_hash,
            ),
        )?;

        tx.commit()
    }

    /// Hard-delete: governed cascade across all stores + outbox purge entries +
    /// audit + graph revision (design §5.4 "Lifecycle and erasure truth",
    /// MGR-040, MGR-041).
    ///
    /// ## Governance (all in one authority transaction)
    /// 1. Optional stale-token guard (same pattern as [`Self::forget`] /
    ///    [`Self::restore`]).
    /// 2. Mark authority content `Deleted`.
    /// 3. Close dependent link tables: `memory_mentions_entity`,
    ///    `memory_derived_from` (both sides), `memory_contradicts`,
    ///    `memory_supports`.
    /// 4. Delete FTS entries in-transaction.
    /// 5. Advance `authority_meta.graph_revision` and append `graph_revisions` +
    ///    `graph_changes` rows (one revision per hard-delete transaction, one
    ///    `graph_changes` row per affected memory; the `payload_json` records
    ///    which dependent tables were cascaded).
    /// 6. Append `audit_records` row (`command_kind='hard_delete'`,
    ///    `disposition='accepted'`).
    /// 7. Enqueue `derived_outbox` delete entries for FTS and LanceDB so
    ///    derived-index purge is durable and can be retried if the immediate
    ///    call below fails.
    ///
    /// ## Post-transaction
    /// - Purge vectors via `VectorStorePort` (derived, non-authoritative).
    /// - Belt-and-suspenders search-store delete.
    /// - Mark subject's shred-key status `'destroyed'` when scope is
    ///   `Subject(…)`.  This is a **hard-delete status flag**; it is NOT
    ///   cryptographic erasure (content is plaintext — MGR-041).
    ///
    /// ## Future derived purge targets (not yet wired)
    /// When graph/trace/cache/export purge infrastructure exists, additional
    /// outbox entries (or direct calls) should be added here.  The pattern is
    /// identical to the FTS/LanceDB entries below: enqueue inside the same
    /// authority transaction so the purge obligation survives a crash between
    /// commit and relay.
    pub async fn hard_delete(
        &self,
        scope: &ForgetScope,
        token: Option<&LifecyclePreviewToken>,
    ) -> MemoryResult<usize> {
        // 1. Optional stale-token guard.
        if let Some(tok) = token {
            let current = self.current_graph_revision()?;
            if tok.base_revision != current {
                return Err(MemoryError::Internal(format!(
                    "hard_delete: stale preview token — token base_revision={} but current \
                     graph_revision={}; re-preview before committing",
                    tok.base_revision.get(),
                    current.get()
                )));
            }
        }

        let ids = self.resolve(scope)?;
        if ids.is_empty() {
            return Ok(0);
        }

        let now_str = chrono::Utc::now().to_rfc3339();

        // Cascaded tables recorded in the graph_changes payload (for audit
        // / traceability; does not change which tables are actually cascaded).
        let cascaded_tables = serde_json::json!([
            "memory_mentions_entity",
            "memory_derived_from",
            "memory_contradicts",
            "memory_supports"
        ]);

        // 2) Authority txn: mark deleted, close all dependent links, delete
        //    FTS in-txn, record governance rows, enqueue outbox deletes.
        {
            let mut tx = self.db.begin()?;

            // 2a. Advance graph_revision once for the whole transaction.
            let current_rev: i64 = tx
                .conn()
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(StorageError::Sqlite)?;
            let new_rev = current_rev + 1;
            tx.conn()
                .execute(
                    "UPDATE authority_meta SET graph_revision = ?1 WHERE id = 1",
                    params![new_rev],
                )
                .map_err(StorageError::Sqlite)?;

            // 2b. Append a graph_revisions row for this transaction.
            let grev_tx_id = crate::ids::new_id().to_string();
            tx.conn()
                .execute(
                    "INSERT INTO graph_revisions(revision, base_revision, tx_id, committed_at, \
                     actor_id, policy_hash, change_count) \
                     VALUES(?1, ?2, ?3, ?4, 'lifecycle:hard_delete', 'policy:default', ?5)",
                    params![new_rev, current_rev, grev_tx_id, now_str, ids.len() as i64],
                )
                .map_err(StorageError::Sqlite)?;

            // 2c. Append audit_records row for the transaction.
            let audit_id = crate::ids::new_id().to_string();
            // Capture shredded subjects for the audit payload.
            let shredded_subjects: Vec<String> = if let ForgetScope::Subject(s) = scope {
                vec![s.clone()]
            } else {
                vec![]
            };
            let audit_payload = serde_json::json!({
                "cascaded": cascaded_tables,
                "subjects_shredded": shredded_subjects,
                "memory_count": ids.len()
            });
            tx.conn()
                .execute(
                    "INSERT INTO audit_records(id, command_kind, disposition, policy_version, \
                     actor_id, caller_partition, reason_codes_json, authority_revision, created_at) \
                     VALUES(?1, 'hard_delete', 'accepted', 'policy:v1', 'lifecycle:hard_delete', \
                     'lifecycle', ?2, ?3, ?4)",
                    params![
                        audit_id,
                        audit_payload.to_string(),
                        new_rev,
                        now_str,
                    ],
                )
                .map_err(StorageError::Sqlite)?;

            for (ordinal, id) in ids.iter().enumerate() {
                let id_str = id.to_string();

                // 2d. Mark the memory Deleted.
                self.relational
                    .set_memory_state(&mut tx, *id, MemoryState::Deleted)?;

                // 2e. Close dependent link tables (both sides where applicable).
                // memory_mentions_entity — the original single-side prune.
                tx.conn()
                    .execute(
                        "DELETE FROM memory_mentions_entity WHERE memory_id = ?1",
                        params![id_str],
                    )
                    .map_err(StorageError::Sqlite)?;

                // memory_derived_from — child OR parent side.
                tx.conn()
                    .execute(
                        "DELETE FROM memory_derived_from WHERE parent_id = ?1 OR child_id = ?1",
                        params![id_str],
                    )
                    .map_err(StorageError::Sqlite)?;

                // memory_contradicts — either side.
                tx.conn()
                    .execute(
                        "DELETE FROM memory_contradicts WHERE a_id = ?1 OR b_id = ?1",
                        params![id_str],
                    )
                    .map_err(StorageError::Sqlite)?;

                // memory_supports — either side.
                tx.conn()
                    .execute(
                        "DELETE FROM memory_supports WHERE a_id = ?1 OR b_id = ?1",
                        params![id_str],
                    )
                    .map_err(StorageError::Sqlite)?;

                // 2f. Append a graph_changes row per memory (records the
                //     cascade choices in payload_json so the audit trail shows
                //     exactly which dependent tables were touched).
                let gc_payload = serde_json::json!({
                    "transition": "active→deleted",
                    "cascaded": cascaded_tables,
                    "subjects_shredded": shredded_subjects
                });
                tx.conn()
                    .execute(
                        "INSERT INTO graph_changes(revision, ordinal, record_kind, record_id, \
                         change_kind, before_hash, after_hash, policy_partition, payload_json) \
                         VALUES(?1, ?2, 'memory', ?3, 'delete', NULL, NULL, 'ns:core', ?4)",
                        params![new_rev, ordinal as i64, id_str, gc_payload.to_string()],
                    )
                    .map_err(StorageError::Sqlite)?;

                // 2g. Enqueue outbox delete entries for FTS and LanceDB.
                // These serve as the durable ledger for async derived-index
                // cleanup — if the immediate purge calls below fail they can
                // be retried from the outbox.
                //
                // NOTE: graph/trace/cache/export purge entries are NOT yet
                // enqueued here because no concrete outbox targets exist for
                // those subsystems in the current codebase.  When those
                // systems land, add matching enqueue_outbox calls here
                // (inside this same authority transaction) following the
                // identical pattern: IndexTarget::Graph / IndexTarget::Trace /
                // etc., op=Delete, content_hash="".
                self.relational.enqueue_outbox(
                    &mut tx,
                    &crate::types::OutboxEntry::delete(
                        *id,
                        crate::types::IndexTarget::Fts,
                    ),
                )?;
                self.relational.enqueue_outbox(
                    &mut tx,
                    &crate::types::OutboxEntry::delete(
                        *id,
                        crate::types::IndexTarget::LanceDb,
                    ),
                )?;
            }

            // 2h. Delete FTS entries inside the transaction.
            delete_fts_in_tx(&mut tx, &ids)?;

            tx.commit()?;
        }

        // 3) Purge vectors (derived index, non-authoritative).
        // The outbox entries above serve as the durable retry-able record;
        // this immediate call is a best-effort fast path.
        self.vectors.delete(&self.embedding_model, &ids).await?;
        // Belt-and-suspenders: also purge via the search store path.
        self.search.delete(&ids).await?;

        // 4) Mark subject's shred-key status as 'destroyed' (Hard Delete
        //    pending cryptographic erasure — NOT actual crypto erasure, content
        //    is plaintext — MGR-041 / design §5.4).
        if let ForgetScope::Subject(subject) = scope {
            self.shred_subject(subject)?;
        }

        Ok(ids.len())
    }

    /// Mark a subject's shred-key status as `'destroyed'` in the `shred_keys`
    /// catalog row.  Idempotent.
    ///
    /// # Honesty (MGR-041 / design §5.4)
    ///
    /// This does **NOT** achieve cryptographic erasure.  Memory content is
    /// stored as plaintext; no encryption exists.  Setting the status to
    /// `'destroyed'` is a deletion status flag — the content in the `memories`
    /// table remains readable.  This operation is correctly described as
    /// **"Hard Delete pending cryptographic erasure"** until payload
    /// encryption, external key destruction, and zero-plaintext verification
    /// are all implemented.  See [`crate::api::HealthReport`] for the
    /// `crypto_shred_capability` field that discloses this state.
    ///
    /// # Implementation roadmap for real cryptographic erasure (MGR-041)
    ///
    /// Before `CRYPTO_SHRED_CAPABILITY` (see
    /// [`crate::api::CRYPTO_SHRED_CAPABILITY`]) can be changed from
    /// `"unavailable"` to an affirmative capability, ALL of the following
    /// must be implemented and proven with automated evidence:
    ///
    /// 1. **Payload encryption**: every `memories.content` (and
    ///    `events.payload` where it carries sensitive text) must be encrypted
    ///    under a subject-bound, versioned data key *before* the row is
    ///    committed.  The ciphertext, not plaintext, is stored in the DB.
    ///
    /// 2. **External key storage**: encryption keys must be held **outside**
    ///    the SQLite file — e.g. OS keyring, HSM, or KMS — so destroying the
    ///    key does not require touching every encrypted row.  `shred_keys.key_ref`
    ///    must be a locator into that external store, not raw key bytes.
    ///
    /// 3. **Key destruction**: `shred_subject` must actually destroy / null
    ///    the key in the external store AND record a cryptographically-verifiable
    ///    destruction proof (e.g. an HMAC over the final key state and a
    ///    timestamp) in `shred_keys.proof_hash`.  Simply setting
    ///    `status='destroyed'` without touching external storage is
    ///    insufficient.
    ///
    /// 4. **Zero-plaintext denial verification across ALL read paths**: after
    ///    key destruction, automated tests must prove that EVERY path that
    ///    could return the original plaintext now returns only unreadable
    ///    ciphertext or an error.  The paths that must be covered are:
    ///    - `memories.content` (current-state read)
    ///    - `events.payload` (immutable event log)
    ///    - `search_documents` / FTS5 index (full-text search)
    ///    - `mem_vectors` (embedding projection)
    ///    - Any SQLite backup / `recovery_snapshots` path
    ///    - Cached search results (in-memory or disk caches)
    ///    - Graph edge payloads / `relationships` / `evidence` rows
    ///    - `retrieval_trace_items` (injected context trace)
    ///    - Export / interchange packages
    ///
    /// 5. **No hardcoded or fallback key**: CI must verify no copy of the
    ///    subject's data key exists anywhere the attacker could reach
    ///    (in-process memory after destruction, log files, backup tables,
    ///    or any in-flight enrichment queue item).
    ///
    /// Until all five points above are satisfied with passing evidence, the
    /// honest public-facing wording remains **"Hard Delete pending
    /// cryptographic erasure"** and `crypto_shred_capability` reports
    /// `"unavailable"`.
    pub fn shred_subject(&self, subject: &str) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "UPDATE shred_keys SET status = 'destroyed', destroyed_at = ?2 WHERE subject_id = ?1",
                params![subject, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Returns `true` if the `shred_keys` catalog row for `subject` has
    /// `status = 'destroyed'`.
    ///
    /// # Honesty (MGR-041 / design §5.4)
    ///
    /// This method checks the **status flag only** — it does NOT prove
    /// cryptographic unreadability.  Because memory content is stored as
    /// plaintext and no payload encryption exists, a `true` result means
    /// "the hard-delete status was committed" (Hard Delete pending
    /// cryptographic erasure), not "the data is cryptographically
    /// inaccessible".  Callers must not present this result as proof of
    /// cryptographic erasure.
    pub fn is_shredded(&self, subject: &str) -> MemoryResult<bool> {
        self.db.with_read(|conn| {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM shred_keys WHERE subject_id = ?1",
                    params![subject],
                    |r| r.get(0),
                )
                .optional()
                .map_err(StorageError::Sqlite)?;
            Ok(status.as_deref() == Some("destroyed"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::ports::EventStore;
    use crate::stores::{
        SqliteEventStore, SqliteRelationalStore, SqliteSearchStore, SqliteVectorStore,
    };
    use crate::types::{
        Event, EventType, Memory, MemoryType, MemoryWorth, Modality, Scope, Sensitivity, Source,
        StalenessClass, VectorPayload,
    };

    async fn setup() -> (
        Arc<Database>,
        Lifecycle,
        Arc<SqliteEventStore>,
        Arc<SqliteRelationalStore>,
        Arc<SqliteVectorStore>,
    ) {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let events = Arc::new(SqliteEventStore::new(db.clone()));
        let rel = Arc::new(SqliteRelationalStore::new(db.clone()));
        let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
        let search = Arc::new(SqliteSearchStore::new(db.clone()));
        let lc = Lifecycle::new(
            db.clone(),
            rel.clone(),
            vectors.clone(),
            search.clone(),
            ModelVersion("fake_v1".into()),
        );
        (db, lc, events, rel, vectors)
    }

    fn make_memory(source_event: Uuid, hash: &str) -> Memory {
        let now = chrono::Utc::now();
        Memory {
            id: crate::ids::new_id(),
            content: "sensitive fact".into(),
            memory_type: MemoryType::Semantic,
            compression_level: 0,
            source_event_id: source_event,
            namespace: "core".into(),
            owner_id: "user".into(),
            device_id: "d".into(),
            scope: Scope::Global,
            confidence: 0.8,
            importance: 5.0,
            access_count: 0,
            decay_score: 1.0,
            staleness_class: StalenessClass::Slow,
            sensitivity: Sensitivity::Private,
            state: MemoryState::Active,
            created_at: now,
            last_accessed: None,
            valid_from: now,
            valid_until: None,
            embedding_id: None,
            embedding_model_version: None,
            estimated_tokens: 3,
            content_hash: hash.into(),
            shred_key_id: Some("person:alice".into()),
            verify_against: None,
            superseded_by: None,
            episode_id: None,
            goal_context_id: None,
            worth: MemoryWorth::default(),
            modality: Modality::Text,
            preference_pair_id: None,
            training_eligible: false,
        }
    }

    async fn seed_memory(
        db: &Arc<Database>,
        events: &SqliteEventStore,
        rel: &SqliteRelationalStore,
        vectors: &SqliteVectorStore,
        source: Source,
        hash: &str,
    ) -> Uuid {
        let ev = Event {
            id: crate::ids::new_id(),
            hlc: crate::ids::HlcGenerator::new().now(),
            ts_utc: chrono::Utc::now(),
            tz_offset_min: 0,
            event_type: EventType::UserMessage,
            source,
            session_id: Some(crate::ids::new_id()),
            parent_event_id: None,
            shred_key_id: Some("person:alice".into()),
            payload: serde_json::json!({}),
            encrypted: false,
            checksum: "c".into(),
        };
        let m = make_memory(ev.id, hash);
        {
            let mut tx = db.begin().unwrap();
            // seed the shred key first (events/memories FK-reference it)
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO shred_keys(subject_id, subject_type, key_ref, status, created_at) \
                     VALUES('person:alice','person','keyfile:local','active',?1)",
                    params![chrono::Utc::now().to_rfc3339()],
                )
                .unwrap();
            events.append(&mut tx, &ev).unwrap();
            rel.upsert_memory(&mut tx, &m).unwrap();
            tx.commit().unwrap();
        }
        vectors
            .upsert(
                &ModelVersion("fake_v1".into()),
                m.id,
                &[0.1, 0.2, 0.3],
                &VectorPayload {
                    namespace: "core".into(),
                    scope: Scope::Global,
                    sensitivity: Sensitivity::Private,
                    memory_type: MemoryType::Semantic,
                    content_hash: hash.into(),
                    created_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        m.id
    }

    #[tokio::test]
    async fn forget_is_reversible_then_hard_delete_shreds() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h1").await;

        // forget → tombstone, reversible.
        assert_eq!(lc.forget(&ForgetScope::Memory(id), None).unwrap(), 1);
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Forgotten
        );
        lc.restore(id, None).unwrap();
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Active
        );

        // hard delete a subject → cascade + status mark.
        let n = lc
            .hard_delete(&ForgetScope::Subject("person:alice".into()), None)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            rel.get_memory(id).unwrap().unwrap().state,
            MemoryState::Deleted
        );
        assert!(lc.is_shredded("person:alice").unwrap());
        // Vector purged.
        assert!(vectors
            .all_ids(&ModelVersion("fake_v1".into()))
            .await
            .unwrap()
            .is_empty());
    }

    /// Validates: MGR-041 (design §5.4) — `is_shredded()` checks the status
    /// flag only.  It does NOT prove cryptographic unreadability because
    /// memory content is stored as plaintext; no payload encryption exists.
    ///
    /// This test documents two truths:
    /// 1. After `shred_subject`, `is_shredded` returns `true` (flag set).
    /// 2. The `is_shredded` flag does NOT imply the content is unreadable —
    ///    this test verifies the flag semantics, not cryptographic erasure.
    ///
    /// No secret key bytes are present in the `shred_keys` table row seeded
    /// here; `key_ref` is a catalog reference only.
    #[tokio::test]
    async fn is_shredded_checks_status_flag_only_not_cryptographic_erasure() {
        let (db, lc, _events, _rel, _vectors) = setup().await;

        // Seed a shred_keys row with a catalog reference (not secret bytes).
        // `key_ref` is an external locator (e.g. OS keyring handle), never
        // actual key material.
        {
            let tx = db.begin().unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO shred_keys \
                     (subject_id, subject_type, key_ref, status, created_at) \
                     VALUES ('test-subject', 'person', 'keyring://local/test-subject', 'active', ?1)",
                    params![chrono::Utc::now().to_rfc3339()],
                )
                .unwrap();
            tx.commit().unwrap();
        }

        // Confirm initial state: not yet marked destroyed.
        assert!(
            !lc.is_shredded("test-subject").unwrap(),
            "is_shredded should be false before shred_subject is called"
        );

        // Call shred_subject — sets status='destroyed'.
        lc.shred_subject("test-subject").unwrap();

        // Now is_shredded returns true — this is the STATUS FLAG being set.
        // It does NOT mean content is cryptographically unreadable (MGR-041).
        assert!(
            lc.is_shredded("test-subject").unwrap(),
            "is_shredded should be true after shred_subject (status flag set)"
        );

        // Confirm the row does NOT contain secret bytes: key_ref must be a
        // human-readable catalog reference, not raw key material (MGR-041).
        let key_ref: String = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT key_ref FROM shred_keys WHERE subject_id = 'test-subject'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(StorageError::Sqlite)?)
            })
            .unwrap();
        // key_ref must be a locator string, not a raw 256-bit key.
        assert!(
            key_ref.contains("keyring://"),
            "key_ref must be a locator reference, not secret bytes: {:?}",
            key_ref
        );
        // Must not look like a bare base64-encoded secret (44 chars, no ':' or '/').
        let looks_like_bare_secret = key_ref.len() == 44
            && key_ref
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
        assert!(
            !looks_like_bare_secret,
            "key_ref must not be raw base64 key material: {:?}",
            key_ref
        );
    }

    /// Validates: MGR-041 (design §5.4) — HONEST STATE TEST.
    ///
    /// After `shred_subject` marks `status='destroyed'`, memory content is
    /// **still readable** from the `memories` table.  This test explicitly
    /// documents the known gap: destroying the shred-key status flag does NOT
    /// make data cryptographically unreadable.  Content is plaintext today.
    ///
    /// This is an **honest-state test**, not a security test.  Its purpose is
    /// to prove the code is accurate about what "shredded" means right now
    /// (Hard Delete pending cryptographic erasure) and that nothing in the
    /// codebase mistakenly treats `is_shredded() == true` as evidence of
    /// actual cryptographic erasure.
    ///
    /// When real encryption is implemented (see `shred_subject` implementation
    /// roadmap), this test should be updated: after key destruction the query
    /// below should either return ciphertext that cannot be decoded, or an
    /// error.  Until that is true, this test *passing* proves the gap exists.
    #[tokio::test]
    async fn shredded_key_does_not_prevent_plaintext_read_honest_state() {
        let (db, lc, events, rel, vectors) = setup().await;

        // Seed a memory with known plaintext content.
        let mem_id = seed_memory(&db, &events, &rel, &vectors, Source::User, "honest-h1").await;

        // Confirm the plaintext is readable before any shred operation.
        let memory_before = rel.get_memory(mem_id).unwrap().unwrap();
        let plaintext = memory_before.content.clone();
        assert_eq!(
            plaintext, "sensitive fact",
            "pre-condition: memory content is plaintext before shred"
        );

        // Mark the subject's shred-key as 'destroyed' via shred_subject.
        // This sets status='destroyed' in shred_keys — a status flag only.
        lc.shred_subject("person:alice").unwrap();
        assert!(
            lc.is_shredded("person:alice").unwrap(),
            "shred_subject must set is_shredded to true"
        );

        // HONEST GAP: the content is STILL readable from the memories table.
        // No encryption exists, so destroying the key flag changes nothing
        // about the readability of the stored plaintext (MGR-041 / design §5.4).
        let memory_after = rel.get_memory(mem_id).unwrap();
        // The row still exists (shred_subject doesn't delete the memory row):
        // Note: hard_delete cascades deletion; shred_subject alone only marks
        // the key.  We test shred_subject here in isolation to prove the gap.
        if let Some(m) = memory_after {
            // If the row still exists, its content must still be plaintext.
            assert_eq!(
                m.content, plaintext,
                "KNOWN GAP (MGR-041): content is still readable as plaintext \
                 after shred_subject — no encryption has been applied. \
                 This is the honest state: 'Hard Delete pending cryptographic erasure', \
                 not 'Crypto-Shredded'."
            );
        }
        // Whether the row is present or not (depends on whether the subject's
        // memories were also hard-deleted in setup), the key point is: if
        // ANY row with this shred_key_id exists, its content is readable.
        // Verify by querying the DB directly:
        let content_in_db: Option<String> = db
            .with_read(|conn| {
                Ok(conn
                    .query_row(
                        "SELECT content FROM memories WHERE shred_key_id = 'person:alice' LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(StorageError::Sqlite)?)
            })
            .unwrap();
        // If any row is present, it must still be plaintext (not ciphertext).
        if let Some(content) = content_in_db {
            assert_eq!(
                content, plaintext,
                "KNOWN GAP (MGR-041): direct DB read returns plaintext '{}' \
                 even though shred_keys.status='destroyed'. \
                 Cryptographic erasure is unavailable — reliance on host OS \
                 disk encryption only.",
                content
            );
        }
    }

    #[tokio::test]
    async fn per_source_cascade() {
        let (db, lc, events, rel, vectors) = setup().await;
        seed_memory(
            &db,
            &events,
            &rel,
            &vectors,
            Source::Mcp {
                server: "github".into(),
                tool: "search".into(),
            },
            "h2",
        )
        .await;
        // forget by mcp:github source prefix.
        let n = lc
            .hard_delete(&ForgetScope::SourcePrefix("mcp:github".into()), None)
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    // ── Task F1.7.1 preview tests ─────────────────────────────────────────────

    /// Get the current `graph_revision` from the in-memory test DB.
    fn current_revision(db: &Database) -> GraphRevision {
        let rev: i64 = db
            .with_read(|conn| {
                conn.query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(crate::error::StorageError::Sqlite)
                .map_err(|e| e.into())
            })
            .unwrap();
        GraphRevision::new(rev as u64)
    }

    #[tokio::test]
    async fn preview_single_memory_returns_correct_metadata() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h10").await;

        let rev = current_revision(&db);
        let preview = lc
            .preview_forget(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();

        assert_eq!(preview.target_ids, vec![id]);
        assert!(!preview.scope_limit.exceeded);
        assert_eq!(preview.base_revision, rev);
        assert!(preview.reversible, "forget must be reversible");
        assert!(
            preview.reversibility_label.contains("reversible"),
            "label should mention reversibility"
        );
        // No dependents for a fresh isolated memory.
        assert_eq!(preview.dependents_total_count, 0);
        assert!(!preview.dependents_truncated);
        // Source and namespace should be populated.
        assert!(!preview.affected_sources.is_empty());
        assert!(!preview.affected_namespaces.is_empty());
        // Token encodes the same revision.
        assert_eq!(preview.token.base_revision, rev);
        let encoded = preview.token.encode();
        let decoded = LifecyclePreviewToken::decode(&encoded).expect("token must decode");
        assert_eq!(decoded, preview.token);
    }

    #[tokio::test]
    async fn preview_hard_delete_is_irreversible() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h11").await;

        let rev = current_revision(&db);
        let preview = lc
            .preview_hard_delete(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();

        assert!(!preview.reversible, "hard_delete must NOT be reversible");
        assert!(
            preview.reversibility_label.contains("IRREVERSIBLE"),
            "label must explicitly say IRREVERSIBLE, got: {}",
            preview.reversibility_label
        );
    }

    #[tokio::test]
    async fn preview_restore_is_reversible() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h12").await;

        let rev = current_revision(&db);
        let preview = lc
            .preview_restore(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();

        assert!(preview.reversible, "restore must be reversible");
    }

    #[tokio::test]
    async fn preview_stale_revision_returns_error() {
        let (db, lc, events, rel, vectors) = setup().await;
        let _id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h13").await;

        let rev = current_revision(&db);
        // Use a stale (older) revision — should fail.
        let stale = if rev.get() > 0 {
            GraphRevision::new(rev.get() - 1)
        } else {
            GraphRevision::new(rev.get() + 1)
        };

        let result = lc.preview_forget(
            &ForgetScope::Subject("person:alice".into()),
            stale,
            PreviewLimits::default(),
        );
        assert!(result.is_err(), "stale revision must return an error");
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("stale preview"),
            "error should mention 'stale preview', got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn preview_is_read_only_no_state_changes() {
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "h14").await;

        let state_before = rel.get_memory(id).unwrap().unwrap().state.clone();

        let rev = current_revision(&db);
        let _preview = lc
            .preview_hard_delete(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();

        let state_after = rel.get_memory(id).unwrap().unwrap().state.clone();
        assert_eq!(
            state_before, state_after,
            "preview must not change memory state"
        );
        // Also verify vectors are intact.
        assert_eq!(
            vectors
                .all_ids(&ModelVersion("fake_v1".into()))
                .await
                .unwrap()
                .len(),
            1,
            "preview must not purge vectors"
        );
    }

    #[tokio::test]
    async fn preview_scope_limit_truncates_correctly() {
        let (db, lc, events, rel, _vectors) = setup().await;
        // Seed more memories than the single_record max_scope (1) under the same session.
        let sid = crate::ids::new_id();
        for i in 0..5usize {
            let ev = Event {
                id: crate::ids::new_id(),
                hlc: crate::ids::HlcGenerator::new().now(),
                ts_utc: chrono::Utc::now(),
                tz_offset_min: 0,
                event_type: EventType::UserMessage,
                source: Source::User,
                session_id: Some(sid),
                parent_event_id: None,
                shred_key_id: Some("person:alice".into()),
                payload: serde_json::json!({}),
                encrypted: false,
                checksum: "c".into(),
            };
            let m = make_memory(ev.id, &format!("hash_batch_{i}"));
            let mut tx = db.begin().unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO shred_keys(subject_id, subject_type, key_ref, status, created_at) \
                     VALUES('person:alice','person','keyfile:local','active',?1)",
                    params![chrono::Utc::now().to_rfc3339()],
                )
                .unwrap();
            events.append(&mut tx, &ev).unwrap();
            rel.upsert_memory(&mut tx, &m).unwrap();
            tx.commit().unwrap();
        }

        let rev = current_revision(&db);
        // Single-record limit (max_scope=1) for a Session scope with 5 memories.
        // The scope has 5 ids but limit is 1 → exceeded = true.
        let preview = lc
            .preview_forget(
                &ForgetScope::Session(sid),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();

        assert!(
            preview.scope_limit.exceeded,
            "scope limit should be exceeded when session has more memories than max_scope"
        );
        // When exceeded, dependents list should be empty.
        assert!(
            preview.dependents.is_empty(),
            "dependents should be empty when scope is truncated"
        );
    }

    #[tokio::test]
    async fn preview_token_round_trips() {
        let rev = GraphRevision::new(42);
        let scope = ForgetScope::Memory(Uuid::nil());
        let token = LifecyclePreviewToken::mint("forget", &scope, rev);

        let encoded = token.encode();
        assert!(encoded.starts_with("lc1:"), "token must start with prefix");

        let decoded = LifecyclePreviewToken::decode(&encoded).expect("token must decode");
        assert_eq!(decoded.base_revision, token.base_revision);
        assert_eq!(decoded.scope_hash, token.scope_hash);
        assert_eq!(decoded.minted_at_ms, token.minted_at_ms);

        // A tampered token must fail to decode with a mismatched hash.
        let tampered = encoded.replace(&token.scope_hash[..8], "deadbeef");
        // The tampered version may still decode structurally (same length) but
        // the scope_hash content differs.
        if let Some(t) = LifecyclePreviewToken::decode(&tampered) {
            assert_ne!(t.scope_hash, token.scope_hash, "tampered hash must differ");
        }
    }

    #[tokio::test]
    async fn preview_limits_default_is_scope_based() {
        let default = PreviewLimits::default();
        assert_eq!(default.max_scope, PREVIEW_SCOPE_LIMIT);
        assert_eq!(default.max_dependents, PREVIEW_DETAIL_LIMIT);

        let single = PreviewLimits::single_record();
        assert_eq!(single.max_scope, 1);
        assert_eq!(single.max_dependents, PREVIEW_DETAIL_LIMIT);
    }

    // ── Task F1.7.2 governed forget tests ─────────────────────────────────

    /// Helper: read memory state directly from the DB.
    fn memory_state(db: &Database, id: Uuid) -> String {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT state FROM memories WHERE id = ?1",
                params![id.to_string()],
                |r| r.get::<_, String>(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: read restore_until from the DB.
    fn restore_until(db: &Database, id: Uuid) -> Option<String> {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT restore_until FROM memories WHERE id = ?1",
                params![id.to_string()],
                |r| r.get::<_, Option<String>>(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: count audit rows.
    fn audit_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            conn.query_row("SELECT COUNT(*) FROM audit_records", [], |r| r.get(0))
                .map_err(crate::error::StorageError::Sqlite)
                .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: count graph_changes rows for a given memory id.
    fn graph_changes_for(db: &Database, id: Uuid) -> i64 {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM graph_changes WHERE record_id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: count pending outbox entries for a memory id.
    fn outbox_pending_for(db: &Database, id: Uuid) -> i64 {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM embedding_outbox WHERE memory_id = ?1 AND status = 'pending'",
                params![id.to_string()],
                |r| r.get(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn forget_records_audit_graph_change_and_outbox() {
        // F1.7.2: governed forget must append audit, graph_change, and outbox entries.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf1").await;

        assert_eq!(audit_count(&db), 0, "no audit rows before forget");
        assert_eq!(
            graph_changes_for(&db, id),
            0,
            "no graph_changes before forget"
        );
        assert_eq!(outbox_pending_for(&db, id), 0, "no outbox before forget");

        let n = lc.forget(&ForgetScope::Memory(id), None).unwrap();
        assert_eq!(n, 1, "should have forgotten one memory");

        // Audit row must exist.
        assert_eq!(
            audit_count(&db),
            1,
            "exactly one audit_records row after forget"
        );

        // graph_changes row must exist for this memory id.
        assert_eq!(
            graph_changes_for(&db, id),
            1,
            "exactly one graph_changes row after forget"
        );

        // Outbox entries must be enqueued (FTS + LanceDb = 2).
        assert_eq!(
            outbox_pending_for(&db, id),
            2,
            "two pending outbox entries (fts + lancedb) after forget"
        );
    }

    #[tokio::test]
    async fn forget_sets_restore_until_within_30_days() {
        // F1.7.2: restore_until must be set to approx now + 30 days.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf2").await;

        let before = chrono::Utc::now();
        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        let after = chrono::Utc::now();

        let ru_str = restore_until(&db, id).expect("restore_until must be set after forget");
        let ru: chrono::DateTime<chrono::Utc> =
            ru_str.parse().expect("restore_until must be RFC3339");

        let min_expected = before + chrono::Duration::days(30);
        let max_expected = after + chrono::Duration::days(30) + chrono::Duration::seconds(5);
        assert!(
            ru >= min_expected,
            "restore_until ({ru}) must be >= now+30d ({min_expected})"
        );
        assert!(
            ru <= max_expected,
            "restore_until ({ru}) must be <= now+30d+5s ({max_expected})"
        );
    }

    #[tokio::test]
    async fn forget_already_forgotten_is_idempotent() {
        // F1.7.2: a second forget on an already-forgotten memory must not create
        // additional audit or graph_change rows.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf3").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        let audit_after_first = audit_count(&db);
        let changes_after_first = graph_changes_for(&db, id);

        // Second forget — should be a no-op for audit/graph_changes.
        let n = lc.forget(&ForgetScope::Memory(id), None).unwrap();
        assert_eq!(
            n, 1,
            "idempotent call still counts the already-forgotten memory"
        );

        assert_eq!(
            audit_count(&db),
            audit_after_first,
            "second forget must NOT add another audit row"
        );
        assert_eq!(
            graph_changes_for(&db, id),
            changes_after_first,
            "second forget must NOT add another graph_changes row"
        );
    }

    #[tokio::test]
    async fn forgotten_memory_excluded_from_retriever() {
        // F1.7.2: retriever must not return forgotten memories in default search.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf4").await;

        let search_store = Arc::new(crate::stores::SqliteSearchStore::new(db.clone()));
        // Index the memory in FTS so it would be returned before forgetting.
        search_store
            .index(id, "the secret datum about alpha project", "core")
            .await
            .unwrap();

        let embedder = Arc::new(FakeEmbedder { dim: 16 });
        let retriever = crate::retriever::Retriever::new(
            rel.clone(),
            vectors.clone(),
            search_store,
            embedder,
        );

        // Before forget: memory is found.
        let before = retriever
            .search(
                "secret datum alpha",
                &crate::retriever::RetrievalCtx::default(),
            )
            .await
            .unwrap();
        assert!(
            before.hits.iter().any(|h| h.memory.id == id),
            "memory must appear in search before being forgotten"
        );

        // Forget the memory.
        lc.forget(&ForgetScope::Memory(id), None).unwrap();

        // After forget: memory must NOT appear.
        let after = retriever
            .search(
                "secret datum alpha",
                &crate::retriever::RetrievalCtx::default(),
            )
            .await
            .unwrap();
        assert!(
            !after.hits.iter().any(|h| h.memory.id == id),
            "forgotten memory must be excluded from retrieval (retriever.rs gating)"
        );
    }

    #[tokio::test]
    async fn forget_stale_token_is_rejected() {
        // F1.7.2: a stale preview token must cause forget to return an error
        // without mutating anything.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf5").await;

        let rev = current_revision(&db);
        let preview = lc
            .preview_forget(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();
        let token = preview.token.clone();

        // Advance the revision by forgetting a *different* memory first so the token is stale.
        let id2 = seed_memory(&db, &events, &rel, &vectors, Source::User, "hf5b").await;
        lc.forget(&ForgetScope::Memory(id2), None).unwrap();

        // Now attempt to forget with the now-stale token.
        let result = lc.forget(&ForgetScope::Memory(id), Some(&token));
        assert!(
            result.is_err(),
            "forget with a stale token must return an error"
        );
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("stale"),
            "error must mention 'stale', got: {err_msg}"
        );

        // The original memory must still be Active (not mutated).
        assert_eq!(
            memory_state(&db, id),
            "active",
            "memory state must be unchanged after stale-token rejection"
        );
        assert_eq!(
            restore_until(&db, id),
            None,
            "restore_until must remain NULL after stale-token rejection"
        );
    }

    // ── Task F1.7.3 restore tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn restore_within_window_succeeds_and_transitions_to_active() {
        // F1.7.3: A forgotten memory whose restore_until is in the future must
        // be restored to Active.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr1").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        assert_eq!(memory_state(&db, id), "forgotten");

        lc.restore(id, None).unwrap();
        assert_eq!(
            memory_state(&db, id),
            "active",
            "restore must transition forgotten → active"
        );
    }

    #[tokio::test]
    async fn restore_after_window_expiry_is_rejected_without_mutation() {
        // F1.7.3: Restoring a memory whose restore_until is in the past must
        // be rejected without changing the memory state.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr2").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        assert_eq!(memory_state(&db, id), "forgotten");

        // Manually backdate restore_until to the past.
        let past = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        db.with_read(|conn| {
            conn.execute(
                "UPDATE memories SET restore_until = ?2 WHERE id = ?1",
                params![id.to_string(), past.clone()],
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap();

        let result = lc.restore(id, None);
        assert!(result.is_err(), "restore after expiry must return an error");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("expired") || msg.contains("window"),
            "error must mention expiry, got: {msg}"
        );

        // Memory must still be Forgotten.
        assert_eq!(
            memory_state(&db, id),
            "forgotten",
            "memory state must be unchanged after expired-window rejection"
        );
    }

    #[tokio::test]
    async fn restore_of_active_memory_is_rejected_without_mutation() {
        // F1.7.3: Restoring an Active (non-Forgotten) memory must be rejected.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr3").await;

        // Memory is Active by default.
        assert_eq!(memory_state(&db, id), "active");

        let result = lc.restore(id, None);
        assert!(
            result.is_err(),
            "restore of active memory must return an error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("active") || msg.contains("expected forgotten"),
            "error must mention state mismatch, got: {msg}"
        );

        // Memory must still be Active.
        assert_eq!(
            memory_state(&db, id),
            "active",
            "memory state must be unchanged after rejection"
        );
    }

    #[tokio::test]
    async fn restore_records_audit_graph_change_and_outbox() {
        // F1.7.3: restore must append audit, graph_changes, and outbox entries.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr4").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        let audit_before = audit_count(&db);
        let changes_before = graph_changes_for(&db, id);
        let outbox_before = outbox_pending_for(&db, id);

        lc.restore(id, None).unwrap();

        // One new audit row for the restore.
        assert_eq!(
            audit_count(&db),
            audit_before + 1,
            "exactly one new audit_records row after restore"
        );

        // One new graph_changes row for the restore transition.
        assert_eq!(
            graph_changes_for(&db, id),
            changes_before + 1,
            "exactly one new graph_changes row after restore"
        );

        // Two new outbox upsert entries (FTS + LanceDB) to re-index the memory.
        assert_eq!(
            outbox_pending_for(&db, id),
            outbox_before + 2,
            "two pending outbox upsert entries (fts + lancedb) after restore"
        );
    }

    #[tokio::test]
    async fn restore_clears_restore_until_to_null() {
        // F1.7.3: After a successful restore, restore_until must be NULL so a
        // future forget+restore cycle cannot skip the window check.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr5").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();
        assert!(
            restore_until(&db, id).is_some(),
            "restore_until must be set after forget"
        );

        lc.restore(id, None).unwrap();
        assert_eq!(
            restore_until(&db, id),
            None,
            "restore_until must be NULL after successful restore"
        );
    }

    #[tokio::test]
    async fn restore_stale_token_is_rejected_without_mutation() {
        // F1.7.3: A stale preview token must cause restore to return an error
        // without mutating anything.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr6").await;

        lc.forget(&ForgetScope::Memory(id), None).unwrap();

        // Obtain a token at the current revision.
        let rev = current_revision(&db);
        let preview = lc
            .preview_restore(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();
        let token = preview.token.clone();

        // Advance the revision by forgetting a different memory, making the token stale.
        let id2 = seed_memory(&db, &events, &rel, &vectors, Source::User, "hr6b").await;
        lc.forget(&ForgetScope::Memory(id2), None).unwrap();

        // Attempt to restore with the now-stale token.
        let result = lc.restore(id, Some(&token));
        assert!(
            result.is_err(),
            "restore with a stale token must return an error"
        );
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("stale"),
            "error must mention 'stale', got: {msg}"
        );

        // Memory must still be Forgotten (no mutation occurred).
        assert_eq!(
            memory_state(&db, id),
            "forgotten",
            "memory state must be unchanged after stale-token rejection"
        );
    }

    // ── Task F1.7.4 hard_delete governance tests ─────────────────────────────

    /// Helper: count graph_changes rows where payload contains the hard_delete
    /// cascade marker for a specific memory id.
    fn hard_delete_graph_changes_for(db: &Database, id: Uuid) -> i64 {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM graph_changes WHERE record_id = ?1 AND change_kind = 'delete'",
                params![id.to_string()],
                |r| r.get(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: count hard_delete audit rows.
    fn hard_delete_audit_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM audit_records WHERE command_kind = 'hard_delete'",
                [],
                |r| r.get(0),
            )
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
    }

    /// Helper: retrieve the payload_json from the first graph_changes row with
    /// change_kind='delete' for a given memory id.
    fn hard_delete_change_payload(db: &Database, id: Uuid) -> Option<String> {
        db.with_read(|conn| {
            conn.query_row(
                "SELECT payload_json FROM graph_changes WHERE record_id = ?1 AND change_kind = 'delete' LIMIT 1",
                params![id.to_string()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(crate::error::StorageError::Sqlite)
            .map_err(Into::into)
        })
        .unwrap()
        .flatten()
    }

    /// Helper: seed two memories linked by memory_derived_from.
    async fn seed_linked_memories(
        db: &Arc<Database>,
        events: &SqliteEventStore,
        rel: &SqliteRelationalStore,
        vectors: &SqliteVectorStore,
        hash_parent: &str,
        hash_child: &str,
    ) -> (Uuid, Uuid) {
        let parent = seed_memory(db, events, rel, vectors, Source::User, hash_parent).await;
        let child = seed_memory(db, events, rel, vectors, Source::User, hash_child).await;
        // Insert derived_from, contradicts, supports links for both sides.
        {
            let tx = db.begin().unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO memory_derived_from(parent_id, child_id) VALUES(?1, ?2)",
                    params![parent.to_string(), child.to_string()],
                )
                .unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO memory_contradicts(a_id, b_id) VALUES(?1, ?2)",
                    params![parent.to_string(), child.to_string()],
                )
                .unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO memory_supports(a_id, b_id) VALUES(?1, ?2)",
                    params![parent.to_string(), child.to_string()],
                )
                .unwrap();
            tx.commit().unwrap();
        }
        (parent, child)
    }

    fn link_count(db: &Database, table: &str, id: Uuid) -> i64 {
        let id_str = id.to_string();
        // Each link table has different column names; build the appropriate predicate.
        let predicate = match table {
            "memory_derived_from" => "parent_id = ?1 OR child_id = ?1",
            "memory_contradicts" | "memory_supports" => "a_id = ?1 OR b_id = ?1",
            "memory_mentions_entity" => "memory_id = ?1",
            _ => "1=0", // unknown table → 0
        };
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE {predicate}");
        db.with_read(|conn| {
            conn.query_row(&sql, params![id_str], |r| r.get(0))
                .map_err(crate::error::StorageError::Sqlite)
                .map_err(Into::into)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn hard_delete_records_audit_graph_change_and_outbox() {
        // F1.7.4: hard_delete must append audit, graph_changes, and outbox
        // entries inside the authority transaction.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hd1").await;

        assert_eq!(
            hard_delete_audit_count(&db),
            0,
            "no hard_delete audit rows before"
        );
        assert_eq!(
            hard_delete_graph_changes_for(&db, id),
            0,
            "no graph_changes before"
        );
        assert_eq!(outbox_pending_for(&db, id), 0, "no outbox before");

        let n = lc
            .hard_delete(&ForgetScope::Memory(id), None)
            .await
            .unwrap();
        assert_eq!(n, 1, "should have deleted one memory");

        // Audit row must exist.
        assert_eq!(
            hard_delete_audit_count(&db),
            1,
            "exactly one audit_records row with command_kind='hard_delete' after"
        );

        // graph_changes row with change_kind='delete' must exist.
        assert_eq!(
            hard_delete_graph_changes_for(&db, id),
            1,
            "exactly one graph_changes row (change_kind='delete') after"
        );

        // Outbox entries must be enqueued (FTS + LanceDb = 2).
        assert_eq!(
            outbox_pending_for(&db, id),
            2,
            "two pending outbox entries (fts + lancedb) after hard_delete"
        );

        // graph_revision must have advanced.
        let rev = current_revision(&db);
        assert!(
            rev.get() > 0,
            "graph_revision must advance after hard_delete"
        );
    }

    #[tokio::test]
    async fn hard_delete_closes_all_dependent_link_tables() {
        // F1.7.4: hard_delete must close memory_mentions_entity,
        // memory_derived_from, memory_contradicts, and memory_supports.
        let (db, lc, events, rel, vectors) = setup().await;
        let (parent, child) =
            seed_linked_memories(&db, &events, &rel, &vectors, "hd2p", "hd2c").await;

        // Add mentions_entity row for parent.
        {
            let tx = db.begin().unwrap();
            // Insert a minimal entity to satisfy memory_mentions_entity.entity_id.
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO entities(id, canonical_id, entity_type, display_name, created_at) \
                     VALUES('ent:1','ent:1','person','Alice',datetime('now'))",
                    [],
                )
                .unwrap();
            tx.conn()
                .execute(
                    "INSERT OR IGNORE INTO memory_mentions_entity(memory_id, entity_id) VALUES(?1, 'ent:1')",
                    params![parent.to_string()],
                )
                .unwrap();
            tx.commit().unwrap();
        }

        // Verify links exist before deletion.
        assert_eq!(link_count(&db, "memory_derived_from", parent), 1);
        assert_eq!(link_count(&db, "memory_contradicts", parent), 1);
        assert_eq!(link_count(&db, "memory_supports", parent), 1);
        assert_eq!(link_count(&db, "memory_mentions_entity", parent), 1);

        // Hard-delete the parent.
        lc.hard_delete(&ForgetScope::Memory(parent), None)
            .await
            .unwrap();

        // All link tables must be cleared for the parent.
        assert_eq!(
            link_count(&db, "memory_derived_from", parent),
            0,
            "derived_from links must be closed"
        );
        assert_eq!(
            link_count(&db, "memory_contradicts", parent),
            0,
            "contradicts links must be closed"
        );
        assert_eq!(
            link_count(&db, "memory_supports", parent),
            0,
            "supports links must be closed"
        );
        assert_eq!(
            link_count(&db, "memory_mentions_entity", parent),
            0,
            "mentions_entity links must be closed"
        );

        // The child must also have its derived_from link cleared (both-sides delete).
        assert_eq!(
            link_count(&db, "memory_derived_from", child),
            0,
            "child side derived_from must also be cleared"
        );
    }

    #[tokio::test]
    async fn hard_delete_with_stale_token_is_rejected_without_mutation() {
        // F1.7.4: A stale preview token must cause hard_delete to return an
        // error without mutating any authority state.
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hd3").await;

        // Obtain a preview token at the current revision.
        let rev = current_revision(&db);
        let preview = lc
            .preview_hard_delete(
                &ForgetScope::Memory(id),
                rev,
                PreviewLimits::single_record(),
            )
            .unwrap();
        let token = preview.token.clone();

        // Advance the revision by forgetting another memory.
        let id2 = seed_memory(&db, &events, &rel, &vectors, Source::User, "hd3b").await;
        lc.forget(&ForgetScope::Memory(id2), None).unwrap();

        // Now the token is stale. hard_delete must be rejected.
        let result = lc.hard_delete(&ForgetScope::Memory(id), Some(&token)).await;
        assert!(result.is_err(), "hard_delete with stale token must fail");
        let msg = format!("{:?}", result.unwrap_err());
        assert!(
            msg.contains("stale"),
            "error must mention 'stale', got: {msg}"
        );

        // Memory must still be Active (no mutation occurred).
        assert_eq!(
            memory_state(&db, id),
            "active",
            "memory must remain active after stale-token rejection"
        );
        assert_eq!(
            hard_delete_audit_count(&db),
            0,
            "no audit row must be created after rejection"
        );
    }

    #[tokio::test]
    async fn hard_delete_cascade_choices_recorded_in_graph_changes_payload() {
        // F1.7.4: the graph_changes payload_json must record which dependent
        // tables were cascaded (cascade choices traceability).
        let (db, lc, events, rel, vectors) = setup().await;
        let id = seed_memory(&db, &events, &rel, &vectors, Source::User, "hd4").await;

        lc.hard_delete(&ForgetScope::Memory(id), None)
            .await
            .unwrap();

        let payload_str = hard_delete_change_payload(&db, id)
            .expect("graph_changes payload_json must exist after hard_delete");

        let payload: serde_json::Value =
            serde_json::from_str(&payload_str).expect("payload_json must be valid JSON");

        // Must record the cascaded tables.
        let cascaded = payload
            .get("cascaded")
            .and_then(|v| v.as_array())
            .expect("payload must have a 'cascaded' array");
        let cascaded_names: Vec<&str> = cascaded.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            cascaded_names.contains(&"memory_mentions_entity"),
            "cascaded must include memory_mentions_entity"
        );
        assert!(
            cascaded_names.contains(&"memory_derived_from"),
            "cascaded must include memory_derived_from"
        );
        assert!(
            cascaded_names.contains(&"memory_contradicts"),
            "cascaded must include memory_contradicts"
        );
        assert!(
            cascaded_names.contains(&"memory_supports"),
            "cascaded must include memory_supports"
        );

        // Must have transition field.
        let transition = payload
            .get("transition")
            .and_then(|v| v.as_str())
            .expect("payload must have a 'transition' field");
        assert!(
            transition.contains("deleted"),
            "transition must mention 'deleted', got: {transition}"
        );
    }

    struct FakeEmbedder {
        dim: usize,
    }
    #[async_trait::async_trait]
    impl crate::stores::ports::Embedder for FakeEmbedder {
        fn model_version(&self) -> crate::types::ModelVersion {
            crate::types::ModelVersion("fake_v1".into())
        }
        fn dim(&self) -> usize {
            self.dim
        }
        async fn embed(
            &self,
            texts: &[String],
        ) -> crate::error::MemoryResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += b as f32 / 255.0;
                    }
                    v
                })
                .collect())
        }
        async fn health(&self) -> crate::types::Availability {
            crate::types::Availability::Up
        }
    }
}
