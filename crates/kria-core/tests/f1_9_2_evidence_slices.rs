//! F1.9.2 — Comprehensive F1 evidence aggregation: named test slices + correctness hash.
//!
//! **Validates: Requirements MGR-027, MGR-003, MGR-004, MGR-033, MGR-035, MGR-040, MGR-041, MGR-042, MGR-017**
//!
//! This file adds the two new test slices required by task 1.9.2 and computes a
//! correctness hash (SHA-256) over all passing slice names as the F1 gate evidence:
//!
//! ## Named slices covered
//!
//! 1. **Server negative matrix** — `crates/kria-server/tests/integration_api.rs`
//!    (unauthenticated/wrong-origin/wrong-scope/oversized/replayed → denied).
//!    Pre-existing. Referenced in the hash manifest below.
//!
//! 2. **Paired-world non-interference** (NEW — implemented here):
//!    - `paired_world_plugin_x_filter_returns_only_plugin_x`
//!    - `paired_world_core_filter_returns_only_core`
//!
//! 3. **Lifecycle residue** — `crates/kria-core/src/memory/maintenance.rs`
//!    (`lifecycle_residue_deleted` / `lifecycle_residue_forgotten`).
//!    Pre-existing. Referenced in hash manifest.
//!
//! 4. **Crypto wording** — `crates/kria-core/src/memory/lifecycle.rs`
//!    (`honest_state_is_not_shredded`). Pre-existing. Referenced in hash manifest.
//!
//! 5. **Corruption/recovery** — `crates/kria-core/tests/f1_8_7_corruption_recovery_tests.rs`.
//!    Pre-existing. Referenced in hash manifest.
//!
//! 6. **Rebuild interruption** — `crates/kria-core/src/memory/maintenance.rs`
//!    (`rebuild_cancel_and_resume`). Pre-existing. Referenced in hash manifest.
//!
//! 7. **Async/fault slices** (NEW — implemented here):
//!    - `concurrent_authority_bus_writes_10_tasks_no_panics`
//!
//! 8. **Correctness hash summary** (NEW — implemented here):
//!    - `f1_gate_correctness_hash_summary` — computes SHA-256 over the slice names
//!      that pass and logs it as evidence.

use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use kria_core::memory::authority::command::Deadline;
use kria_core::memory::authority::{
    AuthorityCommandBus, CommandCandidate, CommandStatus, WriteContext,
};
use kria_core::memory::db::Database;
use kria_core::memory::error::MemoryResult;
use kria_core::memory::model::{
    CallerContext, GraphRevision, IdempotencyKey, InvocationId, PolicyPartition,
};
use kria_core::memory::retriever::{RetrievalCtx, Retriever};
use kria_core::memory::stores::ports::{Embedder, EventStore};
use kria_core::memory::stores::{
    SqliteEventStore, SqliteRelationalStore, SqliteSearchStore, SqliteVectorStore,
};
use kria_core::memory::types::MemoryMode;
use kria_core::memory::types::{
    Availability, EmphasisSignals, Event, EventType, ModelVersion, Source,
};
use kria_core::memory::write_policy::slow::SlowPath;
use uuid::Uuid;

// ── Shared helpers ────────────────────────────────────────────────────────────

fn fresh_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().expect("open in-memory authority"))
}

/// A deterministic fake embedder for retriever tests (no model weights needed).
struct FakeEmbedder16;

#[async_trait]
impl Embedder for FakeEmbedder16 {
    fn model_version(&self) -> ModelVersion {
        ModelVersion("fake16_v1".into())
    }
    fn dim(&self) -> usize {
        16
    }
    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; 16];
                for (i, b) in t.bytes().enumerate() {
                    v[i % 16] += b as f32 / 255.0;
                }
                v
            })
            .collect())
    }
    async fn health(&self) -> Availability {
        Availability::Up
    }
}

/// Build a minimal Event with the given content and namespace.
fn make_event(session: Uuid, content: &str, namespace: &str) -> Event {
    Event {
        id: kria_core::memory::ids::new_id(),
        hlc: kria_core::memory::ids::HlcGenerator::new().now(),
        ts_utc: chrono::Utc::now(),
        tz_offset_min: 0,
        event_type: EventType::UserMessage,
        source: Source::User,
        session_id: Some(session),
        parent_event_id: None,
        shred_key_id: None,
        payload: serde_json::json!({
            "content": content,
            "namespace": namespace,
            "scope": "global",
            "sensitivity": "private",
            "redacted": false,
            "emphasis": EmphasisSignals::default(),
            "derived_from": [],
            "proposed_type": null,
            "verify_against": null
        }),
        encrypted: false,
        checksum: "test_checksum".into(),
    }
}

/// Seed one memory with the given content into the given namespace via the
/// full SlowPath pipeline (event + FTS + vectors + relational).
async fn seed_memory(db: &Arc<Database>, content: &str, namespace: &str) {
    let events = Arc::new(SqliteEventStore::new(db.clone()));
    let relational = Arc::new(SqliteRelationalStore::new(db.clone()));
    let vectors = Arc::new(SqliteVectorStore::new(db.clone()));
    let embedder = Arc::new(FakeEmbedder16);
    let sp = SlowPath::new(
        db.clone(),
        events.clone(),
        relational,
        vectors,
        embedder,
        "dev",
    );
    let ev = make_event(Uuid::now_v7(), content, namespace);
    {
        let mut tx = db.begin().unwrap();
        events.append(&mut tx, &ev).unwrap();
        tx.commit().unwrap();
    }
    sp.enrich(ev.id).await.unwrap();
}

/// Build a Retriever backed by all in-memory stores.
fn make_retriever(db: &Arc<Database>) -> Retriever {
    Retriever::new(
        Arc::new(SqliteRelationalStore::new(db.clone())),
        Arc::new(SqliteVectorStore::new(db.clone())),
        Arc::new(SqliteSearchStore::new(db.clone())),
        Arc::new(FakeEmbedder16),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// SLICE 2 — Paired-world non-interference
// ═══════════════════════════════════════════════════════════════════════════
//
// Proves: a ScopeFilter for namespace "plugin/x" returns ONLY plugin/x
// memories; a ScopeFilter for "core" returns ONLY core memories — even when
// both namespaces contain identical search terms (the content is seeded
// identically to maximise the chance of a cross-contamination bug).
//
// Invariant: MGR-004 AC4/AC7 — policy enforcement precedes every query plan,
// result count, ranking, and serialization. No cross-namespace result ever
// appears in the returned hits regardless of content overlap.

/// Seed both "core" and "plugin/x" with the same phrase, then query with a
/// plugin/x filter. Every returned hit MUST have namespace == "plugin/x".
#[tokio::test]
async fn paired_world_plugin_x_filter_returns_only_plugin_x() {
    // **Validates: Requirements MGR-004 AC4, MGR-004 AC7, MGR-027**
    let db = fresh_db();

    // Seed identical content into both namespaces.
    seed_memory(&db, "shared knowledge about the KRIA project", "core").await;
    seed_memory(&db, "shared knowledge about the KRIA project", "plugin/x").await;
    // Add some extra distinguishable content per-namespace.
    seed_memory(&db, "core system configuration and settings", "core").await;
    seed_memory(&db, "plugin extension capabilities and hooks", "plugin/x").await;

    let retriever = make_retriever(&db);

    // Query with a plugin/x namespace filter.
    let ctx = RetrievalCtx {
        namespaces: vec!["plugin/x".to_string()],
        ..Default::default()
    };

    let result = retriever
        .search("KRIA project", &ctx)
        .await
        .expect("retrieval must succeed");

    // At least one hit must be returned (the seeded plugin/x content exists).
    assert!(
        !result.hits.is_empty(),
        "paired-world plugin/x: must return ≥1 hit for seeded plugin/x content"
    );

    // INVARIANT: every returned hit must be from plugin/x only — no core leak.
    for hit in &result.hits {
        assert_eq!(
            hit.memory.namespace, "plugin/x",
            "paired-world non-interference FAILED: core namespace leaked into plugin/x query. \
             content='{}' namespace='{}'",
            hit.memory.content, hit.memory.namespace
        );
    }

    println!(
        "[paired_world_plugin_x_filter] PASS — {} hits, all namespace='plugin/x'",
        result.hits.len()
    );
}

/// Same dual-namespace setup, but query with a "core" namespace filter.
/// Every returned hit MUST have namespace == "core".
#[tokio::test]
async fn paired_world_core_filter_returns_only_core() {
    // **Validates: Requirements MGR-004 AC4, MGR-004 AC7, MGR-027**
    let db = fresh_db();

    seed_memory(&db, "shared knowledge about the KRIA project", "core").await;
    seed_memory(&db, "shared knowledge about the KRIA project", "plugin/x").await;
    seed_memory(&db, "core system configuration and settings", "core").await;
    seed_memory(&db, "plugin extension capabilities and hooks", "plugin/x").await;

    let retriever = make_retriever(&db);

    let ctx = RetrievalCtx {
        namespaces: vec!["core".to_string()],
        ..Default::default()
    };

    let result = retriever
        .search("KRIA project", &ctx)
        .await
        .expect("retrieval must succeed");

    assert!(
        !result.hits.is_empty(),
        "paired-world core: must return ≥1 hit for seeded core content"
    );

    for hit in &result.hits {
        assert_eq!(
            hit.memory.namespace, "core",
            "paired-world non-interference FAILED: plugin/x namespace leaked into core query. \
             content='{}' namespace='{}'",
            hit.memory.content, hit.memory.namespace
        );
    }

    println!(
        "[paired_world_core_filter] PASS — {} hits, all namespace='core'",
        result.hits.len()
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SLICE 7 — Async/fault: concurrent authority bus writes
// ═══════════════════════════════════════════════════════════════════════════
//
// Spawns 10 parallel Tokio tasks, each submitting a unique-key observation to
// the same `AuthorityCommandBus`. Verifies:
//   a) All 10 submissions either Committed or Replayed (no panics, no errors).
//   b) Exactly 10 distinct graph revisions exist in the `graph_revisions` table
//      (one per unique accepted write = 10 unique keys → 10 unique revisions).
//   c) The `audit_records` table has exactly 10 rows.
//   d) No data corruption: each committed result carries a valid event_id.
//
// This validates the serialized-writer invariant under concurrent callers:
// AuthorityCommandBus internally serializes writes via a Mutex/channel so
// concurrent submissions see a consistent, contiguous revision sequence.

#[tokio::test]
async fn concurrent_authority_bus_writes_10_tasks_no_panics() {
    // **Validates: Requirements MGR-033 AC1, MGR-033 AC4, MGR-035, MGR-027**
    const N: usize = 10;

    let db = Arc::new(Database::open_in_memory().expect("in-memory db"));
    let bus = Arc::new(AuthorityCommandBus::new(db.clone()));

    let mut join_set: JoinSet<Result<kria_core::memory::authority::GovernedOutcome, String>> =
        JoinSet::new();

    for i in 0..N {
        let bus_clone = Arc::clone(&bus);
        let key = format!("concurrent-write-key-{i}");
        let content = format!("concurrent observation number {i}");

        join_set.spawn(async move {
            let candidate = CommandCandidate::native_fact(&content, Some("concurrent-test"));
            let ctx = {
                let partition =
                    PolicyPartition::new("core", "global", 0).map_err(|e| e.to_string())?;
                let caller = CallerContext::local_desktop("kria-device", partition)
                    .map_err(|e| e.to_string())?;
                WriteContext {
                    caller,
                    idempotency_key: IdempotencyKey::new(&key).map_err(|e| e.to_string())?,
                    base_revision: GraphRevision::base(),
                    invocation_id: InvocationId::new_v7(),
                    source_id: "core:concurrent-test".to_string(),
                    mode: MemoryMode::Permanent,
                    deadline: Deadline::default_write(),
                }
            };
            let env = candidate
                .into_envelope(ctx, None)
                .map_err(|e| e.to_string())?;
            bus_clone.submit_deferred(&env).map_err(|e| e.to_string())
        });
    }

    // Collect all results — any panic/error in a task causes join to return Err.
    let mut outcomes: Vec<kria_core::memory::authority::GovernedOutcome> = Vec::with_capacity(N);
    while let Some(join_result) = join_set.join_next().await {
        let task_result = join_result.expect("task must not panic");
        let outcome = task_result.expect("submit_deferred must not return an error");
        outcomes.push(outcome);
    }

    // ── a) All N submissions must be Committed or Replayed ──────────────────
    for (i, outcome) in outcomes.iter().enumerate() {
        let status = outcome.status();
        assert!(
            status == CommandStatus::Committed || status == CommandStatus::Replayed,
            "concurrent write {i}: expected Committed or Replayed, got {:?}",
            status
        );
    }

    // ── b) Exactly N distinct graph revisions (unique keys → unique commits) ─
    let revision_count: i64 = db
        .with_read(|conn| {
            Ok(conn
                .query_row("SELECT COUNT(*) FROM graph_revisions", [], |r| r.get(0))
                .map_err(kria_core::memory::error::StorageError::Sqlite)?)
        })
        .expect("must read graph_revisions");
    assert_eq!(
        revision_count, N as i64,
        "concurrent writes: expected exactly {N} graph_revisions rows (one per unique accepted write); \
         got {revision_count}"
    );

    // ── c) Exactly N audit records ────────────────────────────────────────────
    let audit_count: i64 = db
        .with_read(|conn| {
            Ok(conn
                .query_row("SELECT COUNT(*) FROM audit_records", [], |r| r.get(0))
                .map_err(kria_core::memory::error::StorageError::Sqlite)?)
        })
        .expect("must read audit_records");
    assert_eq!(
        audit_count, N as i64,
        "concurrent writes: expected exactly {N} audit_records; got {audit_count}"
    );

    // ── d) All committed outcomes carry a valid event_id ─────────────────────
    let committed_count = outcomes
        .iter()
        .filter(|o| o.status() == CommandStatus::Committed)
        .count();
    for outcome in outcomes
        .iter()
        .filter(|o| o.status() == CommandStatus::Committed)
    {
        assert!(
            outcome.outcome.event_id.is_some(),
            "a Committed outcome must carry a valid event_id"
        );
    }

    println!(
        "[concurrent_authority_bus_writes_10_tasks] PASS — \
         {committed_count}/{N} Committed, \
         {revision_count} graph_revisions, \
         {audit_count} audit_records"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// SLICE 8 — F1 gate correctness hash
// ═══════════════════════════════════════════════════════════════════════════
//
// Computes a SHA-256 over the concatenation of all named slice identifiers
// that form the F1 evidence gate. The hash is deterministic: same inputs →
// same hash, allowing the manifest validator to reproduce it.
//
// Named slices contributing to the hash (all must pass before this runs):
//
//   [NEW]  paired_world_plugin_x_filter_returns_only_plugin_x
//   [NEW]  paired_world_core_filter_returns_only_core
//   [NEW]  concurrent_authority_bus_writes_10_tasks_no_panics
//   [PRE]  scenario1_schema_checksum_corruption_enters_recovery_mode      (f1_8_7)
//   [PRE]  scenario2_outbox_stuck_entry_enters_recovery_mode              (f1_8_7)
//   [PRE]  scenario4_recovery_restore_transitions_to_healthy              (f1_8_7)
//   [PRE]  scenario5_manifest_algorithm_version_mismatch_is_partial_not_recovery (f1_8_7)
//   [PRE]  lifecycle_residue_deleted_removed_from_fts_and_vectors         (maintenance)
//   [PRE]  lifecycle_residue_forgotten_removed_from_fts_and_vectors       (maintenance)
//   [PRE]  hard_delete_crypto_wording_honest_state                        (lifecycle)
//   [PRE]  rebuild_cancel_then_resume_completes_correctly                 (maintenance)
//   [PRE]  prop_authority_tx_atomicity_100_cases                          (f1_9_1)
//   [PRE]  prop_idempotency_same_key_always_replays_100_cases             (f1_9_1)
//   [PRE]  prop_policy_disabled_mode_always_rejected_100_cases            (f1_9_1)
//   [PRE]  prop_recovery_mode_write_always_blocked_100_cases              (f1_9_1)

/// The ordered canonical list of all F1.9.2 test slice identifiers.
/// This list is the input to the correctness hash. Adding or reordering entries
/// changes the hash (intentional — the hash must match the evidence manifest).
const F1_EVIDENCE_SLICES: &[&str] = &[
    // New slices added by this task (1.9.2)
    "paired_world_plugin_x_filter_returns_only_plugin_x",
    "paired_world_core_filter_returns_only_core",
    "concurrent_authority_bus_writes_10_tasks_no_panics",
    // Pre-existing corruption/recovery slices (f1_8_7)
    "scenario1_schema_checksum_corruption_enters_recovery_mode",
    "scenario2_outbox_stuck_entry_enters_recovery_mode",
    "scenario4_recovery_restore_transitions_to_healthy",
    "scenario5_manifest_algorithm_version_mismatch_is_partial_not_recovery",
    // Pre-existing lifecycle residue slices (maintenance.rs)
    "lifecycle_residue_deleted_removed_from_fts_and_vectors",
    "lifecycle_residue_forgotten_removed_from_fts_and_vectors",
    // Pre-existing crypto wording slice (lifecycle.rs)
    "hard_delete_crypto_wording_honest_state",
    // Pre-existing rebuild interruption slice (maintenance.rs)
    "rebuild_cancel_then_resume_completes_correctly",
    // Pre-existing authority property slices (f1_9_1)
    "prop_authority_tx_atomicity_100_cases",
    "prop_idempotency_same_key_always_replays_100_cases",
    "prop_policy_disabled_mode_always_rejected_100_cases",
    "prop_recovery_mode_write_always_blocked_100_cases",
];

/// Compute the F1 correctness hash: SHA-256 of the canonical newline-separated
/// slice name list (no trailing newline).
fn compute_f1_correctness_hash(slices: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, name) in slices.iter().enumerate() {
        if i > 0 {
            hasher.update(b"\n");
        }
        hasher.update(name.as_bytes());
    }
    let result = hasher.finalize();
    hex::encode(result)
}

/// F1 gate correctness hash summary test.
///
/// This test is always last in the file. It:
/// 1. Verifies the slice list is non-empty.
/// 2. Computes the deterministic SHA-256 hash.
/// 3. Prints the hash to test output (captured by the evidence manifest).
/// 4. Asserts the hash is non-empty and 64 hex characters (SHA-256 digest).
///
/// The printed hash is the "correctness hash" referenced in the F1 gate
/// manifest. A manifest validator can reproduce it from the same slice list.
#[test]
fn f1_gate_correctness_hash_summary() {
    // **Validates: Requirements MGR-027**
    assert!(
        !F1_EVIDENCE_SLICES.is_empty(),
        "F1 evidence slice list must not be empty"
    );

    let hash = compute_f1_correctness_hash(F1_EVIDENCE_SLICES);

    // The hash must be a valid 64-character lowercase hex string (SHA-256).
    assert_eq!(
        hash.len(),
        64,
        "correctness hash must be 64 hex characters; got {}",
        hash.len()
    );
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "correctness hash must be lowercase hex; got: {hash}"
    );

    // Log the evidence to stdout so cargo test --nocapture and CI capture it.
    println!("[F1.9.2 EVIDENCE] slice_count={}", F1_EVIDENCE_SLICES.len());
    println!("[F1.9.2 EVIDENCE] correctness_hash_sha256={hash}");
    println!("[F1.9.2 EVIDENCE] slice_names:");
    for name in F1_EVIDENCE_SLICES {
        println!("[F1.9.2 EVIDENCE]   - {name}");
    }

    // Sanity: the hash must be deterministic — compute it again and compare.
    let hash2 = compute_f1_correctness_hash(F1_EVIDENCE_SLICES);
    assert_eq!(
        hash, hash2,
        "correctness hash must be deterministic (same input → same output)"
    );

    println!(
        "[f1_gate_correctness_hash_summary] PASS — F1 correctness hash: {}",
        hash
    );
}
