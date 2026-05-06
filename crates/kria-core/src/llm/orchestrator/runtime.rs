use anyhow::Result;
use async_trait::async_trait;

use crate::resource::L1Residency;

use super::{L1ResidencyMetrics, OrchestratorSnapshot};

/// Runtime-facing control contract for the managed L1 inference service.
///
/// This trait is intentionally narrow and object-safe so higher layers can
/// depend on lifecycle behavior without coupling to `Orchestrator` internals.
#[async_trait]
pub trait L1Runtime: Send + Sync {
    fn snapshot(&self) -> OrchestratorSnapshot;

    fn residency(&self) -> L1Residency;

    fn residency_metrics(&self) -> L1ResidencyMetrics;

    async fn ensure_ready(&self, reason: &str) -> Result<()>;

    async fn release_if_idle(&self, reason: &str) -> Result<bool>;

    async fn evict_to_ram(&self) -> Result<()>;

    async fn reload_to_vram(&self) -> Result<()>;

    async fn evict_to_cpu(&self) -> Result<()> {
        self.evict_to_ram().await
    }

    async fn restore_from_cpu(&self) -> Result<()> {
        self.reload_to_vram().await
    }
}
