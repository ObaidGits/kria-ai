//! Feature tests for the KRIA memory subsystem.
//!
//! Uses an in-memory SQLite database so tests are fully idempotent.
//! Covers conversation storage, fact management, vector search,
//! and decay pruning.

use chrono::Utc;
use kria_core::memory::embeddings::EmbeddingModel;
use kria_core::memory::vectors::VectorIndex;
use kria_core::memory::{AuditEntry, ConversationTurn, MemoryFact, MemoryStore};
use std::path::Path;

// ── MemoryStore — conversation turns ────────────────────────────────

fn make_turn(session: &str, role: &str, content: &str) -> ConversationTurn {
    ConversationTurn {
        id: None,
        session_id: session.into(),
        role: role.into(),
        content: content.into(),
        tool_name: None,
        tool_result: None,
        tokens_used: Some(5),
        timestamp: Utc::now(),
    }
}

fn make_fact(text: &str, category: &str) -> MemoryFact {
    MemoryFact {
        id: None,
        text: text.into(),
        category: category.into(),
        source: "test".into(),
        created_at: Utc::now(),
        last_accessed: Utc::now(),
        access_count: 0,
        decay_score: 1.0,
    }
}

fn make_audit(session: &str, risk: &str, action: &str) -> AuditEntry {
    AuditEntry {
        id: None,
        session_id: session.into(),
        action: action.into(),
        parameters: "{}".into(),
        risk_level: risk.into(),
        decision: "allow".into(),
        decided_by: "test".into(),
        result: Some("ok".into()),
        error_msg: None,
        rollback_id: None,
        duration_ms: Some(5),
        timestamp: Utc::now(),
    }
}

#[test]
fn store_and_retrieve_conversation_turns() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store
        .store_turn(&make_turn("sess-1", "user", "Hello KRIA"))
        .unwrap();
    store
        .store_turn(&make_turn("sess-1", "assistant", "Hi there!"))
        .unwrap();

    let turns = store.get_recent_turns("sess-1", 10).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[1].role, "assistant");
    assert_eq!(turns[1].content, "Hi there!");
}

#[test]
fn get_recent_turns_respects_limit() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    for i in 0..20 {
        store
            .store_turn(&make_turn("sess-2", "user", &format!("msg {i}")))
            .unwrap();
    }

    let turns = store.get_recent_turns("sess-2", 5).unwrap();
    assert_eq!(turns.len(), 5);
}

#[test]
fn list_sessions_returns_stored_sessions() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store
        .store_turn(&make_turn("sess-a", "user", "hello"))
        .unwrap();
    store
        .store_turn(&make_turn("sess-b", "user", "world"))
        .unwrap();

    let sessions = store.list_sessions().unwrap();
    assert!(sessions.len() >= 2);
}

#[test]
fn delete_session_removes_all_turns() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store
        .store_turn(&make_turn("sess-del", "user", "bye"))
        .unwrap();
    store.delete_session("sess-del").unwrap();

    let turns = store.get_recent_turns("sess-del", 10).unwrap();
    assert!(turns.is_empty());
}

// ── MemoryStore — facts ─────────────────────────────────────────────

#[test]
fn store_and_search_facts() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store
        .store_fact(&make_fact("Rust was created by Mozilla", "tech"))
        .unwrap();
    store
        .store_fact(&make_fact("The user prefers dark themes", "preference"))
        .unwrap();

    let results = store.search_facts("Rust", 10).unwrap();
    assert!(!results.is_empty());
    assert!(results[0].text.contains("Rust"));
}

// ── MemoryStore — preferences ───────────────────────────────────────

#[test]
fn set_and_get_preferences() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store.set_preference("theme", "dark").unwrap();
    let val = store.get_preference("theme").unwrap();
    assert_eq!(val.as_deref(), Some("dark"));
}

#[test]
fn overwrite_preference() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store.set_preference("lang", "en").unwrap();
    store.set_preference("lang", "fr").unwrap();
    let val = store.get_preference("lang").unwrap();
    assert_eq!(val.as_deref(), Some("fr"));
}

#[test]
fn missing_preference_returns_none() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();
    let val = store.get_preference("nonexistent").unwrap();
    assert!(val.is_none());
}

#[test]
fn query_audit_uses_bound_params_and_rejects_injection_payload() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();

    store
        .log_audit(&make_audit("sess-safe", "red", "delete_file"))
        .unwrap();
    store
        .log_audit(&make_audit("sess-other", "red", "delete_file"))
        .unwrap();

    let injected_session = "' OR 1=1 --";
    let injected = store
        .query_audit(50, Some("red"), Some(injected_session))
        .unwrap();
    assert!(
        injected.is_empty(),
        "injection payload should not bypass session filter"
    );

    let scoped = store
        .query_audit(50, Some("red"), Some("sess-safe"))
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].session_id, "sess-safe");
}

// ── VectorIndex ─────────────────────────────────────────────────────

#[test]
fn vector_index_add_and_search() {
    let idx = VectorIndex::in_memory(3);

    idx.add(1, vec![1.0, 0.0, 0.0]).unwrap();
    idx.add(2, vec![0.0, 1.0, 0.0]).unwrap();
    idx.add(3, vec![0.0, 0.0, 1.0]).unwrap();

    let results = idx.search(&[0.9, 0.1, 0.0], 2);
    assert_eq!(results.len(), 2);
    // The first result should be closest to [1, 0, 0]
    assert_eq!(results[0].0, 1);
}

#[test]
fn vector_index_remove() {
    let idx = VectorIndex::in_memory(3);
    idx.add(1, vec![1.0, 0.0, 0.0]).unwrap();
    idx.add(2, vec![0.0, 1.0, 0.0]).unwrap();

    idx.remove(1);
    assert_eq!(idx.len(), 1);

    let results = idx.search(&[1.0, 0.0, 0.0], 5);
    // Only id=2 should remain
    assert!(results.iter().all(|(id, _)| *id != 1));
}

#[test]
fn vector_index_empty_search_returns_empty() {
    let idx = VectorIndex::in_memory(3);
    let results = idx.search(&[1.0, 0.0, 0.0], 5);
    assert!(results.is_empty());
}

// ── EmbeddingModel ──────────────────────────────────────────────────

#[test]
fn embedding_produces_correct_dimension() {
    let model = EmbeddingModel::load(384).unwrap();
    let vec = model.embed("hello world").unwrap();
    assert_eq!(vec.len(), 384);
}

#[test]
fn embedding_is_deterministic() {
    let model = EmbeddingModel::load(384).unwrap();
    let a = model.embed("same text").unwrap();
    let b = model.embed("same text").unwrap();
    assert_eq!(a, b);
}

#[test]
fn embedding_differs_for_different_input() {
    let model = EmbeddingModel::load(384).unwrap();
    let a = model.embed("text a").unwrap();
    let b = model.embed("text b").unwrap();
    assert_ne!(a, b);
}

// ── Chat & memory management — preference cleanup (spec: chat-memory-management) ──

#[test]
fn delete_session_preferences_removes_only_managed_keys() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();
    let sid = "sess-pref";

    // Seed the six managed per-session keys plus an unrelated key.
    for (k, v) in [
        (format!("session_title:{sid}"), "Hello"),
        (format!("session_title_manual:{sid}"), "1"),
        (format!("session_created_at:{sid}"), "2026-01-01T00:00:00Z"),
        (format!("session_pinned:{sid}"), "1"),
        (format!("session_archived:{sid}"), "0"),
        (format!("session_temporary:{sid}"), "1"),
        ("unrelated_global_key".to_string(), "keep-me"),
        (format!("session_title:other-{sid}"), "other"),
    ] {
        store.set_preference(&k, v).unwrap();
    }

    let removed = store.delete_session_preferences(sid).unwrap();
    assert_eq!(removed, 6, "should remove exactly the six managed keys");

    // Managed keys gone.
    for k in [
        format!("session_title:{sid}"),
        format!("session_title_manual:{sid}"),
        format!("session_created_at:{sid}"),
        format!("session_pinned:{sid}"),
        format!("session_archived:{sid}"),
        format!("session_temporary:{sid}"),
    ] {
        assert_eq!(
            store.get_preference(&k).unwrap(),
            None,
            "{k} should be gone"
        );
    }

    // Unrelated keys preserved.
    assert_eq!(
        store.get_preference("unrelated_global_key").unwrap(),
        Some("keep-me".to_string())
    );
    assert_eq!(
        store
            .get_preference(&format!("session_title:other-{sid}"))
            .unwrap(),
        Some("other".to_string()),
        "another session's prefs must not be touched"
    );
}

#[test]
fn delete_session_preferences_is_idempotent_for_unknown_session() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();
    let removed = store.delete_session_preferences("does-not-exist").unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn delete_preference_removes_single_key() {
    let store = MemoryStore::open(Path::new(":memory:")).unwrap();
    store.set_preference("k1", "v1").unwrap();
    assert_eq!(store.delete_preference("k1").unwrap(), 1);
    assert_eq!(store.get_preference("k1").unwrap(), None);
    assert_eq!(store.delete_preference("k1").unwrap(), 0);
}
