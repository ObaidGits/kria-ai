//! DeviceTable (HRA Task 4 / R1.3, R5.1, R21.4).
//!
//! The single authoritative table of devices (CPU, each GPU index, cloud pools) with live capacity,
//! reservations, and health. There is exactly one accounting of free/reserved per device
//! (Property 1 / Property 18 — no duplicate counters). Cloud devices carry a circuit breaker so the
//! Planner avoids tripped pools (Task 29 / R21.4).
//!
//! This in-memory structure is pure and deterministic given its inputs; the live `free` figures are
//! refreshed from the telemetry collector by the runtime, while `reserved` is owned here.

use std::collections::HashMap;

use super::budget::{BandPolicy, Budget};
use super::types::{Capacity, DeviceId, DeviceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceHealth {
    Healthy,
    Degraded,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    Closed,
    HalfOpen,
    Open,
}

/// One device's authoritative record.
#[derive(Debug, Clone)]
pub struct DeviceRecord {
    pub id: DeviceId,
    pub kind: DeviceKind,
    /// Total physical capacity.
    pub total: Capacity,
    /// Currently free (refreshed from telemetry for live devices).
    pub free_vram_mb: u64,
    pub free_ram_mb: u64,
    /// Reserved by active leases (owned by the table; the single reservation accounting).
    pub reserved_vram_mb: u64,
    /// Derived memory bands.
    pub budget: Budget,
    pub health: DeviceHealth,
    /// Cloud-only: circuit breaker. Local devices are always `Closed`.
    pub breaker: BreakerState,
    /// Reserved safety margin used for band derivation.
    pub safety_mb: u64,
}

impl DeviceRecord {
    pub fn gpu(index: u32, total_vram_mb: u64, safety_mb: u64) -> Self {
        Self {
            id: DeviceId::Gpu(index),
            kind: DeviceKind::Gpu,
            total: Capacity::vram(total_vram_mb),
            free_vram_mb: total_vram_mb,
            free_ram_mb: 0,
            reserved_vram_mb: 0,
            budget: Budget::derive(total_vram_mb, safety_mb, BandPolicy::default()),
            health: DeviceHealth::Healthy,
            breaker: BreakerState::Closed,
            safety_mb,
        }
    }

    pub fn cpu(total_ram_mb: u64) -> Self {
        Self {
            id: DeviceId::Cpu,
            kind: DeviceKind::Cpu,
            total: Capacity {
                ram_mb: total_ram_mb,
                ..Default::default()
            },
            free_vram_mb: 0,
            free_ram_mb: total_ram_mb,
            reserved_vram_mb: 0,
            budget: Budget::derive(0, 0, BandPolicy::default()),
            health: DeviceHealth::Healthy,
            breaker: BreakerState::Closed,
            safety_mb: 0,
        }
    }

    pub fn cloud(pool: impl Into<String>) -> Self {
        Self {
            id: DeviceId::CloudPool(pool.into()),
            kind: DeviceKind::Cloud,
            total: Capacity {
                quota_rps: Some(0),
                ..Default::default()
            },
            free_vram_mb: 0,
            free_ram_mb: 0,
            reserved_vram_mb: 0,
            budget: Budget::derive(0, 0, BandPolicy::default()),
            health: DeviceHealth::Healthy,
            breaker: BreakerState::Closed,
            safety_mb: 0,
        }
    }

    /// Effective free VRAM after subtracting current reservations.
    pub fn effective_free_vram_mb(&self) -> u64 {
        self.free_vram_mb.saturating_sub(self.reserved_vram_mb)
    }

    /// Whether `need_vram_mb` can be admitted without breaching the hard limit (Property 1/18).
    pub fn can_admit_vram(&self, need_vram_mb: u64) -> bool {
        if self.kind != DeviceKind::Gpu {
            return true; // CPU/cloud are not VRAM-gated here
        }
        !self
            .budget
            .admission_breaches_hard(self.effective_free_vram_mb(), need_vram_mb)
    }

    /// Whether this device is usable for placement right now.
    pub fn usable(&self) -> bool {
        self.health != DeviceHealth::Offline && self.breaker != BreakerState::Open
    }
}

/// The authoritative device table. Single mutable owner lives in the RA; consumers see read-only.
#[derive(Debug, Clone, Default)]
pub struct DeviceTable {
    devices: HashMap<DeviceId, DeviceRecord>,
}

impl DeviceTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, record: DeviceRecord) {
        self.devices.insert(record.id.clone(), record);
    }

    pub fn get(&self, id: &DeviceId) -> Option<&DeviceRecord> {
        self.devices.get(id)
    }

    pub fn devices(&self) -> impl Iterator<Item = &DeviceRecord> {
        self.devices.values()
    }

    /// Refresh live free figures for a device from telemetry (runtime calls this).
    pub fn refresh_free(&mut self, id: &DeviceId, free_vram_mb: u64, free_ram_mb: u64) {
        if let Some(d) = self.devices.get_mut(id) {
            d.free_vram_mb = free_vram_mb;
            d.free_ram_mb = free_ram_mb;
        }
    }

    /// Reserve VRAM on a device. Returns false if it would breach the hard limit (no over-commit,
    /// Property 1). The reservation is the single accounting of in-flight budget.
    pub fn reserve_vram(&mut self, id: &DeviceId, vram_mb: u64) -> bool {
        let Some(d) = self.devices.get_mut(id) else {
            return false;
        };
        if !d.usable() || !d.can_admit_vram(vram_mb) {
            return false;
        }
        d.reserved_vram_mb = d.reserved_vram_mb.saturating_add(vram_mb);
        true
    }

    pub fn release_vram(&mut self, id: &DeviceId, vram_mb: u64) {
        if let Some(d) = self.devices.get_mut(id) {
            d.reserved_vram_mb = d.reserved_vram_mb.saturating_sub(vram_mb);
        }
    }

    pub fn set_health(&mut self, id: &DeviceId, health: DeviceHealth) {
        if let Some(d) = self.devices.get_mut(id) {
            d.health = health;
        }
    }

    pub fn set_breaker(&mut self, id: &DeviceId, breaker: BreakerState) {
        if let Some(d) = self.devices.get_mut(id) {
            d.breaker = breaker;
        }
    }

    /// All usable GPU devices, ordered by effective free VRAM (most free first) for stable planning.
    pub fn usable_gpus(&self) -> Vec<&DeviceRecord> {
        let mut gpus: Vec<&DeviceRecord> = self
            .devices
            .values()
            .filter(|d| d.kind == DeviceKind::Gpu && d.usable())
            .collect();
        gpus.sort_by(|a, b| {
            b.effective_free_vram_mb()
                .cmp(&a.effective_free_vram_mb())
                .then(device_index(&a.id).cmp(&device_index(&b.id)))
        });
        gpus
    }

    pub fn usable_cloud(&self) -> Vec<&DeviceRecord> {
        let mut pools: Vec<&DeviceRecord> = self
            .devices
            .values()
            .filter(|d| d.kind == DeviceKind::Cloud && d.usable())
            .collect();
        pools.sort_by(|a, b| pool_name(&a.id).cmp(pool_name(&b.id)));
        pools
    }
}

fn device_index(id: &DeviceId) -> u32 {
    match id {
        DeviceId::Gpu(i) => *i,
        _ => u32::MAX,
    }
}

fn pool_name(id: &DeviceId) -> &str {
    match id {
        DeviceId::CloudPool(name) => name.as_str(),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> DeviceTable {
        let mut t = DeviceTable::new();
        t.upsert(DeviceRecord::gpu(0, 12288, 512));
        t.upsert(DeviceRecord::gpu(1, 8192, 512));
        t.upsert(DeviceRecord::cpu(32768));
        t.upsert(DeviceRecord::cloud("openai"));
        t
    }

    #[test]
    fn reserve_respects_hard_limit_no_overcommit() {
        let mut t = table();
        let g0 = DeviceId::Gpu(0);
        // 12 GB total, hard limit ~ derived; reserving almost all should eventually be refused.
        assert!(t.reserve_vram(&g0, 6000));
        // free now 12288-6000 = 6288 effective; reserve another large chunk that breaches hard.
        let ok = t.reserve_vram(&g0, 6000);
        assert!(!ok, "should refuse over-commit past hard limit");
    }

    #[test]
    fn release_returns_capacity() {
        let mut t = table();
        let g0 = DeviceId::Gpu(0);
        assert!(t.reserve_vram(&g0, 4000));
        t.release_vram(&g0, 4000);
        assert_eq!(t.get(&g0).unwrap().reserved_vram_mb, 0);
    }

    #[test]
    fn offline_and_open_breaker_devices_unusable() {
        let mut t = table();
        t.set_health(&DeviceId::Gpu(1), DeviceHealth::Offline);
        t.set_breaker(&DeviceId::CloudPool("openai".into()), BreakerState::Open);
        assert!(t.usable_gpus().iter().all(|d| d.id != DeviceId::Gpu(1)));
        assert!(t.usable_cloud().is_empty());
    }

    #[test]
    fn usable_gpus_ordered_by_free_then_index() {
        let t = table();
        let gpus = t.usable_gpus();
        // gpu0 has 12 GB free, gpu1 8 GB → gpu0 first.
        assert_eq!(gpus[0].id, DeviceId::Gpu(0));
        assert_eq!(gpus[1].id, DeviceId::Gpu(1));
    }

    #[test]
    fn refresh_free_updates_live_value() {
        let mut t = table();
        t.refresh_free(&DeviceId::Gpu(0), 2000, 0);
        assert_eq!(t.get(&DeviceId::Gpu(0)).unwrap().free_vram_mb, 2000);
    }
}
