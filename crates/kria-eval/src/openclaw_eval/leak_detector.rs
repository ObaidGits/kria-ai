//! Leak detector (design.md "Component 2"). Snapshots the rig-owned container
//! count, active pool leases, and (optionally) GPU memory, and asserts a
//! baseline is restored after a run. Used by R2.4, R7.5, R18.2/18.5, R20.4.
//!
//! Only ever counts containers with the `rig::RIG_CONTAINER_PREFIX` — never
//! touches or counts unrelated live containers on the host.

use crate::openclaw_eval::rig::count_rig_containers;
use kria_core::openclaw::ContainerPool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Baseline {
    pub rig_container_count: usize,
    pub active_leases: usize,
    pub warm_count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LeakError {
    #[error("docker query failed: {0}")]
    Docker(String),
    #[error("leak detected: {0}")]
    Leaked(String),
}

/// Snapshot the current rig-owned container/lease state.
pub async fn baseline(pool: &Arc<ContainerPool>) -> Result<Baseline, LeakError> {
    let rig_container_count = count_rig_containers().await.map_err(LeakError::Docker)?;
    let active_leases = pool.active_count().await;
    let warm_count = pool.warm_count_total().await;
    Ok(Baseline {
        rig_container_count,
        active_leases,
        warm_count,
    })
}

/// Assert the current state matches (or is <=) the given baseline. Active
/// leases must return to baseline exactly; the rig container count is allowed
/// to be <= baseline (warm-pool recycling may destroy idle containers, which
/// is a shrink, never a leak).
pub async fn assert_returned_to(pool: &Arc<ContainerPool>, expected: Baseline) -> Result<(), LeakError> {
    let now = baseline(pool).await?;

    if now.active_leases != expected.active_leases {
        return Err(LeakError::Leaked(format!(
            "active leases did not return to baseline: expected {}, got {}",
            expected.active_leases, now.active_leases
        )));
    }

    if now.rig_container_count > expected.rig_container_count {
        return Err(LeakError::Leaked(format!(
            "rig container count grew: baseline {}, now {}",
            expected.rig_container_count, now.rig_container_count
        )));
    }

    Ok(())
}

/// Best-effort GPU memory snapshot in MiB, via `nvidia-smi`. Returns `None`
/// honestly when no NVIDIA GPU/driver is present — never fabricates a value
/// (R15 honesty invariant).
pub async fn gpu_memory_used_mib() -> Option<u64> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_equality_is_structural() {
        let a = Baseline {
            rig_container_count: 2,
            active_leases: 0,
            warm_count: 2,
        };
        let b = a;
        assert_eq!(a, b);
    }
}
