//! Live functional tests for the OpenClaw skill registry.
//!
//! These tests run against a real temporary skills.db and prove the full
//! command-layer flow: list → toggle → uninstall → query audit_log.

use kria_core::openclaw::audit::AuditLedger;
use kria_core::openclaw::init::initialize_curated_skills;
use kria_core::openclaw::registry::SkillRegistry;
use rusqlite::Connection;
use tempfile::TempDir;

fn boot(dir: &TempDir) -> (SkillRegistry, AuditLedger) {
    let db = dir.path().join("skills.db");
    let registry = SkillRegistry::open(&db).unwrap();
    let audit = AuditLedger::open(&db, b"test-key".to_vec()).unwrap();
    initialize_curated_skills(&registry);
    (registry, audit)
}

#[test]
fn live_list_installed_returns_3_curated() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = boot(&dir);

    let skills = registry.list_installed().unwrap();
    assert_eq!(skills.len(), 3);

    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Calculator"));
    assert!(names.contains(&"Web Search"));
    assert!(names.contains(&"Web Fetch"));
    println!("✓ list_installed: {:?}", names);
}

#[test]
fn live_search_filter_by_query() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = boot(&dir);

    let all = registry.list_installed().unwrap();

    // Simulate clawhub_search_skills logic
    let q = "web".to_lowercase();
    let matched: Vec<_> = all
        .iter()
        .filter(|s| {
            s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
                || s.category.to_lowercase().contains(&q)
        })
        .collect();

    assert_eq!(
        matched.len(),
        2,
        "web query should match Web Search + Web Fetch"
    );
    println!("✓ search 'web': {} results", matched.len());

    // Empty query returns all
    let q2 = "".to_lowercase();
    let all_results: Vec<_> = all
        .iter()
        .filter(|s| q2.is_empty() || s.name.to_lowercase().contains(&q2))
        .collect();
    assert_eq!(all_results.len(), 3);
    println!("✓ search '': {} results (all)", all_results.len());
}

#[test]
fn live_toggle_disables_and_reenables_skill() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = boot(&dir);

    // Disable
    registry.toggle("oc_calculator", false).unwrap();
    let active = registry.list_active().unwrap();
    assert!(!active.iter().any(|s| s.skill_id == "oc_calculator"));
    println!("✓ toggle off: oc_calculator not in active list");

    // Re-enable
    registry.toggle("oc_calculator", true).unwrap();
    let active2 = registry.list_active().unwrap();
    assert!(active2.iter().any(|s| s.skill_id == "oc_calculator"));
    println!("✓ toggle on: oc_calculator back in active list");
}

#[test]
fn live_uninstall_removes_skill() {
    let dir = TempDir::new().unwrap();
    let (registry, _) = boot(&dir);

    registry.uninstall("oc_web_fetch").unwrap();
    let skills = registry.list_installed().unwrap();
    assert_eq!(skills.len(), 2);
    assert!(!skills.iter().any(|s| s.skill_id == "oc_web_fetch"));
    println!("✓ uninstall: oc_web_fetch removed, {} remain", skills.len());

    // Uninstalling again should error
    assert!(registry.uninstall("oc_web_fetch").is_err());
    println!("✓ double-uninstall: correctly returns error");
}

#[test]
fn live_audit_log_table_queryable_via_raw_sql() {
    let dir = TempDir::new().unwrap();
    let (registry, _audit) = boot(&dir);

    // Record an invocation to confirm use_count is tracked
    registry.record_invocation("oc_calculator").unwrap();
    registry.record_invocation("oc_calculator").unwrap();

    // Verify via raw SQLite (simulates: sqlite3 skills.db "SELECT ...")
    let db_path = dir.path().join("skills.db");
    let conn = Connection::open(&db_path).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "audit_log is empty (entries written only by OpenClawToolHandler during live invocations)"
    );

    // Invocation counts live in `skill_statistics.usage_count`. This query used to read
    // `installed_skills.use_count` — a table and column that do not exist in the schema
    // (`registry.rs` creates `skills`, `skill_statistics`, `capability_profiles`,
    // `market_catalog`). The test was failing with "no such table" rather than telling
    // anyone the count was wrong, so it was reporting a schema mismatch in itself.
    let use_count: i64 = conn
        .query_row(
            "SELECT usage_count FROM skill_statistics WHERE skill_id = 'oc_calculator'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(use_count, 2);
    println!(
        "✓ audit_log exists and queryable; oc_calculator usage_count = {}",
        use_count
    );
}
