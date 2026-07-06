//! Telemetry Collector model (HRA Task 3 / R3).
//!
//! Single host-wide snapshot type + the logic that applies a snapshot to the authoritative
//! `DeviceTable`. The runtime owns one collector thread (evolving `orchestrator/telemetry.rs`) that
//! samples every GPU + CPU + RAM + thermal and publishes an immutable `HostSnapshot`; the RA and
//! consumers read snapshots — no subsystem samples independently (R3.2).
//!
//! This module is pure: snapshot construction + `apply_to(DeviceTable)` are deterministic and unit
//! tested with synthetic snapshots. Live sampling (NVML/sysinfo) is injected by the runtime.

use super::device_table::{DeviceRecord, DeviceTable};
use super::types::DeviceId;

/// Per-GPU live figures.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceLive {
    pub index: u32,
    pub free_vram_mb: u64,
    pub total_vram_mb: u64,
    pub util_pct: u32,
    pub temp_c: Option<f32>,
    /// (pid, vram_mb) of processes seen on this GPU.
    pub processes: Vec<(u32, u64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CpuLive {
    pub per_core_pct: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamLive {
    pub total_mb: u64,
    pub free_mb: u64,
}

/// Immutable host snapshot published by the collector.
#[derive(Debug, Clone, PartialEq)]
pub struct HostSnapshot {
    pub seq: u64,
    pub gpus: Vec<DeviceLive>,
    pub cpu: CpuLive,
    pub ram: RamLive,
    /// Monotonic milliseconds since collector start (staleness check).
    pub sampled_at_ms: u64,
}

impl HostSnapshot {
    /// Staleness against a `now_ms`; used to flag decisions on stale telemetry (R3.4).
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.sampled_at_ms)
    }

    /// All GPU processes flattened (pid, vram_mb) — used by the Reconciler.
    pub fn all_gpu_processes(&self) -> Vec<(u32, u64)> {
        self.gpus.iter().flat_map(|g| g.processes.clone()).collect()
    }

    /// Apply this snapshot to the device table: ensure a record exists per GPU and refresh live
    /// free figures. Reservations (owned by the table) are preserved.
    pub fn apply_to(&self, table: &mut DeviceTable, gpu_safety_mb: u64) {
        for g in &self.gpus {
            let id = DeviceId::Gpu(g.index);
            if table.get(&id).is_none() {
                table.upsert(DeviceRecord::gpu(g.index, g.total_vram_mb, gpu_safety_mb));
            }
            table.refresh_free(&id, g.free_vram_mb, self.ram.free_mb);
        }
        // Refresh CPU RAM.
        if table.get(&DeviceId::Cpu).is_none() {
            table.upsert(DeviceRecord::cpu(self.ram.total_mb));
        }
        table.refresh_free(&DeviceId::Cpu, 0, self.ram.free_mb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> HostSnapshot {
        HostSnapshot {
            seq: 1,
            gpus: vec![
                DeviceLive {
                    index: 0,
                    free_vram_mb: 8000,
                    total_vram_mb: 12288,
                    util_pct: 40,
                    temp_c: Some(65.0),
                    processes: vec![(111, 4000)],
                },
                DeviceLive {
                    index: 1,
                    free_vram_mb: 6000,
                    total_vram_mb: 8192,
                    util_pct: 10,
                    temp_c: Some(55.0),
                    processes: vec![],
                },
            ],
            cpu: CpuLive {
                per_core_pct: vec![20, 30, 10, 5],
            },
            ram: RamLive {
                total_mb: 32768,
                free_mb: 20000,
            },
            sampled_at_ms: 1000,
        }
    }

    #[test]
    fn apply_creates_and_refreshes_devices() {
        let mut t = DeviceTable::new();
        snap().apply_to(&mut t, 512);
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().free_vram_mb, 8000);
        assert_eq!(t.get(&DeviceId::Gpu(1)).unwrap().free_vram_mb, 6000);
        assert!(t.get(&DeviceId::Cpu).is_some());
        // multi-GPU ordering by free
        assert_eq!(t.usable_gpus()[0].id, DeviceId::Gpu(0));
    }

    #[test]
    fn apply_preserves_reservations() {
        let mut t = DeviceTable::new();
        snap().apply_to(&mut t, 512);
        assert!(t.reserve_vram(&DeviceId::Gpu(0), 3000));
        // a fresh snapshot refresh must not wipe the reservation
        snap().apply_to(&mut t, 512);
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().reserved_vram_mb, 3000);
    }

    #[test]
    fn staleness_and_process_flatten() {
        let s = snap();
        assert_eq!(s.age_ms(1500), 500);
        assert_eq!(s.all_gpu_processes(), vec![(111, 4000)]);
    }

    #[test]
    fn empty_gpu_snapshot_preserves_prior_free() {
        // C5: an Unknown reading (no GPUs in the snapshot — e.g. nvidia-smi briefly unavailable)
        // must NOT overwrite a previously-known GPU free figure with 0. The Planner must never
        // decide on invalid (zeroed) data; it keeps the last measured value instead.
        let mut t = DeviceTable::new();
        snap().apply_to(&mut t, 512); // establishes GPU0 free = 8000
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().free_vram_mb, 8000);

        // An Unknown snapshot (no gpus) is applied — GPU free must be unchanged.
        let unknown = HostSnapshot {
            seq: 2,
            gpus: vec![],
            cpu: CpuLive {
                per_core_pct: vec![],
            },
            ram: RamLive {
                total_mb: 32768,
                free_mb: 19000,
            },
            sampled_at_ms: 2000,
        };
        unknown.apply_to(&mut t, 512);
        assert_eq!(
            t.get(&DeviceId::Gpu(0)).unwrap().free_vram_mb,
            8000,
            "Unknown snapshot must not zero a previously-measured GPU free figure"
        );
    }
}
