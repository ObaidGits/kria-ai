//! F1.5.1 — proof that every writer-category [`CommandCandidate`] constructor
//! (core/native, conversation, library, feedback, goal, and cognition:
//! reasoning/planning/causal/knowledge-gap) flows through the single governed
//! [`AuthorityCommandBus`] rather than bypassing it: each submission commits
//! atomically with an idempotency-dedup ledger entry, an immutable audit row,
//! and exactly one reserved graph revision when accepted (design §5.1, MGR-033,
//! MGR-035, MGR-043–044).
//!
//! This does not assert concrete per-kind semantic persistence (the F2 builders
//! that replace [`DeferredSemanticStore`] land in F2.1+); it proves the F1.5.1
//! scaffolding invariant: **every** writer-category candidate is a governed,
//! auditable, idempotent, revision-bound command today, with no second write
//! path.

use std::sync::Arc;

use kria_core::memory::authority::command::{Deadline, SourceKind, SourceTrust};
use kria_core::memory::authority::{AuthorityCommandBus, CommandCandidate, WriteContext};
use kria_core::memory::db::Database;
use kria_core::memory::model::{
    CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
};
use kria_core::memory::types::MemoryMode;

fn fresh_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().expect("open in-memory authority"))
}

fn write_ctx(key: &str) -> WriteContext {
    let partition = PolicyPartition::new("core", "internal", 0).unwrap();
    let caller = CallerContext::local_desktop("kria-core", partition).unwrap();
    WriteContext {
        caller,
        idempotency_key: IdempotencyKey::new(key).unwrap(),
        base_revision: GraphRevision::base(),
        invocation_id: InvocationId::new_v7(),
        source_id: "core:test".to_string(),
        mode: MemoryMode::Permanent,
        deadline: Deadline::default_write(),
    }
}

fn audit_row_count(db: &Arc<Database>) -> i64 {
    db.with_read(|c| {
        Ok(
            c.query_row("SELECT COUNT(*) FROM audit_records", [], |r| r.get(0))
                .map_err(kria_core::memory::error::StorageError::Sqlite)?,
        )
    })
    .unwrap()
}

fn idempotency_row_count(db: &Arc<Database>) -> i64 {
    db.with_read(|c| {
        Ok(
            c.query_row("SELECT COUNT(*) FROM idempotency_results", [], |r| r.get(0))
                .map_err(kria_core::memory::error::StorageError::Sqlite)?,
        )
    })
    .unwrap()
}

/// Submit `candidate` through a fresh bus and assert the governed invariants:
/// committed, exactly one reserved revision, an audit row, and an idempotency
/// ledger entry.
fn assert_governed_commit(candidate: CommandCandidate, key: &str) {
    let db = fresh_db();
    let bus = AuthorityCommandBus::new(db.clone());
    let env = candidate.into_envelope(write_ctx(key), None).unwrap();

    let governed = bus.submit_deferred(&env).unwrap();
    assert!(
        governed.is_committed(),
        "candidate must commit through the governed bus"
    );
    assert!(governed.rejection.is_none());
    assert_eq!(
        governed.outcome.revision,
        GraphRevision::new(1),
        "an accepted observation reserves exactly one revision"
    );
    assert!(governed.outcome.event_id.is_some());
    assert_eq!(
        audit_row_count(&db),
        1,
        "the commit must append exactly one audit row"
    );
    assert_eq!(
        idempotency_row_count(&db),
        1,
        "the commit must record exactly one idempotency ledger entry"
    );
}

// ── core / native ──────────────────────────────────────────────────────────

#[test]
fn native_fact_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::native_fact("the user prefers dark mode", Some("preference")),
        "cov-native-fact",
    );
}

#[test]
fn tool_outcome_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::tool_outcome(SourceKind::Mcp, SourceTrust::Trusted, "search completed"),
        "cov-tool-outcome",
    );
}

/// F1.5.4 — every invocation `SourceKind` a tool/MCP/OpenClaw/sidecar outcome
/// can carry (design §7.4, MGR-043 AC1) commits through the same governed bus
/// with no source-specific bypass. `tool_outcome_candidate_is_governed` above
/// already covers `Mcp`; this fills in the remaining invocation kinds
/// ([`SourceKind::is_invocation`]) so every kind `classify_tool_outcome_source`
/// (`kria_core::agent::loop_engine`, task F1.5.4) can produce is proven
/// governed here, not just the one this file originally covered.
#[test]
fn tool_outcome_candidate_is_governed_for_every_invocation_source_kind() {
    assert_governed_commit(
        CommandCandidate::tool_outcome(SourceKind::Native, SourceTrust::System, "read completed"),
        "cov-tool-outcome-native",
    );
    assert_governed_commit(
        CommandCandidate::tool_outcome(
            SourceKind::OpenClaw,
            SourceTrust::Limited,
            "parsed 3 pages",
        ),
        "cov-tool-outcome-openclaw",
    );
    assert_governed_commit(
        CommandCandidate::tool_outcome(
            SourceKind::Sidecar,
            SourceTrust::Limited,
            "embeddings generated",
        ),
        "cov-tool-outcome-sidecar",
    );
}

// ── conversation ───────────────────────────────────────────────────────────

#[test]
fn conversation_turn_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::conversation_turn("user", "hello there"),
        "cov-conversation-turn",
    );
}

// ── library ────────────────────────────────────────────────────────────────

#[test]
fn library_chunk_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::library_chunk("item-1", 0, "chunk body text"),
        "cov-library-chunk",
    );
}

// ── feedback ───────────────────────────────────────────────────────────────

#[test]
fn feedback_signal_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::feedback_signal("rec-1", "memory", "thumbs_up"),
        "cov-feedback-signal",
    );
}

// ── goals ──────────────────────────────────────────────────────────────────

#[test]
fn goal_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::goal("ship the memory upgrade", "user"),
        "cov-goal",
    );
}

// ── cognition: reasoning / planning / causal / knowledge-gap ───────────────

#[test]
fn reasoning_trace_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::reasoning_trace("solve x", "tried a then b, worked"),
        "cov-reasoning-trace",
    );
}

#[test]
fn plan_outcome_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::plan_outcome("sig-1", true),
        "cov-plan-outcome",
    );
}

#[test]
fn causal_link_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::causal_link("missing dependency", "build fails", true),
        "cov-causal-link",
    );
}

#[test]
fn knowledge_gap_candidate_is_governed() {
    assert_governed_commit(
        CommandCandidate::knowledge_gap("how does X work", Some("domain")),
        "cov-knowledge-gap",
    );
}

// ── cross-cutting: idempotent replay is category-agnostic ──────────────────

#[test]
fn duplicate_submission_replays_for_every_category() {
    // One representative non-native category is enough to prove the bus-level
    // replay behavior is not special-cased per writer category (the bus never
    // inspects the candidate's `candidate` discriminator).
    let db = fresh_db();
    let bus = AuthorityCommandBus::new(db.clone());
    let env = CommandCandidate::goal("dedup goal", "user")
        .into_envelope(write_ctx("cov-goal-dup"), None)
        .unwrap();

    let first = bus.submit_deferred(&env).unwrap();
    assert!(first.is_committed());

    let second = bus.submit_deferred(&env).unwrap();
    assert!(second.is_replayed(), "same key + hash must replay");
    assert_eq!(second.outcome.revision, first.outcome.revision);
    // Replay must not append a second audit row or idempotency entry.
    assert_eq!(audit_row_count(&db), 1);
    assert_eq!(idempotency_row_count(&db), 1);
}
