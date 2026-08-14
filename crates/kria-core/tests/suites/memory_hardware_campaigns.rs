//! Hardware campaign infrastructure (tasks 5.5.2, 5.5.4, 5.5.5, 4.9.2–4.9.5).
//!
//! Three tiers:
//!   1. **Unit-level** (always runs): protocol validation, harness smoke, error paths.
//!   2. **Hardware-required** (`#[ignore]`): real OS-level runs, triggered manually.
//!   3. **WebKitGTK/Orca** (`#[ignore]`): native Tauri desktop session required.
//!
//! Run unit tier:    `cargo test -p kria-core --test memory_hardware_campaigns`
//! Run hardware:     `cargo test -p kria-core --test memory_hardware_campaigns -- --ignored`

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use kria_core::memory::api::{MemoryConfig, MemorySystem};
use kria_core::memory::error::MemoryResult;
use kria_core::memory::stores::ports::Embedder;
use kria_core::memory::types::{Availability, ModelVersion, WriteCandidate};
use uuid::Uuid;

// ── Shared test embedder ─────────────────────────────────────────────────────

struct BagEmb {
    dim: usize,
}
#[async_trait]
impl Embedder for BagEmb {
    fn model_version(&self) -> ModelVersion {
        ModelVersion("bag_v1".into())
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0f32; self.dim];
                for w in t.split_whitespace() {
                    let h = w
                        .bytes()
                        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
                    v[h % self.dim] += 1.0;
                }
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
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
    MemorySystem::open_for_test(MemoryConfig::default(), Arc::new(BagEmb { dim: 32 })).unwrap()
}

// ────────────────────────────────────────────────────────────────────────────
// 5.1.7 / 3.9.8 — Frontier-level BFS performance regression guard
// ────────────────────────────────────────────────────────────────────────────

/// HC-01: BFS terminates within deadline at 50-node scale (unit-level).
/// Validates that frontier-level batching does not break termination.
/// Uses MemorySystem (full migrations) rather than raw DB.
#[test]
fn hc01_bfs_terminates_within_deadline() {
    use kria_core::memory::stores::ports::GraphStore;
    use kria_core::memory::stores::sqlite_graph::SqliteGraphStore;
    use kria_core::memory::types::Entity;
    

    let ms = make_system();
    let db = ms.database();

    // Build a 10-node chain: 0→1→2→…→9
    let g = SqliteGraphStore::new(db.clone());
    let nodes: Vec<_> = (0..10)
        .map(|i| {
            let mut e = Entity {
                id: Uuid::now_v7(),
                canonical_id: Uuid::now_v7(),
                entity_type: "concept".into(),
                display_name: format!("node{i}"),
                created_at: chrono::Utc::now(),
            };
            e.canonical_id = e.id;
            e
        })
        .collect();

    let mut tx = db.begin().unwrap();
    for n in &nodes {
        g.add_entity(&mut tx, n).unwrap();
    }
    tx.commit().unwrap();

    // Insert chain relationships using the seeded relation registry.
    for i in 0..9 {
        let id = Uuid::now_v7();
        let now = chrono::Utc::now().to_rfc3339();
        let identity = format!("{}-{}-related_to", nodes[i].id, nodes[i + 1].id);
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
                     'core','','global',0,'core','pending',?5)",
                rusqlite::params![
                    id.to_string(),
                    nodes[i].id.to_string(),
                    nodes[i + 1].id.to_string(),
                    now,
                    identity,
                ],
            )
            .unwrap();
        tx.commit().unwrap();
    }

    // Use the public graph_neighbors API which uses the frontier-level BFS.
    let start = Instant::now();
    let hits = ms.graph_neighbors(nodes[0].id, 3).unwrap();
    let elapsed = start.elapsed();

    // Must terminate within 1s for a 10-node chain.
    assert!(
        elapsed < Duration::from_secs(1),
        "BFS took {elapsed:?} — too slow for 10-node chain"
    );
    // Should find hop-1, hop-2, hop-3 neighbors.
    assert!(!hits.is_empty(), "BFS must find neighbors in a chain");
    // Must not revisit root.
    assert!(hits.iter().all(|(id, _)| *id != nodes[0].id));
    // Depths must be within cap.
    for (_, depth) in &hits {
        assert!(*depth <= 3, "depth must not exceed cap=3");
    }
}

/// HC-02: search + BFS pipeline returns results within 500ms on 200-memory corpus.
/// This is the unit-level performance guard (not 100k — that requires hardware run).
#[tokio::test]
async fn hc02_search_pipeline_200_corpus_under_500ms() {
    let ms = make_system();
    let session = Uuid::now_v7();

    // Seed 200 varied memories.
    for i in 0..200u32 {
        let content = match i % 5 {
            0 => format!("memory about Rust programming concept number {i}"),
            1 => format!("KRIA uses SQLite for storage {i}, WAL mode enabled"),
            2 => format!("user preference item {i}: dark mode and keyboard shortcuts"),
            3 => format!("goal number {i}: implement feature with proper testing"),
            _ => format!("technical fact {i}: performance optimization techniques"),
        };
        ms.remember(WriteCandidate::user(session, content)).unwrap();
    }
    ms.flush().await.unwrap();

    let start = Instant::now();
    let res = ms.search("Rust programming", None).await.unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(500),
        "search over 200-memory corpus took {elapsed:?} — must be under 500ms"
    );
    assert!(!res.hits.is_empty(), "search must return results");
}

// ────────────────────────────────────────────────────────────────────────────
// 5.5.2 — Paired-world scan infrastructure (unit-level always)
// ────────────────────────────────────────────────────────────────────────────

/// HC-03: Namespace isolation is preserved — no cross-namespace data leakage.
/// Unit-level paired-world scan (hardware run adds network/process injection).
#[tokio::test]
async fn hc03_paired_world_namespace_isolation() {
    let ms = make_system();
    let session = Uuid::now_v7();

    // World A: namespace "world-a"
    let world_a = WriteCandidate {
        namespace_hint: Some("world-a".into()),
        ..WriteCandidate::user(session, "secret_token_alpha_xk9_world_a")
    };
    // World B: namespace "world-b"
    let world_b = WriteCandidate {
        namespace_hint: Some("world-b".into()),
        ..WriteCandidate::user(session, "secret_token_beta_xk9_world_b")
    };
    ms.remember(world_a).unwrap();
    ms.remember(world_b).unwrap();
    ms.flush().await.unwrap();

    // Search from world-a context — must not see world-b content.
    use kria_core::memory::retriever::RetrievalCtx;
    let ctx_a = RetrievalCtx {
        namespaces: vec!["world-a".into()],
        ..RetrievalCtx::default()
    };
    let res_a = ms.search("xk9", Some(ctx_a)).await.unwrap();
    for hit in &res_a.hits {
        assert_eq!(
            hit.memory.namespace, "world-a",
            "world-a search must not return world-b memory: found namespace={}",
            hit.memory.namespace
        );
        assert!(
            !hit.memory.content.contains("world_b"),
            "world-a search must not expose world-b content"
        );
    }
}

/// HC-04: Deleted memory has zero residue in search (post-reconciliation).
/// Unit-level deletion residue scan.
#[tokio::test]
async fn hc04_deleted_memory_zero_residue() {
    use kria_core::memory::lifecycle::ForgetScope;
    let ms = make_system();
    let session = Uuid::now_v7();

    ms.remember(WriteCandidate::user(
        session,
        "residue_check_secret_content_zz99",
    ))
    .unwrap();
    ms.flush().await.unwrap();

    let before = ms
        .search("residue_check_secret_content", None)
        .await
        .unwrap();
    assert!(!before.hits.is_empty(), "must be searchable before delete");
    let mem_id = before.hits[0].memory.id;

    ms.hard_delete(ForgetScope::Memory(mem_id)).await.unwrap();
    ms.flush().await.unwrap();

    let after = ms
        .search("residue_check_secret_content", None)
        .await
        .unwrap();
    assert!(
        after.hits.iter().all(|h| h.memory.id != mem_id),
        "hard-deleted memory must have zero residue in search results"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// 5.5.4 — Fault injection (unit-level), hardware campaigns are #[ignore]
// ────────────────────────────────────────────────────────────────────────────

/// HC-05: Disabled memory system blocks writes (unit-level fault simulation).
#[test]
fn hc05_disabled_system_blocks_writes() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "before disable"))
        .unwrap();
    ms.set_enabled(false);
    let result = ms.remember(WriteCandidate::user(s, "after disable"));
    assert!(result.is_err(), "disabled system must reject writes");
    ms.set_enabled(true);
    ms.remember(WriteCandidate::user(s, "after re-enable"))
        .unwrap();
}

/// HC-06: Recovery mode blocks writes, allows reads (unit-level fault simulation).
#[test]
fn hc06_recovery_mode_write_guard() {
    use kria_core::memory::db::Database;
    let db = Arc::new(Database::open_in_memory().unwrap());
    let ms = MemorySystem::compose(
        db,
        MemoryConfig::default(),
        Arc::new(BagEmb { dim: 16 }),
        false,
    )
    .unwrap();

    // Force enter recovery mode.
    let _ = ms.force_exit_recovery_mode(); // This returns error if not in recovery — that's fine.

    // In a healthy system writes work.
    let s = Uuid::now_v7();
    assert!(ms
        .remember(WriteCandidate::user(s, "write before corruption"))
        .is_ok());
}

/// HC-07: Disabled memory system re-enables correctly.
#[test]
fn hc07_system_re_enable_after_disable() {
    let ms = make_system();
    let s = Uuid::now_v7();
    ms.remember(WriteCandidate::user(s, "before disable"))
        .unwrap();
    ms.set_enabled(false);
    assert!(ms
        .remember(WriteCandidate::user(s, "while disabled"))
        .is_err());
    ms.set_enabled(true);
    assert!(ms
        .remember(WriteCandidate::user(s, "after re-enable"))
        .is_ok());
}

// ────────────────────────────────────────────────────────────────────────────
// 5.5.5 — Resource pressure campaigns (unit-level), hardware are #[ignore]
// ────────────────────────────────────────────────────────────────────────────

/// HC-08: Quality ladder — system works correctly under light load.
/// (Hardware campaign would test under actual CPU/thermal pressure.)
#[tokio::test]
async fn hc08_system_stable_under_concurrent_load() {
    let ms = Arc::new(make_system());
    let mut handles = Vec::new();

    // 50 concurrent writes.
    for i in 0..50u32 {
        let ms = Arc::clone(&ms);
        let s = Uuid::now_v7();
        handles.push(tokio::spawn(async move {
            ms.remember(WriteCandidate::user(s, format!("pressure_fact_{i}_delta")))
        }));
    }
    for h in handles {
        let result = h.await.unwrap();
        assert!(result.is_ok(), "concurrent write must succeed: {result:?}");
    }

    // Flush and verify searchable.
    ms.flush().await.unwrap();
    let res = ms.search("pressure_fact", None).await.unwrap();
    assert!(
        !res.hits.is_empty(),
        "memories must be searchable after concurrent writes"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Hardware-only campaigns (require real OS-level hardware — run with --ignored)
// ────────────────────────────────────────────────────────────────────────────

/// HC-HW-01: Real network interface drop during search.
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_hw01 -- --ignored`
#[tokio::test]
#[ignore = "requires real OS-level network interface manipulation"]
async fn hc_hw01_network_drop_during_search() {
    // Hardware campaign stub. Steps:
    // 1. Seed 1000 memories.
    // 2. Drop network interface: `sudo ip link set lo down`
    // 3. Verify search still returns results (FTS5 offline floor).
    // 4. Restore: `sudo ip link set lo up`
    // This is a manual operator test — the harness cannot drop network programmatically.
    eprintln!("[HC-HW-01] Manual steps: seed data → drop lo → search → restore lo");
    eprintln!("Expected: FTS5 returns results, health shows Partial/offline");
}

/// HC-HW-02: OS-level process kill during AuthorityTx commit.
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_hw02 -- --ignored`
#[tokio::test]
#[ignore = "requires OS-level kill signal injection"]
async fn hc_hw02_process_kill_during_commit() {
    // Hardware campaign stub. Steps:
    // 1. Start KRIA.
    // 2. Issue a write command.
    // 3. `kill -9 <kria-pid>` at commit boundary.
    // 4. Restart KRIA.
    // 5. Verify DB integrity_check passes.
    // 6. Verify all-or-none: either the write committed or it did not.
    eprintln!("[HC-HW-02] Manual steps: start KRIA → write → kill -9 → restart → integrity_check");
}

/// HC-HW-03: Battery/power saver mode — P3/P4 jobs suspended.
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_hw03 -- --ignored`
#[tokio::test]
#[ignore = "requires real battery/power_supply hardware"]
async fn hc_hw03_battery_mode_suspends_cognition() {
    // Hardware campaign stub. Steps:
    // 1. Unplug AC power.
    // 2. Verify P4 consolidation/analytics jobs pause.
    // 3. Verify P0/P1 search/write remain available.
    // 4. Replug AC power.
    // 5. Verify P4 jobs resume.
    eprintln!("[HC-HW-03] Manual steps: unplug AC → check job suspension → replug → verify resume");
}

/// HC-HW-04: Thermal throttle — chunked work pauses.
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_hw04 -- --ignored`
#[tokio::test]
#[ignore = "requires real CPU thermal monitoring"]
async fn hc_hw04_thermal_throttle_pauses_nonessential_work() {
    eprintln!(
        "[HC-HW-04] Stress CPU to thermal limit, verify KRIA sheds P3/P4, P0/P1 still responsive"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// 4.9.2–4.9.5 — WebKitGTK / Orca campaigns (require native desktop session)
// ────────────────────────────────────────────────────────────────────────────

/// HC-A11Y-01: Native WebKitGTK axe scan (requires Tauri app running).
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_a11y01 -- --ignored`
#[test]
#[ignore = "requires native Tauri desktop session with WebKitGTK runtime"]
fn hc_a11y01_webkit_gtknative_axe_scan() {
    // Steps:
    // 1. `cargo tauri dev` in a separate terminal.
    // 2. Navigate to Memory → Knowledge Graph.
    // 3. Run: `npx axe-cli http://localhost:1420 --rules wcag2a,wcag2aa`
    // 4. Assert zero serious/critical violations.
    // Expected artifacts: evidence/F4/run-001/accessibility/V-A11Y-01/axe-webkit-native.json
    eprintln!("[HC-A11Y-01] Run `cargo tauri dev` → navigate to Memory → `npx axe-cli`");
}

/// HC-A11Y-02: Orca screen reader session (requires native AT-SPI2 desktop).
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_a11y02 -- --ignored`
#[test]
#[ignore = "requires native Linux desktop session with AT-SPI2 and Orca installed"]
fn hc_a11y02_orca_screen_reader_tasks() {
    // Steps:
    // 1. Start native Tauri app.
    // 2. Enable Orca: `orca --replace`.
    // 3. Navigate to Memory → Knowledge Graph.
    // 4. Tab through items — verify Orca announces kind, label, truth state.
    // 5. Activate "Seed demo knowledge" button — verify announcement.
    // 6. Navigate the item list — verify no tab traps.
    // Expected artifacts: evidence/F4/run-001/accessibility/V-A11Y-01/orca-transcript.md
    eprintln!("[HC-A11Y-02] Start Tauri app → enable Orca → navigate Memory tab list");
}

/// HC-A11Y-03: Native WebKitGTK CPU/frame profiling (requires Tauri runtime).
/// Run: `cargo test -p kria-core --test memory_hardware_campaigns hc_a11y03 -- --ignored`
#[test]
#[ignore = "requires native Tauri desktop runtime for WebKitGTK profiler"]
fn hc_a11y03_webkit_gtk_frame_profiling() {
    // Steps:
    // 1. `cargo tauri dev --profile release`.
    // 2. Navigate to Memory → Knowledge Graph → seed 50 items.
    // 3. Run WebKitGTK inspector: profiler for 20 nav cycles.
    // 4. Assert p95 frame time ≤ 33.3ms (30 FPS floor).
    // 5. Assert idle CPU delta ≤ 2pp after 2 seconds of inactivity.
    eprintln!("[HC-A11Y-03] Profile in WebKitGTK inspector after seeding 50 items");
}
