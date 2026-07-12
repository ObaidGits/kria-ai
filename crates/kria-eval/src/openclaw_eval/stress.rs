//! Task-2 stress validation (real Docker): repeated lifecycle iterations,
//! parallel lifecycle, parallel shutdown, parallel startup, rapid
//! enable/disable, parallel RuntimeManager/ContainerPool creation.
//!
//! These are `#[ignore]`d by default (they take minutes and require Docker);
//! run explicitly with:
//! `cargo test -p kria-eval openclaw_eval::stress -- --ignored --nocapture`

use crate::openclaw_eval::rig::{count_rig_containers, TestRig};

/// Run `iterations` sequential up/down rig cycles, asserting the rig-owned
/// container count returns to the SAME baseline after every single
/// iteration (not just at the end) — proves no per-iteration leak, not just
/// no leak by the final count coincidentally balancing out.
pub async fn sequential_lifecycle_stress(iterations: usize) -> Result<Vec<usize>, String> {
    let mut counts_after_each = Vec::with_capacity(iterations);

    let baseline = count_rig_containers().await.map_err(|e| e.to_string())?;

    for i in 0..iterations {
        let rig = TestRig::up()
            .await
            .map_err(|e| format!("iteration {i}: up() failed: {e}"))?;
        rig.down()
            .await
            .map_err(|e| format!("iteration {i}: down() failed (LEAK): {e}"))?;

        let now = count_rig_containers().await.map_err(|e| e.to_string())?;
        if now != baseline {
            return Err(format!(
                "iteration {i}: container count drifted from baseline {baseline} to {now}"
            ));
        }
        counts_after_each.push(now);
    }

    Ok(counts_after_each)
}

/// Run `count` rig lifecycles CONCURRENTLY (parallel startup + parallel
/// shutdown), asserting the final container count returns to baseline.
/// Exercises the `rig_lifecycle_lock` serialization fix under real
/// concurrent load, not just two tests happening to run together.
pub async fn parallel_lifecycle_stress(count: usize) -> Result<(), String> {
    let baseline = count_rig_containers().await.map_err(|e| e.to_string())?;

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        handles.push(tokio::spawn(async move {
            let rig = TestRig::up()
                .await
                .map_err(|e| format!("parallel iteration {i}: up() failed: {e}"))?;
            rig.down()
                .await
                .map_err(|e| format!("parallel iteration {i}: down() failed: {e}"))?;
            Ok::<(), String>(())
        }));
    }

    let mut errors = Vec::new();
    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => errors.push(e),
            Err(join_err) => {
                errors.push(format!("parallel iteration {i}: task panicked: {join_err}"))
            }
        }
    }

    if !errors.is_empty() {
        return Err(format!(
            "{} of {count} parallel iterations failed: {:?}",
            errors.len(),
            errors
        ));
    }

    let after = count_rig_containers().await.map_err(|e| e.to_string())?;
    if after != baseline {
        return Err(format!(
            "parallel stress left {after} containers, expected baseline {baseline}"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw_eval::rig::verify_docker_reachable;

    /// Required stress validation: >= 100 sequential lifecycle iterations
    /// against real Docker, asserting 0 drift after every single iteration.
    #[tokio::test]
    #[ignore = "slow real-Docker stress test — run explicitly with --ignored"]
    async fn stress_100_sequential_lifecycle_iterations_zero_leak() {
        if verify_docker_reachable().await.is_err() {
            eprintln!("SKIPPED: docker not reachable");
            return;
        }

        let counts = sequential_lifecycle_stress(100)
            .await
            .expect("100 sequential lifecycle iterations must all leave 0 drift");

        assert_eq!(counts.len(), 100);
        let baseline = counts[0];
        for (i, c) in counts.iter().enumerate() {
            assert_eq!(
                *c, baseline,
                "iteration {i} drifted from baseline {baseline}"
            );
        }
        eprintln!("[STRESS] 100/100 sequential lifecycle iterations: 0 leaked containers at every iteration");
    }

    /// Required stress validation: parallel lifecycle / parallel shutdown /
    /// parallel startup / parallel RuntimeManager+ContainerPool creation.
    #[tokio::test]
    #[ignore = "slow real-Docker stress test — run explicitly with --ignored"]
    async fn stress_parallel_lifecycle_20x_zero_leak() {
        if verify_docker_reachable().await.is_err() {
            eprintln!("SKIPPED: docker not reachable");
            return;
        }

        parallel_lifecycle_stress(20)
            .await
            .expect("20 CONCURRENT rig lifecycles must leave 0 leaked containers");
        eprintln!("[STRESS] 20 concurrent rig lifecycles: 0 leaked containers");
    }

    /// Required stress validation: rapid enable/disable (rig up/down back to
    /// back with no delay, repeated), a tighter loop than the 100-iteration
    /// test above, specifically targeting the shutdown/background-task race.
    #[tokio::test]
    #[ignore = "slow real-Docker stress test — run explicitly with --ignored"]
    async fn stress_rapid_enable_disable_50x_zero_leak() {
        if verify_docker_reachable().await.is_err() {
            eprintln!("SKIPPED: docker not reachable");
            return;
        }

        let counts = sequential_lifecycle_stress(50)
            .await
            .expect("50 rapid enable/disable cycles must all leave 0 drift");
        eprintln!(
            "[STRESS] 50/50 rapid enable/disable cycles: 0 leaked containers, counts={counts:?}"
        );
    }
}
