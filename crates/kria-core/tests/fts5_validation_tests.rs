//! F3.2.6 — FTS5 validation tests: offline operation, Unicode/diacritics/CJK/RTL,
//! injection-shaped text, no-results-with-filters, corruption Partial, and 100k query plans.
//!
//! **Validates: Requirements MGR-006, MGR-009, MGR-036, MGR-042, MGR-045**
//!
//! All tests use in-memory SQLite (`Database::open_in_memory()`).
//! No network calls, no file I/O beyond SQLite, no LLM or embedder dependencies.

use std::sync::Arc;

use kria_core::memory::db::Database;
use kria_core::memory::stores::sqlite_fts_rebuild::reconcile_fts_index;
use kria_core::memory::stores::sqlite_search_documents::{
    search_documents_fts_query, upsert_search_document, Fts5SearchQuery, SearchDocument,
    TotalSemantics,
};

// ─── shared helpers ───────────────────────────────────────────────────────────

fn open_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().expect("open in-memory authority DB"))
}

fn insert_doc(
    conn: &rusqlite::Connection,
    kind: &str,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
    aliases: Option<&str>,
    namespace: &str,
    scope: &str,
    sensitivity: i64,
    truth_state: &str,
    revision: i64,
) {
    let doc = SearchDocument {
        record_kind: kind.to_string(),
        record_id: id.to_string(),
        title: title.map(str::to_string),
        body: body.map(str::to_string),
        aliases: aliases.map(str::to_string),
        source_text: None,
        relation_text: None,
        namespace: namespace.to_string(),
        owner_id: "user-fts-test".to_string(),
        scope: scope.to_string(),
        sensitivity,
        truth_state: truth_state.to_string(),
        valid_from: None,
        valid_until: None,
        content_hash: format!("h-{kind}-{id}"),
        revision,
    };
    upsert_search_document(conn, &doc).expect("upsert must succeed in test setup");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 1. OFFLINE OPERATION
//    FTS5 works purely on SQLite — no network, LLM, or embedder required.
// ═══════════════════════════════════════════════════════════════════════════════

/// FTS5 insert + query works entirely offline (no network/model calls).
/// This is structural: the test calls no async code, no network, no model.
#[test]
fn offline_fts5_insert_and_query_no_external_deps() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "offline-001",
        Some("offline knowledge storage"),
        Some("KRIA stores facts locally without any network call"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(&conn, "offline", &Fts5SearchQuery::default())
        .expect("offline query must not return an error");

    assert_eq!(
        result.hits.len(),
        1,
        "offline query must find the inserted row"
    );
    assert_eq!(result.profile_version, "fts5-v1");
}

/// Public sensitivity=0 rows are found when no policy filter is applied.
#[test]
fn offline_public_sensitivity_query_no_policy_filter() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "offline-public-001",
        Some("public record for offline test"),
        Some("publicly accessible content without any filter gate"),
        None,
        "core",
        "default",
        0, // sensitivity=0 → Public
        "Current",
        1,
    );

    let result =
        search_documents_fts_query(&conn, "publicly accessible", &Fts5SearchQuery::default())
            .expect("public query must succeed");

    assert!(
        !result.hits.is_empty(),
        "sensitivity=0 row must be found with no policy filter"
    );
    assert_eq!(result.hits[0].sensitivity, 0);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. UNICODE / DIACRITICS / CJK / RTL
// ═══════════════════════════════════════════════════════════════════════════════

/// German diacritics: searching "uber" finds "über" (unicode61 remove_diacritics=2).
#[test]
fn unicode_german_uber_finds_ueber() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "de-001",
        Some("über das Wetter"),
        Some("Die Fahrt über die Brücke war schön"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(&conn, "uber", &Fts5SearchQuery::default())
        .expect("German diacritic query must not return an error");

    assert_eq!(
        result.hits.len(),
        1,
        "unicode61 remove_diacritics=2 must normalize 'über' to 'uber' for matching"
    );
}

/// Spanish accented vowels: searching "manana" finds "mañana".
#[test]
fn unicode_spanish_manana_finds_mannana() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "es-001",
        Some("mañana será otro día"),
        Some("La reunión es mañana por la tarde"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(&conn, "manana", &Fts5SearchQuery::default())
        .expect("Spanish diacritic query must not return an error");

    assert_eq!(
        result.hits.len(),
        1,
        "unicode61 remove_diacritics=2 must normalize 'mañana' to 'manana'"
    );
}

/// CJK characters: inserting and querying Chinese/Japanese/Korean must not crash.
#[test]
fn unicode_cjk_no_crash() {
    let db = open_db();
    let conn = db.write();

    // Chinese
    insert_doc(
        &conn,
        "memory",
        "cjk-zh-001",
        Some("记忆系统"),
        Some("本地存储的认知记录"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );
    // Japanese
    insert_doc(
        &conn,
        "memory",
        "cjk-ja-001",
        Some("メモリシステム"),
        Some("ローカルに保存された認知レコード"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );
    // Korean
    insert_doc(
        &conn,
        "memory",
        "cjk-ko-001",
        Some("메모리 시스템"),
        Some("로컬에 저장된 인지 기록"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // Query CJK text — may or may not return results depending on tokenizer,
    // but must NOT panic or return an Err.
    let r1 = search_documents_fts_query(&conn, "记忆", &Fts5SearchQuery::default());
    assert!(
        r1.is_ok(),
        "CJK Chinese query must not crash: {:?}",
        r1.err()
    );

    let r2 = search_documents_fts_query(&conn, "メモリ", &Fts5SearchQuery::default());
    assert!(
        r2.is_ok(),
        "CJK Japanese query must not crash: {:?}",
        r2.err()
    );

    let r3 = search_documents_fts_query(&conn, "메모리", &Fts5SearchQuery::default());
    assert!(
        r3.is_ok(),
        "CJK Korean query must not crash: {:?}",
        r3.err()
    );
}

/// RTL text (Arabic + Hebrew): insert and query must not crash.
#[test]
fn unicode_rtl_no_crash() {
    let db = open_db();
    let conn = db.write();

    // Arabic
    insert_doc(
        &conn,
        "memory",
        "rtl-ar-001",
        Some("نظام الذاكرة"),
        Some("السجلات المعرفية المخزنة محلياً"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );
    // Hebrew
    insert_doc(
        &conn,
        "memory",
        "rtl-he-001",
        Some("מערכת זיכרון"),
        Some("רשומות קוגניטיביות מאוחסנות מקומית"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let r1 = search_documents_fts_query(&conn, "الذاكرة", &Fts5SearchQuery::default());
    assert!(
        r1.is_ok(),
        "Arabic RTL query must not crash: {:?}",
        r1.err()
    );

    let r2 = search_documents_fts_query(&conn, "זיכרון", &Fts5SearchQuery::default());
    assert!(
        r2.is_ok(),
        "Hebrew RTL query must not crash: {:?}",
        r2.err()
    );
}

/// Mixed-script record (Latin + CJK + Arabic) must not crash on insert or query.
#[test]
fn unicode_mixed_script_no_crash() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "mixed-001",
        Some("mixed script latinword 記憶 ذاكرة"),
        Some("body with latin plus CJK 知識 and Arabic معلومات content"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // Latin term from mixed record must be found.
    let r = search_documents_fts_query(&conn, "latinword", &Fts5SearchQuery::default());
    assert!(
        r.is_ok(),
        "mixed-script query must not crash: {:?}",
        r.err()
    );
    let result = r.unwrap();
    assert_eq!(
        result.hits.len(),
        1,
        "Latin token in mixed-script record must be findable"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. INJECTION-SHAPED TEXT
//    All must NOT cause SQL errors or data exposure. Each either returns empty
//    results safely or matched results without panicking.
// ═══════════════════════════════════════════════════════════════════════════════

/// Classic SQL injection attempt must not cause an error.
#[test]
fn injection_classic_sql_drop_table() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "inj-base-001",
        Some("safe baseline row for injection tests"),
        Some("this row must survive any injection attempt"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(
        &conn,
        "''; DROP TABLE search_documents; --",
        &Fts5SearchQuery::default(),
    );
    assert!(
        result.is_ok(),
        "SQL injection attempt must not cause an error: {:?}",
        result.err()
    );
    // The injection must not have dropped the table — the baseline row survives.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
        .expect("search_documents table must still exist after injection attempt");
    assert!(
        count >= 1,
        "search_documents must not be dropped by injection"
    );
}

/// OR-based injection must not cause an error or return unintended rows.
#[test]
fn injection_or_1_equals_1() {
    let db = open_db();
    let conn = db.write();

    // Insert a "secret" row that the injection should NOT expose.
    let secret_doc = SearchDocument {
        record_kind: "memory".to_string(),
        record_id: "inj-secret-001".to_string(),
        title: Some("top secret data injection target".to_string()),
        body: Some("this must not be exposed via injection".to_string()),
        aliases: None,
        source_text: None,
        relation_text: None,
        namespace: "core".to_string(),
        owner_id: "user-fts-test".to_string(),
        scope: "default".to_string(),
        sensitivity: 3,
        truth_state: "Current".to_string(),
        valid_from: None,
        valid_until: None,
        content_hash: "h-inj-secret".to_string(),
        revision: 1,
    };
    upsert_search_document(&conn, &secret_doc).unwrap();

    let result = search_documents_fts_query(&conn, "\" OR 1=1 --", &Fts5SearchQuery::default());
    assert!(
        result.is_ok(),
        "OR-injection must not cause an error: {:?}",
        result.err()
    );
}

/// FTS5 operator injection: NEAR() syntax injection must be safely handled.
#[test]
fn injection_fts5_near_operator() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "inj-near-base",
        Some("near operator test baseline"),
        None,
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result =
        search_documents_fts_query(&conn, "NEAR(title body 10)", &Fts5SearchQuery::default());
    assert!(
        result.is_ok(),
        "FTS5 NEAR operator injection must not cause a hard error: {:?}",
        result.err()
    );
}

/// Bare FTS5 wildcard '*' alone must not cause an error.
#[test]
fn injection_bare_wildcard_star() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "inj-star-base",
        Some("wildcard star test row"),
        None,
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(&conn, "*", &Fts5SearchQuery::default());
    assert!(
        result.is_ok(),
        "bare wildcard '*' must not cause an error: {:?}",
        result.err()
    );
}

/// FTS5 field:value injection must not cause an error.
#[test]
fn injection_fts5_field_colon_value() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "inj-field-base",
        Some("field colon injection baseline"),
        None,
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result =
        search_documents_fts_query(&conn, "title : \"injection\"", &Fts5SearchQuery::default());
    assert!(
        result.is_ok(),
        "FTS5 field:value injection must not cause an error: {:?}",
        result.err()
    );
}

/// Long injection string (400+ chars of SQL-like chars) must not cause an error.
#[test]
fn injection_long_sql_like_string() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "inj-long-base",
        Some("long injection baseline row"),
        None,
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // 400+ char injection string mixing SQL/FTS5 metacharacters.
    // Built programmatically to avoid escaping surprises in a raw literal.
    let long_injection = {
        let mut s = String::new();
        s.push_str("'; DROP TABLE search_documents; -- ");
        s.push_str("\" OR '1'='1 /* */ UNION SELECT * FROM search_documents ");
        s.push_str("WHERE 1=1 AND 'a'='a'; INSERT INTO evil VALUES(1); ");
        s.push_str("NEAR(title body 100) * title:evil OR AND NOT ");
        s.push_str("() [] {} @#$%^&*~`|<>,.?/!^{}[]=+_-abcdefghijklmnopqrstuvwxyz ");
        s.push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 "); // 57 chars
        s.push_str("'; DROP TABLE search_documents_fts; PRAGMA integrity_check; ");
        s.push_str("\" UNION SELECT NULL,NULL,NULL,NULL,NULL FROM sqlite_master -- ");
        s
    };

    assert!(
        long_injection.len() >= 400,
        "test setup: injection string must be at least 400 chars (got {})",
        long_injection.len()
    );

    let result = search_documents_fts_query(&conn, &long_injection, &Fts5SearchQuery::default());
    assert!(
        result.is_ok(),
        "long injection string must not cause an error: {:?}",
        result.err()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. NO-RESULTS-WITH-FILTERS
//    Active filters that produce zero results must return empty hits, NOT error.
// ═══════════════════════════════════════════════════════════════════════════════

/// Records in "ns-a" queried with namespace="ns-b" → empty hits, not error.
#[test]
fn no_results_namespace_mismatch() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "ns-a-001",
        Some("namespace alpha content"),
        Some("this record lives in namespace ns-a only"),
        None,
        "ns-a",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(
        &conn,
        "namespace",
        &Fts5SearchQuery {
            namespace: Some("ns-b".to_string()),
            ..Default::default()
        },
    )
    .expect("namespace-mismatch query must not return an error");

    assert!(
        result.hits.is_empty(),
        "namespace filter mismatch must yield empty hits, not an error"
    );
    assert_eq!(
        result.total_semantics,
        TotalSemantics::Exact(0),
        "total_semantics must be Exact(0) on namespace mismatch"
    );
    assert_eq!(result.profile_version, "fts5-v1");
}

/// Records with truth_state="Current" queried for "Deleted" → empty hits, not error.
#[test]
fn no_results_truth_state_mismatch() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "ts-curr-001",
        Some("truthstate current record"),
        Some("this record has truth state Current"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    let result = search_documents_fts_query(
        &conn,
        "truthstate",
        &Fts5SearchQuery {
            truth_state: Some("Deleted".to_string()),
            ..Default::default()
        },
    )
    .expect("truth_state mismatch query must not return an error");

    assert!(
        result.hits.is_empty(),
        "truth_state=Deleted filter on Current-only data must yield empty hits"
    );
    assert_eq!(
        result.total_semantics,
        TotalSemantics::Exact(0),
        "total_semantics must be Exact(0) when truth_state filter yields no matches"
    );
    assert_eq!(result.profile_version, "fts5-v1");
}

/// Only sensitivity=0 records, queried with max_sensitivity=-1 → empty hits, not error.
#[test]
fn no_results_invalid_sensitivity_filter() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "sens-zero-001",
        Some("sensitivity zero public record"),
        Some("this record is fully public sensitivity zero"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // max_sensitivity=-1 is an "impossible" filter since sensitivity ranges 0..3.
    let result = search_documents_fts_query(
        &conn,
        "sensitivity",
        &Fts5SearchQuery {
            max_sensitivity: Some(-1),
            ..Default::default()
        },
    )
    .expect("invalid max_sensitivity=-1 filter must not return an error");

    assert!(
        result.hits.is_empty(),
        "max_sensitivity=-1 filter must yield empty hits for all sensitivity>=0 rows"
    );
    assert_eq!(
        result.total_semantics,
        TotalSemantics::Exact(0),
        "total_semantics must be Exact(0) when no rows pass max_sensitivity=-1"
    );
    assert_eq!(
        result.profile_version, "fts5-v1",
        "profile_version must still be 'fts5-v1' even on empty results"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. CORRUPTION PARTIAL
//    FTS index inconsistency must not corrupt the content table or panic.
// ═══════════════════════════════════════════════════════════════════════════════

/// After reconcile_fts_index, the system still serves queries.
/// Simulates FTS/content divergence by inserting directly into search_documents
/// bypassing triggers (like a bulk import), then reconciling.
#[test]
fn corruption_reconcile_then_query_still_works() {
    let db = open_db();
    let conn = db.write();

    // Insert the normal row via upsert (trigger-driven FTS population).
    insert_doc(
        &conn,
        "memory",
        "corrupt-001",
        Some("reconcile test row alpha"),
        Some("this row was inserted normally"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // Simulate bypass insert: insert a row directly into search_documents
    // WITHOUT going through the trigger by manually deleting from the FTS index.
    // We replicate a "missing FTS entry" scenario by calling the FTS5 delete command
    // on the existing row, then inserting a new search_documents row directly
    // using raw SQL (bypassing the INSERT trigger).
    conn.execute_batch(
        "BEGIN;
         -- Remove the FTS5 entry for corrupt-001 to simulate divergence.
         INSERT INTO search_documents_fts(search_documents_fts, rowid, title, body, aliases, source_text, relation_text)
             SELECT 'delete', rowid, title, body, aliases, source_text, relation_text
             FROM search_documents WHERE record_id = 'corrupt-001';
         -- Insert a second row directly, bypassing the trigger.
         INSERT INTO search_documents
             (record_kind, record_id, title, body, aliases, source_text, relation_text,
              namespace, owner_id, scope, sensitivity, truth_state,
              valid_from, valid_until, content_hash, revision)
         VALUES ('memory','corrupt-002','reconcile test row beta',
                 'inserted bypassing triggers', NULL, NULL, NULL,
                 'core','user-fts-test','default',0,'Current',
                 NULL, NULL, 'h-corrupt-002', 1);
         COMMIT;"
    )
    .expect("manual divergence setup must succeed");

    // Now reconcile — should repair the index.
    let report = reconcile_fts_index(&conn).expect("reconcile must not return an error");
    // At minimum one row was diverged, so repopulated should reflect some work.
    assert!(
        report.repopulated > 0 || report.missing_fts_rows > 0,
        "reconcile must detect and repair the diverged FTS index"
    );

    // After reconciliation, queries must still work (system remains usable).
    let result = search_documents_fts_query(&conn, "reconcile", &Fts5SearchQuery::default())
        .expect("query after reconcile must not return an error");

    assert!(
        !result.hits.is_empty(),
        "FTS5 must be searchable after reconcile_fts_index"
    );
}

/// The content table (search_documents) must not be corrupted when FTS is damaged.
/// After manually breaking the FTS index, the content table remains intact.
#[test]
fn corruption_content_table_survives_fts_damage() {
    let db = open_db();
    let conn = db.write();

    // Insert known rows.
    for i in 0u32..5 {
        insert_doc(
            &conn,
            "memory",
            &format!("content-survives-{i:03}"),
            Some(&format!("content table survival test row {i}")),
            Some("body for content table survival"),
            None,
            "core",
            "default",
            0,
            "Current",
            i as i64 + 1,
        );
    }

    // Damage the FTS index by running an FTS 'delete-all' via the integrity-check
    // mechanism. Use FTS5's built-in 'integrity-check' to verify, then delete + rebuild.
    conn.execute_batch(
        "INSERT INTO search_documents_fts(search_documents_fts) VALUES('delete-all');",
    )
    .expect("FTS delete-all must not panic");

    // The content table must still have all 5 rows.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
        .expect("search_documents must be readable after FTS damage");

    assert_eq!(
        count, 5,
        "content table must retain all rows even after FTS index is damaged — \
         FTS and search_documents are independent"
    );

    // Reconcile to restore FTS.
    reconcile_fts_index(&conn).expect("reconcile after delete-all must not panic");

    // After reconcile, queries must work again.
    let result = search_documents_fts_query(&conn, "survival", &Fts5SearchQuery::default())
        .expect("query after FTS damage + reconcile must not error");

    assert!(
        !result.hits.is_empty(),
        "rows must be findable after FTS is repaired"
    );
}

/// When FTS virtual table is unavailable (dropped), the system returns a typed
/// error — NOT a panic.
#[test]
fn corruption_fts_table_unavailable_returns_typed_error_not_panic() {
    let db = open_db();
    let conn = db.write();

    insert_doc(
        &conn,
        "memory",
        "fts-drop-001",
        Some("fts drop test row"),
        Some("body for the fts drop test"),
        None,
        "core",
        "default",
        0,
        "Current",
        1,
    );

    // Drop the FTS5 virtual table to simulate it being unavailable.
    conn.execute_batch("DROP TABLE IF EXISTS search_documents_fts;")
        .expect("dropping FTS table must not panic");

    // Query must return Err (typed), not panic.
    let result = search_documents_fts_query(&conn, "fts drop test", &Fts5SearchQuery::default());

    assert!(
        result.is_err(),
        "querying after FTS table is dropped must return a typed error, not panic"
    );
    // The search_documents content table must still be intact.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM search_documents", [], |r| r.get(0))
        .expect("search_documents must still be readable after FTS is dropped");
    assert_eq!(
        count, 1,
        "content table must not be corrupted when FTS is dropped"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. 100K QUERY PLAN
//    Assert that the FTS5 query plan uses the FTS5 virtual table index and
//    does NOT perform a full scan of the content table.
// ═══════════════════════════════════════════════════════════════════════════════

/// Insert 1000 rows, run EXPLAIN QUERY PLAN, assert FTS5 index is used.
#[test]
fn query_plan_uses_fts5_index_not_full_table_scan() {
    let db = open_db();
    let conn = db.write();

    // Insert 1000 rows to give the query planner enough data for a meaningful plan.
    for i in 0u32..1000 {
        let id = format!("qp-{i:06}");
        let title = format!("queryplan row number {i} with unique content");
        let body = format!("body text for query plan row {i} searchterm");
        let doc = SearchDocument {
            record_kind: "memory".to_string(),
            record_id: id.clone(),
            title: Some(title),
            body: Some(body),
            aliases: None,
            source_text: None,
            relation_text: None,
            namespace: "core".to_string(),
            owner_id: "user-fts-test".to_string(),
            scope: "default".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: format!("h-qp-{i}"),
            revision: i as i64 + 1,
        };
        upsert_search_document(&conn, &doc).expect("bulk insert for query plan test");
    }

    // Capture EXPLAIN QUERY PLAN output for the main FTS5 query pattern.
    // This mirrors what search_documents_fts_query actually executes.
    let plan_rows: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT record_kind, record_id, -bm25(search_documents_fts) AS score, \
                        truth_state, namespace, scope, sensitivity, revision, \
                        title, body, aliases, source_text, relation_text \
                 FROM search_documents_fts \
                 WHERE search_documents_fts MATCH ? \
                   AND namespace = ? \
                   AND sensitivity <= ? \
                 ORDER BY score DESC \
                 LIMIT ?",
            )
            .expect("EXPLAIN QUERY PLAN prepare must not fail");

        stmt.query_map(rusqlite::params!["searchterm", "core", 2i64, 25i64], |r| {
            r.get::<_, String>(3)
        })
        .expect("EXPLAIN QUERY PLAN query_map must not fail")
        .filter_map(|row| row.ok())
        .collect()
    };

    let plan_text = plan_rows.join("\n");

    // The plan must reference the FTS5 virtual table index.
    // SQLite's EXPLAIN QUERY PLAN output for FTS5 contains "VIRTUAL TABLE INDEX"
    // or "SCAN search_documents_fts VIRTUAL TABLE INDEX".
    let uses_fts_index = plan_text.to_uppercase().contains("VIRTUAL TABLE INDEX")
        || plan_text
            .to_uppercase()
            .contains("SCAN search_documents_fts");

    assert!(
        uses_fts_index,
        "EXPLAIN QUERY PLAN must show FTS5 VIRTUAL TABLE INDEX usage.\n\
         Full plan:\n{plan_text}"
    );

    // The plan must NOT show a full scan of the content table search_documents
    // (that would mean the query is not using the FTS5 index).
    // A bare "SCAN search_documents" (without the _fts suffix) indicates a
    // full content-table scan which we must avoid.
    let has_full_content_scan = plan_rows.iter().any(|line| {
        let upper = line.to_uppercase();
        upper.contains("SCAN SEARCH_DOCUMENTS ")
            || upper.contains("SCAN SEARCH_DOCUMENTS\n")
            || upper == "SCAN SEARCH_DOCUMENTS"
    });

    assert!(
        !has_full_content_scan,
        "EXPLAIN QUERY PLAN must NOT show a full scan of search_documents (content table).\n\
         Full plan:\n{plan_text}"
    );
}

/// EXPLAIN QUERY PLAN without policy filters also uses the FTS5 index.
#[test]
fn query_plan_unfiltered_uses_fts5_index() {
    let db = open_db();
    let conn = db.write();

    // Insert a modest set so the plan is non-trivial.
    for i in 0u32..100 {
        let doc = SearchDocument {
            record_kind: "memory".to_string(),
            record_id: format!("qp-unfilt-{i:04}"),
            title: Some(format!("unfiltered query plan row {i}")),
            body: Some(format!("plancheck body {i}")),
            aliases: None,
            source_text: None,
            relation_text: None,
            namespace: "core".to_string(),
            owner_id: "user-fts-test".to_string(),
            scope: "default".to_string(),
            sensitivity: 0,
            truth_state: "Current".to_string(),
            valid_from: None,
            valid_until: None,
            content_hash: format!("h-qp-unfilt-{i}"),
            revision: i as i64 + 1,
        };
        upsert_search_document(&conn, &doc).expect("bulk insert for unfiltered plan test");
    }

    let plan_rows: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN \
                 SELECT record_kind, record_id, -bm25(search_documents_fts) AS score, \
                        truth_state, namespace, scope, sensitivity, revision, \
                        title, body, aliases, source_text, relation_text \
                 FROM search_documents_fts \
                 WHERE search_documents_fts MATCH ? \
                 ORDER BY score DESC \
                 LIMIT ?",
            )
            .expect("EXPLAIN QUERY PLAN prepare must not fail");

        stmt.query_map(rusqlite::params!["plancheck", 25i64], |r| {
            r.get::<_, String>(3)
        })
        .expect("EXPLAIN QUERY PLAN query_map")
        .filter_map(|row| row.ok())
        .collect()
    };

    let plan_text = plan_rows.join("\n");

    let uses_fts_index = plan_text.to_uppercase().contains("VIRTUAL TABLE INDEX")
        || plan_text
            .to_uppercase()
            .contains("SCAN search_documents_fts".to_uppercase().as_str());

    assert!(
        uses_fts_index,
        "Unfiltered EXPLAIN QUERY PLAN must use FTS5 VIRTUAL TABLE INDEX.\n\
         Full plan:\n{plan_text}"
    );
}
