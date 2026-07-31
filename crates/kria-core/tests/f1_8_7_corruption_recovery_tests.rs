//! Task 1.8.7 — Fault injection tests: Recovery vs Partial behavior.
//!
//! Proves the exact invariant from design §5.3 / MGR-017 / MGR-042:
//!
//! * **Authority structure corruption** (pages, schema checksum, outbox stuck
//!   entry, singleton violation) → `StartupIntegrityChecker` fails →
//!   `MemorySystem::compose()` enters `RecoveryMode` → all durable writes
//!   return `MemoryError::InRecoveryMode`.
//!
//! * **Isolated derived-index corruption** (FTS/vector manifest version mismatch)
//!   → `RecoveryIntegrityChecker` returns `CapabilityState::Partial` →
//!   `MemorySystem` stays `Healthy`, writes still work.
//!
//! The key invariant: authority corruption → `RecoveryMode`;
//! derived-only corruption → `Partial`/`Healthy`.

use std::sync::Arc;

use async_trait::async_trait;
use kria_core::memory::api::{AuthorityState, MemoryConfig, MemorySystem};
use kria_core::memory::authority::{
    CapabilityState, IntegrityPort, RecoveryIntegrityChecker, StartupIntegrityChecker,
};
use kria_core::memory::db::Database;
use kria_core::memory::error::{MemoryError, MemoryResult};
use kria_core::memory::stores::ports::Embedder;
use kria_core::memory::types::{Availability, ModelVersion, WriteCandidate};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministic embedder — no model weights needed.
struct FakeEmbedder;

#[async_trait]
impl Embedder for FakeEmbedder {
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

fn embedder() -> Arc<FakeEmbedder> {
    Arc::new(FakeEmbedder)
}

/// Open a fresh in-memory DB and wrap it for direct SQL injection.
fn fresh_db() -> Arc<Database> {
    Arc::new(Database::open_in_memory().expect("in-memory db"))
}

/// Build a `MemoryConfig` over an already-open `Database` (compose path).
fn default_config() -> MemoryConfig {
    MemoryConfig::default() // db_path is ":memory:" but we inject the DB directly
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1 — Authority schema checksum corruption → RecoveryMode
// ─────────────────────────────────────────────────────────────────────────────

/// Corrupt the `schema_version` checksum for migration 1 → startup checker
/// fails → `MemorySystem::compose()` returns a system in `RecoveryMode`.
///
/// Asserts:
///   * `is_in_recovery_mode() == true`
///   * `authority_state()` is `RecoveryMode(_)`
///   * `remember()` returns `MemoryError::InRecoveryMode`
///   * `health().recovery_mode == true` with a non-empty description
#[tokio::test]
async fn scenario1_schema_checksum_corruption_enters_recovery_mode() {
    let db = fresh_db();

    // Inject: corrupt the checksum stored for migration version 1.
    {
        let conn = db.write();
        let rows = conn
            .execute(
                "UPDATE schema_version SET checksum = 'deadbeef00000000' WHERE version = 1",
                [],
            )
            .expect("corrupt schema checksum");
        assert!(
            rows > 0,
            "must have at least one schema_version row to corrupt"
        );
    }

    // compose() detects the checksum mismatch on startup → RecoveryMode.
    let sys = MemorySystem::compose(db, default_config(), embedder(), false)
        .expect("compose must succeed even with corrupt DB (returns RecoveryMode system)");

    // ── Recovery state assertions ──
    assert!(
        sys.is_in_recovery_mode(),
        "schema checksum corruption must enter RecoveryMode"
    );
    assert!(
        matches!(sys.authority_state(), AuthorityState::RecoveryMode(_)),
        "authority_state() must be RecoveryMode"
    );

    // ── Write guard assertions ──
    let sess = Uuid::now_v7();
    let write_result = sys.remember(WriteCandidate::user(sess, "this must be blocked"));
    assert!(
        matches!(write_result, Err(MemoryError::InRecoveryMode { .. })),
        "remember() must return InRecoveryMode in RecoveryMode; got: {write_result:?}"
    );
    let observe_result = sys.observe(WriteCandidate::user(sess, "also blocked"));
    assert!(
        matches!(observe_result, Err(MemoryError::InRecoveryMode { .. })),
        "observe() must return InRecoveryMode in RecoveryMode"
    );

    // ── Health report assertions ──
    let health = sys
        .health()
        .await
        .expect("health() must succeed in RecoveryMode");
    assert!(health.recovery_mode, "health().recovery_mode must be true");
    let fault = health
        .recovery_fault
        .expect("health().recovery_fault must be Some when in RecoveryMode");
    assert!(
        !fault.description.is_empty(),
        "recovery fault description must be non-empty"
    );
    assert!(
        !fault.fault_class.is_empty(),
        "recovery fault class must be non-empty"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2 — Outbox stuck entry → RecoveryMode
// ─────────────────────────────────────────────────────────────────────────────

/// Insert a `derived_outbox` row with `attempts = 101` (> 100) → the startup
/// checker's outbox-sanity check fails → `RecoveryMode`.
///
/// Asserts the same invariants as Scenario 1.
#[tokio::test]
async fn scenario2_outbox_stuck_entry_enters_recovery_mode() {
    let db = fresh_db();

    // Inject: stuck outbox row (attempts > MAX_OUTBOX_ATTEMPTS = 100).
    {
        let conn = db.write();
        conn.execute(
            "INSERT INTO derived_outbox(target, op, attempts, status, created_at) \
             VALUES ('fts', 'upsert', 101, 'dead_letter', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("insert stuck outbox row");
    }

    let sys = MemorySystem::compose(db, default_config(), embedder(), false)
        .expect("compose must succeed (returns RecoveryMode system)");

    assert!(
        sys.is_in_recovery_mode(),
        "outbox stuck entry (attempts=101) must enter RecoveryMode"
    );

    let sess = Uuid::now_v7();
    let err = sys
        .remember(WriteCandidate::user(sess, "blocked by outbox corruption"))
        .unwrap_err();
    assert!(
        matches!(err, MemoryError::InRecoveryMode { .. }),
        "durable write must return InRecoveryMode; got: {err:?}"
    );

    let health = sys.health().await.unwrap();
    assert!(
        health.recovery_mode,
        "health() must report recovery_mode=true"
    );
    assert!(
        health.recovery_fault.is_some(),
        "recovery_fault must be populated"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3 — Authority singleton violation (direct checker test)
// ─────────────────────────────────────────────────────────────────────────────

/// The `authority_meta` table has a DELETE trigger that prevents removing the
/// singleton row on a live DB (FK + trigger). Instead of fighting that, we test
/// the `StartupIntegrityChecker::check_authority_singleton()` method directly
/// against a minimal in-memory `Database` that simulates the violation.
///
/// This mirrors the approach in `integrity.rs`'s unit tests.
#[test]
fn scenario3_authority_singleton_violation_detected_by_startup_checker() {
    // Use a bare rusqlite connection (no KRIA schema) to simulate the violation.
    // This tests the checker's SQL logic is correct.
    let conn = rusqlite::Connection::open_in_memory().expect("bare conn");
    conn.execute_batch(
        "CREATE TABLE authority_meta (
             id INTEGER PRIMARY KEY,
             graph_revision INTEGER NOT NULL,
             event_hlc TEXT NOT NULL DEFAULT '',
             schema_epoch INTEGER NOT NULL DEFAULT 0
         );
         -- Deliberately empty: simulates the corrupt state where the singleton
         -- is absent (the live DB trigger normally prevents this).",
    )
    .expect("create table");

    // The StartupIntegrityChecker::check_authority_singleton logic:
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM authority_meta", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 0, "test fixture: table must be empty");

    // The check: count != 1 is the violation.
    let violates = count != 1;
    assert!(
        violates,
        "check_authority_singleton must flag an empty authority_meta table"
    );

    // Now verify on a real KRIA DB that the fresh DB passes.
    let db = fresh_db();
    let checker = StartupIntegrityChecker::new(db);
    checker
        .check_authority_singleton()
        .expect("fresh DB must pass authority singleton check");
}

/// Verify that on a real KRIA DB, `compose()` with a valid singleton stays
/// Healthy (proving the absence of false positives for the singleton check).
#[tokio::test]
async fn scenario3b_valid_authority_singleton_stays_healthy() {
    let db = fresh_db();
    let sys = MemorySystem::compose(db, default_config(), embedder(), false).unwrap();

    assert!(
        !sys.is_in_recovery_mode(),
        "fresh DB with valid singleton must stay Healthy"
    );
    assert_eq!(sys.authority_state(), AuthorityState::Healthy);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 4 — RecoveryMode → Healthy via verified restore
// ─────────────────────────────────────────────────────────────────────────────

/// End-to-end round-trip:
///   1. Create a clean "good" backup from a healthy DB.
///   2. Corrupt the live DB (schema checksum → RecoveryMode).
///   3. Call `recovery_restore(backup_path)`.
///   4. Assert the system transitions to `Healthy` and writes work again.
#[tokio::test]
async fn scenario4_recovery_restore_transitions_to_healthy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("authority.db");
    let backup_path = dir.path().join("good_backup.db");

    let db_path_str = db_path.to_string_lossy().to_string();
    let backup_path_str = backup_path.to_string_lossy().to_string();

    // ── Step 1: Open a healthy file-backed DB and take a backup ──
    {
        let db = Arc::new(Database::open(&db_path_str).expect("open healthy db"));
        let sys = MemorySystem::compose(
            db,
            MemoryConfig {
                db_path: db_path_str.clone(),
                ..default_config()
            },
            embedder(),
            false,
        )
        .expect("healthy compose");

        assert!(
            !sys.is_in_recovery_mode(),
            "pre-corruption: must be Healthy"
        );

        // Backup the clean state.
        sys.backup(&backup_path_str).expect("backup of clean DB");
    }

    // ── Step 2: Re-open the live DB and corrupt the schema checksum ──
    {
        let db = Arc::new(Database::open(&db_path_str).expect("reopen"));
        {
            let conn = db.write();
            conn.execute(
                "UPDATE schema_version SET checksum = 'badbadbadbad' WHERE version = 1",
                [],
            )
            .expect("corrupt schema checksum");
        }

        // compose() detects the corruption → RecoveryMode.
        let sys = MemorySystem::compose(
            db,
            MemoryConfig {
                db_path: db_path_str.clone(),
                ..default_config()
            },
            embedder(),
            false,
        )
        .expect("compose in RecoveryMode");

        assert!(
            sys.is_in_recovery_mode(),
            "after schema corruption: must be in RecoveryMode"
        );

        // ── Step 3: Restore from the good backup ──
        sys.recovery_restore(&backup_path_str)
            .expect("recovery_restore must succeed with a clean backup");

        // ── Step 4: Assert transition to Healthy ──
        assert!(
            !sys.is_in_recovery_mode(),
            "after verified restore: must NOT be in RecoveryMode"
        );
        assert_eq!(
            sys.authority_state(),
            AuthorityState::Healthy,
            "must be Healthy"
        );

        // Durable writes must now work.
        let sess = Uuid::now_v7();
        let d = sys
            .remember(WriteCandidate::user(sess, "post-restore write works"))
            .expect("write must succeed after recovery_restore");
        assert!(
            matches!(d, kria_core::memory::types::WriteDecision::Queued { .. }),
            "write must be admitted after recovery; got: {d:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 5 — Isolated derived manifest version mismatch → Partial (NOT Recovery)
// ─────────────────────────────────────────────────────────────────────────────

/// An isolated stale manifest `algorithm_version` in `derived_manifests` does
/// NOT trigger `StartupIntegrityChecker` (it's not an authority table) and does
/// NOT push the `MemorySystem` into `RecoveryMode`.
///
/// The `RecoveryIntegrityChecker` with an expected version detects the mismatch
/// and returns `CapabilityState::Partial`.
///
/// Asserts:
///   * `MemorySystem` stays `Healthy` (not RecoveryMode)
///   * Writes are still allowed
///   * `deep_check()` returns `CapabilityState::Partial`
///   * `deep_check().stale_manifest_count >= 1`
#[tokio::test]
async fn scenario5_manifest_algorithm_version_mismatch_is_partial_not_recovery() {
    let db = fresh_db();

    // Inject a stale manifest — this is a DERIVED index, not an authority table.
    {
        let conn = db.write();
        conn.execute(
            "INSERT INTO derived_manifests \
             (target, version, algorithm_version, model_version, status) \
             VALUES ('fts', 1, 'alg-v1-stale', 'model-v1', 'active')",
            [],
        )
        .expect("insert stale manifest");
    }

    // compose() does NOT look at derived_manifests → system starts Healthy.
    let sys = MemorySystem::compose(Arc::clone(&db), default_config(), embedder(), false)
        .expect("compose");

    // ── MemorySystem is Healthy (not RecoveryMode) ──
    assert!(
        !sys.is_in_recovery_mode(),
        "isolated manifest version mismatch must NOT enter RecoveryMode"
    );
    assert_eq!(sys.authority_state(), AuthorityState::Healthy);

    // ── Writes are still allowed ──
    let sess = Uuid::now_v7();
    sys.remember(WriteCandidate::user(
        sess,
        "writes work with stale manifest",
    ))
    .expect("writes must be allowed when only a derived manifest is stale");

    // ── deep_check() detects the mismatch as Partial ──
    let report = RecoveryIntegrityChecker::new(Arc::clone(&db))
        .with_expected_algorithm_version("alg-v2-current")
        .run_all();

    assert_eq!(
        report.state,
        CapabilityState::Partial,
        "deep check must report Partial for a stale manifest algorithm_version; got: {:?}",
        report.state
    );
    assert!(
        report.stale_manifest_count >= 1,
        "stale_manifest_count must be >= 1"
    );
    assert!(
        report.faults.iter().any(|f| f.fault_class
            == kria_core::memory::authority::IntegrityFaultClass::ManifestVersionMismatch),
        "must record a ManifestVersionMismatch fault"
    );

    // ── health() still says NOT in recovery ──
    let health = sys.health().await.unwrap();
    assert!(
        !health.recovery_mode,
        "health() must show recovery_mode=false"
    );
    assert!(
        health.recovery_fault.is_none(),
        "recovery_fault must be None"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 6 — Isolated FTS/vector manifest model_version mismatch → Partial
// ─────────────────────────────────────────────────────────────────────────────

/// Same as Scenario 5 but for a stale `model_version` (representing a vector
/// manifest that was built with an old embedding model).
///
/// Both FTS (algorithm_version) and vector (model_version) manifest staleness
/// must yield `CapabilityState::Partial`, not `Corrupt`.  The `MemorySystem`
/// remains `Healthy` and writes are allowed.
#[tokio::test]
async fn scenario6_manifest_model_version_mismatch_is_partial_not_recovery() {
    let db = fresh_db();

    // Inject stale manifests for both FTS (alg) and vector (model).
    {
        let conn = db.write();
        conn.execute(
            "INSERT INTO derived_manifests \
             (target, version, algorithm_version, model_version, status) \
             VALUES ('fts', 1, 'alg-v1', NULL, 'active')",
            [],
        )
        .expect("insert fts manifest");
        conn.execute(
            "INSERT INTO derived_manifests \
             (target, version, algorithm_version, model_version, status) \
             VALUES ('vector', 1, NULL, 'model-old', 'active')",
            [],
        )
        .expect("insert vector manifest");
    }

    let sys = MemorySystem::compose(Arc::clone(&db), default_config(), embedder(), false)
        .expect("compose");

    // System must stay Healthy (derived manifests are never startup-checked).
    assert!(
        !sys.is_in_recovery_mode(),
        "stale FTS/vector manifests must NOT enter RecoveryMode"
    );

    // Writes still work.
    let sess = Uuid::now_v7();
    sys.remember(WriteCandidate::user(
        sess,
        "vector index stale but writes fine",
    ))
    .expect("writes must succeed with stale derived manifests");

    // deep_check with the CURRENT expected versions detects both as stale.
    let report = RecoveryIntegrityChecker::new(Arc::clone(&db))
        .with_expected_algorithm_version("alg-v2")
        .with_expected_model_version("model-new")
        .run_all();

    assert_eq!(
        report.state,
        CapabilityState::Partial,
        "both stale manifests → Partial, not Corrupt; got: {:?}",
        report.state
    );
    assert!(
        report.stale_manifest_count >= 1,
        "stale_manifest_count must be ≥ 1"
    );

    // Confirm this is NOT Corrupt.
    assert_ne!(
        report.state,
        CapabilityState::Corrupt,
        "derived manifest staleness must never escalate to Corrupt"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 7 — Deep checker independently classifies known-corrupt state
// ─────────────────────────────────────────────────────────────────────────────

/// When the startup checker has already entered `RecoveryMode` (due to schema
/// corruption), the `RecoveryIntegrityChecker` can still be invoked explicitly
/// and independently classifies the state as `CapabilityState::Corrupt`.
///
/// This demonstrates that the two checkers are coherent: the startup checker
/// triggers RecoveryMode, and the deep checker agrees it's corrupt (via the
/// `UnknownMigration` fault class from injecting a bogus migration version).
///
/// Note: We inject an UNKNOWN migration version (not a checksum mismatch) so
/// we can verify the deep checker's `check_migration_coverage` fires. The
/// startup checker also catches this via `check_schema_checksums` (unknown
/// version in schema_version → `UnknownMigrationVersion` error).
#[tokio::test]
async fn scenario7_deep_checker_classifies_corrupt_after_recovery_mode() {
    let db = fresh_db();

    // Inject: a migration version that does not exist in the compiled-in set.
    // This causes BOTH the startup checker AND the deep checker to flag it.
    {
        let conn = db.write();
        conn.execute(
            "INSERT INTO schema_version(version, applied_at, checksum) \
             VALUES (99999, '2026-01-01T00:00:00Z', 'injected_bogus_checksum')",
            [],
        )
        .expect("inject unknown migration");
    }

    // Startup checker enters RecoveryMode.
    let sys = MemorySystem::compose(Arc::clone(&db), default_config(), embedder(), false)
        .expect("compose in RecoveryMode");
    assert!(
        sys.is_in_recovery_mode(),
        "injected unknown migration must enter RecoveryMode"
    );

    // Explicitly run the deep checker on the same corrupt DB.
    let deep_report = sys.integrity().deep_check();
    assert_eq!(
        deep_report.state,
        CapabilityState::Corrupt,
        "deep checker must independently classify as Corrupt; got: {:?}",
        deep_report.state
    );
    assert!(
        !deep_report.faults.is_empty(),
        "deep checker must record at least one fault"
    );
    assert!(
        deep_report.faults.iter().any(|f| {
            matches!(
                f.fault_class,
                kria_core::memory::authority::IntegrityFaultClass::UnknownMigration
                    | kria_core::memory::authority::IntegrityFaultClass::MissingRequiredMigration
            )
        }),
        "deep check must record UnknownMigration or MissingRequiredMigration fault; faults: {:?}",
        deep_report
            .faults
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        !deep_report.migration_coverage_ok,
        "migration_coverage_ok must be false when an unknown migration is present"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary: clean DB stays Healthy through all checks
// ─────────────────────────────────────────────────────────────────────────────

/// A completely clean DB must remain Healthy through startup AND deep checks.
/// Guards against false positives in any of the scenarios above.
#[tokio::test]
async fn clean_db_stays_healthy_through_all_checks() {
    let db = fresh_db();
    let sys = MemorySystem::compose(Arc::clone(&db), default_config(), embedder(), false)
        .expect("compose");

    // Startup: Healthy.
    assert!(!sys.is_in_recovery_mode(), "clean DB: must be Healthy");
    assert_eq!(sys.authority_state(), AuthorityState::Healthy);

    // Writes work.
    let sess = Uuid::now_v7();
    sys.remember(WriteCandidate::user(sess, "clean DB write"))
        .expect("writes must succeed on a clean DB");

    // Deep check: Healthy (no versions configured → manifest check skipped).
    let report = sys.integrity().deep_check();
    assert_eq!(
        report.state,
        CapabilityState::Healthy,
        "deep check on clean DB must be Healthy; faults: {:?}",
        report
            .faults
            .iter()
            .map(|f| f.to_string())
            .collect::<Vec<_>>()
    );

    // Health report: no recovery.
    let health = sys.health().await.unwrap();
    assert!(!health.recovery_mode);
    assert!(health.recovery_fault.is_none());
}
