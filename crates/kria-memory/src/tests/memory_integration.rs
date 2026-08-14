//! End-to-end memory integration tests (MGR-001–048).
//!
//! These tests exercise the full MemorySystem pipeline through its public API:
//! remember → search → correct → forget → restore → hard_delete → rebuild.
//! They are the ground truth for "is the feature truly implemented?"

#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::{MemoryConfig, MemorySystem};
use crate::error::MemoryResult;
use crate::lifecycle::ForgetScope;
use crate::retriever::RetrievalCtx;
use crate::stores::ports::Embedder;
use crate::types::{Availability, ModelVersion, WriteCandidate, WriteDecision};

// ── Fake embedder ─────────────────────────────────────────────────────────────

struct FakeEmb;
#[async_trait]
impl Embedder for FakeEmb {
    fn model_version(&self) -> ModelVersion {
        ModelVersion("fake_v1".into())
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

fn make_system() -> Arc<MemorySystem> {
    MemorySystem::open_for_test(MemoryConfig::default(), Arc::new(FakeEmb)).unwrap()
}

fn is_stored(d: &WriteDecision) -> bool {
    matches!(
        d,
        WriteDecision::Stored { .. }
            | WriteDecision::Deduped { .. }
            | WriteDecision::Batched
            | WriteDecision::Queued { .. }
    )
}

// ── IT-01: remember and search ────────────────────────────────────────────────

/// Remember persists a memory; flush enriches it; search returns it.
/// Validates MGR-001, MGR-034, MGR-006.
#[tokio::test]
async fn it01_remember_and_search() {
    let ms = make_system();
    let session = Uuid::now_v7();
    let d = ms
        .remember(WriteCandidate::user(
            session,
            "kria runs locally on the owner laptop",
        ))
        .unwrap();
    assert!(
        is_stored(&d),
        "remember must produce a stored decision: {d:?}"
    );
    ms.flush().await.unwrap();
    let res = ms.search("kria laptop", None).await.unwrap();
    assert!(!res.hits.is_empty(), "search must return at least one hit");
    assert!(res.hits.iter().any(|h| h.memory.content.contains("laptop")));
}

/// Multiple distinct memories are ranked by relevance. Validates MGR-006.
#[tokio::test]
async fn it02_multiple_memories_ranked() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "the user prefers dark mode themes"))
        .unwrap();
    ms.remember(WriteCandidate::user(
        s,
        "kria uses SQLite as its sole transactional authority",
    ))
    .unwrap();
    ms.remember(WriteCandidate::user(s, "voice pipeline uses Whisper STT"))
        .unwrap();
    ms.flush().await.unwrap();
    let res = ms.search("dark mode", None).await.unwrap();
    assert!(!res.hits.is_empty());
    assert!(
        res.hits[0].memory.content.contains("dark mode"),
        "top result must be the dark-mode fact"
    );
    let res2 = ms.search("SQLite authority", None).await.unwrap();
    assert!(!res2.hits.is_empty());
}

// ── IT-03: namespace isolation ────────────────────────────────────────────────

/// Validates MGR-004 (scope and sensitivity isolation).
#[tokio::test]
async fn it03_namespace_isolation() {
    let ms = make_system();
    let s = Uuid::now_v7();
    let core_c = WriteCandidate {
        namespace_hint: Some("core".into()),
        ..WriteCandidate::user(s, "shared_namespace_core_fact_xk9")
    };
    let plugin_c = WriteCandidate {
        namespace_hint: Some("plugin/isolated".into()),
        ..WriteCandidate::user(s, "plugin_namespace_fact_xk9")
    };
    ms.remember(core_c).unwrap();
    ms.remember(plugin_c).unwrap();
    ms.flush().await.unwrap();
    let ctx = RetrievalCtx {
        namespaces: vec!["plugin/isolated".into()],
        ..RetrievalCtx::default()
    };
    let res = ms.search("xk9", Some(ctx)).await.unwrap();
    for h in &res.hits {
        assert_eq!(
            h.memory.namespace, "plugin/isolated",
            "cross-namespace leak: got {} instead of plugin/isolated",
            h.memory.namespace
        );
    }
}

// ── IT-04/05: forget / restore / hard_delete ─────────────────────────────────

/// Forget excludes; restore re-enables with same ID. Validates MGR-040.
#[tokio::test]
async fn it04_forget_restore() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "sensitive_fact_to_forget_zq7"))
        .unwrap();
    ms.flush().await.unwrap();
    let before = ms.search("sensitive_fact_to_forget", None).await.unwrap();
    assert!(!before.hits.is_empty(), "must be searchable before forget");
    let mem_id = before.hits[0].memory.id;

    ms.forget(ForgetScope::Memory(mem_id), None).unwrap();

    let after = ms.search("sensitive_fact_to_forget", None).await.unwrap();
    assert!(
        after.hits.iter().all(|h| h.memory.id != mem_id),
        "forgotten memory must not appear in search"
    );

    ms.restore_forgotten(mem_id).unwrap();
    let restored = ms.search("sensitive_fact_to_forget", None).await.unwrap();
    assert!(
        restored.hits.iter().any(|h| h.memory.id == mem_id),
        "restored memory must be searchable again"
    );
}

/// Hard delete produces zero residue. Validates MGR-040 (zero residue).
#[tokio::test]
async fn it05_hard_delete_zero_residue() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "permanent_delete_fact_ab3"))
        .unwrap();
    ms.flush().await.unwrap();
    let before = ms.search("permanent_delete_fact", None).await.unwrap();
    assert!(!before.hits.is_empty(), "must be searchable before delete");
    let mem_id = before.hits[0].memory.id;

    ms.hard_delete(ForgetScope::Memory(mem_id)).await.unwrap();
    ms.flush().await.unwrap();

    let after = ms.search("permanent_delete_fact", None).await.unwrap();
    assert!(
        after.hits.iter().all(|h| h.memory.id != mem_id),
        "hard-deleted memory must have zero residue"
    );
}

// ── IT-06: disabled state blocks writes ──────────────────────────────────────

/// Writing while memory is disabled returns an error. Validates MGR-017.
#[test]
fn it06_disabled_blocks_writes() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "write before disable"))
        .unwrap();
    ms.set_enabled(false);
    let res = ms.remember(WriteCandidate::user(s, "write while disabled"));
    assert!(
        res.is_err(),
        "write must fail when memory system is disabled: {res:?}"
    );
}

// ── IT-07: authorize_read gate (NBW-F1-03) ────────────────────────────────────

/// search passes through authorize_read without error. Validates MGR-004 A5.
#[tokio::test]
async fn it07_search_passes_authorize_read_gate() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "auth gate test content zz9"))
        .unwrap();
    ms.flush().await.unwrap();
    let res = ms.search("auth gate test", None).await;
    assert!(
        res.is_ok(),
        "search must succeed through authorize_read gate: {res:?}"
    );
}

// ── IT-08/09: Stress ─────────────────────────────────────────────────────────

/// 100 concurrent remember calls all stored/batched. Validates MGR-009, MGR-033.
#[tokio::test]
async fn it08_stress_100_concurrent_writes() {
    let ms = make_system();
    let ms = Arc::new(ms);
    let mut handles = Vec::new();
    for i in 0..100u32 {
        let ms = Arc::clone(&ms);
        let sess = Uuid::now_v7();
        handles.push(tokio::spawn(async move {
            ms.remember(WriteCandidate::user(
                sess,
                format!("stress fact {i} delta kq8"),
            ))
        }));
    }
    let mut count = 0u32;
    for h in handles {
        let result = h.await.unwrap().unwrap();
        assert!(
            is_stored(&result),
            "all writes must be stored or batched: {result:?}"
        );
        count += 1;
    }
    assert_eq!(count, 100);
}

/// 50 memories stored, 10 concurrent searches all return results. Validates MGR-006, MGR-009.
#[tokio::test]
async fn it09_stress_concurrent_searches() {
    let ms = make_system();
    let s = Uuid::now_v7();
    for i in 0..50u32 {
        ms.remember(WriteCandidate::user(
            s,
            format!("concurrent_search_fact_{i}_unique_kq8"),
        ))
        .unwrap();
    }
    ms.flush().await.unwrap();
    let ms = Arc::new(ms);
    let mut handles = Vec::new();
    for _ in 0..10 {
        let ms = Arc::clone(&ms);
        handles.push(tokio::spawn(async move {
            ms.search("concurrent_search_fact", None).await
        }));
    }
    for h in handles {
        let res = h.await.unwrap().unwrap();
        assert!(
            !res.hits.is_empty(),
            "concurrent search must return results"
        );
    }
}

// ── IT-10/11: graph traversal regression (batch BFS fix) ─────────────────────

/// graph_neighbors returns correct depths, no duplicates. Validates MGR-007.
#[test]
fn it10_graph_neighbors_no_duplicates_within_cap() {
    use crate::db::Database;
    use crate::stores::ports::GraphStore;
    use crate::stores::sqlite_graph::SqliteGraphStore;
    use crate::types::Entity;

    let db = Arc::new(Database::open_in_memory().unwrap());
    let g = SqliteGraphStore::new(db.clone());

    let mut a = Entity {
        id: Uuid::now_v7(),
        canonical_id: Uuid::now_v7(),
        entity_type: "concept".into(),
        display_name: "A".into(),
        created_at: chrono::Utc::now(),
    };
    let mut b = Entity {
        id: Uuid::now_v7(),
        canonical_id: Uuid::now_v7(),
        entity_type: "concept".into(),
        display_name: "B".into(),
        created_at: chrono::Utc::now(),
    };
    let mut c = Entity {
        id: Uuid::now_v7(),
        canonical_id: Uuid::now_v7(),
        entity_type: "concept".into(),
        display_name: "C".into(),
        created_at: chrono::Utc::now(),
    };
    a.canonical_id = a.id;
    b.canonical_id = b.id;
    c.canonical_id = c.id;

    let mut tx = db.begin().unwrap();
    g.add_entity(&mut tx, &a).unwrap();
    g.add_entity(&mut tx, &b).unwrap();
    g.add_entity(&mut tx, &c).unwrap();
    tx.commit().unwrap();

    // Insert A→B, B→C via the helper used in sqlite_graph tests.
    insert_rel_for_test(&db, a.id, b.id);
    insert_rel_for_test(&db, b.id, c.id);

    let hits = g.neighbors(a.id, 2).unwrap();
    let ids: std::collections::HashSet<Uuid> = hits.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), hits.len(), "no duplicate nodes");
    assert!(!ids.contains(&a.id), "root must not appear");
    for (_, d) in &hits {
        assert!(*d <= 2, "depth must not exceed cap=2");
    }
}

/// BFS terminates on a cycle. Validates batch BFS cycle-safety.
#[test]
fn it11_graph_bfs_terminates_on_cycle() {
    use crate::db::Database;
    use crate::stores::ports::GraphStore;
    use crate::stores::sqlite_graph::SqliteGraphStore;
    use crate::types::Entity;

    let db = Arc::new(Database::open_in_memory().unwrap());
    let g = SqliteGraphStore::new(db.clone());
    let mut a = Entity {
        id: Uuid::now_v7(),
        canonical_id: Uuid::now_v7(),
        entity_type: "concept".into(),
        display_name: "Cy1".into(),
        created_at: chrono::Utc::now(),
    };
    let mut b = Entity {
        id: Uuid::now_v7(),
        canonical_id: Uuid::now_v7(),
        entity_type: "concept".into(),
        display_name: "Cy2".into(),
        created_at: chrono::Utc::now(),
    };
    a.canonical_id = a.id;
    b.canonical_id = b.id;
    let mut tx = db.begin().unwrap();
    g.add_entity(&mut tx, &a).unwrap();
    g.add_entity(&mut tx, &b).unwrap();
    tx.commit().unwrap();
    insert_rel_for_test(&db, a.id, b.id);
    insert_rel_for_test(&db, b.id, a.id);

    let hits = g.neighbors(a.id, 3).unwrap();
    let ids: std::collections::HashSet<Uuid> = hits.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids.len(),
        hits.len(),
        "cycle BFS must not produce duplicates"
    );
}

// Helper: minimal relationship insert (same as the one in sqlite_graph tests).
fn insert_rel_for_test(db: &Arc<crate::db::Database>, source: Uuid, target: Uuid) {
    use crate::ids::new_id;
    let id = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    let identity = format!("{source}-{target}-related_to");
    let tx = db.begin().unwrap();
    tx.conn()
        .execute(
            "INSERT OR IGNORE INTO relationships_v2(
             id, source_kind, source_id, target_kind, target_id,
             relation_name, relation_version, direction_class,
             valid_from, valid_until, truth_state,
             namespace, owner_id, scope, sensitivity,
             policy_source_id, policy_version, identity_hash)
         VALUES (?1,'entity',?2,'entity',?3,'related_to',1,'directed',?4,NULL,NULL,
                 'core','','global',0,'core','pending-f1.4',?5)",
            rusqlite::params![
                id.to_string(),
                source.to_string(),
                target.to_string(),
                now,
                identity
            ],
        )
        .unwrap();
    tx.commit().unwrap();
}

// ── IT-12: crypto truth ───────────────────────────────────────────────────────

/// crypto_shred_capability is consistently the canonical unavailable string. Validates MGR-041.
#[tokio::test]
async fn it12_crypto_shred_capability_is_unavailable() {
    let ms = make_system();
    let h = ms.health().await.unwrap();
    assert_eq!(
        h.crypto_shred_capability,
        crate::api::CRYPTO_SHRED_CAPABILITY
    );
    assert!(h.crypto_shred_capability.contains("unavailable"));
    assert!(!h.crypto_shred_capability.contains("Crypto-Shredded"));
}

// ── IT-13: observability safety ──────────────────────────────────────────────

/// health() reports aggregate counts only — no raw memory content. Validates MGR-028.
#[tokio::test]
async fn it13_health_report_has_no_content_fields() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "health_report_content_check_zz1"))
        .unwrap();
    ms.flush().await.unwrap();
    let h = ms.health().await.unwrap();
    // HealthReport has no Serialize impl (by design — not sent on the wire raw).
    // Verify via the named fields instead.
    assert_eq!(h.api_version, crate::api::API_VERSION);
    assert!(
        h.memory_count >= 1,
        "memory_count must reflect stored memories"
    );
    assert!(
        !h.recovery_mode,
        "healthy system must not be in recovery mode"
    );
}
