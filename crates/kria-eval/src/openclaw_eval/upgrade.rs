//! R19 — upgrade / migration compatibility (tasks.md task 19).
//!
//! FIXED this session (post user sign-off): `registry.rs` now has a real,
//! versioned migration system (`SCHEMA_VERSION`, `MIGRATIONS`,
//! `run_migrations`, driven by `PRAGMA user_version`). `CREATE TABLE IF NOT
//! EXISTS` alone was always a no-op against an existing older-schema
//! database — the fix is the migration RUNS ON EVERY `ProductionSkillRegistry::new`
//! call, after base-schema creation, and applies any pending additive
//! `ALTER TABLE` step. Migration 1 adds the `granted_capabilities` column
//! (needed for the capability-grant-wiring fix) to any pre-existing
//! database that predates it.
//!
//! `validate_no_migration_exists_for_older_schema` below is KEPT (renamed in
//! spirit, not deleted) as the real regression proof: it still constructs a
//! genuinely older schema missing a real column, but now demonstrates the
//! FIX — opening it with current code actually adds the column and a real
//! install succeeds, using the real data, not a workaround.

use rusqlite::Connection;

/// Creates a deliberately OLDER version of the real `skills` table — every
/// column that predates the capability-grant-wiring migration, but missing
/// `granted_capabilities` (the exact column real migration 1 adds) — to
/// simulate a genuine pre-upgrade user database (any real `~/.kria/skills.db`
/// created before this session's migration-system fix).
fn create_older_schema_skills_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE skills (
            skill_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            publisher TEXT NOT NULL,
            version TEXT NOT NULL,
            category TEXT NOT NULL,
            discovery_source TEXT NOT NULL,
            discovered_at TEXT NOT NULL,
            capabilities TEXT NOT NULL,
            runtime_requirements TEXT NOT NULL,
            risk_level TEXT NOT NULL,
            resource_class TEXT NOT NULL,
            tags TEXT NOT NULL,
            categories TEXT NOT NULL,
            semantic_version TEXT NOT NULL,
            dependencies TEXT NOT NULL,
            compatibility_requirements TEXT NOT NULL,
            trust_tier TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            signature TEXT,
            -- granted_capabilities INTENTIONALLY OMITTED (pre-migration-1 schema)
            bundle_path TEXT,
            manifest_toml TEXT,
            state TEXT NOT NULL DEFAULT 'discovered',
            state_changed_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())
}

/// R19.2 real FIX proof: open a genuinely older-schema database (missing
/// `granted_capabilities`, pre-migration-1) with the CURRENT
/// `ProductionSkillRegistry`, and confirm the real, versioned migration
/// system actually adds the missing column via a real `ALTER TABLE`, and a
/// real subsequent install (which writes `granted_capabilities`) succeeds —
/// proving forward migration now genuinely works, not just claims to.
pub fn validate_no_migration_exists_for_older_schema() -> Result<UpgradeFindings, String> {
    use kria_core::openclaw::registry::ProductionSkillRegistry;

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("older_schema.db");

    {
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        create_older_schema_skills_table(&conn).map_err(|e| e.to_string())?;
    }

    // Open with the CURRENT registry code — this now runs `run_migrations`
    // after base-schema creation, which must apply migration 1 to THIS
    // existing, older-schema database file.
    let registry = ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?;

    // Confirm the column now EXISTS after opening (proving the real
    // migration ran, not just `CREATE TABLE IF NOT EXISTS`).
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    let has_column: bool = conn
        .prepare("SELECT granted_capabilities FROM skills LIMIT 1")
        .is_ok();
    drop(conn);

    // A real install (which writes granted_capabilities) must now succeed —
    // this is the real operation that used to fail for an upgrading user.
    let sample = build_sample_skill_metadata();
    let install_result = registry.install_skill(&sample);

    Ok(UpgradeFindings {
        column_added_by_open: has_column,
        install_succeeded_despite_missing_column: install_result.is_ok(),
        install_error: install_result.err().map(|e| e.to_string()),
    })
}

#[derive(Debug)]
pub struct UpgradeFindings {
    pub column_added_by_open: bool,
    pub install_succeeded_despite_missing_column: bool,
    pub install_error: Option<String>,
}

fn build_sample_skill_metadata() -> kria_core::openclaw::registry::SkillMetadata {
    use kria_core::openclaw::registry::{DiscoverySource, SkillMetadata, SkillState};
    use kria_core::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use kria_core::safety::RiskLevel;

    SkillMetadata {
        skill_id: "oc_upgrade_fixture".into(),
        name: "Upgrade Fixture".into(),
        description: "R19 upgrade-compatibility fixture.".into(),
        publisher: "test".into(),
        version: "1.0.0".into(),
        category: "test".into(),
        discovery_source: DiscoverySource::Bundled {
            path: "test".into(),
        },
        discovered_at: chrono::Utc::now(),
        capabilities: SkillCapabilities::default(),
        runtime_requirements: "docker".into(),
        risk_level: RiskLevel::Green,
        resource_class: ResourceClass::Light,
        tags: vec![],
        categories: vec![],
        semantic_version: "1.0.0".into(),
        dependencies: vec![],
        compatibility_requirements: vec![],
        trust_tier: TrustTier::Local,
        content_hash: "hash".into(),
        signature: None,
        granted_capabilities: Vec::new(),
        bundle_path: None,
        manifest_toml: None,
        input_schema: None,
        state: SkillState::Discovered,
        state_changed_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R19 FIX proof: opening a genuinely older-schema database (missing
    /// `granted_capabilities`) with current code must now add the missing
    /// column via the real versioned migration, and a real subsequent
    /// install must succeed. This is the real regression test for the
    /// capability-grant-wiring + schema-migration fix — if this ever
    /// regresses to `column_added_by_open == false`, the migration system
    /// broke and this must be investigated as a real production bug, not
    /// silenced.
    #[test]
    fn real_migration_brings_older_schema_forward() {
        let findings =
            validate_no_migration_exists_for_older_schema().expect("test setup must succeed");
        eprintln!("[R19] {findings:?}");

        assert!(
            findings.column_added_by_open,
            "REGRESSION: the real versioned migration system must add granted_capabilities to \
             an older-schema database on open — if this fails, run_migrations() broke"
        );
        assert!(
            findings.install_succeeded_despite_missing_column,
            "REGRESSION: a real install must succeed after migration brings the schema forward: {:?}",
            findings.install_error
        );
    }
}
