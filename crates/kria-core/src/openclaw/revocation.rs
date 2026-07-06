//! Capability / execution revocation (A3.9).
//!
//! A process-global registry of in-flight executions keyed by skill_id. Revoking a skill cancels
//! every in-flight execution's `CancellationToken`; the runtime's cancellation path then tears
//! down the container and releases the HRA lease (no leaked runtime/lease). Also used by
//! `global_halt` to stop everything.

use dashmap::DashMap;
use once_cell::sync::Lazy;
use tokio_util::sync::CancellationToken;

/// One registered in-flight execution.
#[derive(Clone)]
struct Handle {
    execution_id: String,
    token: CancellationToken,
}

static REGISTRY: Lazy<DashMap<String, Vec<Handle>>> = Lazy::new(DashMap::new);

/// Register an in-flight execution. Returns a guard that auto-unregisters on drop.
pub fn register(skill_id: &str, execution_id: &str, token: CancellationToken) -> ExecutionGuard {
    REGISTRY
        .entry(skill_id.to_string())
        .or_default()
        .push(Handle {
            execution_id: execution_id.to_string(),
            token,
        });
    ExecutionGuard {
        skill_id: skill_id.to_string(),
        execution_id: execution_id.to_string(),
    }
}

/// Revoke a skill: cancel all its in-flight executions. Returns the number cancelled.
pub fn revoke(skill_id: &str) -> usize {
    if let Some(handles) = REGISTRY.get(skill_id) {
        for h in handles.iter() {
            h.token.cancel();
        }
        return handles.len();
    }
    0
}

/// Cancel every in-flight OpenClaw execution (wired to `global_halt`).
pub fn revoke_all() -> usize {
    let mut n = 0;
    for entry in REGISTRY.iter() {
        for h in entry.value().iter() {
            h.token.cancel();
            n += 1;
        }
    }
    n
}

/// Number of in-flight executions for a skill (diagnostics/tests).
pub fn in_flight(skill_id: &str) -> usize {
    REGISTRY.get(skill_id).map(|h| h.len()).unwrap_or(0)
}

/// RAII guard: removes the execution from the registry on drop (leak-free bookkeeping).
pub struct ExecutionGuard {
    skill_id: String,
    execution_id: String,
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        if let Some(mut handles) = REGISTRY.get_mut(&self.skill_id) {
            handles.retain(|h| h.execution_id != self.execution_id);
        }
        REGISTRY.remove_if(&self.skill_id, |_, v| v.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoke_cancels_in_flight_and_guard_cleans_up() {
        let token = CancellationToken::new();
        {
            let _guard = register("oc_rev", "exec-1", token.clone());
            assert_eq!(in_flight("oc_rev"), 1);
            let n = revoke("oc_rev");
            assert_eq!(n, 1);
            assert!(token.is_cancelled(), "execution token cancelled on revoke");
        }
        // Guard dropped → unregistered.
        assert_eq!(in_flight("oc_rev"), 0);
    }

    #[test]
    fn revoke_all_cancels_every_execution() {
        let t1 = CancellationToken::new();
        let t2 = CancellationToken::new();
        let _g1 = register("oc_a", "e1", t1.clone());
        let _g2 = register("oc_b", "e2", t2.clone());
        let n = revoke_all();
        assert!(n >= 2);
        assert!(t1.is_cancelled() && t2.is_cancelled());
    }
}
