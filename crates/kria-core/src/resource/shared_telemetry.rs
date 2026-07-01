//! Resource telemetry source for the shared GPU lease (HRA production item 2).
//!
//! Backs `GpuLeaseManager::set_resource_telemetry` with REAL device data (VRAM via the platform
//! `VramProfiler`, RAM via sysinfo) so lease recovery/reconciliation actually verifies the GPU is
//! free after a release — instead of the old dead path (no telemetry → recovery times out →
//! Degraded → blocks all consumers).

use std::sync::Arc;

use async_trait::async_trait;

use crate::platform::vram::{build_profiler, VramProfiler};
use crate::resource::telemetry::{
    ImageRuntimeSnapshot, L1Residency, L1RuntimeSnapshot, RamSnapshot, ResourceSnapshot,
    ResourceTelemetry, VramSnapshot,
};
use crate::resource::telemetry_hub::global_telemetry_hub;

/// Live resource telemetry: real VRAM (profiler) + RAM (sysinfo). Per-process VRAM attribution is
/// not available cheaply cross-platform, so `processes` is left empty (reconciliation treats an
/// empty process list with no expected owner as healthy).
///
/// HRA Phase A1 (telemetry unification): when the process-wide [`TelemetryHub`] is registered, this
/// source borrows the hub's single shared profiler instead of building its own NVML/ROCm context,
/// and a fresh reading goes through the hub so the lease, the authority, and the watchdog all see a
/// coherent VRAM value. Without a hub it falls back to its own profiler (e.g. headless tests).
///
/// [`TelemetryHub`]: crate::resource::telemetry_hub::TelemetryHub
pub struct SharedResourceTelemetry {
    profiler: Arc<dyn VramProfiler>,
}

impl SharedResourceTelemetry {
    pub fn new() -> Self {
        // Prefer the single shared profiler owned by the global telemetry hub so the whole process
        // uses ONE device context. Fall back to building one only if the hub is not yet set.
        let profiler = global_telemetry_hub()
            .map(|h| h.profiler())
            .unwrap_or_else(build_profiler);
        Self { profiler }
    }

    pub fn with_profiler(profiler: Arc<dyn VramProfiler>) -> Self {
        Self { profiler }
    }
}

impl Default for SharedResourceTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ResourceTelemetry for SharedResourceTelemetry {
    async fn sample(&self) -> anyhow::Result<ResourceSnapshot> {
        let v = self.profiler.snapshot().await;

        let (ram_total_mb, ram_free_mb) = tokio::task::spawn_blocking(|| {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            (
                sys.total_memory() / (1024 * 1024),
                sys.available_memory() / (1024 * 1024),
            )
        })
        .await
        .unwrap_or((0, 0));

        Ok(ResourceSnapshot {
            vram: VramSnapshot::from_totals(v.total_mb, v.free_mb),
            ram: RamSnapshot {
                total_mb: ram_total_mb,
                free_mb: ram_free_mb,
            },
            l1: L1RuntimeSnapshot {
                residency: L1Residency::Stopped,
                process_id: None,
            },
            image: ImageRuntimeSnapshot {
                backend_id: "shared".to_string(),
                is_generating: false,
                process_id: None,
            },
            processes: Vec::new(),
            sampled_at: std::time::Instant::now(),
        })
    }
}
