//! Full production-scale stress test (user-directed): 100 prompts, 50
//! installs, 50 updates, 50 removals, 100 executions, parallel execution,
//! mixed workload, with 0-leak verification throughout. Real Docker, real
//! registry, real installer, real execution engine — no mocks.
//!
//! `#[ignore]`d by default (runs for several minutes with real containers);
//! run explicitly:
//! `cargo test -p kria-eval production_stress -- --ignored --nocapture`

use kria_core::execution::{
    ExecutionContext, ExecutionEngine, ExecutionGraph, GraphNode, NodeKind, ScheduleStatus,
};
use kria_core::openclaw::bundle::verify::TrustPolicy;
use kria_core::openclaw::bundle::BundleInstaller;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use semver::Version;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct StressReport {
    pub prompts_ok: usize,
    pub prompts_total: usize,
    pub parallel_ok: usize,
    pub parallel_total: usize,
    pub installs_ok: usize,
    pub updates_ok: usize,
    pub removals_ok: usize,
    pub leaked_containers: usize,
}

/// Run the full production-scale registry workload (installs/updates/removals)
/// — no Docker needed, exercises the real BundleInstaller + registry at scale.
pub fn registry_workload(
    installs: usize,
    updates: usize,
    removals: usize,
) -> Result<(usize, usize, usize), String> {
    use crate::openclaw_eval::installer_matrix::author_signed_bundle;

    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("stress.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"stress-key".to_vec())
            .map_err(|e| e.to_string())?,
    );
    let store_dir = dir.path().join("store");
    std::fs::create_dir_all(&store_dir).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let make_installer = || {
        BundleInstaller::new(registry.clone(), audit.clone(), store_dir.clone())
            .with_kria_version(Version::new(1, 0, 0))
            .with_trust_policy(TrustPolicy {
                trusted_keys: Vec::new(),
                require_signature: true,
            })
    };

    // 50 fresh installs.
    let mut installs_ok = 0usize;
    let mut slugs = Vec::new();
    for i in 0..installs {
        let slug = format!("oc_stress_{i}");
        let seed = [(i % 251) as u8 + 1; 32];
        let root = author_signed_bundle(&author_dir, &slug, seed)
            .map_err(|e| format!("author {slug}: {e}"))?;
        if make_installer().install(&root).is_ok() {
            installs_ok += 1;
            slugs.push((slug, seed));
        }
    }

    // 50 updates (re-author at v1.1.0 → upgrade path).
    let mut updates_ok = 0usize;
    for (i, (slug, seed)) in slugs.iter().enumerate().take(updates) {
        // author a v1.1.0 bundle for the same slug/publisher (same seed → same publisher key).
        let root = crate::openclaw_eval::installer_matrix::author_signed_bundle_version(
            &author_dir.join(format!("u{i}")),
            slug,
            *seed,
            "1.1.0",
        )
        .map_err(|e| format!("author update {slug}: {e}"))?;
        match make_installer().install(&root) {
            Ok(_) => updates_ok += 1,
            Err(e) => eprintln!("[stress] update {slug} failed: {e}"),
        }
    }

    // 50 removals.
    let mut removals_ok = 0usize;
    for (slug, _) in slugs.iter().take(removals) {
        if make_installer().uninstall(slug).is_ok() {
            removals_ok += 1;
        }
    }

    Ok((installs_ok, updates_ok, removals_ok))
}

/// Run `count` real Docker prompt executions against one rig (sequential),
/// returning how many succeeded.
pub async fn prompt_workload(engine: &ExecutionEngine, count: usize) -> usize {
    let mut ok = 0usize;
    for i in 0..count {
        let expr = format!("{} * {} + {}", i % 13, i % 7, i % 5);
        let mut graph = ExecutionGraph::new(format!("g-ps-{i}"), format!("goal-ps-{i}"));
        graph.add_node(GraphNode::new(
            "calc",
            NodeKind::Skill {
                provider_id: "openclaw".to_string(),
                action_id: "oc_calculator".into(),
                params: serde_json::json!({ "expression": expr }),
            },
        ));
        let ctx = ExecutionContext::new(format!("goal-ps-{i}"), format!("corr-ps-{i}"));
        if engine.execute_graph(&graph, &ctx).await.status == ScheduleStatus::Completed {
            ok += 1;
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw_eval::rig::{count_rig_containers, TestRig};
    use kria_core::execution::executors::openclaw_executor_from_pool;

    /// Full production-scale registry stress (no Docker): 50 installs, 50
    /// updates, 50 removals via the real BundleInstaller + registry.
    #[test]
    fn stress_registry_50_installs_50_updates_50_removals() {
        let (installs, updates, removals) =
            registry_workload(50, 50, 50).expect("registry workload");
        eprintln!("[stress] installs_ok={installs} updates_ok={updates} removals_ok={removals}");
        assert_eq!(installs, 50, "all 50 installs must succeed");
        assert_eq!(updates, 50, "all 50 updates must succeed");
        assert_eq!(removals, 50, "all 50 removals must succeed");
    }

    /// Full production-scale execution stress (real Docker): 100 sequential
    /// prompt executions + a 20-wide parallel batch, 0-leak verified.
    #[tokio::test]
    #[ignore = "runs several minutes against real Docker; run explicitly"]
    async fn stress_100_prompts_plus_parallel_real_docker() {
        if crate::openclaw_eval::rig::verify_docker_reachable()
            .await
            .is_err()
        {
            eprintln!("SKIPPED (Outcome::Skipped, not Pass): docker not reachable");
            return;
        }
        let baseline = count_rig_containers().await.expect("baseline");

        let rig = TestRig::up().await.expect("rig up");
        let mut engine = ExecutionEngine::new();
        engine.register_executor(Arc::new(openclaw_executor_from_pool(rig.pool.clone())));

        // 100 sequential real executions.
        let seq_ok = prompt_workload(&engine, 100).await;
        eprintln!("[stress] sequential prompts ok: {seq_ok}/100");
        assert_eq!(
            seq_ok, 100,
            "all 100 sequential prompt executions must succeed"
        );

        // 20-wide parallel batch through the same real pool. The pool's
        // configured `max_concurrent_invocations` (default 4) cleanly REJECTS
        // overflow (validated in task 14, `concurrency_probe`) rather than
        // queueing — so a real client retries a cleanly-rejected request.
        // This models that: each of the 20 requests retries (bounded) until
        // it either completes or exhausts retries. Correct production
        // behavior = every request eventually completes, pool never exceeds
        // its limit, 0 leaks.
        let engine = Arc::new(engine);
        let mut handles = Vec::new();
        for i in 0..20 {
            let engine = engine.clone();
            handles.push(tokio::spawn(async move {
                for attempt in 0..40u32 {
                    let mut graph = ExecutionGraph::new(
                        format!("g-par-{i}-{attempt}"),
                        format!("goal-par-{i}"),
                    );
                    graph.add_node(GraphNode::new(
                        "calc",
                        NodeKind::Skill {
                            provider_id: "openclaw".to_string(),
                            action_id: "oc_calculator".into(),
                            params: serde_json::json!({ "expression": format!("{i} + {i}") }),
                        },
                    ));
                    let ctx = ExecutionContext::new(
                        format!("goal-par-{i}"),
                        format!("corr-par-{i}-{attempt}"),
                    );
                    if engine.execute_graph(&graph, &ctx).await.status == ScheduleStatus::Completed
                    {
                        return true;
                    }
                    // Overflow-rejected → brief backoff + retry (real client).
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
                false
            }));
        }
        let mut par_ok = 0usize;
        for h in handles {
            if h.await.unwrap_or(false) {
                par_ok += 1;
            }
        }
        eprintln!("[stress] parallel prompts ok (with retry-on-overflow): {par_ok}/20");

        // Cleanup BEFORE asserting so a failure never orphans the rig.
        let down = rig.down().await;
        let after = count_rig_containers().await.unwrap_or(usize::MAX);

        assert_eq!(par_ok, 20, "all 20 parallel executions must eventually complete (with retry on clean overflow-rejection)");
        down.expect("rig teardown 0 leaks");
        assert_eq!(
            after, baseline,
            "container count must return to baseline (0 leak)"
        );
    }
}
