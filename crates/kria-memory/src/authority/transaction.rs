//! The authority transaction orchestrator (task **F1.3.3**, design §5.1
//! "AuthorityTx takes the serialized writer … appends start/completion events …
//! reserves exactly one revision … writes ordered changes, audit, idempotency
//! result, and outbox, then commits").
//!
//! [`AuthorityTransaction`] is the design's *"AuthorityTx"* orchestrator: the
//! single object that owns one open serialized-writer transaction
//! ([`crate::db::AuthorityTx`], the low-level SQL primitive) for the
//! duration of one governed command, and drives the ordered commit stages
//! **1.3.3 → 1.3.7** across it. It is a distinct, higher-level type from the
//! low-level `db::AuthorityTx` (which is just the `BEGIN IMMEDIATE` guard); this
//! keeps one name for "the SQL transaction handle" and one for "the command
//! commit workflow" without renaming the widely-used primitive.
//!
//! ## Ordering contract (design §5.1, F1.3 non-negotiables)
//!
//! Validation (F1.3.2) runs **before** [`AuthorityTransaction::begin`] — the
//! writer transaction is only opened once the pre-transaction validator returns
//! `Proceed`. Once open, the stages run strictly in transaction order:
//!
//! 1. **1.3.3 (this task)** — [`append_start_event`] appends the invocation
//!    start Event (when applicable), then [`apply_semantic_mutation`] applies
//!    the semantic change **using only transaction-scoped repositories**.
//! 2. **1.3.4** — completion/command Event + Audit_Record.
//! 3. **1.3.5** — `authority_meta` bump + `graph_revisions` + `graph_changes`
//!    for a graph-visible change; revision-neutral otherwise.
//! 4. **1.3.6** — outbox work + `idempotency_results`, before invariant/FK
//!    checks and [`commit`].
//! 5. **1.3.7** — post-commit publish/wake (outside the transaction).
//!
//! Every write above flows through **this object's** single [`AuthorityTx`], so
//! the whole thing commits atomically or — on any pre-commit error, including a
//! drop without [`commit`] — rolls back *everything* (the `db::AuthorityTx`
//! `Drop` issues `ROLLBACK`). Later stages slot in as additional `&mut self`
//! methods between [`apply_semantic_mutation`] and [`commit`]; they need no new
//! transaction wiring.
//!
//! [`append_start_event`]: AuthorityTransaction::append_start_event
//! [`apply_semantic_mutation`]: AuthorityTransaction::apply_semantic_mutation
//! [`commit`]: AuthorityTransaction::commit
//! [`AuthorityTx`]: crate::db::AuthorityTx

use crate::db::{AuthorityTx, Database};
use crate::error::MemoryResult;
use crate::model::{AuditId, EventId, GraphRevision};

use super::audit::{AppendedAudit, AuditDisposition, AuditDraft, TxAuditLog};
use super::command::CommandEnvelope;
use super::event_log::{AppendedEvent, TxEventLog};
use super::idempotency::{self, TxIdempotency};
use super::outbox::{self, TxOutbox};
use super::publish::{RevisionWake, WakePublisher};
use super::revision::{GraphChange, GraphChangeKind, TxRevisionLog};
use super::validation::RejectionReason;
use super::OutboxWork;

// ─────────────────────────────────────────────────────────────────────────
// Transaction-scoped semantic mutation seam
// ─────────────────────────────────────────────────────────────────────────

/// The outcome of applying a command's semantic mutation. It carries whether
/// the change is **graph-visible** — the single fact the revision stage
/// (F1.3.5) needs to decide whether to reserve a revision ("reserve a revision
/// only for a graph-visible accepted change") — and, when it is, the **ordered
/// list of [`GraphChange`] descriptors** the revision stage appends to
/// `graph_changes` with stable ordinals.
///
/// The changes are produced by the semantic seam ([`TxSemanticStore`], the F2
/// per-kind builders), never invented by the transaction: a graph-visible
/// outcome carries the exact rows its mutation touched, in the deterministic
/// order they should occupy. A [`revision_neutral`](Self::revision_neutral)
/// outcome carries no changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticOutcome {
    /// Whether this mutation changes graph-visible authority state (and so must
    /// reserve exactly one revision in F1.3.5). A revision-neutral command
    /// (e.g. one that observed nothing new) reports `false`.
    pub graph_visible: bool,
    /// The ordered `graph_changes` descriptors for a graph-visible mutation, in
    /// the deterministic order they should receive ordinals `0..n`. Always empty
    /// for a revision-neutral outcome.
    pub changes: Vec<GraphChange>,
}

impl SemanticOutcome {
    /// A graph-visible mutation carrying its ordered change descriptors
    /// (reserves a revision in F1.3.5 and appends `changes` with stable
    /// ordinals). The `changes` order is preserved verbatim.
    pub fn graph_visible(changes: Vec<GraphChange>) -> Self {
        Self {
            graph_visible: true,
            changes,
        }
    }

    /// A revision-neutral mutation (no revision reserved, no changes appended).
    pub fn revision_neutral() -> Self {
        Self {
            graph_visible: false,
            changes: Vec::new(),
        }
    }
}

/// The **transaction-scoped semantic repository** seam: the boundary through
/// which a command's semantic mutation is applied, using *only* the transaction
/// connection handed to [`apply`](TxSemanticStore::apply).
///
/// This is the seam every later per-kind semantic builder (Observe/Correct/
/// Forget/Restore/HardDelete over the F2 cognitive-record tables) plugs into: it
/// receives `&mut AuthorityTx` and MUST NOT open any other connection or the
/// read pool, so the semantic rows commit atomically with the events/audit/
/// revision/outbox rows of the same command. Establishing the trait now fixes
/// that dependency direction before the concrete builders exist.
pub trait TxSemanticStore {
    /// Apply the semantic mutation for `env` on the transaction `tx`, returning
    /// whether the change is graph-visible.
    fn apply(
        &self,
        tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome>;
}

/// The representative semantic store used until the per-kind semantic builders
/// land (F2 cognitive records).
///
/// It performs **no** semantic-row write — the v2 authority schema has no
/// cognitive-record table yet, so writing one here would be fabricated
/// behavior. Instead it honestly reports graph-visibility from the command kind
/// (every governed command kind mutates graph-visible authority state once its
/// builder exists; revision-neutrality becomes a runtime decision the F2
/// builders return), which is exactly the fact F1.3.5 consumes.
///
/// For the change set it returns a **single representative** [`GraphChange`]
/// describing the command being applied — a placeholder, not a claim about
/// concrete cognitive rows (there are none yet). It is explicitly marked
/// `record_kind = "deferred"` / `record_id = None` and carries a
/// `"placeholder": true` payload so it can never be mistaken for a real record
/// mutation. This keeps the graph-visible ↔ "≥1 recorded change" relationship
/// coherent and exercises the stable-ordinal append end-to-end. When the real
/// builders arrive they replace this by implementing [`TxSemanticStore`] over
/// `&mut AuthorityTx` and returning the exact rows they touched — the
/// [`AuthorityTransaction`] wiring does not change.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeferredSemanticStore;

impl TxSemanticStore for DeferredSemanticStore {
    fn apply(
        &self,
        _tx: &mut AuthorityTx<'_>,
        env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        // A representative, clearly-labelled placeholder change — never a claim
        // about concrete cognitive rows, which do not exist until F2.
        let change_kind = match env.kind() {
            crate::authority::CommandKind::Observe => GraphChangeKind::Insert,
            crate::authority::CommandKind::Correct => GraphChangeKind::Update,
            crate::authority::CommandKind::Forget
            | crate::authority::CommandKind::Restore => GraphChangeKind::State,
            crate::authority::CommandKind::HardDelete => GraphChangeKind::Delete,
            // A declassification inserts a new audited provenance record.
            crate::authority::CommandKind::Declassify => GraphChangeKind::Insert,
        };
        let mut change = GraphChange::new(change_kind, env.caller().partition_key());
        change.record_kind = Some("deferred".to_string());
        let change = change.with_payload(
            serde_json::json!({
                "placeholder": true,
                "command_kind": env.kind().as_str(),
                "note": "deferred semantic seam: no concrete cognitive rows until F2",
            })
            .to_string(),
        );
        Ok(SemanticOutcome::graph_visible(vec![change]))
    }
}

// ─────────────────────────────────────────────────────────────────────────
// AuthorityTransaction — the command commit workflow over one serialized tx
// ─────────────────────────────────────────────────────────────────────────

/// The authority transaction orchestrator for one governed command.
///
/// Owns one open [`AuthorityTx`] (the serialized writer) and drives the ordered
/// commit stages over it. Constructed only *after* validation returns `Proceed`
/// (F1.3.2). Drops-without-[`commit`](Self::commit) roll everything back.
pub struct AuthorityTransaction<'a> {
    tx: AuthorityTx<'a>,
    events: TxEventLog,
    audit: TxAuditLog,
    revisions: TxRevisionLog,
    outbox: TxOutbox,
    idempotency: TxIdempotency,
}

/// The paired event + audit rows appended for one terminal command disposition
/// (F1.3.4). Returned by [`AuthorityTransaction::record_disposition`] so the
/// caller can link a later compensating command's audit row via `reversal_of`
/// and echo the completion event id in the command outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    /// The completion (invocation) or observation (ingestion/turn) event.
    pub event: AppendedEvent,
    /// The audit disposition row linked to that event.
    pub audit: AppendedAudit,
    /// The revision the command committed at, when the change was graph-visible
    /// (reserved by the F1.3.5 revision stage and carried on the audit row).
    /// `None` for a revision-neutral / deferred command. The command outcome
    /// echoes this so an accepted graph-visible command reports its committed
    /// revision.
    pub revision: Option<GraphRevision>,
}

impl<'a> AuthorityTransaction<'a> {
    /// Open the serialized-writer transaction (`BEGIN IMMEDIATE`) for a command.
    ///
    /// Call this **only** after the pre-transaction validator (F1.3.2) returned
    /// [`Proceed`](super::validation::ValidationOutcome::Proceed): "validate
    /// before BEGIN".
    pub fn begin(db: &'a Database) -> MemoryResult<Self> {
        Ok(Self {
            tx: db.begin()?,
            events: TxEventLog::new(),
            audit: TxAuditLog::new(),
            revisions: TxRevisionLog::new(),
            outbox: TxOutbox::new(),
            idempotency: TxIdempotency::new(),
        })
    }

    /// **Stage 1.3.3a** — append the invocation start Event when applicable.
    ///
    /// A start Event is appended iff the command's source is an *active
    /// invocation* ([`SourceKind::is_invocation`](super::command::SourceKind::is_invocation));
    /// ingestion/turn sources record their event without a start marker, so this
    /// returns `Ok(None)` for them. The append goes through this transaction, so
    /// it commits/rolls back with the whole command.
    pub fn append_start_event(
        &mut self,
        env: &CommandEnvelope,
    ) -> MemoryResult<Option<AppendedEvent>> {
        if !env.source().source_kind().is_invocation() {
            return Ok(None);
        }
        let appended = self.events.append_start(&mut self.tx, env)?;
        Ok(Some(appended))
    }

    /// **Stage 1.3.3b** — apply the semantic mutation via a transaction-scoped
    /// [`TxSemanticStore`].
    ///
    /// The `store` writes only through this transaction's connection (the seam's
    /// contract), so the semantic rows are part of the same atomic commit.
    /// Returns the [`SemanticOutcome`] the revision stage (F1.3.5) consumes.
    pub fn apply_semantic_mutation<S: TxSemanticStore>(
        &mut self,
        store: &S,
        env: &CommandEnvelope,
    ) -> MemoryResult<SemanticOutcome> {
        store.apply(&mut self.tx, env)
    }

    /// **Stage 1.3.4a** — append the immutable completion / command Event.
    ///
    /// An *invocation* source (native/MCP/OpenClaw/sidecar) records a
    /// `completion` event that pairs with its start event; a passive ingestion /
    /// turn source records a single `observation` event instead (design §5.1).
    /// Either way the event carries the semantic command body, its HLC / time /
    /// source / checksum columns, and the typed `disposition` as its `outcome`.
    /// The append goes through this transaction, so it commits/rolls back with
    /// the whole command.
    pub fn append_completion_event(
        &mut self,
        env: &CommandEnvelope,
        disposition: AuditDisposition,
    ) -> MemoryResult<AppendedEvent> {
        let outcome = disposition.as_str();
        if env.source().source_kind().is_invocation() {
            self.events.append_completion(&mut self.tx, env, outcome)
        } else {
            self.events
                .append_observation(&mut self.tx, env, Some(outcome))
        }
    }

    /// **Stage 1.3.4b** — append the immutable Audit_Record for the command.
    ///
    /// The [`AuditDraft`] supplies the disposition, the linked event, the
    /// (F1.3.5-supplied) committed revision, the reason codes, and the optional
    /// `reversal_of` link; provenance is derived from `env`. Written through
    /// this transaction so it is part of the same atomic commit.
    pub fn append_audit(
        &mut self,
        env: &CommandEnvelope,
        draft: AuditDraft<'_>,
    ) -> MemoryResult<AppendedAudit> {
        self.audit.append(&mut self.tx, env, draft)
    }

    /// **Stage 1.3.4 (combined)** — append the completion/observation Event then
    /// its Audit_Record for a single terminal disposition, atomically.
    ///
    /// This is the normal in-transaction path: an accepted command calls it with
    /// [`AuditDisposition::Accepted`] (optionally supplying the committed
    /// revision once F1.3.5 reserves one, and a `reversal_of` link for a
    /// compensating command); a deferred command with
    /// [`AuditDisposition::Deferred`]. `reasons` is empty for an accepted
    /// command and carries the explanatory codes otherwise.
    ///
    /// A command *rejected before the transaction opened* is audited without an
    /// event via [`record_rejected_command`] instead — that path is the reason
    /// the `audit_records.event_id` FK is nullable.
    pub fn record_disposition(
        &mut self,
        env: &CommandEnvelope,
        disposition: AuditDisposition,
        authority_revision: Option<GraphRevision>,
        reasons: &[RejectionReason],
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<CommandRecord> {
        let event = self.append_completion_event(env, disposition)?;
        let draft = AuditDraft {
            disposition,
            event_id: Some(&event.event_id),
            authority_revision,
            reasons,
            reversal_of,
        };
        let audit = self.append_audit(env, draft)?;
        Ok(CommandRecord {
            event,
            audit,
            revision: authority_revision,
        })
    }

    /// **Stage 1.3.5** — reserve the authority revision for a graph-visible
    /// accepted change and append its `graph_revisions` + `graph_changes` rows.
    ///
    /// Called after [`append_completion_event`](Self::append_completion_event)
    /// (which mints the completion/observation event used as the unique
    /// `graph_revisions.tx_id`) and *before* the audit row, so the reserved
    /// revision can be wired into `audit_records.authority_revision`.
    ///
    /// Enforces the F1.3.5 non-negotiables from the [`SemanticOutcome`]:
    /// * **Graph-visible** → increments `authority_meta.graph_revision` exactly
    ///   once, appends one contiguous `graph_revisions` row, and appends the
    ///   ordered `changes` to `graph_changes` with stable ordinals; returns the
    ///   reserved [`GraphRevision`].
    /// * **Revision-neutral** → touches nothing (no counter bump, no
    ///   `graph_revisions`/`graph_changes` rows) and returns `None`.
    ///
    /// All writes go through this transaction, so they are part of the same
    /// atomic commit (or roll back together on any pre-commit failure).
    pub fn reserve_revision(
        &mut self,
        env: &CommandEnvelope,
        outcome: &SemanticOutcome,
        tx_id: &str,
    ) -> MemoryResult<Option<GraphRevision>> {
        if !outcome.graph_visible {
            // Revision-neutral: nothing is written; the audit row records `None`.
            return Ok(None);
        }
        let revision = self
            .revisions
            .reserve(&mut self.tx, env, tx_id, &outcome.changes)?;
        Ok(Some(revision))
    }

    /// **Stage 1.3.4 + 1.3.5 (combined)** — record a command's terminal
    /// disposition *and* reserve its revision atomically, wiring the reserved
    /// revision into the audit row and the returned [`CommandRecord`].
    ///
    /// This is the accepted-command in-transaction path once F1.3.5 lands:
    /// 1. append the completion/observation event (its id becomes the unique
    ///    revision `tx_id`);
    /// 2. reserve the revision from the [`SemanticOutcome`] — one increment +
    ///    contiguous `graph_revisions` + ordered `graph_changes` for a
    ///    graph-visible change; nothing for a revision-neutral one;
    /// 3. append the audit row carrying that (possibly `None`) revision.
    ///
    /// A graph-visible accepted command therefore commits with its
    /// `authority_meta` bump, its revision/changes ledger rows, its completion
    /// event, and its audit row all in one atomic transaction; a revision-neutral
    /// command commits event + audit only, leaving the revision counter and
    /// ledgers untouched.
    pub fn record_disposition_with_revision(
        &mut self,
        env: &CommandEnvelope,
        disposition: AuditDisposition,
        outcome: &SemanticOutcome,
        reasons: &[RejectionReason],
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<CommandRecord> {
        let event = self.append_completion_event(env, disposition)?;
        let revision = self.reserve_revision(env, outcome, event.event_id.as_str())?;
        let draft = AuditDraft {
            disposition,
            event_id: Some(&event.event_id),
            authority_revision: revision,
            reasons,
            reversal_of,
        };
        let audit = self.append_audit(env, draft)?;
        Ok(CommandRecord {
            event,
            audit,
            revision,
        })
    }

    /// **Stage 1.3.6a** — enqueue idempotent derived-projection outbox work for
    /// one work item.
    ///
    /// Delegates to [`TxOutbox::enqueue`] on this transaction, so the row commits
    /// (or rolls back) atomically with the rest of the command. The enqueue is
    /// **idempotent** with respect to the semantic-uniqueness key: re-enqueuing
    /// the same `(target, op, record_kind, record_id, content_hash,
    /// model_partition)` work is a no-op that returns `Ok(false)`; a fresh
    /// enqueue returns `Ok(true)`.
    pub fn enqueue_outbox(&mut self, work: &OutboxWork) -> MemoryResult<bool> {
        self.outbox.enqueue(&mut self.tx, work)
    }

    /// **Stage 1.3.6a** — enqueue the idempotent derived-projection work implied
    /// by a graph-visible [`SemanticOutcome`] at `authority_revision`.
    ///
    /// Maps each record-scoped [`GraphChange`] to its projection work via
    /// [`projection_work_for_changes`](super::outbox::projection_work_for_changes)
    /// (a change with no concrete record — e.g. the deferred placeholder — maps
    /// to no work) and enqueues each item idempotently on this transaction.
    /// Returns the work list that was mapped (whether or not each item was newly
    /// inserted), so a later post-commit publish stage (F1.3.7) can wake the
    /// affected targets.
    pub fn enqueue_projection_work(
        &mut self,
        outcome: &SemanticOutcome,
        authority_revision: Option<GraphRevision>,
    ) -> MemoryResult<Vec<OutboxWork>> {
        let work = outbox::projection_work_for_changes(&outcome.changes, authority_revision);
        for item in &work {
            self.outbox.enqueue(&mut self.tx, item)?;
        }
        Ok(work)
    }

    /// **Stage 1.3.6b** — store the canonical command result in
    /// `idempotency_results`, before invariant/FK checks and [`commit`](Self::commit).
    ///
    /// Serializes the outcome (`status` / `event_id` / `revision`) into the
    /// canonical result JSON, then writes the `idempotency_results` row on this
    /// transaction keyed by the envelope's `(caller_partition, idempotency_key)`
    /// and stamped with its `command_hash`. Because it is written on the same
    /// transaction as the semantic / event / audit / revision / outbox rows, a
    /// later replay (F1.3.2) sees the stored result iff the command's effects
    /// committed (all-or-none). Returns the serialized `result_json` that was
    /// stored.
    pub fn persist_idempotency_result(
        &mut self,
        env: &CommandEnvelope,
        status: &str,
        event_id: Option<&EventId>,
        committed_revision: Option<GraphRevision>,
    ) -> MemoryResult<String> {
        let result_json = idempotency::canonical_result_json(status, event_id, committed_revision)?;
        self.idempotency.persist(
            &mut self.tx,
            env,
            &result_json,
            committed_revision,
            event_id,
        )?;
        Ok(result_json)
    }

    /// **Stages 1.3.3 → 1.3.6 (combined)** — run the whole accepted-command flow
    /// over one serialized transaction and commit it atomically.
    ///
    /// Drives the strict commit order for an accepted command:
    /// 1. **1.3.3** — start event (when the source is an invocation) + semantic
    ///    mutation via `store`;
    /// 2. **1.3.4 + 1.3.5** — completion/observation event, revision reservation
    ///    (for a graph-visible change), and the audit row
    ///    ([`record_disposition_with_revision`](Self::record_disposition_with_revision));
    /// 3. **1.3.6** — enqueue the idempotent derived-projection outbox work, then
    ///    store the canonical idempotency result;
    /// 4. **commit** — SQLite enforces the FK/invariant checks at `COMMIT`
    ///    (`foreign_keys = ON`), so the whole command commits all-or-none.
    ///
    /// Consumes the orchestrator and returns the [`CommandRecord`]. Post-commit
    /// publication (F1.3.7) runs *after* this returns, outside the transaction,
    /// and must not alter committed truth.
    pub fn commit_accepted_command<S: TxSemanticStore>(
        mut self,
        env: &CommandEnvelope,
        store: &S,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<CommandRecord> {
        self.append_start_event(env)?;
        let outcome = self.apply_semantic_mutation(store, env)?;
        let record = self.record_disposition_with_revision(
            env,
            AuditDisposition::Accepted,
            &outcome,
            &[],
            reversal_of,
        )?;
        self.enqueue_projection_work(&outcome, record.revision)?;
        self.persist_idempotency_result(
            env,
            super::idempotency::COMMITTED_STATUS,
            Some(&record.event.event_id),
            record.revision,
        )?;
        self.commit()?;
        Ok(record)
    }

    /// **Stages 1.3.3 → 1.3.7 (combined)** — run the whole accepted-command flow,
    /// commit it atomically, then emit the advisory post-commit wake.
    ///
    /// Identical to [`commit_accepted_command`](Self::commit_accepted_command)
    /// through commit, then adds **stage 1.3.7**: after the transaction has
    /// committed and been consumed, it builds a [`RevisionWake`] cursor from the
    /// already-committed [`CommandRecord`] (only for a graph-visible change that
    /// advanced the revision, and flagging whether F1.3.6 enqueued outbox work)
    /// and hands it to `publisher`.
    ///
    /// The publish runs strictly *after* [`commit`](Self::commit) returns, with
    /// no open transaction in scope, so — by construction — a publication failure
    /// (no subscribers, dropped channel, or a crash before publish) **cannot roll
    /// back or alter the committed rows**. A consumer that misses the wake
    /// recovers the same information from the durable revision/outbox cursor
    /// ([`revisions_since`](super::publish::revisions_since) +
    /// [`OutboxPort::pending`](super::OutboxPort::pending)). The returned
    /// [`CommandRecord`] is the committed truth regardless of wake delivery.
    pub fn commit_and_publish<S: TxSemanticStore, P: WakePublisher>(
        mut self,
        env: &CommandEnvelope,
        store: &S,
        reversal_of: Option<&AuditId>,
        publisher: &P,
    ) -> MemoryResult<CommandRecord> {
        self.append_start_event(env)?;
        let outcome = self.apply_semantic_mutation(store, env)?;
        let record = self.record_disposition_with_revision(
            env,
            AuditDisposition::Accepted,
            &outcome,
            &[],
            reversal_of,
        )?;
        let work = self.enqueue_projection_work(&outcome, record.revision)?;
        self.persist_idempotency_result(
            env,
            super::idempotency::COMMITTED_STATUS,
            Some(&record.event.event_id),
            record.revision,
        )?;
        // ── Durable truth boundary: everything above commits all-or-none. ──
        self.commit()?;
        // ── Stage 1.3.7: post-commit publication (outside the transaction). ──
        // The tx is committed and consumed; nothing writable is in scope, so a
        // failed/dropped publish cannot touch committed truth. The wake is a
        // pure {base → target} cursor + pending-work flag, never the data.
        if let Some(wake) =
            RevisionWake::for_committed(env.base_revision(), &record, !work.is_empty())
        {
            publisher.publish(&wake);
        }
        Ok(record)
    }

    /// The transaction-scoped writer, for stages/repositories that operate
    /// directly on it (F1.3.4–F1.3.6 append audit/revision/outbox rows here).
    /// Exposing the `&mut AuthorityTx` — rather than a fresh connection — is how
    /// every later stage inherits the "writes only on the transaction
    /// connection" invariant.
    pub fn tx_mut(&mut self) -> &mut AuthorityTx<'a> {
        &mut self.tx
    }

    /// Commit the whole command atomically (design §5.1 `TxOpen --> Committed`).
    /// Consumes the orchestrator. After F1.3.6 this is called once outbox +
    /// idempotency rows are written and pre-commit invariant/FK checks pass.
    pub fn commit(self) -> MemoryResult<()> {
        self.tx.commit()
    }
}

/// Record the audit trail for a command **rejected before the transaction
/// opened** (F1.3.2 validation returned
/// [`Rejected`](super::validation::ValidationOutcome::Rejected)).
///
/// A rejected command never appends a semantic mutation, a completion event, or
/// a revision — there is nothing to make atomic with other command rows — so it
/// is audited in its **own minimal serialized-writer transaction** with
/// `event_id = NULL` (the `audit_records.event_id` FK is nullable precisely for
/// this case) and the deterministic [`RejectionReason`] codes serialized into
/// `reason_codes_json`. The single append + commit is itself atomic.
///
/// Returns the appended audit identity. `reasons` must be non-empty for a
/// genuine rejection (mirrors [`ValidationOutcome::Rejected`] never being
/// empty), but the append does not enforce that.
///
/// [`ValidationOutcome::Rejected`]: super::validation::ValidationOutcome::Rejected
pub fn record_rejected_command(
    db: &Database,
    env: &CommandEnvelope,
    reasons: &[RejectionReason],
) -> MemoryResult<AppendedAudit> {
    let mut tx = db.begin()?;
    let appended = TxAuditLog::new().append(&mut tx, env, AuditDraft::rejected(None, reasons))?;
    tx.commit()?;
    Ok(appended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::command::{
        Deadline, PreviewToken, SourceContext, SourceKind, SourceTrust,
    };
    use crate::authority::CommandKind;
    use crate::db::Database;
    use crate::ids::Hlc;
    use crate::model::{
        CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
    };
    use crate::types::MemoryMode;
    use rusqlite::params;
    use std::sync::Arc;

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn caller() -> CallerContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        CallerContext::local_desktop("local-desktop", partition).unwrap()
    }

    fn source(kind: SourceKind) -> SourceContext {
        SourceContext::new(
            InvocationId::new_v7(),
            kind,
            "core:cognition",
            SourceTrust::System,
        )
        .unwrap()
    }

    fn observe_env(kind: SourceKind) -> CommandEnvelope {
        CommandEnvelope::new(
            caller(),
            CommandKind::Observe,
            IdempotencyKey::new("cmd-1").unwrap(),
            GraphRevision::base(),
            source(kind),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"content": "hello"}),
            None,
        )
        .unwrap()
    }

    fn forget_env() -> CommandEnvelope {
        CommandEnvelope::new(
            caller(),
            CommandKind::Forget,
            IdempotencyKey::new("cmd-forget").unwrap(),
            GraphRevision::base(),
            source(SourceKind::Native),
            MemoryMode::Permanent,
            Deadline::default_write(),
            serde_json::json!({"target": "rec-1"}),
            Some(PreviewToken::new("tok-1").unwrap()),
        )
        .unwrap()
    }

    /// Count events_v2 rows for an invocation id, using the read surface.
    fn event_count(db: &Database, invocation_id: &str) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM events_v2 WHERE invocation_id = ?1",
                    params![invocation_id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn watermark(db: &Database) -> String {
        db.with_read(|conn| {
            let s: String = conn
                .query_row(
                    "SELECT event_hlc FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(s)
        })
        .unwrap()
    }

    // ── Happy path: begin → start event → semantic mutation → commit ──────
    #[test]
    fn commit_persists_start_event_and_advances_hlc() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        assert_eq!(
            event_count(&db, &inv),
            0,
            "no events before the transaction"
        );
        assert_eq!(watermark(&db), "", "watermark seeds empty");

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let appended = txn.append_start_event(&env).unwrap();
        assert!(
            appended.is_some(),
            "native source is an invocation → start event"
        );
        let appended = appended.unwrap();
        let outcome = txn
            .apply_semantic_mutation(&DeferredSemanticStore, &env)
            .unwrap();
        assert!(outcome.graph_visible);
        txn.commit().unwrap();

        // The start event is durably present after commit …
        assert_eq!(event_count(&db, &inv), 1, "start event committed");
        // … and the authority HLC watermark advanced to the appended HLC.
        assert_eq!(watermark(&db), appended.hlc.encode());
        assert!(appended.hlc > Hlc::ZERO, "allocated HLC advances past ZERO");

        // The committed row carries the expected phase / provenance / checksum.
        db.with_read(|conn| {
            let (phase, source_kind, actor_id, checksum, encoding, schema_version): (
                String,
                String,
                String,
                String,
                String,
                i64,
            ) = conn
                .query_row(
                    "SELECT phase, source_kind, actor_id, payload_checksum, payload_encoding, schema_version
                     FROM events_v2 WHERE id = ?1",
                    params![appended.event_id.as_str()],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(phase, "start");
            assert_eq!(source_kind, "native");
            assert_eq!(actor_id, "local-desktop");
            assert_eq!(encoding, super::super::event_log::PAYLOAD_ENCODING_PLAIN_JSON);
            assert_eq!(schema_version, super::super::event_log::EVENT_SCHEMA_VERSION);
            assert_eq!(checksum.len(), 64, "blake3 hex checksum is 64 chars");
            Ok(())
        })
        .unwrap();
    }

    // ── Rollback: drop without commit reverts the start event AND the HLC ──
    #[test]
    fn rollback_on_drop_reverts_event_and_watermark() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        {
            let mut txn = AuthorityTransaction::begin(&db).unwrap();
            txn.append_start_event(&env).unwrap();
            txn.apply_semantic_mutation(&DeferredSemanticStore, &env)
                .unwrap();
            // Drop WITHOUT commit → the whole transaction rolls back.
        }

        assert_eq!(
            event_count(&db, &inv),
            0,
            "rolled-back start event leaves no row"
        );
        assert_eq!(
            watermark(&db),
            "",
            "rolled-back HLC watermark returns to its pre-transaction value"
        );
    }

    // ── Atomicity across the semantic seam: both rows or neither ──────────
    /// A test-only semantic store that writes a real row (into `sources`, an
    /// existing v2 table) through the *same* transaction, proving the seam
    /// carries semantic writes atomically with the start event.
    struct SourceRegisteringStore;

    impl TxSemanticStore for SourceRegisteringStore {
        fn apply(
            &self,
            tx: &mut AuthorityTx<'_>,
            env: &CommandEnvelope,
        ) -> MemoryResult<SemanticOutcome> {
            tx.conn()
                .execute(
                    "INSERT INTO sources(
                         id, source_kind, namespace, owner_id, scope, sensitivity, policy_version)
                     VALUES (?1, ?2, 'user', 'owner-1', 'chat', 0, 'pending-f1.4')",
                    params![
                        env.source().invocation_id().as_str(),
                        env.source().source_kind().as_str(),
                    ],
                )
                .map_err(crate::error::StorageError::Sqlite)?;
            Ok(SemanticOutcome::graph_visible(Vec::new()))
        }
    }

    fn source_count(db: &Database, id: &str) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sources WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    #[test]
    fn semantic_write_and_start_event_commit_atomically() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        txn.append_start_event(&env).unwrap();
        txn.apply_semantic_mutation(&SourceRegisteringStore, &env)
            .unwrap();
        txn.commit().unwrap();

        assert_eq!(event_count(&db, &inv), 1, "start event present");
        assert_eq!(source_count(&db, &inv), 1, "semantic source row present");
    }

    #[test]
    fn semantic_write_and_start_event_roll_back_together() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        {
            let mut txn = AuthorityTransaction::begin(&db).unwrap();
            txn.append_start_event(&env).unwrap();
            txn.apply_semantic_mutation(&SourceRegisteringStore, &env)
                .unwrap();
            // No commit → both writes roll back together.
        }

        assert_eq!(event_count(&db, &inv), 0, "start event rolled back");
        assert_eq!(source_count(&db, &inv), 0, "semantic write rolled back");
    }

    // ── Immutability: a committed start event cannot be updated or deleted ─
    #[test]
    fn committed_start_event_is_immutable() {
        let db = fresh_db();
        let env = forget_env();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let appended = txn.append_start_event(&env).unwrap().unwrap();
        txn.commit().unwrap();

        let conn = db.write();
        let upd = conn.execute(
            "UPDATE events_v2 SET outcome = 'tampered' WHERE id = ?1",
            params![appended.event_id.as_str()],
        );
        assert!(
            upd.is_err(),
            "UPDATE on a committed start event must be rejected (L1)"
        );
        let del = conn.execute(
            "DELETE FROM events_v2 WHERE id = ?1",
            params![appended.event_id.as_str()],
        );
        assert!(
            del.is_err(),
            "DELETE on a committed start event must be rejected (L1)"
        );
    }

    // ── HLC monotonicity across two appends in one transaction ────────────
    #[test]
    fn successive_appends_allocate_strictly_increasing_hlc() {
        let db = fresh_db();
        let env_a = observe_env(SourceKind::Native);
        let env_b = observe_env(SourceKind::Mcp);

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let a = txn.append_start_event(&env_a).unwrap().unwrap();
        let b = txn.append_start_event(&env_b).unwrap().unwrap();
        txn.commit().unwrap();

        assert!(
            b.hlc > a.hlc,
            "second appended HLC must be strictly greater"
        );
        assert_eq!(
            watermark(&db),
            b.hlc.encode(),
            "watermark is the latest HLC"
        );
    }

    // ── "when applicable": non-invocation sources skip the start event ────
    #[test]
    fn non_invocation_source_skips_start_event() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Conversation);
        let inv = env.source().invocation_id().as_str().to_string();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let appended = txn.append_start_event(&env).unwrap();
        assert!(
            appended.is_none(),
            "conversation source is not an invocation"
        );
        txn.commit().unwrap();

        assert_eq!(
            event_count(&db, &inv),
            0,
            "no start event for a non-invocation"
        );
        assert_eq!(
            watermark(&db),
            "",
            "no HLC allocated when no event appended"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // F1.3.4 — completion/command event + audit record
    // ─────────────────────────────────────────────────────────────────────

    use crate::authority::validation::{RejectionCode, RejectionReason};

    /// A single audit row read back for assertions.
    struct AuditRow {
        event_id: Option<String>,
        command_kind: String,
        disposition: String,
        policy_version: String,
        actor_id: String,
        caller_partition: String,
        reason_codes_json: String,
        authority_revision: Option<i64>,
        reversal_of: Option<String>,
    }

    fn audit_row(db: &Database, audit_id: &str) -> AuditRow {
        db.with_read(|conn| {
            let row = conn
                .query_row(
                    "SELECT event_id, command_kind, disposition, policy_version, actor_id,
                            caller_partition, reason_codes_json, authority_revision, reversal_of
                     FROM audit_records WHERE id = ?1",
                    params![audit_id],
                    |r| {
                        Ok(AuditRow {
                            event_id: r.get(0)?,
                            command_kind: r.get(1)?,
                            disposition: r.get(2)?,
                            policy_version: r.get(3)?,
                            actor_id: r.get(4)?,
                            caller_partition: r.get(5)?,
                            reason_codes_json: r.get(6)?,
                            authority_revision: r.get(7)?,
                            reversal_of: r.get(8)?,
                        })
                    },
                )
                .unwrap();
            Ok(row)
        })
        .unwrap()
    }

    fn audit_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM audit_records", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn reasons() -> Vec<RejectionReason> {
        vec![
            RejectionReason {
                code: RejectionCode::ModeRejected,
                detail: "mode forbids write".into(),
            },
            RejectionReason {
                code: RejectionCode::LimitExceeded,
                detail: "payload too large".into(),
            },
        ]
    }

    // ── Accepted: start + completion event + accepted audit, atomically ───
    #[test]
    fn accepted_command_records_completion_event_and_audit() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let start = txn.append_start_event(&env).unwrap().unwrap();
        txn.apply_semantic_mutation(&DeferredSemanticStore, &env)
            .unwrap();
        let record = txn
            .record_disposition(&env, AuditDisposition::Accepted, None, &[], None)
            .unwrap();
        txn.commit().unwrap();

        // start + completion events both present for the invocation.
        assert_eq!(event_count(&db, &inv), 2, "start + completion events");
        assert!(
            record.event.hlc > start.hlc,
            "completion HLC strictly after start HLC"
        );

        // The completion event carries phase=completion and outcome=accepted.
        db.with_read(|conn| {
            let (phase, outcome, checksum): (String, String, String) = conn
                .query_row(
                    "SELECT phase, outcome, payload_checksum FROM events_v2 WHERE id = ?1",
                    params![record.event.event_id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            assert_eq!(phase, "completion");
            assert_eq!(outcome, "accepted");
            assert_eq!(checksum.len(), 64, "blake3 hex checksum is 64 chars");
            Ok(())
        })
        .unwrap();

        // The audit row links the completion event and carries provenance.
        let a = audit_row(&db, &record.audit.audit_id.as_str());
        assert_eq!(a.event_id.as_deref(), Some(record.event.event_id.as_str()));
        assert_eq!(a.command_kind, "observe");
        assert_eq!(a.disposition, "accepted");
        assert_eq!(a.policy_version, "pending-f1.4");
        assert_eq!(a.actor_id, "local-desktop");
        assert_eq!(a.caller_partition, "user/chat/0");
        assert_eq!(a.reason_codes_json, "[]", "accepted → empty reason array");
        assert_eq!(a.authority_revision, None);
        assert_eq!(a.reversal_of, None);
    }

    // ── Non-invocation source records an observation completion event ─────
    #[test]
    fn accepted_non_invocation_uses_observation_phase() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Conversation);

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .record_disposition(&env, AuditDisposition::Accepted, None, &[], None)
            .unwrap();
        txn.commit().unwrap();

        db.with_read(|conn| {
            let phase: String = conn
                .query_row(
                    "SELECT phase FROM events_v2 WHERE id = ?1",
                    params![record.event.event_id.as_str()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(phase, "observation", "ingestion/turn source → observation");
            Ok(())
        })
        .unwrap();
    }

    // ── Deferred disposition variant ──────────────────────────────────────
    #[test]
    fn deferred_command_records_deferred_audit_with_reasons() {
        let db = fresh_db();
        let env = forget_env();
        let rs = reasons();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .record_disposition(&env, AuditDisposition::Deferred, None, &rs, None)
            .unwrap();
        txn.commit().unwrap();

        let a = audit_row(&db, &record.audit.audit_id.as_str());
        assert_eq!(a.disposition, "deferred");
        assert_eq!(a.command_kind, "forget");
        // reason_codes_json round-trips back into the same reason list.
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&a.reason_codes_json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["code"], "mode_rejected");
        assert_eq!(parsed[1]["code"], "limit_exceeded");
    }

    // ── reason_codes_json round-trips exactly ─────────────────────────────
    #[test]
    fn reason_codes_json_round_trips() {
        let rs = reasons();
        let json = serde_json::to_string(&rs).unwrap();

        // The stored form is what the audit column holds; re-parsing yields the
        // same code/detail pairs (RejectionReason is Serialize-only, so compare
        // the structural JSON).
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0]["code"], "mode_rejected");
        assert_eq!(back[0]["detail"], "mode forbids write");
        assert_eq!(back[1]["code"], "limit_exceeded");
        assert_eq!(back[1]["detail"], "payload too large");
    }

    // ── reversal_of self-link between two audit rows ──────────────────────
    #[test]
    fn reversal_of_links_a_compensating_audit_row() {
        let db = fresh_db();

        // Original accepted command.
        let original = observe_env(SourceKind::Native);
        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let first = txn
            .record_disposition(&original, AuditDisposition::Accepted, None, &[], None)
            .unwrap();
        txn.commit().unwrap();

        // A compensating (undo) command links its audit row back to the first.
        let undo = forget_env();
        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let second = txn
            .record_disposition(
                &undo,
                AuditDisposition::Accepted,
                None,
                &[],
                Some(&first.audit.audit_id),
            )
            .unwrap();
        txn.commit().unwrap();

        let a = audit_row(&db, &second.audit.audit_id.as_str());
        assert_eq!(
            a.reversal_of.as_deref(),
            Some(first.audit.audit_id.as_str()),
            "reversal_of points at the reversed audit row"
        );
    }

    // ── Rejected pre-transaction command: audit with NULL event_id ────────
    #[test]
    fn rejected_command_audits_with_null_event_and_no_event_row() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();
        let rs = reasons();

        let appended = record_rejected_command(&db, &env, &rs).unwrap();

        // No event was appended for a rejected command …
        assert_eq!(
            event_count(&db, &inv),
            0,
            "rejected command appends no event"
        );
        // … but exactly one audit row exists, with a NULL event_id FK.
        assert_eq!(audit_count(&db), 1);
        let a = audit_row(&db, &appended.audit_id.as_str());
        assert_eq!(a.event_id, None, "rejected audit has NULL event_id");
        assert_eq!(a.disposition, "rejected");
        assert_eq!(a.command_kind, "observe");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&a.reason_codes_json).unwrap();
        assert_eq!(parsed.len(), 2, "both rejection reasons persisted");
    }

    // ── Committed completion event is immutable (append-only, L1) ─────────
    #[test]
    fn committed_completion_event_is_immutable() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .record_disposition(&env, AuditDisposition::Accepted, None, &[], None)
            .unwrap();
        txn.commit().unwrap();

        let conn = db.write();
        let upd = conn.execute(
            "UPDATE events_v2 SET outcome = 'tampered' WHERE id = ?1",
            params![record.event.event_id.as_str()],
        );
        assert!(
            upd.is_err(),
            "UPDATE on a committed completion event is rejected (L1)"
        );
        let del = conn.execute(
            "DELETE FROM events_v2 WHERE id = ?1",
            params![record.event.event_id.as_str()],
        );
        assert!(
            del.is_err(),
            "DELETE on a committed completion event is rejected (L1)"
        );
    }

    // ── Committed audit row is immutable (append-only, L1) ────────────────
    #[test]
    fn committed_audit_record_is_immutable() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .record_disposition(&env, AuditDisposition::Accepted, None, &[], None)
            .unwrap();
        txn.commit().unwrap();

        let conn = db.write();
        let upd = conn.execute(
            "UPDATE audit_records SET disposition = 'rejected' WHERE id = ?1",
            params![record.audit.audit_id.as_str()],
        );
        assert!(
            upd.is_err(),
            "UPDATE on a committed audit row is rejected (L1)"
        );
        let del = conn.execute(
            "DELETE FROM audit_records WHERE id = ?1",
            params![record.audit.audit_id.as_str()],
        );
        assert!(
            del.is_err(),
            "DELETE on a committed audit row is rejected (L1)"
        );
    }

    // ── Rollback: completion event + audit revert together on drop ────────
    #[test]
    fn completion_event_and_audit_roll_back_together() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();

        {
            let mut txn = AuthorityTransaction::begin(&db).unwrap();
            txn.append_start_event(&env).unwrap();
            txn.record_disposition(&env, AuditDisposition::Accepted, None, &[], None)
                .unwrap();
            // No commit → completion event AND audit row roll back together.
        }

        assert_eq!(event_count(&db, &inv), 0, "events rolled back");
        assert_eq!(audit_count(&db), 0, "audit row rolled back");
    }

    // ─────────────────────────────────────────────────────────────────────
    // F1.3.5 — authority_meta bump + graph_revisions + graph_changes
    //          (V-AUTH-01 revision invariants, V-AUTH-02 immutability)
    // ─────────────────────────────────────────────────────────────────────

    use crate::authority::revision::{GraphChange, GraphChangeKind};

    /// The current authority revision counter (`authority_meta.graph_revision`).
    fn revision_counter(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT graph_revision FROM authority_meta WHERE id = 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn graph_revisions_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM graph_revisions", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn graph_changes_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM graph_changes", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    /// A `graph_revisions` row read back for assertions.
    struct RevisionRow {
        base_revision: i64,
        tx_id: String,
        actor_id: String,
        policy_hash: String,
        change_count: i64,
    }

    fn revision_row(db: &Database, revision: i64) -> RevisionRow {
        db.with_read(|conn| {
            let row = conn
                .query_row(
                    "SELECT base_revision, tx_id, actor_id, policy_hash, change_count
                     FROM graph_revisions WHERE revision = ?1",
                    params![revision],
                    |r| {
                        Ok(RevisionRow {
                            base_revision: r.get(0)?,
                            tx_id: r.get(1)?,
                            actor_id: r.get(2)?,
                            policy_hash: r.get(3)?,
                            change_count: r.get(4)?,
                        })
                    },
                )
                .unwrap();
            Ok(row)
        })
        .unwrap()
    }

    /// The ordinals (in stored `ordinal` order) and their change kinds for a
    /// revision's `graph_changes` rows.
    fn graph_changes_for(db: &Database, revision: i64) -> Vec<(i64, String)> {
        db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT ordinal, change_kind FROM graph_changes
                     WHERE revision = ?1 ORDER BY ordinal",
                )
                .unwrap();
            let rows = stmt
                .query_map(params![revision], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .unwrap();
            let mut out = Vec::new();
            for row in rows {
                out.push(row.unwrap());
            }
            Ok(out)
        })
        .unwrap()
    }

    /// A test store returning a graph-visible outcome with an explicit ordered
    /// list of `n` change descriptors (to exercise stable ordinals / change
    /// count). Writes no semantic rows — it only supplies descriptors.
    struct MultiChangeStore {
        n: usize,
    }

    impl TxSemanticStore for MultiChangeStore {
        fn apply(
            &self,
            _tx: &mut AuthorityTx<'_>,
            env: &CommandEnvelope,
        ) -> MemoryResult<SemanticOutcome> {
            let kinds = [
                GraphChangeKind::Insert,
                GraphChangeKind::Update,
                GraphChangeKind::State,
                GraphChangeKind::Delete,
                GraphChangeKind::Invalidate,
            ];
            let changes = (0..self.n)
                .map(|i| {
                    GraphChange::new(kinds[i % kinds.len()], env.caller().partition_key())
                        .with_payload(format!("{{\"i\":{i}}}"))
                })
                .collect();
            Ok(SemanticOutcome::graph_visible(changes))
        }
    }

    /// A test store returning a revision-neutral outcome (no changes).
    struct NeutralStore;

    impl TxSemanticStore for NeutralStore {
        fn apply(
            &self,
            _tx: &mut AuthorityTx<'_>,
            _env: &CommandEnvelope,
        ) -> MemoryResult<SemanticOutcome> {
            Ok(SemanticOutcome::revision_neutral())
        }
    }

    /// Drive one full accepted command through the combined 1.3.4+1.3.5 path.
    fn commit_visible<S: TxSemanticStore>(
        db: &Database,
        env: &CommandEnvelope,
        store: &S,
    ) -> CommandRecord {
        let mut txn = AuthorityTransaction::begin(db).unwrap();
        txn.append_start_event(env).unwrap();
        let outcome = txn.apply_semantic_mutation(store, env).unwrap();
        let record = txn
            .record_disposition_with_revision(env, AuditDisposition::Accepted, &outcome, &[], None)
            .unwrap();
        txn.commit().unwrap();
        record
    }

    // ── V-AUTH-01: one visible commit advances exactly one revision ───────
    #[test]
    fn one_visible_commit_advances_exactly_one_revision() {
        let db = fresh_db();
        assert_eq!(revision_counter(&db), 0, "fresh authority is at revision 0");

        let env = observe_env(SourceKind::Native);
        let record = commit_visible(&db, &env, &DeferredSemanticStore);

        // Counter advanced 0 → 1, exactly one revision row.
        assert_eq!(revision_counter(&db), 1, "exactly one increment");
        assert_eq!(graph_revisions_count(&db), 1, "one graph_revisions row");
        assert_eq!(record.revision, Some(GraphRevision::new(1)));

        // The revision row is contiguous (base = revision - 1) and self-consistent.
        let row = revision_row(&db, 1);
        assert_eq!(row.base_revision, 0, "base_revision = revision - 1");
        assert_eq!(
            row.tx_id,
            record.event.event_id.as_str(),
            "tx_id is the completion event id (unique per command)"
        );
        assert_eq!(row.actor_id, "local-desktop");
        assert_eq!(row.policy_hash, super::super::revision::PENDING_POLICY_HASH);

        // change_count matches the number of appended graph_changes rows.
        let changes = graph_changes_for(&db, 1);
        assert_eq!(
            row.change_count as usize,
            changes.len(),
            "change_count equals appended graph_changes rows"
        );
        assert_eq!(graph_changes_count(&db), changes.len() as i64);

        // The audit row carries the committed revision.
        let a = audit_row(&db, &record.audit.audit_id.as_str());
        assert_eq!(a.authority_revision, Some(1), "audit carries the revision");
    }

    // ── V-AUTH-01: N sequential visible commits produce contiguous revisions
    #[test]
    fn sequential_visible_commits_produce_contiguous_revisions() {
        let db = fresh_db();
        let n = 5u64;
        for expected in 1..=n {
            let env = observe_env(SourceKind::Native);
            let record = commit_visible(&db, &env, &DeferredSemanticStore);
            assert_eq!(record.revision, Some(GraphRevision::new(expected)));
            assert_eq!(revision_counter(&db), expected as i64);
            // Each revision row is contiguous with its predecessor.
            let row = revision_row(&db, expected as i64);
            assert_eq!(
                row.base_revision,
                (expected - 1) as i64,
                "revision {expected} chains to {}",
                expected - 1
            );
        }
        assert_eq!(graph_revisions_count(&db), n as i64, "one row per commit");
    }

    // ── V-AUTH-01: revision-neutral command changes no revision state ─────
    #[test]
    fn revision_neutral_command_is_revision_neutral() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        txn.append_start_event(&env).unwrap();
        let outcome = txn.apply_semantic_mutation(&NeutralStore, &env).unwrap();
        assert!(!outcome.graph_visible, "store reports revision-neutral");
        let record = txn
            .record_disposition_with_revision(&env, AuditDisposition::Accepted, &outcome, &[], None)
            .unwrap();
        txn.commit().unwrap();

        // No counter bump, no revision/change rows, audit revision is None.
        assert_eq!(revision_counter(&db), 0, "counter unchanged");
        assert_eq!(graph_revisions_count(&db), 0, "no graph_revisions row");
        assert_eq!(graph_changes_count(&db), 0, "no graph_changes row");
        assert_eq!(record.revision, None);
        let a = audit_row(&db, &record.audit.audit_id.as_str());
        assert_eq!(a.authority_revision, None, "audit revision is NULL");
    }

    // ── V-AUTH-01: graph_changes ordinals are 0..n-1 in stable order ──────
    #[test]
    fn graph_changes_ordinals_are_stable_and_contiguous() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let store = MultiChangeStore { n: 4 };
        let record = commit_visible(&db, &env, &store);

        let revision = record.revision.unwrap().get() as i64;
        let changes = graph_changes_for(&db, revision);
        assert_eq!(changes.len(), 4, "all four changes appended");

        // Ordinals are exactly 0,1,2,3 in stored order (stable, contiguous).
        let ordinals: Vec<i64> = changes.iter().map(|(o, _)| *o).collect();
        assert_eq!(ordinals, vec![0, 1, 2, 3]);

        // In the order the store supplied them (insert, update, state, delete).
        let kinds: Vec<&str> = changes.iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(kinds, vec!["insert", "update", "state", "delete"]);

        // change_count matches.
        assert_eq!(revision_row(&db, revision).change_count, 4);
    }

    // ── V-AUTH-01: rollback reverts the revision bump and ledger rows ─────
    #[test]
    fn rollback_reverts_revision_bump_and_ledger_rows() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        {
            let mut txn = AuthorityTransaction::begin(&db).unwrap();
            txn.append_start_event(&env).unwrap();
            let outcome = txn
                .apply_semantic_mutation(&MultiChangeStore { n: 3 }, &env)
                .unwrap();
            txn.record_disposition_with_revision(
                &env,
                AuditDisposition::Accepted,
                &outcome,
                &[],
                None,
            )
            .unwrap();
            // Drop WITHOUT commit → the revision bump + ledger rows roll back.
        }

        assert_eq!(revision_counter(&db), 0, "counter restored to 0");
        assert_eq!(graph_revisions_count(&db), 0, "no graph_revisions row");
        assert_eq!(graph_changes_count(&db), 0, "no graph_changes row");
    }

    // ── V-AUTH-02: committed graph_revisions / graph_changes are immutable ─
    #[test]
    fn committed_revision_and_changes_are_immutable() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let record = commit_visible(&db, &env, &MultiChangeStore { n: 2 });
        let revision = record.revision.unwrap().get() as i64;

        let conn = db.write();

        // graph_revisions: UPDATE and DELETE both rejected (append-only, L1).
        assert!(
            conn.execute(
                "UPDATE graph_revisions SET change_count = 99 WHERE revision = ?1",
                params![revision],
            )
            .is_err(),
            "UPDATE on graph_revisions is rejected (L1)"
        );
        assert!(
            conn.execute(
                "DELETE FROM graph_revisions WHERE revision = ?1",
                params![revision],
            )
            .is_err(),
            "DELETE on graph_revisions is rejected (L1)"
        );

        // graph_changes: UPDATE and DELETE both rejected (append-only, L1).
        assert!(
            conn.execute(
                "UPDATE graph_changes SET ordinal = 42 WHERE revision = ?1 AND ordinal = 0",
                params![revision],
            )
            .is_err(),
            "UPDATE on graph_changes is rejected (L1)"
        );
        assert!(
            conn.execute(
                "DELETE FROM graph_changes WHERE revision = ?1",
                params![revision],
            )
            .is_err(),
            "DELETE on graph_changes is rejected (L1)"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // F1.3.6 — idempotent outbox enqueue + canonical idempotency result
    // ─────────────────────────────────────────────────────────────────────

    use crate::authority::validation::{
        validate_command, ValidationOutcome, ValidationReads,
    };
    use crate::authority::SqliteValidationReads;
    use crate::model::RecordId;

    /// A test store returning a graph-visible outcome with a single
    /// **record-scoped** insert change, so it drives real projection work
    /// through the outbox (unlike the deferred placeholder, which is
    /// record-less and enqueues nothing).
    struct RecordInsertStore {
        record_id: RecordId,
    }

    impl TxSemanticStore for RecordInsertStore {
        fn apply(
            &self,
            _tx: &mut AuthorityTx<'_>,
            env: &CommandEnvelope,
        ) -> MemoryResult<SemanticOutcome> {
            let change = GraphChange::new(GraphChangeKind::Insert, env.caller().partition_key())
                .with_record("memory", self.record_id.clone())
                .with_hashes(None, Some("after-hash".to_string()));
            Ok(SemanticOutcome::graph_visible(vec![change]))
        }
    }

    fn outbox_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM derived_outbox", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    fn idempotency_count(db: &Database) -> i64 {
        db.with_read(|conn| {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM idempotency_results", [], |r| r.get(0))
                .unwrap();
            Ok(n)
        })
        .unwrap()
    }

    // ── Full accepted-command flow commits every row atomically ───────────
    #[test]
    fn full_accepted_command_flow_commits_all_rows() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let inv = env.source().invocation_id().as_str().to_string();
        let rid = RecordId::new_v7();

        let txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .commit_accepted_command(&env, &RecordInsertStore { record_id: rid }, None)
            .unwrap();

        // Events: start + completion for the invocation.
        assert_eq!(event_count(&db, &inv), 2, "start + completion events");
        // One audit row, one revision, its change set.
        assert_eq!(audit_count(&db), 1, "one audit row");
        assert_eq!(graph_revisions_count(&db), 1, "one revision");
        assert_eq!(graph_changes_count(&db), 1, "one graph change");
        assert_eq!(record.revision, Some(GraphRevision::new(1)));
        // Outbox: one item per projection target for the single record-scoped change.
        assert_eq!(
            outbox_count(&db),
            super::super::outbox::PROJECTION_TARGETS.len() as i64,
            "one outbox item per projection target"
        );
        // Exactly one idempotency result stored.
        assert_eq!(idempotency_count(&db), 1, "one idempotency result");
    }

    // ── The stored idempotency result carries the correct fields ──────────
    #[test]
    fn stored_idempotency_result_has_correct_fields() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        let txn = AuthorityTransaction::begin(&db).unwrap();
        let record = txn
            .commit_accepted_command(&env, &DeferredSemanticStore, None)
            .unwrap();

        let reads = SqliteValidationReads::new(db.clone());
        let stored = reads
            .lookup_idempotency(&env.caller().partition_key(), env.idempotency_key())
            .unwrap()
            .expect("idempotency result is readable after commit");

        assert_eq!(
            stored.command_hash,
            env.command_hash().as_str(),
            "stored command_hash matches the envelope"
        );
        assert_eq!(
            stored.committed_revision, record.revision,
            "stored committed_revision matches the reserved revision"
        );
        assert_eq!(
            stored.event_id.as_ref(),
            Some(&record.event.event_id),
            "stored event_id is the completion event id"
        );
        // result_json carries the canonical status/event/revision.
        let parsed: serde_json::Value = serde_json::from_str(&stored.result_json).unwrap();
        assert_eq!(parsed["status"], "committed");
        assert_eq!(parsed["event_id"], record.event.event_id.as_str());
        assert_eq!(parsed["revision"], record.revision.unwrap().get() as i64);
    }

    // ── End-to-end replay: same command twice → second validates as Replay ─
    #[test]
    fn same_command_twice_replays_via_validation_reads() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);

        // First submission: the full accepted flow commits + stores the result.
        let txn = AuthorityTransaction::begin(&db).unwrap();
        txn.commit_accepted_command(&env, &DeferredSemanticStore, None)
            .unwrap();

        // Re-validating the SAME command now returns Replay of the stored result
        // (proving the F1.3.6 write is what F1.3.2 reads back).
        let reads = SqliteValidationReads::new(db.clone());
        let outcome = validate_command(&env, &reads).unwrap();
        match outcome {
            ValidationOutcome::Replay(stored) => {
                assert_eq!(stored.command_hash, env.command_hash().as_str());
            }
            other => panic!("expected Replay, got {other:?}"),
        }
    }

    // ── Rollback reverts BOTH the outbox and idempotency rows ─────────────
    #[test]
    fn rollback_reverts_outbox_and_idempotency_rows() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let rid = RecordId::new_v7();

        {
            let mut txn = AuthorityTransaction::begin(&db).unwrap();
            txn.append_start_event(&env).unwrap();
            let outcome = txn
                .apply_semantic_mutation(&RecordInsertStore { record_id: rid }, &env)
                .unwrap();
            let record = txn
                .record_disposition_with_revision(
                    &env,
                    AuditDisposition::Accepted,
                    &outcome,
                    &[],
                    None,
                )
                .unwrap();
            txn.enqueue_projection_work(&outcome, record.revision)
                .unwrap();
            txn.persist_idempotency_result(
                &env,
                super::super::idempotency::COMMITTED_STATUS,
                Some(&record.event.event_id),
                record.revision,
            )
            .unwrap();
            // Drop WITHOUT commit → outbox + idempotency rows roll back together.
        }

        assert_eq!(outbox_count(&db), 0, "outbox rows rolled back");
        assert_eq!(idempotency_count(&db), 0, "idempotency row rolled back");
        // And the read surface sees no stored result.
        let reads = SqliteValidationReads::new(db.clone());
        assert!(
            reads
                .lookup_idempotency(&env.caller().partition_key(), env.idempotency_key())
                .unwrap()
                .is_none(),
            "no stored idempotency result after rollback"
        );
    }

    // ── Idempotent enqueue inside the command tx: same key twice → one row ─
    #[test]
    fn projection_work_enqueue_is_idempotent_within_tx() {
        let db = fresh_db();
        let env = observe_env(SourceKind::Native);
        let rid = RecordId::new_v7();

        let mut txn = AuthorityTransaction::begin(&db).unwrap();
        let outcome = txn
            .apply_semantic_mutation(&RecordInsertStore { record_id: rid }, &env)
            .unwrap();
        // Enqueue the SAME derived work twice within the transaction.
        txn.enqueue_projection_work(&outcome, Some(GraphRevision::new(1)))
            .unwrap();
        txn.enqueue_projection_work(&outcome, Some(GraphRevision::new(1)))
            .unwrap();
        txn.commit().unwrap();

        // Still exactly one row per target — no duplicate semantic work.
        assert_eq!(
            outbox_count(&db),
            super::super::outbox::PROJECTION_TARGETS.len() as i64,
            "re-enqueuing identical semantic work creates no duplicates"
        );
    }
}
