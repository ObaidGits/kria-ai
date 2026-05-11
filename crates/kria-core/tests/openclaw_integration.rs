//! OpenClaw integration tests.
//!
//! These tests verify the full install → invoke → lifecycle pipeline
//! using a temporary in-memory-equivalent SQLite registry.

use kria_core::openclaw::init::initialize_curated_skills;
use kria_core::openclaw::registry::SkillRegistry;
use tempfile::TempDir;

fn temp_registry() -> (TempDir, SkillRegistry) {
    let dir = TempDir::new().expect("failed to create temp dir");
    let db_path = dir.path().join("test_skills.db");
    let registry = SkillRegistry::open(&db_path).expect("failed to open registry");
    (dir, registry)
}

#[test]
fn integration_oc_install_and_invoke() {
    let (_dir, registry) = temp_registry();

    // Seed curated skills
    initialize_curated_skills(&registry);

    // Verify all 3 curated skills are installed
    let skills = registry.list_installed().expect("list_installed failed");
    assert_eq!(skills.len(), 3);

    let calc = registry.get("oc_calculator").expect("oc_calculator not found");
    assert_eq!(calc.name, "Calculator");
    assert!(calc.is_usable());

    // Record an invocation — the registry updates the use_count column
    registry
        .record_invocation("oc_calculator")
        .expect("record_invocation failed");

    // Verify record_invocation does not error and can be called multiple times
    registry
        .record_invocation("oc_calculator")
        .expect("second record_invocation failed");
}

#[test]
fn integration_oc_container_crash_recovery() {
    let (_dir, registry) = temp_registry();

    // Seed curated skills
    initialize_curated_skills(&registry);

    // Verify skill is active before crash
    let before = registry.get("oc_web_fetch").expect("get before crash failed");
    assert!(before.is_usable());

    // Simulate a "crash" by toggling the skill to disabled (quarantine)
    registry
        .toggle("oc_web_fetch", false)
        .expect("toggle to disabled failed");

    // Verify it's listed correctly when filtering by status
    let active_skills = registry.list_active().expect("list_active failed");
    let fetch_active = active_skills.iter().any(|s| s.skill_id == "oc_web_fetch");
    assert!(!fetch_active, "oc_web_fetch should not appear in active list after disable");

    // Recovery: re-enable the skill
    registry
        .toggle("oc_web_fetch", true)
        .expect("toggle to enabled failed");

    // Verify recovery: skill should now appear in active list again
    let active_after = registry.list_active().expect("list_active after recovery failed");
    let fetch_recovered = active_after.iter().any(|s| s.skill_id == "oc_web_fetch");
    assert!(fetch_recovered, "oc_web_fetch should be active after recovery");
}

#[test]
fn integration_oc_idempotent_seed() {
    let (_dir, registry) = temp_registry();

    // Seed twice — should not duplicate
    initialize_curated_skills(&registry);
    initialize_curated_skills(&registry);

    let skills = registry.list_installed().expect("list_installed failed");
    assert_eq!(skills.len(), 3);
}

#[test]
fn integration_oc_subsystem_boot_creates_both_tables() {
    use kria_core::openclaw::OpenClawSubsystem;
    use rusqlite::Connection;

    let dir = TempDir::new().expect("failed to create temp dir");

    // Boot the subsystem — this should create skills.db with both tables
    let subsystem = OpenClawSubsystem::boot(dir.path())
        .expect("OpenClawSubsystem::boot failed");

    // Verify skills were seeded
    let skills = subsystem.registry.list_installed().expect("list failed");
    assert_eq!(skills.len(), 3, "expected 3 curated skills after boot");

    // Directly query SQLite to confirm audit_log table exists and is queryable
    let db_path = dir.path().join("skills.db");
    let conn = Connection::open(&db_path).expect("failed to open skills.db");

    let table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='audit_log'",
            [],
            |row| row.get(0),
        )
        .expect("sqlite_master query failed");
    assert_eq!(table_count, 1, "audit_log table must exist after boot");

    // Verify we can query the audit_log (even if empty)
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))
        .expect("audit_log SELECT failed");
    assert_eq!(row_count, 0, "audit_log should be empty initially");
}
