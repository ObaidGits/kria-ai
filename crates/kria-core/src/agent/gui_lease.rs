//! Foreground lease runtime for GUI-sensitive workflows.
//!
//! The lease is intentionally small: one active GUI owner at a time, bounded
//! expiry, and explicit release. It prevents concurrent workflows from typing or
//! clicking into each other's windows without becoming a heavyweight scheduler.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ForegroundLease {
    pub workflow_id: String,
    pub owner: String,
    pub acquired_at: Instant,
    pub expires_at: Instant,
}

#[derive(Debug, thiserror::Error)]
pub enum ForegroundLeaseError {
    #[error(
        "GUI foreground is leased by workflow '{workflow_id}' until {remaining_ms}ms from now"
    )]
    AlreadyLeased {
        workflow_id: String,
        remaining_ms: u128,
    },
}

#[derive(Debug, Default)]
struct LeaseState {
    current: Option<ForegroundLease>,
}

#[derive(Debug, Clone, Default)]
pub struct ForegroundLeaseManager {
    state: Arc<Mutex<LeaseState>>,
}

impl ForegroundLeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(
        &self,
        workflow_id: impl Into<String>,
        owner: impl Into<String>,
        ttl: Duration,
    ) -> Result<ForegroundLeaseGuard, ForegroundLeaseError> {
        let workflow_id = workflow_id.into();
        let owner = owner.into();
        let now = Instant::now();
        let mut state = self.state.lock().await;

        if let Some(current) = &state.current {
            if current.expires_at > now && current.workflow_id != workflow_id {
                return Err(ForegroundLeaseError::AlreadyLeased {
                    workflow_id: current.workflow_id.clone(),
                    remaining_ms: current.expires_at.duration_since(now).as_millis(),
                });
            }
        }

        let lease = ForegroundLease {
            workflow_id: workflow_id.clone(),
            owner,
            acquired_at: now,
            expires_at: now + ttl,
        };
        state.current = Some(lease.clone());
        Ok(ForegroundLeaseGuard {
            manager: self.clone(),
            workflow_id,
            released: false,
        })
    }

    pub async fn current(&self) -> Option<ForegroundLease> {
        let now = Instant::now();
        let mut state = self.state.lock().await;
        if state
            .current
            .as_ref()
            .map(|lease| lease.expires_at <= now)
            .unwrap_or(false)
        {
            state.current = None;
        }
        state.current.clone()
    }

    async fn release(&self, workflow_id: &str) {
        let mut state = self.state.lock().await;
        if state
            .current
            .as_ref()
            .map(|lease| lease.workflow_id == workflow_id)
            .unwrap_or(false)
        {
            state.current = None;
        }
    }
}

#[derive(Debug)]
pub struct ForegroundLeaseGuard {
    manager: ForegroundLeaseManager,
    workflow_id: String,
    released: bool,
}

impl ForegroundLeaseGuard {
    pub async fn release(mut self) {
        if !self.released {
            self.manager.release(&self.workflow_id).await;
            self.released = true;
        }
    }
}

impl Drop for ForegroundLeaseGuard {
    fn drop(&mut self) {
        if !self.released {
            let manager = self.manager.clone();
            let workflow_id = self.workflow_id.clone();
            tokio::spawn(async move {
                manager.release(&workflow_id).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn foreground_lease_allows_one_owner_at_a_time() {
        let manager = ForegroundLeaseManager::new();
        let _guard = manager
            .acquire("workflow-a", "test", Duration::from_secs(30))
            .await
            .expect("first lease should acquire");

        let err = manager
            .acquire("workflow-b", "test", Duration::from_secs(30))
            .await
            .expect_err("second workflow should be denied");

        assert!(matches!(
            err,
            ForegroundLeaseError::AlreadyLeased {
                workflow_id,
                ..
            } if workflow_id == "workflow-a"
        ));
    }

    #[tokio::test]
    async fn foreground_lease_releases_explicitly() {
        let manager = ForegroundLeaseManager::new();
        let guard = manager
            .acquire("workflow-a", "test", Duration::from_secs(30))
            .await
            .expect("lease should acquire");
        guard.release().await;

        manager
            .acquire("workflow-b", "test", Duration::from_secs(30))
            .await
            .expect("lease should be available after release");
    }
}
