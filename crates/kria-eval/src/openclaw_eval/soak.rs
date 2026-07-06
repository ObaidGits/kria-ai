//! R18 — long-running / soak stability (tasks.md task 18, design.md "Soak
//! driver"). This task's soak is a BOUNDED real-Docker soak (many cycles,
//! not many hours) — the full multi-hour continuous soak is task 27's
//! explicit scope (`long-session stability 4-8h`), per this effort's pacing
//! note. This task proves the SAME mechanism at a scale that is honest to
//! run within a single validation session.

use crate::openclaw_eval::leak_detector;
use crate::openclaw_eval::rig::TestRig;
use kria_core::openclaw::ResourceClass;
use std::time::Duration;

/// Runs a bounded mixed workload against ONE real rig for `iterations`
/// cycles, sampling the leak baseline every `sample_every` cycles — proving
/// container/lease counts return to baseline repeatedly over sustained
/// (not just single) real usage, and that the registry/pool stay healthy
/// throughout.
pub async fn run_bounded_soak(iterations: usize, sample_every: usize) -> Result<Vec<(usize, bool)>, String> {
    let rig = TestRig::up().await.map_err(|e| e.to_string())?;
    let baseline = leak_detector::baseline(&rig.pool).await.map_err(|e| e.to_string())?;

    let mut samples = Vec::new();

    for i in 0..iterations {
        let handle = rig
            .pool
            .checkout(ResourceClass::Light, "oc_calculator")
            .await
            .map_err(|e| format!("iteration {i}: checkout failed: {e}"))?;
        rig.pool.checkin(handle).await.map_err(|e| format!("iteration {i}: checkin failed: {e}"))?;

        if i % sample_every == 0 {
            let ok = leak_detector::assert_returned_to(&rig.pool, baseline).await.is_ok();
            samples.push((i, ok));
            if !ok {
                eprintln!("[R18] iteration {i}: leases/containers NOT at baseline (sampled, continuing)");
            }
        }

        // Small real yield between cycles rather than a tight spin, closer
        // to real sustained usage than a synthetic benchmark loop.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Final assertion must hold regardless of any transient mid-run sample.
    leak_detector::assert_returned_to(&rig.pool, baseline)
        .await
        .map_err(|e| format!("final post-soak baseline check failed: {e}"))?;

    rig.down().await.map_err(|e| e.to_string())?;
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bounded real-Docker soak: 30 checkout/checkin cycles against ONE rig,
    /// sampling every 5th iteration — proves sustained (not one-shot) real
    /// usage returns to baseline every time, not just at the end.
    #[tokio::test]
    #[ignore = "real-Docker soak — run explicitly with --ignored"]
    async fn r18_bounded_soak_30_cycles() {
        if crate::openclaw_eval::rig::verify_docker_reachable().await.is_err() {
            eprintln!("SKIPPED (Outcome::Skipped, not Pass): docker not reachable");
            return;
        }

        let samples = run_bounded_soak(30, 5).await.expect("bounded soak must complete with baseline restored");
        let failures: Vec<_> = samples.iter().filter(|(_, ok)| !ok).collect();
        assert!(
            failures.is_empty(),
            "R18: every sampled point during the soak must be at baseline, failures at: {failures:?}"
        );
        eprintln!("[R18] 30-cycle bounded soak: {} samples, all at baseline", samples.len());
    }
}
