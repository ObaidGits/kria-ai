//! Decisive smoke test for the registry-empty ("No enabled skills found in
//! registry") production bug, run against a COPY of a REAL user `skills.db`.
//!
//! Gated on the `KRIA_REAL_SKILLS_DB` env var (absolute path to a *copy* of a
//! real `~/.kria/skills.db`). Skips cleanly when unset so normal `cargo test`
//! runs are unaffected and no real user file is ever touched by CI.
//!
//! This exists because the bug ONLY manifests on a database that was created
//! before the `granted_capabilities` column existed and later migrated (the
//! column gets appended at the end of the table). Synthetic fresh DBs hide it.

use kria_core::openclaw::registry::ProductionSkillRegistry;

#[test]
fn real_user_db_returns_enabled_skills() {
    let Some(src) = std::env::var_os("KRIA_REAL_SKILLS_DB") else {
        eprintln!("KRIA_REAL_SKILLS_DB unset — skipping real-DB smoke test");
        return;
    };
    let src = std::path::PathBuf::from(src);
    assert!(
        src.exists(),
        "KRIA_REAL_SKILLS_DB does not exist: {}",
        src.display()
    );

    let registry = ProductionSkillRegistry::open(&src).expect("open real skills.db copy");
    let enabled = registry.get_enabled_skills().expect("get_enabled_skills");

    eprintln!("real DB enabled skills = {}", enabled.len());
    for s in &enabled {
        eprintln!("  - {} ({:?})", s.skill_id, s.state);
    }
    assert!(
        !enabled.is_empty(),
        "real user DB reported ZERO enabled skills — the registry-empty bug is present"
    );
}
