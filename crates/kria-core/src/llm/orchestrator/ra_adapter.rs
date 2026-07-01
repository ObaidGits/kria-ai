//! LLM → Resource Authority adapter (HRA Task 12 integration surface).
//!
//! Wraps the existing `Orchestrator` as a `ModelLifecycle` so the L1 LLM becomes a model the
//! `ResidencyManager` can drive (load/warm/cool/unload/swap) WITHOUT changing orchestrator
//! behavior. This is the additive bridge that lets the LLM consumer route residency through the RA
//! while the legacy code path remains intact behind the bypass switch.
//!
//! It delegates to the orchestrator's already-implemented, tested operations:
//! - `ensure_ready` (load/warm), `evict_to_ram` (cool), `release_if_idle` (unload),
//!   `reload_to_vram`/`evict_to_ram` (swap), `snapshot` (health).

use std::sync::Arc;

use async_trait::async_trait;

use crate::resource::authority::lifecycle::{
    ModelDescriptor, ModelHealth, ModelLifecycle, ResidencyState,
};
use crate::resource::authority::types::{ConsumerId, Residency};

use super::Orchestrator;

/// `ModelLifecycle` adapter over the hardware `Orchestrator`.
pub struct OrchestratorModel {
    orchestrator: Arc<Orchestrator>,
    model_id: String,
    vram_est_mb: u64,
    ram_est_mb: u64,
}

impl OrchestratorModel {
    pub fn new(
        orchestrator: Arc<Orchestrator>,
        model_id: impl Into<String>,
        vram_est_mb: u64,
        ram_est_mb: u64,
    ) -> Self {
        Self {
            orchestrator,
            model_id: model_id.into(),
            vram_est_mb,
            ram_est_mb,
        }
    }

    /// Map the orchestrator snapshot to a ResidencyState (for diagnostics/UI).
    pub fn residency_state(&self) -> ResidencyState {
        let snap = self.orchestrator.snapshot();
        if !snap.server_healthy {
            ResidencyState::Unloaded
        } else if snap.current_ngl > 0 {
            ResidencyState::VramHot
        } else {
            ResidencyState::RamWarm
        }
    }
}

#[async_trait]
impl ModelLifecycle for OrchestratorModel {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor {
            id: self.model_id.clone(),
            kind: ConsumerId::Llm,
            vram_est_mb: self.vram_est_mb,
            ram_est_mb: self.ram_est_mb,
        }
    }

    async fn load(&self) -> anyhow::Result<()> {
        self.orchestrator.ensure_ready("ra_residency_load").await
    }

    async fn warm(&self) -> anyhow::Result<()> {
        // ensure_ready already warms; a second call is a cheap fast-path no-op when healthy.
        self.orchestrator.ensure_ready("ra_residency_warm").await
    }

    async fn cool(&self) -> anyhow::Result<()> {
        self.orchestrator.evict_to_ram().await
    }

    async fn unload(&self) -> anyhow::Result<()> {
        // Best-effort release when idle; not an error if a process was still live.
        self.orchestrator.release_if_idle("ra_residency_unload").await?;
        Ok(())
    }

    async fn swap(&self, target: Residency) -> anyhow::Result<()> {
        match target {
            Residency::VramHot | Residency::Cloud => self.orchestrator.reload_to_vram().await,
            Residency::RamWarm | Residency::DiskCold => self.orchestrator.evict_to_ram().await,
            Residency::Unloaded => {
                self.orchestrator.release_if_idle("ra_residency_swap_unload").await?;
                Ok(())
            }
        }
    }

    fn health(&self) -> ModelHealth {
        if self.orchestrator.snapshot().server_healthy {
            ModelHealth::Healthy
        } else {
            ModelHealth::Degraded
        }
    }
}
