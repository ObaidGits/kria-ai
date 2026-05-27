//! Generic resource capability leases for workflow arbitration.
//!
//! This is a small coordination primitive, not a global scheduler. Existing
//! specialized managers such as `ForegroundLeaseManager`, `GpuLeaseManager`,
//! and VM target leases remain authoritative for their domains. This module
//! gives HITL/workflow code a common ownership vocabulary.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::agent::collaborative_decision::ActionProposal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceKind {
    GuiForeground,
    KeyboardMouse,
    BrowserProfile,
    FilesystemPath,
    VmTarget,
    GpuModel,
    VerifierSlot,
    DelegatedWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AccessMode {
    Read,
    Write,
    Exclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OwnershipState {
    Observed,
    Reserved,
    Leased,
    Delegated,
    Released,
    Orphaned,
}

#[derive(Debug, Clone)]
pub struct ResourceLease {
    pub lease_id: String,
    pub workflow_id: String,
    pub stage_id: Option<String>,
    pub action_hash: String,
    pub kind: ResourceKind,
    pub scope: String,
    pub access_mode: AccessMode,
    pub owner: String,
    pub state: OwnershipState,
    pub acquired_at: Instant,
    pub expires_at: Instant,
    pub preemptible: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ResourceLeaseError {
    #[error("resource '{scope}' is leased by workflow '{workflow_id}' for {remaining_ms}ms")]
    Conflict {
        scope: String,
        workflow_id: String,
        remaining_ms: u128,
    },
}

#[derive(Debug, Default)]
struct ResourceLeaseState {
    active: HashMap<String, Vec<ResourceLease>>,
}

static GLOBAL_RESOURCE_LEASE_STATE: Lazy<Arc<Mutex<ResourceLeaseState>>> =
    Lazy::new(|| Arc::new(Mutex::new(ResourceLeaseState::default())));

#[derive(Debug, Clone, Default)]
pub struct ResourceLeaseManager {
    state: Arc<Mutex<ResourceLeaseState>>,
}

impl ResourceLeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn global() -> Self {
        Self {
            state: Arc::clone(&GLOBAL_RESOURCE_LEASE_STATE),
        }
    }

    pub async fn acquire(
        &self,
        request: ResourceLeaseRequest,
    ) -> Result<ResourceLeaseGuard, ResourceLeaseError> {
        let now = Instant::now();
        let mut state = self.state.lock().await;
        prune_expired(&mut state, now);

        let key = lease_key(request.kind, &request.scope);
        if let Some(existing) = state.active.get(&key) {
            for lease in existing {
                if lease.workflow_id == request.workflow_id {
                    continue;
                }
                if conflicts(lease.access_mode, request.access_mode) {
                    return Err(ResourceLeaseError::Conflict {
                        scope: request.scope,
                        workflow_id: lease.workflow_id.clone(),
                        remaining_ms: lease.expires_at.duration_since(now).as_millis(),
                    });
                }
            }
        }

        let lease = ResourceLease {
            lease_id: uuid::Uuid::new_v4().to_string(),
            workflow_id: request.workflow_id,
            stage_id: request.stage_id,
            action_hash: request.action_hash,
            kind: request.kind,
            scope: request.scope,
            access_mode: request.access_mode,
            owner: request.owner,
            state: OwnershipState::Leased,
            acquired_at: now,
            expires_at: now + request.ttl,
            preemptible: request.preemptible,
        };

        state.active.entry(key).or_default().push(lease.clone());
        Ok(ResourceLeaseGuard {
            manager: self.clone(),
            lease_id: lease.lease_id.clone(),
            released: false,
            lease,
        })
    }

    pub async fn active_leases(&self) -> Vec<ResourceLease> {
        let now = Instant::now();
        let mut state = self.state.lock().await;
        prune_expired(&mut state, now);
        state
            .active
            .values()
            .flat_map(|leases| leases.iter().cloned())
            .collect()
    }

    pub async fn acquire_requirements(
        &self,
        action: &str,
        proposal: &ActionProposal,
        requirements: &[ResourceRequirement],
    ) -> Result<Vec<ResourceLeaseGuard>, ResourceLeaseError> {
        let mut guards = Vec::new();
        for requirement in requirements {
            let request = ResourceLeaseRequest {
                workflow_id: proposal.workflow_id.clone(),
                stage_id: Some(proposal.stage_id.clone()),
                action_hash: proposal.action_hash.clone(),
                kind: requirement.kind,
                scope: requirement.scope.clone(),
                access_mode: requirement.access_mode,
                owner: format!("tool:{action}"),
                ttl: requirement.ttl(),
                preemptible: requirement.preemptible,
            };
            match self.acquire(request).await {
                Ok(guard) => guards.push(guard),
                Err(error) => {
                    drop(guards);
                    return Err(error);
                }
            }
        }
        Ok(guards)
    }

    async fn release(&self, lease_id: &str) {
        let mut state = self.state.lock().await;
        for leases in state.active.values_mut() {
            leases.retain(|lease| lease.lease_id != lease_id);
        }
        state.active.retain(|_, leases| !leases.is_empty());
    }
}

#[derive(Debug, Clone)]
pub struct ResourceLeaseRequest {
    pub workflow_id: String,
    pub stage_id: Option<String>,
    pub action_hash: String,
    pub kind: ResourceKind,
    pub scope: String,
    pub access_mode: AccessMode,
    pub owner: String,
    pub ttl: Duration,
    pub preemptible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequirement {
    pub kind: ResourceKind,
    pub scope: String,
    pub access_mode: AccessMode,
    pub ttl_ms: u64,
    pub preemptible: bool,
}

impl ResourceRequirement {
    pub fn new(
        kind: ResourceKind,
        scope: impl Into<String>,
        access_mode: AccessMode,
        ttl: Duration,
    ) -> Self {
        Self {
            kind,
            scope: scope.into(),
            access_mode,
            ttl_ms: ttl.as_millis().min(u128::from(u64::MAX)) as u64,
            preemptible: false,
        }
    }

    pub fn ttl(&self) -> Duration {
        Duration::from_millis(self.ttl_ms)
    }
}

#[derive(Debug)]
pub struct ResourceLeaseGuard {
    manager: ResourceLeaseManager,
    lease_id: String,
    released: bool,
    pub lease: ResourceLease,
}

impl ResourceLeaseGuard {
    pub async fn release(mut self) {
        if !self.released {
            self.manager.release(&self.lease_id).await;
            self.released = true;
        }
    }
}

impl Drop for ResourceLeaseGuard {
    fn drop(&mut self) {
        if !self.released {
            let manager = self.manager.clone();
            let lease_id = self.lease_id.clone();
            tokio::spawn(async move {
                manager.release(&lease_id).await;
            });
        }
    }
}

fn lease_key(kind: ResourceKind, scope: &str) -> String {
    format!("{kind:?}:{scope}")
}

fn conflicts(existing: AccessMode, requested: AccessMode) -> bool {
    !matches!((existing, requested), (AccessMode::Read, AccessMode::Read))
}

fn prune_expired(state: &mut ResourceLeaseState, now: Instant) {
    for leases in state.active.values_mut() {
        leases.retain(|lease| lease.expires_at > now);
    }
    state.active.retain(|_, leases| !leases.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(workflow_id: &str, access_mode: AccessMode) -> ResourceLeaseRequest {
        ResourceLeaseRequest {
            workflow_id: workflow_id.to_string(),
            stage_id: Some("stage-1".to_string()),
            action_hash: "action-hash-1".to_string(),
            kind: ResourceKind::FilesystemPath,
            scope: "/tmp/kria-test".to_string(),
            access_mode,
            owner: "test".to_string(),
            ttl: Duration::from_secs(30),
            preemptible: false,
        }
    }

    #[tokio::test]
    async fn shared_reads_can_coexist() {
        let manager = ResourceLeaseManager::new();
        let _a = manager
            .acquire(request("workflow-a", AccessMode::Read))
            .await
            .expect("first read lease");
        let _b = manager
            .acquire(request("workflow-b", AccessMode::Read))
            .await
            .expect("second read lease");

        assert_eq!(manager.active_leases().await.len(), 2);
    }

    #[tokio::test]
    async fn write_conflicts_with_other_workflow_read() {
        let manager = ResourceLeaseManager::new();
        let _a = manager
            .acquire(request("workflow-a", AccessMode::Read))
            .await
            .expect("read lease");

        let err = manager
            .acquire(request("workflow-b", AccessMode::Write))
            .await
            .expect_err("write should conflict");

        assert!(matches!(err, ResourceLeaseError::Conflict { .. }));
    }

    #[tokio::test]
    async fn releasing_lease_allows_new_writer() {
        let manager = ResourceLeaseManager::new();
        let guard = manager
            .acquire(request("workflow-a", AccessMode::Exclusive))
            .await
            .expect("exclusive lease");
        guard.release().await;

        manager
            .acquire(request("workflow-b", AccessMode::Write))
            .await
            .expect("writer after release");
    }
}
