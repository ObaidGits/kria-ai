//! Model lifecycle contract (HRA Task 11 / R4.1).
//!
//! Uniform contract every model type (LLM, STT, TTS, Vision, OCR, Embedding, Image, Cloud LLM)
//! implements. The `ResidencyManager` (Task 42) is the SOLE executor of these transitions; engines
//! request residency targets through it rather than calling these methods directly (Property 15).

use async_trait::async_trait;

use super::types::{ConsumerId, Residency};

/// Residency state machine states owned by the ResidencyManager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyState {
    Unloaded,
    Loading,
    VramHot,
    RamWarm,
    Cooling,
    Swapping,
    Restoring,
}

impl ResidencyState {
    pub fn from_residency(r: Residency) -> Self {
        match r {
            Residency::VramHot => Self::VramHot,
            Residency::RamWarm | Residency::DiskCold => Self::RamWarm,
            Residency::Cloud => Self::VramHot, // cloud models are "hot" when connected
            Residency::Unloaded => Self::Unloaded,
        }
    }

    /// Rank residency tiers for comparison: Unloaded < RamWarm < VramHot. Transient states
    /// (Loading/Cooling/Swapping/Restoring) rank as their not-yet-arrived floor (Unloaded) so a
    /// success check is conservative — only a settled state counts as resident.
    fn tier_rank(self) -> u8 {
        match self {
            Self::VramHot => 2,
            Self::RamWarm => 1,
            _ => 0,
        }
    }

    /// True when this state is resident at least at the requested tier (used to confirm a
    /// transition actually arrived, since the executor reports load failure as Unloaded).
    pub fn is_resident_at_least(self, target: Residency) -> bool {
        self.tier_rank() >= Self::from_residency(target).tier_rank()
            && self.tier_rank() > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelHealth {
    Unknown,
    Healthy,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub kind: ConsumerId,
    pub vram_est_mb: u64,
    pub ram_est_mb: u64,
}

/// Operations a model runtime exposes. Implemented by adapters around llama-server, whisper,
/// piper, ComfyUI, embedding pools, cloud backends. Object-safe.
#[async_trait]
pub trait ModelLifecycle: Send + Sync {
    fn descriptor(&self) -> ModelDescriptor;
    async fn load(&self) -> anyhow::Result<()>;
    async fn warm(&self) -> anyhow::Result<()>;
    async fn cool(&self) -> anyhow::Result<()>;
    async fn unload(&self) -> anyhow::Result<()>;
    async fn swap(&self, target: Residency) -> anyhow::Result<()>;
    fn health(&self) -> ModelHealth;
}
