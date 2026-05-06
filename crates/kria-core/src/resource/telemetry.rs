use std::time::Instant;

use super::gpu_lease::{GpuOwner, ImageLeaseBackendId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VramSnapshot {
    pub total_mb: u64,
    pub free_mb: u64,
    pub used_mb: u64,
}

impl VramSnapshot {
    pub fn from_totals(total_mb: u64, free_mb: u64) -> Self {
        let free_mb = free_mb.min(total_mb);
        let used_mb = total_mb.saturating_sub(free_mb);
        Self {
            total_mb,
            free_mb,
            used_mb,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamSnapshot {
    pub total_mb: u64,
    pub free_mb: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L1Residency {
    Stopped,
    Starting,
    GpuHot,
    RamHotVramCold,
    CpuResidentLegacy,
    ReloadingGpu,
    Error,
}

pub type L1ResidencySnapshot = L1Residency;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct L1RuntimeSnapshot {
    pub residency: L1Residency,
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRuntimeSnapshot {
    pub backend_id: String,
    pub is_generating: bool,
    pub process_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProcess {
    pub pid: u32,
    pub name: String,
    pub vram_usage_mb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub vram: VramSnapshot,
    pub ram: RamSnapshot,
    pub l1: L1RuntimeSnapshot,
    pub image: ImageRuntimeSnapshot,
    pub processes: Vec<ResourceProcess>,
    pub sampled_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationSnapshot {
    pub expected_owner: Option<GpuOwner>,
    pub available_vram_mb: u64,
    pub used_vram_mb: u64,
    pub active_processes: Vec<ResourceProcess>,
    pub sampled_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationResult {
    Healthy,
    VramWarning { available: u64 },
    ProcessMismatch { expected: String, actual: Vec<String> },
    CriticalOomRisk,
}

impl ResourceSnapshot {
    pub fn reconcile(&self, expected_owner: &Option<GpuOwner>) -> ReconciliationResult {
        let reconciliation = self.reconciliation_snapshot(expected_owner);

        if reconciliation.available_vram_mb < 200 || reconciliation.is_near_full() {
            return ReconciliationResult::CriticalOomRisk;
        }

        if let Some(result) = reconciliation.owner_consistency_result() {
            return result;
        }

        if reconciliation.available_vram_mb < 500 {
            return ReconciliationResult::VramWarning {
                available: reconciliation.available_vram_mb,
            };
        }

        ReconciliationResult::Healthy
    }

    pub fn reconciliation_snapshot(
        &self,
        expected_owner: &Option<GpuOwner>,
    ) -> ReconciliationSnapshot {
        let active_processes = self
            .processes
            .iter()
            .filter(|process| process.vram_usage_mb >= 256)
            .cloned()
            .collect::<Vec<_>>();

        ReconciliationSnapshot {
            expected_owner: expected_owner.clone(),
            available_vram_mb: self.vram.free_mb,
            used_vram_mb: self
                .vram
                .used_mb
                .max(self.vram.total_mb.saturating_sub(self.vram.free_mb)),
            active_processes,
            sampled_at: self.sampled_at,
        }
    }
}

impl ReconciliationSnapshot {
    fn owner_consistency_result(&self) -> Option<ReconciliationResult> {
        let actual = self
            .active_processes
            .iter()
            .map(|process| process.name.clone())
            .collect::<Vec<_>>();

        if let Some(expected_owner) = self.expected_owner.as_ref() {
            if actual.is_empty() {
                return None;
            }

            let has_expected_process = self
                .active_processes
                .iter()
                .any(|process| process_name_matches_owner(&process.name, expected_owner));

            if !has_expected_process {
                return Some(ReconciliationResult::ProcessMismatch {
                    expected: owner_label(expected_owner),
                    actual,
                });
            }

            return None;
        }

        if actual.is_empty() {
            None
        } else {
            Some(ReconciliationResult::ProcessMismatch {
                expected: "Idle".to_string(),
                actual,
            })
        }
    }

    fn is_near_full(&self) -> bool {
        let total = self.available_vram_mb.saturating_add(self.used_vram_mb);
        if total == 0 {
            return false;
        }

        self.used_vram_mb.saturating_mul(100) >= total.saturating_mul(95)
    }
}

fn owner_label(owner: &GpuOwner) -> String {
    match owner {
        GpuOwner::L1Worker => "L1Worker".to_string(),
        GpuOwner::ImageBackend(backend) => match backend {
            ImageLeaseBackendId::ComfyUi => "ImageBackend(ComfyUi)".to_string(),
            ImageLeaseBackendId::CloudFallback => "ImageBackend(CloudFallback)".to_string(),
            ImageLeaseBackendId::Other(name) => format!("ImageBackend({name})"),
        },
        GpuOwner::Vision => "Vision".to_string(),
        GpuOwner::Speech => "Speech".to_string(),
        GpuOwner::Maintenance => "Maintenance".to_string(),
    }
}

fn process_name_matches_owner(name: &str, owner: &GpuOwner) -> bool {
    let name = name.to_ascii_lowercase();
    let markers = owner_markers(owner);
    markers.iter().any(|marker| name.contains(marker))
}

fn owner_markers(owner: &GpuOwner) -> &'static [&'static str] {
    match owner {
        GpuOwner::L1Worker => &["llama", "l1", "phi", "qwen"],
        GpuOwner::ImageBackend(ImageLeaseBackendId::ComfyUi) => {
            &["comfy", "diffusion", "sd", "python"]
        }
        GpuOwner::ImageBackend(ImageLeaseBackendId::CloudFallback) => &["cloud", "remote"],
        GpuOwner::ImageBackend(ImageLeaseBackendId::Other(_)) => &["image", "backend"],
        GpuOwner::Vision => &["vision", "ocr", "sidecar", "python"],
        GpuOwner::Speech => &["speech", "whisper", "piper", "tts", "stt"],
        GpuOwner::Maintenance => &["maintenance", "cleanup"],
    }
}

#[async_trait::async_trait]
pub trait ResourceTelemetry: Send + Sync {
    async fn sample(&self) -> anyhow::Result<ResourceSnapshot>;
}
