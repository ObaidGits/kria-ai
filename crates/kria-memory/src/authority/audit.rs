//! The transaction-scoped append-only audit ledger (task **F1.3.4**, design
//! §4.1 `audit_records`, §5.1 "AuthorityTx … writes … audit …").
//!
//! [`TxAuditLog`] is the **transaction-scoped repository** that appends rows to
//! the append-only `audit_records` ledger *using only the serialized-writer
//! transaction connection* handed to it — exactly like [`TxEventLog`]
//! (F1.3.3). It carries no [`Database`] handle, so mis-wiring an audit write
//! onto a second connection is structurally impossible (F1.3 non-negotiable:
//! "all writes must occur on the transaction connection").
//!
//! ## Scope of this module (F1.3.4)
//!
//! Every governed command records exactly one disposition row: `accepted`,
//! `rejected`, or `deferred` ([`AuditDisposition`], mirroring the schema
//! `audit_records.disposition CHECK`). The row carries the command kind, the
//! pending policy version (until F1.4 stamps the resolved one — the same
//! sentinel the event log uses), the actor / caller-partition provenance, the
//! serialized [`RejectionReason`] list (`reason_codes_json`), the authority
//! revision (set by the revision stage F1.3.5 for graph-visible accepted
//! commands; `None` here), and the optional `reversal_of` self-link for a
//! compensating/undo command.
//!
//! ### The `event_id` FK and rejected commands
//!
//! `audit_records.event_id` is **nullable** (`TEXT REFERENCES events_v2(id)` —
//! no `NOT NULL`), so a command rejected *before* the transaction ever opened
//! (F1.3.2 validation) is audited with `event_id = NULL` and no completion
//! event — see [`crate::authority::transaction::record_rejected_command`].
//! An accepted/deferred command appends its completion (or observation) event
//! first and links the audit row to it.
//!
//! [`Database`]: crate::db::Database
//! [`TxEventLog`]: super::event_log::TxEventLog

use rusqlite::params;

use crate::db::AuthorityTx;
use crate::error::{MemoryResult, StorageError};
use crate::model::{AuditId, EventId, GraphRevision, UtcTimestamp};

use super::command::CommandEnvelope;
use super::event_log::PENDING_POLICY_VERSION;
use super::validation::RejectionReason;

// ─────────────────────────────────────────────────────────────────────────
// AuditDisposition — the disposition column (audit_records.disposition CHECK)
// ─────────────────────────────────────────────────────────────────────────

/// The terminal disposition recorded for a governed command, mirroring the
/// schema `audit_records.disposition CHECK (disposition IN
/// ('accepted','rejected','deferred'))` (design §4.1). A closed set so
/// `disposition` can never be a raw unchecked string; also reused verbatim as
/// the completion/observation event `outcome` so the event log and audit ledger
/// speak one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDisposition {
    /// The command committed: authority rows + event + audit + (revision) all
    /// together (design A2 atomicity).
    Accepted,
    /// The command was denied by a deterministic check (a normal outcome — the
    /// carried [`RejectionReason`] codes explain why).
    Rejected,
    /// The command is parked awaiting confirmation / a later decision (e.g. a
    /// previewed destructive command not yet confirmed).
    Deferred,
}

impl AuditDisposition {
    /// The canonical text stored in `audit_records.disposition` (and mirrored in
    /// the event `outcome`).
    pub fn as_str(self) -> &'static str {
        match self {
            AuditDisposition::Accepted => "accepted",
            AuditDisposition::Rejected => "rejected",
            AuditDisposition::Deferred => "deferred",
        }
    }
}

impl std::fmt::Display for AuditDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AuditDraft — the caller-supplied parts of an audit row
// ─────────────────────────────────────────────────────────────────────────

/// The per-command inputs to a single audit append that are *not* derivable
/// from the [`CommandEnvelope`]. Everything else (command kind, actor,
/// caller-partition, policy version, timestamp) is taken from the envelope /
/// constants by [`TxAuditLog::append`], so those provenance fields can never
/// disagree with the command being audited.
#[derive(Debug, Clone)]
pub struct AuditDraft<'a> {
    /// The terminal disposition being recorded.
    pub disposition: AuditDisposition,
    /// The event this disposition concerns, if one was appended. `None` for a
    /// command rejected before the transaction opened (nullable FK).
    pub event_id: Option<&'a EventId>,
    /// The authority revision the command committed at, when graph-visible
    /// (set by the revision stage F1.3.5). `None` for revision-neutral /
    /// rejected / deferred commands.
    pub authority_revision: Option<GraphRevision>,
    /// The rejection reasons to persist as `reason_codes_json`. Empty for an
    /// accepted command (serialized as `[]`).
    pub reasons: &'a [RejectionReason],
    /// The audit row this command reverses/undoes, when it is a compensating
    /// command (`reversal_of` self-link). `None` for an ordinary command.
    pub reversal_of: Option<&'a AuditId>,
}

impl<'a> AuditDraft<'a> {
    /// An accepted-command draft (no reasons; optional committed revision and
    /// reversal link).
    pub fn accepted(
        event_id: &'a EventId,
        authority_revision: Option<GraphRevision>,
        reversal_of: Option<&'a AuditId>,
    ) -> Self {
        Self {
            disposition: AuditDisposition::Accepted,
            event_id: Some(event_id),
            authority_revision,
            reasons: &[],
            reversal_of,
        }
    }

    /// A rejected-command draft carrying the deterministic reason codes. The
    /// `event_id` is `None` when the command was rejected before any event was
    /// appended (nullable FK).
    pub fn rejected(event_id: Option<&'a EventId>, reasons: &'a [RejectionReason]) -> Self {
        Self {
            disposition: AuditDisposition::Rejected,
            event_id,
            authority_revision: None,
            reasons,
            reversal_of: None,
        }
    }

    /// A deferred-command draft (e.g. a previewed destructive command awaiting
    /// confirmation), carrying any explanatory reason codes.
    pub fn deferred(event_id: Option<&'a EventId>, reasons: &'a [RejectionReason]) -> Self {
        Self {
            disposition: AuditDisposition::Deferred,
            event_id,
            authority_revision: None,
            reasons,
            reversal_of: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AppendedAudit — the outcome of an audit append
// ─────────────────────────────────────────────────────────────────────────

/// The identity assigned to a freshly appended audit row. Returned so a later
/// compensating command can link its own audit row back via `reversal_of`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendedAudit {
    /// The new audit row's canonical id (`audit_records.id`).
    pub audit_id: AuditId,
}

// ─────────────────────────────────────────────────────────────────────────
// TxAuditLog — the transaction-scoped audit-log repository
// ─────────────────────────────────────────────────────────────────────────

/// The transaction-scoped append surface over `audit_records`.
///
/// A zero-sized handle: [`append`](Self::append) takes the `&mut AuthorityTx` it
/// must write through, so — exactly like [`TxEventLog`](super::event_log::TxEventLog)
/// — it is structurally impossible for this repository to write anywhere other
/// than the serialized-writer transaction (F1.3 invariant). It owns no
/// [`Database`](crate::db::Database) / connection / pool.
#[derive(Debug, Default, Clone, Copy)]
pub struct TxAuditLog;

impl TxAuditLog {
    /// Construct the (stateless) audit-log repository.
    pub fn new() -> Self {
        TxAuditLog
    }

    /// Append one immutable audit row for `env` on `tx`.
    ///
    /// Provenance columns (`command_kind`, `actor_id`, `caller_partition`,
    /// `policy_version`, `created_at`) are derived from the envelope / constants
    /// so they cannot drift from the audited command; the [`AuditDraft`]
    /// supplies the disposition, the event link, the committed revision, the
    /// reason codes, and the optional reversal link. Runs on `tx`'s connection,
    /// so it commits (or rolls back) atomically with the rest of the command.
    pub fn append(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
        draft: AuditDraft<'_>,
    ) -> MemoryResult<AppendedAudit> {
        let audit_id = AuditId::new_v7();
        let created_at = UtcTimestamp::now();

        // `reason_codes_json` is always a JSON array (empty `[]` for accepted)
        // so it round-trips deterministically; never NULL/absent.
        let reason_codes_json =
            serde_json::to_string(draft.reasons).map_err(|e| StorageError::Serde(e.to_string()))?;

        tx.conn()
            .execute(
                "INSERT INTO audit_records(
                     id, event_id, command_kind, disposition, policy_version,
                     actor_id, caller_partition, reason_codes_json,
                     authority_revision, created_at, reversal_of)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    audit_id.as_str(),
                    draft.event_id.map(EventId::as_str),
                    env.kind().as_str(),
                    draft.disposition.as_str(),
                    PENDING_POLICY_VERSION,
                    env.caller().actor_id(),
                    env.caller().partition_key(),
                    reason_codes_json,
                    draft.authority_revision.map(|r| r.get() as i64),
                    created_at.to_rfc3339(),
                    draft.reversal_of.map(AuditId::as_str),
                ],
            )
            .map_err(StorageError::Sqlite)?;

        Ok(AppendedAudit { audit_id })
    }
}
