//! The governed command bus — the single concrete submission seam every durable
//! writer routes through (task **F1.5.1**, design §5.1 command state machine,
//! §19.1 `AuthorityCommandBus`).
//!
//! [`AuthorityCommandBus`] is the concrete realization of the design's
//! *"AuthorityCommandBus"*: the one object a writer submits a validated
//! [`CommandEnvelope`] to. It wires the two halves the earlier gates built into
//! a single call so no writer has to orchestrate them itself:
//!
//! 1. **Validate before BEGIN** (F1.3.2) — runs the deterministic
//!    [`CommandValidator`] over a side-effect-free [`SqliteValidationReads`]
//!    surface, producing a typed [`ValidationOutcome`].
//! 2. **Governed transaction** (F1.3.3–F1.3.7) — on
//!    [`ValidationOutcome::Proceed`] it opens one serialized
//!    [`AuthorityTransaction`] and commits the accepted command atomically via
//!    [`AuthorityTransaction::commit_and_publish`], emitting the post-commit
//!    revision wake through the injected [`WakePublisher`].
//!
//! A [`ValidationOutcome::Replay`] returns the stored idempotent result without
//! re-executing (MGR-005 AC3); a [`ValidationOutcome::Rejected`] records the
//! rejection audit row via [`record_rejected_command`] and returns the reasons.
//!
//! ## Scope of this seam (F1.5.1 scaffolding)
//!
//! This is the **routing boundary** writers submit to. The *semantic* rows a
//! command mutates are produced by the [`TxSemanticStore`] the caller passes in.
//! For the core writers whose concrete cognitive-record builders are **F2**, the
//! injected store is [`DeferredSemanticStore`] (see
//! [`AuthorityCommandBus::submit_deferred`]): the command is fully governed —
//! validated, evented, audited, revisioned, idempotent, and outbox-enqueued —
//! but the per-kind semantic persistence lands in F2. This lets adapters and
//! orchestration depend on one governed seam now, before the F2 builders exist,
//! **without** opening a second live write path.
//!
//! The bus intentionally operates on an already-constructed [`CommandEnvelope`]
//! (the trusted, validated unit — a caller cannot rehydrate one from untrusted
//! input) rather than the looser [`AuthorityCommand`](super::AuthorityCommand)
//! DTO, so the caller context and provenance can never be forged at this
//! boundary.

use std::sync::Arc;

use crate::memory::db::Database;
use crate::memory::error::MemoryResult;
use crate::memory::model::AuditId;

use super::command::CommandEnvelope;
use super::publish::{NoopWakePublisher, WakePublisher};
use super::transaction::{record_rejected_command, AuthorityTransaction, DeferredSemanticStore};
use super::validation::{CommandValidator, RejectionReason, ValidationConfig, ValidationOutcome};
use super::{CommandOutcome, CommandStatus, SqliteValidationReads, TxSemanticStore};

/// The outcome of submitting a command to the [`AuthorityCommandBus`]: the
/// canonical [`CommandOutcome`] plus — when the command was rejected — the
/// deterministic [`RejectionReason`] codes the pre-transaction validator
/// produced (also recorded verbatim in `audit_records.reason_codes_json`).
///
/// Adapters (F1.5.2/F1.5.3) map `rejection` onto their transport error surface;
/// an accepted/replayed command carries `rejection = None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernedOutcome {
    /// The canonical command outcome (status + event id + revision).
    pub outcome: CommandOutcome,
    /// The rejection reasons, present iff `outcome.status == Rejected`.
    pub rejection: Option<Vec<RejectionReason>>,
}

impl GovernedOutcome {
    /// The terminal status of the submitted command.
    pub fn status(&self) -> CommandStatus {
        self.outcome.status
    }

    /// Whether the command committed a new durable change.
    pub fn is_committed(&self) -> bool {
        self.outcome.status == CommandStatus::Committed
    }

    /// Whether the command replayed a prior committed result.
    pub fn is_replayed(&self) -> bool {
        self.outcome.status == CommandStatus::Replayed
    }

    /// Whether the command was rejected by the deterministic validator.
    pub fn is_rejected(&self) -> bool {
        self.outcome.status == CommandStatus::Rejected
    }
}

/// The governed command bus: `validate → (proceed | replay | reject)` over the
/// single authority [`Database`] and a [`WakePublisher`].
///
/// Construct one at the composition root with the memory system's
/// [`WakePublisher`](super::WakePublisher) so post-commit wakes reuse the single
/// memory-change broadcast channel (see
/// [`MemorySystem::command_bus`](crate::memory::api::MemorySystem::command_bus)).
/// It is cheap to clone-share (holds an `Arc<Database>` + a small config + the
/// publisher).
///
/// The publisher type parameter defaults to [`NoopWakePublisher`] for standalone
/// / test construction where no live broadcast channel exists.
pub struct AuthorityCommandBus<P: WakePublisher = NoopWakePublisher> {
    db: Arc<Database>,
    config: ValidationConfig,
    publisher: P,
}

impl AuthorityCommandBus<NoopWakePublisher> {
    /// Build a bus over `db` that discards post-commit wakes
    /// ([`NoopWakePublisher`]). Suitable for standalone / test use; the live
    /// system uses [`AuthorityCommandBus::with_publisher`].
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            config: ValidationConfig::default(),
            publisher: NoopWakePublisher,
        }
    }
}

impl<P: WakePublisher> AuthorityCommandBus<P> {
    /// Build a bus over `db` that publishes post-commit revision wakes through
    /// `publisher` (the live system passes its
    /// [`RevisionWakeBroadcaster`](crate::memory::api::RevisionWakeBroadcaster)).
    pub fn with_publisher(db: Arc<Database>, publisher: P) -> Self {
        Self {
            db,
            config: ValidationConfig::default(),
            publisher,
        }
    }

    /// Override the bounded [`ValidationConfig`] (payload/deadline/schema
    /// ceilings). Defaults are applied otherwise.
    pub fn with_config(mut self, config: ValidationConfig) -> Self {
        self.config = config;
        self
    }

    /// Submit a governed command whose semantic mutation is applied by `store`.
    ///
    /// Runs the full governed path: validate before BEGIN, then — on
    /// [`ValidationOutcome::Proceed`] — one serialized [`AuthorityTransaction`]
    /// that commits the accepted command (start/completion events, the semantic
    /// mutation via `store`, one reserved revision for a graph-visible change,
    /// the audit row, the idempotency result, and the derived-projection outbox
    /// work) atomically, then publishes the post-commit revision wake.
    ///
    /// `reversal_of` links the audit row to the row a compensating command
    /// undoes (`None` for an original command).
    ///
    /// A policy/validation rejection is a **normal** outcome, not an `Err`:
    /// [`GovernedOutcome::rejection`] carries the reasons and the audit trail is
    /// recorded. `Err` is reserved for genuine storage/consistency failures.
    pub fn submit<S: TxSemanticStore>(
        &self,
        env: &CommandEnvelope,
        store: &S,
        reversal_of: Option<&AuditId>,
    ) -> MemoryResult<GovernedOutcome> {
        let reads = SqliteValidationReads::new(self.db.clone());
        let outcome = CommandValidator::new(self.config, &reads).validate(env)?;

        match outcome {
            ValidationOutcome::Proceed => {
                let tx = AuthorityTransaction::begin(&self.db)?;
                let record = tx.commit_and_publish(env, store, reversal_of, &self.publisher)?;
                Ok(GovernedOutcome {
                    outcome: CommandOutcome {
                        status: CommandStatus::Committed,
                        event_id: Some(record.event.event_id.clone()),
                        revision: record.revision.unwrap_or_else(|| env.base_revision()),
                    },
                    rejection: None,
                })
            }
            ValidationOutcome::Replay(stored) => Ok(GovernedOutcome {
                outcome: CommandOutcome {
                    status: CommandStatus::Replayed,
                    event_id: stored.event_id,
                    revision: stored
                        .committed_revision
                        .unwrap_or_else(|| env.base_revision()),
                },
                rejection: None,
            }),
            ValidationOutcome::Rejected(reasons) => {
                record_rejected_command(&self.db, env, &reasons)?;
                Ok(GovernedOutcome {
                    outcome: CommandOutcome {
                        status: CommandStatus::Rejected,
                        event_id: None,
                        revision: env.base_revision(),
                    },
                    rejection: Some(reasons),
                })
            }
        }
    }

    /// Submit a governed command whose concrete per-kind semantic builder is
    /// **deferred to F2** ([`DeferredSemanticStore`]).
    ///
    /// The command is fully governed (validated, evented, audited, revisioned,
    /// idempotent, outbox-enqueued) but persists no concrete cognitive row yet —
    /// the F2 per-kind builders replace [`DeferredSemanticStore`] by
    /// implementing [`TxSemanticStore`] over the same transaction, at which point
    /// callers switch to [`submit`](Self::submit) with the real store and this
    /// convenience is retired. This is the seam the core writers
    /// (native/conversation/library/feedback/goal/cognition) route through until
    /// F2 lands, so there is never a second live write path.
    pub fn submit_deferred(&self, env: &CommandEnvelope) -> MemoryResult<GovernedOutcome> {
        self.submit(env, &DeferredSemanticStore, None)
    }

    /// The authority handle the bus governs (for building read surfaces / the
    /// current base revision a writer should issue against).
    pub fn database(&self) -> &Arc<Database> {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::authority::candidates::{CommandCandidate, WriteContext};
    use crate::memory::authority::command::Deadline;
    use crate::memory::model::{
        CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
    };
    use crate::memory::types::MemoryMode;

    fn fresh_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory authority"))
    }

    fn local_caller() -> CallerContext {
        let partition = PolicyPartition::new("user", "chat", 0).unwrap();
        CallerContext::local_desktop("local-desktop", partition).unwrap()
    }

    fn write_ctx(key: &str) -> WriteContext {
        WriteContext {
            caller: local_caller(),
            idempotency_key: IdempotencyKey::new(key).unwrap(),
            base_revision: GraphRevision::base(),
            invocation_id: InvocationId::new_v7(),
            source_id: "core:cognition".to_string(),
            mode: MemoryMode::Permanent,
            deadline: Deadline::default_write(),
        }
    }

    fn fact_env(key: &str) -> CommandEnvelope {
        CommandCandidate::native_fact("the user prefers dark mode", Some("preference"))
            .into_envelope(write_ctx(key), None)
            .unwrap()
    }

    // ── An accepted observe command commits through the governed path. ──
    #[test]
    fn observe_commits_and_advances_revision() {
        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());
        let env = fact_env("cmd-1");

        let governed = bus.submit_deferred(&env).unwrap();
        assert!(governed.is_committed());
        assert!(governed.rejection.is_none());
        // A graph-visible observe reserves exactly one revision.
        assert_eq!(governed.outcome.revision, GraphRevision::new(1));
        assert!(governed.outcome.event_id.is_some());
    }

    // ── Same key + same intent replays without re-executing. ──
    #[test]
    fn duplicate_submission_replays() {
        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());

        let first = bus.submit_deferred(&fact_env("cmd-dup")).unwrap();
        assert!(first.is_committed());

        // A second envelope with the same caller/key/semantic content replays.
        let second = bus.submit_deferred(&fact_env("cmd-dup")).unwrap();
        assert!(second.is_replayed(), "same key + hash must replay");
        // Replay returns the originally committed revision, not a new one.
        assert_eq!(second.outcome.revision, first.outcome.revision);
    }

    // ── A mode-forbidden write is rejected (normal outcome + audit). ──
    #[test]
    fn read_only_mode_write_is_rejected() {
        let db = fresh_db();
        let bus = AuthorityCommandBus::new(db.clone());
        let mut ctx = write_ctx("cmd-ro");
        ctx.mode = MemoryMode::ReadOnly;
        let env = CommandCandidate::native_fact("blocked", None)
            .into_envelope(ctx, None)
            .unwrap();

        let governed = bus.submit_deferred(&env).unwrap();
        assert!(governed.is_rejected());
        let reasons = governed.rejection.expect("rejection carries reasons");
        assert!(!reasons.is_empty(), "a rejection is never empty");
        // Nothing committed: the authority stays at the base revision.
        assert_eq!(governed.outcome.revision, GraphRevision::base());
    }
}
