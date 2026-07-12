//! Concurrency validation (tasks.md task 14, design.md "Concurrency
//! validation"). Real concurrent load against the real rig/registry/pool —
//! no new locking system, reuses A0-A9 components as-is.

use crate::openclaw_eval::installer_matrix::author_signed_bundle;
use kria_core::openclaw::bundle::verify::TrustPolicy;
use kria_core::openclaw::bundle::BundleInstaller;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use semver::Version;
use std::sync::Arc;
use std::time::Duration;

/// Parallel installs of N DIFFERENT skills against the same real registry —
/// asserts all succeed and all end up as distinct, correctly-stored rows
/// (no lost updates, no cross-talk between concurrent SQLite writes).
pub async fn validate_parallel_distinct_installs(count: usize) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("concurrency_distinct.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"concurrency-test-key".to_vec())
            .map_err(|e| e.to_string())?,
    );
    let store = dir.path().join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let installer = Arc::new(
        BundleInstaller::new(registry.clone(), audit, store)
            .with_kria_version(Version::new(1, 0, 0))
            .with_trust_policy(TrustPolicy {
                trusted_keys: Vec::new(),
                require_signature: true,
            }),
    );

    // Author all bundles up front (filesystem authoring is not the thing
    // under test; the concurrent REGISTRY WRITE is).
    let mut bundle_roots = Vec::with_capacity(count);
    for i in 0..count {
        let slug = format!("oc_concurrent_{i}");
        let root = author_signed_bundle(&author_dir, &slug, [i as u8; 32])?;
        bundle_roots.push((slug, root));
    }

    let mut handles = Vec::with_capacity(count);
    for (slug, root) in bundle_roots {
        let installer = installer.clone();
        handles.push(tokio::spawn(async move {
            installer.install(&root).map(|_| slug)
        }));
    }

    let mut installed_slugs = Vec::with_capacity(count);
    for handle in handles {
        match handle.await {
            Ok(Ok(slug)) => installed_slugs.push(slug),
            Ok(Err(e)) => return Err(format!("a parallel install failed: {e}")),
            Err(join_err) => return Err(format!("a parallel install task panicked: {join_err}")),
        }
    }

    if installed_slugs.len() != count {
        return Err(format!(
            "expected {count} successful installs, got {}",
            installed_slugs.len()
        ));
    }

    // Every installed skill must be independently retrievable — no lost
    // writes, no cross-talk.
    for slug in &installed_slugs {
        registry
            .get(slug)
            .map_err(|e| format!("skill '{slug}' not found after parallel install: {e}"))?;
    }

    Ok(())
}

/// Same-target race: concurrent enable + disable on the SAME skill. The
/// final state must be one of the two valid outcomes (Enabled or Disabled)
/// deterministically observable — never a corrupted/partial state, never a
/// panic, never a deadlock (bounded by the overall test timeout).
pub async fn validate_concurrent_enable_disable_same_skill() -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let db_path = dir.path().join("concurrency_race.db");
    let registry = Arc::new(ProductionSkillRegistry::new(&db_path).map_err(|e| e.to_string())?);
    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(&db_path, b"concurrency-race-key".to_vec())
            .map_err(|e| e.to_string())?,
    );
    let store = dir.path().join("store");
    std::fs::create_dir_all(&store).map_err(|e| e.to_string())?;
    let author_dir = dir.path().join("authored");
    std::fs::create_dir_all(&author_dir).map_err(|e| e.to_string())?;

    let bundle_root = author_signed_bundle(&author_dir, "oc_race_target", [77u8; 32])?;
    let installer = Arc::new(
        BundleInstaller::new(registry.clone(), audit, store)
            .with_kria_version(Version::new(1, 0, 0))
            .with_trust_policy(TrustPolicy {
                trusted_keys: Vec::new(),
                require_signature: true,
            }),
    );
    installer
        .install(&bundle_root)
        .map_err(|e| format!("install failed: {e}"))?;

    let mut handles = Vec::new();
    for i in 0..20 {
        let installer = installer.clone();
        if i % 2 == 0 {
            handles.push(tokio::spawn(
                async move { installer.enable("oc_race_target") },
            ));
        } else {
            handles.push(tokio::spawn(
                async move { installer.disable("oc_race_target") },
            ));
        }
    }

    let deadline = tokio::time::timeout(Duration::from_secs(20), async {
        for handle in handles {
            handle
                .await
                .map_err(|e| format!("race task panicked: {e}"))?
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })
    .await;

    match deadline {
        Err(_) => {
            return Err(
                "DEADLOCK/LIVELOCK: concurrent enable/disable did not complete within 20s".into(),
            )
        }
        Ok(Err(e)) => return Err(format!("a concurrent enable/disable call failed: {e}")),
        Ok(Ok(())) => {}
    }

    // Final state must be deterministically ONE valid state, not corrupted.
    let final_skill = registry
        .get_skill("oc_race_target")
        .map_err(|e| e.to_string())?;
    use kria_core::openclaw::registry::SkillState;
    if !matches!(
        final_skill.state,
        SkillState::Enabled | SkillState::Disabled
    ) {
        return Err(format!(
            "concurrent enable/disable left the skill in an invalid state: {:?}",
            final_skill.state
        ));
    }

    Ok(())
}

/// Real Docker: parallel container checkout/checkin against the same rig
/// pool — proves `ContainerPool`/`RuntimeManager` concurrency (semaphore +
/// scheduler) holds under real concurrent load, with a leak-baseline check.
///
/// `count` MUST be <= `OpenClawConfig::default().max_concurrent_invocations`
/// (4, confirmed in `config.rs`) — requesting more than the configured
/// semaphore permits is EXPECTED to reject the excess with
/// `RuntimeError::MaxConcurrent`, which is the semaphore correctly enforcing
/// its limit (real, correct behavior), not a deadlock or a bug. That
/// rejection-at-the-limit behavior is validated separately by
/// `validate_checkout_beyond_limit_rejects_cleanly`.
pub async fn validate_parallel_container_checkout(count: usize) -> Result<(), String> {
    use crate::openclaw_eval::leak_detector;
    use crate::openclaw_eval::rig::TestRig;
    use kria_core::openclaw::ResourceClass;

    let rig = TestRig::up().await.map_err(|e| e.to_string())?;
    let baseline = leak_detector::baseline(&rig.pool)
        .await
        .map_err(|e| e.to_string())?;

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let pool = rig.pool.clone();
        handles.push(tokio::spawn(async move {
            let handle = pool
                .checkout(ResourceClass::Light, &format!("concurrency-test-{i}"))
                .await?;
            pool.checkin(handle).await
        }));
    }

    let deadline = tokio::time::timeout(Duration::from_secs(60), async {
        for handle in handles {
            handle
                .await
                .map_err(|e| format!("checkout/checkin task panicked: {e}"))?
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    })
    .await;

    // Always tear the rig down regardless of outcome, so a failed assertion
    // never leaks the containers this test created (same cleanup-ordering
    // bug class caught and fixed in validate_checkout_beyond_limit_rejects_cleanly
    // above — fixed here identically).
    let deadline_result: Result<(), String> = match deadline {
        Err(_) => {
            Err("DEADLOCK/LIVELOCK: parallel container checkout did not complete within 60s".into())
        }
        Ok(Err(e)) => Err(format!("a parallel checkout/checkin failed: {e}")),
        Ok(Ok(())) => Ok(()),
    };

    // Real finding (caught by running this for real, not assumed): `count`
    // concurrent Light checkouts against a pool pre-warmed with fewer than
    // `count` idle containers legitimately creates NEW containers to satisfy
    // demand (confirmed: `checkin_container`'s real logic keeps a returned
    // container `Idle` for reuse rather than always destroying it — "Idle for
    // reuse - first choice"). That means warm container COUNT is allowed to
    // GROW and stay grown after concurrent load — that is the pool working
    // correctly, not a leak. The real leak-free invariant here is ACTIVE
    // LEASES returning to baseline (0 in-flight), not the warm container
    // count shrinking back down. Checking lease count specifically instead
    // of the stricter (and, for this scenario, incorrect) container-count
    // check `assert_returned_to` performs.
    let active_after = rig.pool.active_count().await;
    let lease_result: Result<(), String> = if active_after != baseline.active_leases {
        Err(format!(
            "active leases did not return to baseline: expected {}, got {active_after}",
            baseline.active_leases
        ))
    } else {
        Ok(())
    };

    rig.down().await.map_err(|e| e.to_string())?;

    deadline_result?;
    lease_result
}

/// Real Docker: requesting MORE concurrent checkouts than
/// `max_concurrent_invocations` allows must be REJECTED cleanly (honest
/// `MaxConcurrent` error) — never hang, never silently queue forever, never
/// panic. Proves the semaphore's limit-enforcement is itself a real,
/// observable concurrency-safety property.
pub async fn validate_checkout_beyond_limit_rejects_cleanly() -> Result<(), String> {
    use crate::openclaw_eval::rig::TestRig;
    use kria_core::openclaw::ResourceClass;

    let rig = TestRig::up().await.map_err(|e| e.to_string())?;
    let limit = rig.config.max_concurrent_invocations;

    // Hold `limit` checkouts open simultaneously, then attempt one more.
    let mut held = Vec::with_capacity(limit);
    for i in 0..limit {
        let handle = rig
            .pool
            .checkout(ResourceClass::Light, &format!("limit-test-{i}"))
            .await
            .map_err(|e| format!("expected checkout {i}/{limit} to succeed: {e}"))?;
        held.push(handle);
    }

    let overflow = tokio::time::timeout(
        Duration::from_secs(5),
        rig.pool
            .checkout(ResourceClass::Light, "limit-test-overflow"),
    )
    .await;

    // Always attempt to check in every held handle, even if the overflow
    // assertion below is about to fail — an early `return Err(..)` here
    // would otherwise skip checkin/rig.down() and leak the `limit` held
    // containers (a real bug in an EARLIER version of this test, caught by
    // running it for real: the panic message showed containers still
    // present after a failed assertion, because checkin/down were never
    // reached). Collect all checkin results, then decide the final outcome.
    let mut checkin_errors = Vec::new();
    for handle in held {
        if let Err(e) = rig.pool.checkin(handle).await {
            checkin_errors.push(e.to_string());
        }
    }

    let overflow_result = match overflow {
        Err(_) => Err(
            "overflow checkout hung instead of being rejected cleanly (should fail fast)"
                .to_string(),
        ),
        Ok(Ok(_)) => Err(format!(
            "expected overflow checkout beyond the limit of {limit} to be rejected"
        )),
        Ok(Err(e)) => {
            eprintln!("[concurrency] overflow correctly rejected: {e}");
            Ok(())
        }
    };

    rig.down().await.map_err(|e| e.to_string())?;

    if !checkin_errors.is_empty() {
        return Err(format!(
            "checkin failures during cleanup: {checkin_errors:?}"
        ));
    }
    overflow_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parallel_distinct_installs_no_lost_writes() {
        validate_parallel_distinct_installs(10)
            .await
            .expect("10 parallel distinct installs must all succeed with no lost writes");
    }

    #[tokio::test]
    async fn concurrent_enable_disable_same_skill_no_deadlock() {
        validate_concurrent_enable_disable_same_skill()
            .await
            .expect(
                "concurrent enable/disable on the same skill must not deadlock or corrupt state",
            );
    }

    #[tokio::test]
    async fn parallel_container_checkout_real_docker() {
        if crate::openclaw_eval::rig::verify_docker_reachable()
            .await
            .is_err()
        {
            eprintln!("SKIPPED (Outcome::Skipped, not Pass): docker not reachable");
            return;
        }
        // Within the default max_concurrent_invocations (4) — see doc on
        // validate_parallel_container_checkout for why exceeding it is a
        // SEPARATE, correctly-rejecting scenario, not a deadlock.
        validate_parallel_container_checkout(4)
            .await
            .expect("4 parallel real container checkouts (at the configured limit) must complete with 0 leak and no deadlock");
    }

    #[tokio::test]
    async fn checkout_beyond_limit_rejects_cleanly_real_docker() {
        if crate::openclaw_eval::rig::verify_docker_reachable()
            .await
            .is_err()
        {
            eprintln!("SKIPPED (Outcome::Skipped, not Pass): docker not reachable");
            return;
        }
        validate_checkout_beyond_limit_rejects_cleanly()
            .await
            .expect(
                "checkout beyond max_concurrent_invocations must be rejected cleanly, never hang",
            );
    }
}
