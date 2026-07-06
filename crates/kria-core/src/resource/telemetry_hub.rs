//! Single host-wide telemetry sampler (HRA Phase A1 — telemetry unification).
//!
//! Before this hub the runtime had 4–5 independent VRAM samplers, each calling
//! `platform::vram::build_profiler()` (each a separate NVML/ROCm context) and each reading the GPU
//! on its own cadence: the orchestrator `TelemetryActor`, the shared-lease recovery telemetry, the
//! HRA snapshot loop, the image VRAM barrier, and an ad-hoc read in the agent loop. That meant the
//! authority could decide on a different VRAM reading than the watchdog acted on (a correctness
//! hazard) and wasted device-query cost.
//!
//! The hub fixes that by owning the ONE `VramProfiler` for the process. Every consumer borrows that
//! profiler (or reads the last published `HostSnapshot`) through the hub instead of building its
//! own. A single background loop publishes snapshots on a `watch` channel; on-demand callers that
//! need a guaranteed-fresh reading (admission decisions, lease recovery verification) call
//! [`TelemetryHub::sample_now`], which also republishes so every reader stays coherent.
//!
//! Design notes:
//! - Pure-ish: the only side effect is reading the device + sysinfo. Deterministic snapshot shape.
//! - Fail-open: if no GPU profiler is present, snapshots simply carry an empty GPU list.
//! - Global accessor mirrors `global_gpu_lease` / `global_hra` so legacy components can reach the
//!   single sampler without a constructor dependency.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tokio::sync::watch;

use crate::platform::vram::{build_profiler, VramProfiler};
use crate::resource::authority::collector::{CpuLive, DeviceLive, HostSnapshot, RamLive};

/// The single process-wide telemetry sampler.
pub struct TelemetryHub {
    profiler: Arc<dyn VramProfiler>,
    ram_total_mb: u64,
    seq: AtomicU64,
    started: Instant,
    tx: watch::Sender<HostSnapshot>,
    rx: watch::Receiver<HostSnapshot>,
}

impl TelemetryHub {
    /// Build the hub with the single shared profiler. `ram_total_mb` is the detected host RAM (used
    /// when sysinfo is unavailable in a sampling context).
    pub fn new(ram_total_mb: u64) -> Arc<Self> {
        Self::with_profiler(build_profiler(), ram_total_mb)
    }

    /// Build with an injected profiler (tests / explicit backend selection).
    pub fn with_profiler(profiler: Arc<dyn VramProfiler>, ram_total_mb: u64) -> Arc<Self> {
        let initial = HostSnapshot {
            seq: 0,
            gpus: Vec::new(),
            cpu: CpuLive {
                per_core_pct: Vec::new(),
            },
            ram: RamLive {
                total_mb: ram_total_mb,
                free_mb: ram_total_mb,
            },
            sampled_at_ms: 0,
        };
        let (tx, rx) = watch::channel(initial);
        Arc::new(Self {
            profiler,
            ram_total_mb,
            seq: AtomicU64::new(0),
            started: Instant::now(),
            tx,
            rx,
        })
    }

    /// The single shared profiler. Consumers that still need a raw profiler (e.g. the image VRAM
    /// barrier) borrow this instead of calling `build_profiler()` again.
    pub fn profiler(&self) -> Arc<dyn VramProfiler> {
        self.profiler.clone()
    }

    /// Last published snapshot (cheap clone; never blocks).
    pub fn latest(&self) -> HostSnapshot {
        self.rx.borrow().clone()
    }

    /// Subscribe for change notifications (dashboards, forecasting).
    pub fn subscribe(&self) -> watch::Receiver<HostSnapshot> {
        self.rx.clone()
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Take a fresh reading from the single profiler + sysinfo, publish it, and return it. This is
    /// the only place that touches the device for a snapshot, so all readers stay coherent.
    pub async fn sample_now(&self) -> HostSnapshot {
        let v = self.profiler.snapshot().await;
        let ram_total_fallback = self.ram_total_mb;
        let (ram_total_mb, ram_free_mb, per_core_pct) = tokio::task::spawn_blocking(move || {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            let total = sys.total_memory() / (1024 * 1024);
            let free = sys.available_memory() / (1024 * 1024);
            // CPU usage needs two samples separated by a short interval.
            sys.refresh_cpu_usage();
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            sys.refresh_cpu_usage();
            let cores: Vec<u8> = sys
                .cpus()
                .iter()
                .map(|c| c.cpu_usage().round().clamp(0.0, 100.0) as u8)
                .collect();
            let (total, free) = if total == 0 {
                (ram_total_fallback, ram_total_fallback)
            } else {
                (total, free)
            };
            (total, free, cores)
        })
        .await
        .unwrap_or((ram_total_fallback, ram_total_fallback, Vec::new()));

        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let gpus = if v.total_mb > 0 {
            vec![DeviceLive {
                index: 0,
                free_vram_mb: v.free_mb,
                total_vram_mb: v.total_mb,
                util_pct: 0,
                temp_c: None,
                processes: Vec::new(),
            }]
        } else {
            Vec::new()
        };
        let snap = HostSnapshot {
            seq,
            gpus,
            cpu: CpuLive { per_core_pct },
            ram: RamLive {
                total_mb: ram_total_mb,
                free_mb: ram_free_mb,
            },
            sampled_at_ms: self.now_ms(),
        };
        // Publish (ignore error: only fails if all receivers dropped, which cannot happen — the hub
        // holds one).
        let _ = self.tx.send(snap.clone());
        snap
    }

    /// Run the periodic background sampler until the process exits. Replaces every other periodic
    /// VRAM-poll loop. `interval` is the publish cadence for dashboards/shadow telemetry.
    pub async fn run(self: Arc<Self>, interval: std::time::Duration) {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let _ = self.sample_now().await;
        }
    }
}

/// Process-wide telemetry hub (set once at startup).
static GLOBAL_HUB: OnceLock<Arc<TelemetryHub>> = OnceLock::new();

/// Register the single process-wide telemetry hub (first set wins).
pub fn set_global_telemetry_hub(hub: Arc<TelemetryHub>) {
    let _ = GLOBAL_HUB.set(hub);
}

/// Get the process-wide telemetry hub, if registered.
pub fn global_telemetry_hub() -> Option<Arc<TelemetryHub>> {
    GLOBAL_HUB.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::vram::NullProfiler;

    #[tokio::test]
    async fn sample_now_publishes_and_increments_seq() {
        let hub = TelemetryHub::with_profiler(Arc::new(NullProfiler), 16384);
        let s1 = hub.sample_now().await;
        let s2 = hub.sample_now().await;
        assert_eq!(s1.seq + 1, s2.seq);
        // latest reflects the most recent publish
        assert_eq!(hub.latest().seq, s2.seq);
    }

    #[tokio::test]
    async fn null_profiler_yields_empty_gpu_list_but_real_ram() {
        let hub = TelemetryHub::with_profiler(Arc::new(NullProfiler), 16384);
        let s = hub.sample_now().await;
        assert!(s.gpus.is_empty());
        assert!(s.ram.total_mb > 0);
    }

    #[tokio::test]
    async fn subscribe_sees_updates() {
        let hub = TelemetryHub::with_profiler(Arc::new(NullProfiler), 8192);
        let mut rx = hub.subscribe();
        let _ = hub.sample_now().await;
        assert!(rx.changed().await.is_ok());
        assert_eq!(rx.borrow().seq, hub.latest().seq);
    }
}
