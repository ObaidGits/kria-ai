//! First-boot capability-profile backfill + live registry subscription
//! (task 2.4, design §5 / §7 / §13.2).
//!
//! On the first flag-ON boot, the `capability_profiles` derived view is empty:
//! no skill has a profile yet. This module builds that view **without blocking
//! boot** and keeps it current thereafter, honoring the KRIA invariants:
//!
//! - **Registry is the sole source of truth.** Both the backfill job and the
//!   live subscriber only ever *read* `ProductionSkillRegistry` and *write* the
//!   derived `capability_profiles` table via [`ProfileStore`]. They never author
//!   skills, never introduce a second store, and never touch a second database.
//! - **Profiles are rebuildable derived views.** Every write is an idempotent
//!   `INSERT OR REPLACE` ([`ProfileStore::derive_and_persist`]); a removal is a
//!   plain `DELETE` ([`ProfileStore::delete_profile`]). Re-running the backfill
//!   from scratch converges to the same view.
//! - **Honest degraded fallback.** Until the backfill job reports
//!   [`BackfillStatus::is_complete`], the profile view is only *partial*. The
//!   facade/handler consults that signal and falls back to the frozen router
//!   (R13.2) rather than presenting a partial view as complete.
//! - **No uncontrolled loops.** The backfill is a single bounded pass over the
//!   enumerated skills (it cooperatively yields between skills so it never
//!   starves the runtime); the subscriber is driven purely by the frozen
//!   [`RegistryEvent`] broadcast and exits cleanly when the channel closes.
//!
//! # Readiness signal
//!
//! [`spawn_backfill`] returns an `Arc<BackfillStatus>` immediately (boot is never
//! blocked). The background task flips [`BackfillStatus::mark_complete`] only
//! after every enumerated skill has a persisted profile. Callers read
//! [`is_complete`](BackfillStatus::is_complete) (a lock-free `AtomicBool`) to
//! decide between the full CIL discovery path and the honest degraded fallback.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use super::extract::ProfileStore;
use super::CilError;
use crate::openclaw::registry::{ProductionSkillRegistry, RegistryEvent};

/// Completion/readiness signal for the first-boot backfill job (design §13.2).
///
/// Cheap to clone (via `Arc`) and lock-free to read. The facade/handler consults
/// [`is_complete`](Self::is_complete) before using the derived profile view:
/// while incomplete, discovery must fall back to the frozen router (honest
/// degraded). [`total`](Self::total) / [`processed`](Self::processed) expose
/// coarse progress for status surfaces without implying full fidelity.
#[derive(Debug, Default)]
pub struct BackfillStatus {
    complete: AtomicBool,
    total: AtomicUsize,
    processed: AtomicUsize,
}

impl BackfillStatus {
    /// A fresh, incomplete status (nothing processed yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` once every enumerated skill has a persisted profile. Until then,
    /// the derived view is partial and callers must degrade honestly.
    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    /// Total number of skills the backfill pass intends to process (known once
    /// enumeration completes).
    pub fn total(&self) -> usize {
        self.total.load(Ordering::Acquire)
    }

    /// Number of skills whose profile has been persisted so far.
    pub fn processed(&self) -> usize {
        self.processed.load(Ordering::Acquire)
    }

    fn set_total(&self, n: usize) {
        self.total.store(n, Ordering::Release);
    }

    fn incr_processed(&self) {
        self.processed.fetch_add(1, Ordering::AcqRel);
    }

    /// Mark the backfill complete. Called by the background job only after the
    /// full pass persists; a `Release` store pairs with the `Acquire` load in
    /// [`is_complete`], so a reader that observes `true` also observes every
    /// prior profile write.
    fn mark_complete(&self) {
        self.complete.store(true, Ordering::Release);
    }
}

/// Run one bounded backfill pass **synchronously**: enumerate the registry's
/// enabled skills, derive + persist a profile for each, and mark the status
/// complete. Deterministic and idempotent (`INSERT OR REPLACE`). Returns the
/// number of profiles persisted.
///
/// This is the unit the background task drives; it is exposed directly so tests
/// (and a caller that prefers to await completion) can run it without a spawn.
pub fn run_backfill(
    registry: &ProductionSkillRegistry,
    store: &ProfileStore,
    profile_epoch: i64,
    status: &BackfillStatus,
) -> Result<usize, CilError> {
    // Registry is the sole source of truth — read the current enabled set.
    let skills = registry
        .get_enabled_skills()
        .map_err(|e| CilError::Io(format!("enumerate skills for backfill: {e}")))?;
    status.set_total(skills.len());

    let mut count = 0usize;
    for meta in &skills {
        // Idempotent derived-view write; no embedding at extraction time.
        store.derive_and_persist(meta, None, profile_epoch)?;
        status.incr_processed();
        count += 1;
    }

    status.mark_complete();
    Ok(count)
}

/// Spawn the first-boot backfill as a bounded background job (design §5.1/§13.2).
///
/// Returns the readiness [`BackfillStatus`] **immediately** so boot is never
/// blocked; the returned `JoinHandle` is provided for lifecycle management and
/// may be ignored. Until [`BackfillStatus::is_complete`] flips, the caller must
/// fall back to the frozen router (honest degraded).
///
/// The pass cooperatively yields between skills (`tokio::task::yield_now`) so a
/// large registry never starves the async runtime — a bounded, controlled loop,
/// not an uncontrolled one.
pub fn spawn_backfill(
    registry: Arc<ProductionSkillRegistry>,
    store: Arc<ProfileStore>,
    profile_epoch: i64,
) -> (Arc<BackfillStatus>, JoinHandle<Result<usize, CilError>>) {
    let status = Arc::new(BackfillStatus::new());
    let job_status = Arc::clone(&status);
    let handle = tokio::spawn(async move {
        let skills = registry
            .get_enabled_skills()
            .map_err(|e| CilError::Io(format!("enumerate skills for backfill: {e}")))?;
        job_status.set_total(skills.len());

        let mut count = 0usize;
        for meta in &skills {
            store.derive_and_persist(meta, None, profile_epoch)?;
            job_status.incr_processed();
            count += 1;
            // Bounded/incremental: yield so backfill never blocks the runtime.
            tokio::task::yield_now().await;
        }

        job_status.mark_complete();
        Ok(count)
    });
    (status, handle)
}

/// Spawn a subscriber that keeps the derived profile view current from the
/// frozen [`RegistryEvent`] broadcast (design §5.3).
///
/// On install / enable / update / verify, it re-derives and upserts the skill's
/// profile; on remove, it deletes the profile row. Every action is an idempotent
/// derived-view write against the registry's authoritative state — the
/// subscriber authors nothing. The loop exits cleanly when the broadcast channel
/// closes ([`RecvError::Closed`]) and skips ahead on lag
/// ([`RecvError::Lagged`]) without dying, so a burst of events never stalls it.
pub fn spawn_registry_subscriber(
    registry: Arc<ProductionSkillRegistry>,
    store: Arc<ProfileStore>,
    profile_epoch: i64,
) -> JoinHandle<()> {
    let mut rx = registry.subscribe_events();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Err(e) = apply_event(&registry, &store, profile_epoch, &event) {
                        // Honest telemetry: a derived-view update failure is
                        // logged, never silently swallowed. The view stays
                        // rebuildable via a future full backfill.
                        tracing::warn!(
                            target: "openclaw::cil::backfill",
                            error = %e,
                            "failed to apply RegistryEvent to capability_profiles"
                        );
                    }
                }
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        target: "openclaw::cil::backfill",
                        skipped,
                        "registry event subscriber lagged; profile view may need a rebuild"
                    );
                }
            }
        }
    })
}

/// Apply a single [`RegistryEvent`] to the derived profile view. Upsert on
/// presence-implying events, delete on removal; other events (health/usage) are
/// no-ops for the profile view.
fn apply_event(
    registry: &ProductionSkillRegistry,
    store: &ProfileStore,
    profile_epoch: i64,
    event: &RegistryEvent,
) -> Result<(), CilError> {
    match event {
        // Skill is now present/current in the registry → (re)derive its profile.
        RegistryEvent::Installed { skill_id, .. }
        | RegistryEvent::Updated { skill_id, .. }
        | RegistryEvent::Enabled { skill_id }
        | RegistryEvent::Verified { skill_id }
        | RegistryEvent::Recovered { skill_id } => {
            let meta = registry
                .get_skill(skill_id)
                .map_err(|e| CilError::Io(format!("fetch skill {skill_id} for profile: {e}")))?;
            store.derive_and_persist(&meta, None, profile_epoch)
        }
        // Skill is gone → drop its derived row (idempotent).
        RegistryEvent::Removed { skill_id } => store.delete_profile(skill_id),
        // Health/usage/disable events do not change the provides/consumes view.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
    };
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use crate::safety::RiskLevel;

    /// Minimal enabled `SkillMetadata` for backfill smoke tests.
    fn sample_meta(skill_id: &str) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: format!("Skill {skill_id}"),
            description: "backfill smoke-test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "media".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec!["test".to_string()],
            categories: vec!["media.image".to_string()],
            semantic_version: "1.0.0".to_string(),
            dependencies: vec![],
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            // Enabled so get_enabled_skills() returns it.
            state: SkillState::Enabled,
            state_changed_at: chrono::Utc::now(),
        }
    }

    /// Register `n` skills (enabled) and return the registry + store over one db.
    fn setup(
        n: usize,
    ) -> (
        Arc<ProductionSkillRegistry>,
        Arc<ProfileStore>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        for i in 0..n {
            // sample_meta seeds state=Enabled, which install_skill honors, so
            // get_enabled_skills() returns it without a separate transition.
            let meta = sample_meta(&format!("acme.skill{i}"));
            registry.install_skill(&meta).expect("install skill");
        }
        let store = ProfileStore::open(&db_path).expect("profile store open");
        (Arc::new(registry), Arc::new(store), dir)
    }

    /// Backfill over a registry with N skills produces N profiles, and the
    /// readiness signal flips to complete.
    #[tokio::test]
    async fn backfill_produces_profile_per_skill() {
        let n = 5;
        let (registry, store, _dir) = setup(n);

        let (status, handle) = spawn_backfill(Arc::clone(&registry), Arc::clone(&store), 0);
        let count = handle.await.expect("join").expect("backfill ok");

        assert_eq!(count, n, "one profile persisted per enabled skill");
        assert!(status.is_complete(), "readiness signal flips to complete");
        assert_eq!(status.total(), n);
        assert_eq!(status.processed(), n);
        // Every skill has a persisted derived profile.
        for i in 0..n {
            let row = store
                .get_profile(&format!("acme.skill{i}"))
                .expect("get")
                .expect("profile present");
            assert_eq!(row.profile.skill_id, format!("acme.skill{i}"));
        }
    }

    /// Backfill is idempotent: a second pass converges to the same view.
    #[tokio::test]
    async fn backfill_is_idempotent() {
        let (registry, store, _dir) = setup(3);
        let status1 = BackfillStatus::new();
        let first = run_backfill(&registry, &store, 0, &status1).expect("first pass");
        let status2 = BackfillStatus::new();
        let second = run_backfill(&registry, &store, 1, &status2).expect("second pass");
        assert_eq!(first, second, "same skill count each pass");
        assert!(status2.is_complete());
        // Re-persist replaced the row (new epoch), single row per skill.
        let row = store
            .get_profile("acme.skill0")
            .expect("get")
            .expect("present");
        assert_eq!(row.profile_epoch, 1);
    }

    /// A RegistryEvent::Installed adds a profile via the subscriber; a
    /// RegistryEvent::Removed deletes it.
    #[tokio::test]
    async fn subscriber_upserts_on_install_and_deletes_on_remove() {
        let (registry, store, _dir) = setup(0);
        let _sub = spawn_registry_subscriber(Arc::clone(&registry), Arc::clone(&store), 0);

        // install_skill emits RegistryEvent::Installed → subscriber upserts.
        let meta = sample_meta("acme.new");
        registry.install_skill(&meta).expect("install");

        // Poll for eventual consistency (subscriber runs on the runtime).
        let mut present = false;
        for _ in 0..50 {
            if store.get_profile("acme.new").expect("get").is_some() {
                present = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(present, "install event added a profile");

        // uninstall sets state=Removed and emits RegistryEvent::Removed →
        // subscriber deletes the derived profile row.
        registry.uninstall("acme.new").expect("uninstall");
        let mut absent = false;
        for _ in 0..50 {
            if store.get_profile("acme.new").expect("get").is_none() {
                absent = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(absent, "remove event deleted the profile");
    }

    // =======================================================================
    // Task 16.4 — backfill-correctness + flag-off rollback drill
    //
    // Property 1  (Single source of truth)  — Validates: Requirements 5.1
    // Property 11 (Flag-off rollback)        — Validates: Requirements 7.2, 7.3
    //
    // These tests drive the BACKFILL job specifically (`run_backfill`) — the
    // path that materializes the `capability_profiles` derived view on first
    // flag-ON boot — and assert two production-validation invariants:
    //
    //   • Property 1: rebuilding the derived view from `ProductionSkillRegistry`
    //     (the sole source of truth) reproduces byte-for-byte identical query
    //     results (idempotent reindex). A backfill re-run over unchanged
    //     registry state is a no-op on observable query results.
    //
    //   • Property 11 (derived-table rollback facet): flipping the flag OFF is
    //     modeled as a CLEAN DROP of the derived `capability_profiles` data (a
    //     single truncating statement). This must (a) leave the authoritative
    //     registry byte-for-byte unchanged — so the frozen flag-OFF router path
    //     is entirely unaffected — and (b) be fully recoverable: flipping the
    //     flag back ON and re-running the backfill reproduces the exact prior
    //     derived query results. The single-source-of-truth invariant makes the
    //     derived tables safely droppable and rebuildable.
    //
    // NOTE: `execute_semantic` byte-for-byte flag-off parity is covered by task
    // 1.4 in handler/facade; this file's Property 11 focus is DERIVED-TABLE
    // drop/rebuild rollback correctness, deliberately NOT re-testing that path.
    // =======================================================================

    use crate::openclaw::registry::SkillDependency;
    use proptest::prelude::*;
    use rusqlite::Connection;

    /// A deterministic, comparable snapshot of ALL derived query results, keyed
    /// and ordered by `skill_id`. Each entry is the serialized derived profile
    /// plus its row-level `profile_epoch` and `embedding`. This is the "query
    /// result" `R` the idempotency/rollback invariants compare.
    fn derived_query_results(
        store: &ProfileStore,
        skill_ids: &[String],
    ) -> Vec<(String, String, i64, Option<Vec<f32>>)> {
        let mut ids = skill_ids.to_vec();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| {
                let row = store
                    .get_profile(&id)
                    .expect("get_profile must not error")?;
                let json = serde_json::to_string(&row.profile).expect("profile serializes");
                Some((id, json, row.profile_epoch, row.embedding))
            })
            .collect()
    }

    /// A deterministic snapshot of the authoritative registry state (the source
    /// of truth), ordered by `skill_id`. Used to prove a derived-table drop
    /// leaves the registry — and therefore the frozen flag-OFF path — untouched.
    fn registry_source_of_truth(registry: &ProductionSkillRegistry) -> Vec<String> {
        let mut skills = registry.get_enabled_skills().expect("enabled query");
        skills.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
        skills
            .iter()
            .map(|m| serde_json::to_string(m).expect("metadata serializes"))
            .collect()
    }

    /// Clean derived-table DROP: truncate the entire `capability_profiles`
    /// derived view in a single statement, over a fresh connection to the SAME
    /// `skills.db` (WAL allows the concurrent reader). Models a flag-OFF
    /// rollback dropping the derived data without touching the authoritative
    /// `skills` table (forward-only invariant: we truncate rows, never the
    /// table/schema). Returns the number of derived rows remaining (must be 0).
    fn drop_derived_profiles(db_path: &std::path::Path) -> i64 {
        let conn = Connection::open(db_path).expect("open skills.db for drop");
        conn.execute("DELETE FROM capability_profiles", [])
            .expect("truncate capability_profiles");
        conn.query_row("SELECT COUNT(*) FROM capability_profiles", [], |r| r.get(0))
            .expect("count derived rows")
    }

    // ---- proptest generators (open-vocabulary, novel tags included) --------

    /// A generated skill "shape"; `skill_id` is index-assigned for uniqueness.
    #[derive(Debug, Clone)]
    struct RollbackSpec {
        id_stub: String,
        category: String,
        categories: Vec<String>,
        dep_ids: Vec<String>,
        schema: Option<serde_json::Value>,
    }

    /// Open-vocabulary capability strings incl. a deliberately novel domain, so
    /// the drop/rebuild drill exercises the projection across the open input
    /// space (no hardcoded/enumerated capabilities).
    fn cap_string() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("media.image".to_string()),
            Just("doc.pdf".to_string()),
            Just("io.file.read".to_string()),
            // Never-before-seen domain: flows through as an open string.
            Just("quantum.entangle.route".to_string()),
            "[a-z]{1,8}(\\.[a-z]{1,8}){0,2}",
        ]
    }

    fn schema_strategy() -> impl Strategy<Value = Option<serde_json::Value>> {
        prop_oneof![
            Just(None),
            Just(Some(serde_json::json!({ "type": "object" }))),
            Just(Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "format": "binary" },
                    "opt": { "type": ["string", "null"] }
                }
            }))),
        ]
    }

    fn rollback_spec_strategy() -> impl Strategy<Value = RollbackSpec> {
        (
            "[a-z]{1,6}",
            cap_string(),
            prop::collection::vec(cap_string(), 0..4),
            prop::collection::vec("[a-z]{1,6}\\.[a-z]{1,6}", 0..3),
            schema_strategy(),
        )
            .prop_map(
                |(id_stub, category, categories, dep_ids, schema)| RollbackSpec {
                    id_stub,
                    category,
                    categories,
                    dep_ids,
                    schema,
                },
            )
    }

    /// Build enabled `SkillMetadata` from a spec with an index-unique id.
    fn spec_to_meta(index: usize, spec: &RollbackSpec) -> SkillMetadata {
        let mut meta = sample_meta(&format!("skill.{}.{index}", spec.id_stub));
        meta.category = spec.category.clone();
        meta.categories = spec.categories.clone();
        meta.dependencies = spec
            .dep_ids
            .iter()
            .map(|d| SkillDependency {
                skill_id: d.clone(),
                version_requirement: "*".to_string(),
                optional: false,
            })
            .collect();
        meta.input_schema = spec.schema.clone();
        meta
    }

    proptest! {
        // Bounded case count keeps the DB-backed backfill test fast.
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Property 1 — Single source of truth (Validates: Requirements 5.1).
        ///
        /// The FIRST-BOOT BACKFILL (`run_backfill`) that materializes the
        /// `capability_profiles` view from the registry is idempotent: a second
        /// backfill over unchanged registry state reproduces byte-for-byte
        /// identical query results, and a full drop-then-backfill recovers the
        /// same results. (Property 11 rollback facet is exercised in the
        /// dedicated drill test below; this asserts the pure idempotency.)
        #[test]
        fn backfill_rebuild_reproduces_query_results(
            specs in prop::collection::vec(rollback_spec_strategy(), 0..12)
        ) {
            let dir = tempfile::tempdir().expect("tempdir");
            let db_path = dir.path().join("skills.db");
            // Registry = SOLE source of truth; frozen migration 3 creates the view.
            let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
            let store = ProfileStore::open(&db_path).expect("profile store open");

            let mut skill_ids = Vec::with_capacity(specs.len());
            for (i, spec) in specs.iter().enumerate() {
                let meta = spec_to_meta(i, spec);
                registry.install_skill(&meta).expect("register skill");
                skill_ids.push(meta.skill_id);
            }

            // Pass 1: first-boot backfill → R1 (one profile per enabled skill).
            let s1 = BackfillStatus::new();
            let c1 = run_backfill(&registry, &store, 0, &s1).expect("backfill pass 1");
            prop_assert_eq!(c1, skill_ids.len());
            prop_assert!(s1.is_complete());
            let r1 = derived_query_results(&store, &skill_ids);
            prop_assert_eq!(r1.len(), skill_ids.len());

            // Pass 2: re-run backfill over unchanged registry state → R2 == R1.
            let s2 = BackfillStatus::new();
            run_backfill(&registry, &store, 0, &s2).expect("backfill pass 2");
            let r2 = derived_query_results(&store, &skill_ids);
            prop_assert_eq!(&r1, &r2, "backfill re-run changed query results (R1 != R2)");

            // Drop the derived view entirely, then backfill again → R3 == R1.
            let remaining = drop_derived_profiles(&db_path);
            prop_assert_eq!(remaining, 0, "derived table not fully dropped");
            let cleared = derived_query_results(&store, &skill_ids);
            prop_assert!(cleared.is_empty(), "derived rows still present after drop");
            let s3 = BackfillStatus::new();
            run_backfill(&registry, &store, 0, &s3).expect("backfill rebuild");
            let r3 = derived_query_results(&store, &skill_ids);
            prop_assert_eq!(&r1, &r3, "rebuild-after-drop did not reproduce results (R1 != R3)");
        }

        /// Property 11 — Flag-off rollback (Validates: Requirements 7.2, 7.3),
        /// derived-table drop/rebuild facet.
        ///
        /// Models the flag-ON → flag-OFF → flag-ON lifecycle at the DERIVED
        /// TABLE level:
        ///   1. flag ON  → backfill materializes `capability_profiles` (R1).
        ///   2. flag OFF → the derived view is cleanly DROPPED. This MUST leave
        ///      the authoritative registry byte-for-byte unchanged, so the
        ///      frozen flag-OFF router path is entirely unaffected (rollback is
        ///      lossless w.r.t. the source of truth).
        ///   3. flag ON  → re-running the backfill reproduces the exact prior
        ///      derived query results (safely rebuildable).
        #[test]
        fn flag_off_rollback_drops_derived_view_registry_intact(
            specs in prop::collection::vec(rollback_spec_strategy(), 1..12)
        ) {
            let dir = tempfile::tempdir().expect("tempdir");
            let db_path = dir.path().join("skills.db");
            let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
            let store = ProfileStore::open(&db_path).expect("profile store open");

            let mut skill_ids = Vec::with_capacity(specs.len());
            for (i, spec) in specs.iter().enumerate() {
                let meta = spec_to_meta(i, spec);
                registry.install_skill(&meta).expect("register skill");
                skill_ids.push(meta.skill_id);
            }

            // (1) Flag ON: build the derived view; snapshot derived results +
            // the authoritative registry state.
            let s1 = BackfillStatus::new();
            run_backfill(&registry, &store, 0, &s1).expect("flag-ON backfill");
            let derived_before = derived_query_results(&store, &skill_ids);
            let registry_before = registry_source_of_truth(&registry);
            prop_assert_eq!(derived_before.len(), skill_ids.len());

            // (2) Flag OFF: cleanly drop the derived view.
            let remaining = drop_derived_profiles(&db_path);
            prop_assert_eq!(remaining, 0, "flag-off must fully drop the derived view");
            prop_assert!(
                derived_query_results(&store, &skill_ids).is_empty(),
                "derived view still queryable after flag-off drop"
            );
            // The authoritative registry — and thus the frozen flag-OFF path —
            // is byte-for-byte unchanged by dropping the derived view.
            let registry_after_drop = registry_source_of_truth(&registry);
            prop_assert_eq!(
                &registry_before, &registry_after_drop,
                "dropping the derived view mutated the authoritative registry"
            );

            // (3) Flag back ON: rebuild reproduces the exact prior derived view.
            let s3 = BackfillStatus::new();
            run_backfill(&registry, &store, 0, &s3).expect("flag-ON rebuild");
            let derived_after = derived_query_results(&store, &skill_ids);
            prop_assert_eq!(
                &derived_before, &derived_after,
                "flag-off→on rebuild did not reproduce the derived query results"
            );
        }
    }

    /// Deterministic (non-proptest) flag-off rollback drill over the async
    /// `spawn_backfill` path: build via the spawned job, drop the derived view,
    /// confirm the registry is intact, then rebuild and reproduce results. This
    /// exercises the real boot path (`spawn_backfill`) end-to-end.
    #[tokio::test]
    async fn flag_off_rollback_drill_over_spawned_backfill() {
        let n = 6;
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("registry init"));
        let mut skill_ids = Vec::with_capacity(n);
        for i in 0..n {
            let meta = sample_meta(&format!("acme.roll{i}"));
            registry.install_skill(&meta).expect("install");
            skill_ids.push(meta.skill_id);
        }
        let store = Arc::new(ProfileStore::open(&db_path).expect("profile store open"));

        // Flag ON: spawned first-boot backfill materializes the view.
        let (status, handle) = spawn_backfill(Arc::clone(&registry), Arc::clone(&store), 0);
        let built = handle.await.expect("join").expect("backfill ok");
        assert_eq!(built, n);
        assert!(status.is_complete());
        let derived_before = derived_query_results(&store, &skill_ids);
        let registry_before = registry_source_of_truth(&registry);
        assert_eq!(derived_before.len(), n);

        // Flag OFF: clean derived-table drop; registry (source of truth) intact.
        assert_eq!(
            drop_derived_profiles(&db_path),
            0,
            "derived view must drop clean"
        );
        assert!(
            derived_query_results(&store, &skill_ids).is_empty(),
            "derived view still present after drop"
        );
        assert_eq!(
            registry_before,
            registry_source_of_truth(&registry),
            "drop mutated the authoritative registry"
        );

        // Flag back ON: rebuild reproduces byte-for-byte prior derived results.
        let (status2, handle2) = spawn_backfill(Arc::clone(&registry), Arc::clone(&store), 0);
        let rebuilt = handle2.await.expect("join").expect("rebuild ok");
        assert_eq!(rebuilt, n);
        assert!(status2.is_complete());
        assert_eq!(
            derived_before,
            derived_query_results(&store, &skill_ids),
            "rebuild after flag-off did not reproduce the derived view"
        );
    }
}
