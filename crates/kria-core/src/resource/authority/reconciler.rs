//! Reconciler (HRA Task 9 / R12.1, R21.1, R23.1).
//!
//! Pure reconciliation logic for crash recovery + split-brain protection. Given the set of
//! journaled/known leases, the observed GPU processes, the RA-spawned PID registry, and the
//! current epoch, it computes a `ReconcilePlan`:
//! - leases from a previous epoch are invalidated (epoch fencing, Property 11),
//! - observed GPU processes that are RA-spawned but have no valid lease are orphans to reclaim,
//! - processes NOT in the RA-spawned registry are NEVER reclaimed (Property: R23.1 kill-scope).
//!
//! The runtime executes the plan (the actual process kill is destructive, capability-token gated,
//! and audited — that wiring lives in Task 38, not here).

use std::collections::HashSet;

use super::scheduler::{Lease, LeaseToken};
use super::types::Epoch;

/// An observed GPU process from telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedProcess {
    pub pid: u32,
    pub vram_mb: u64,
}

/// What the runtime must do to converge to a consistent state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcilePlan {
    /// Leases that are no longer valid (stale epoch) and must be dropped.
    pub invalidate_leases: Vec<LeaseToken>,
    /// PIDs that are RA-spawned orphans (no backing valid lease) and should be reclaimed.
    pub reclaim_pids: Vec<u32>,
}

/// Compute the reconcile plan. Pure and deterministic.
///
/// - `current_epoch`: the authority's epoch after (re)start.
/// - `known_leases`: leases the authority believes are active.
/// - `observed`: GPU processes seen in telemetry.
/// - `ra_spawned_pids`: PIDs the authority itself spawned (kill-scope allow-list).
pub fn reconcile(
    current_epoch: Epoch,
    known_leases: &[Lease],
    observed: &[ObservedProcess],
    ra_spawned_pids: &HashSet<u32>,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();

    // 1. Epoch fencing: any lease not on the current epoch is invalid.
    for lease in known_leases {
        if lease.epoch != current_epoch {
            plan.invalidate_leases.push(lease.token);
        }
    }

    // A lease is "valid" if it is on the current epoch.
    let have_valid_lease = known_leases.iter().any(|l| l.epoch == current_epoch);

    // 2. Orphan reclaim — ONLY RA-spawned PIDs, and only when no valid lease backs GPU usage.
    //    Never touch foreign PIDs (R23.1 kill-scope).
    for proc in observed {
        if ra_spawned_pids.contains(&proc.pid) && !have_valid_lease {
            plan.reclaim_pids.push(proc.pid);
        }
    }

    plan.invalidate_leases.sort_by_key(|t| t.0);
    plan.reclaim_pids.sort_unstable();
    plan.reclaim_pids.dedup();
    plan
}

#[cfg(test)]
mod tests {
    use super::super::scheduler::{Lease, LeaseToken};
    use super::super::types::{Capacity, DeviceId, Residency, TurnId};
    use super::*;

    fn lease(token: u64, epoch: u64) -> Lease {
        Lease {
            token: LeaseToken(token),
            device: DeviceId::Gpu(0),
            budget: Capacity::vram(4000),
            class: super::super::types::PriorityClass::InteractiveFg,
            residency: Residency::VramHot,
            epoch: Epoch(epoch),
            turn_id: TurnId("t".into()),
            speculative: false,
        }
    }

    #[test]
    fn stale_epoch_leases_are_invalidated() {
        let leases = vec![lease(1, 1), lease(2, 2)];
        let plan = reconcile(Epoch(2), &leases, &[], &HashSet::new());
        assert_eq!(plan.invalidate_leases, vec![LeaseToken(1)]);
    }

    #[test]
    fn ra_spawned_orphan_reclaimed_when_no_valid_lease() {
        // After restart epoch=2, the only lease is stale (epoch 1) → no valid lease.
        let leases = vec![lease(1, 1)];
        let observed = vec![ObservedProcess { pid: 4242, vram_mb: 5000 }];
        let mut ra = HashSet::new();
        ra.insert(4242);
        let plan = reconcile(Epoch(2), &leases, &observed, &ra);
        assert_eq!(plan.reclaim_pids, vec![4242]);
    }

    #[test]
    fn foreign_pid_never_reclaimed() {
        let observed = vec![ObservedProcess { pid: 9999, vram_mb: 5000 }];
        // 9999 is NOT in the RA-spawned set → must never be killed.
        let plan = reconcile(Epoch(2), &[], &observed, &HashSet::new());
        assert!(plan.reclaim_pids.is_empty());
    }

    #[test]
    fn valid_lease_protects_its_process() {
        let leases = vec![lease(7, 2)]; // valid on current epoch
        let observed = vec![ObservedProcess { pid: 4242, vram_mb: 5000 }];
        let mut ra = HashSet::new();
        ra.insert(4242);
        let plan = reconcile(Epoch(2), &leases, &observed, &ra);
        assert!(plan.reclaim_pids.is_empty());
        assert!(plan.invalidate_leases.is_empty());
    }
}
