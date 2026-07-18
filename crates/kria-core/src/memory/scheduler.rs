//! Cognitive Scheduler — sole owner of background work (memory-upgrade design §25, ADR-008).
//!
//! Priority-classed (P0–P4), resource-aware (battery/memory), single-flight,
//! cancellable background runtime. On battery or under memory pressure, P3
//! (cognition) and P4 (maintenance) are suspended so background cognition never
//! starves the user or drains the laptop (§25 Runtime Budget Manager). Named
//! `CognitiveScheduler` to stay distinct from `automation::scheduler` (§45.5).

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashSet;
use tokio_util::sync::CancellationToken;

use crate::memory::error::MemoryResult;

/// Background job priority. Lower ordinal = higher priority (P0 preempts all).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Foreground user work (never scheduled here; reserved for ordering).
    P0Foreground = 0,
    /// Integrity: reconciliation, backup, orphan repair. Must run.
    P1Integrity = 1,
    /// Enrichment: outbox relay, embedding, entity resolution.
    P2Enrichment = 2,
    /// Cognition: consolidation, dreaming, reflection, salience.
    P3Cognition = 3,
    /// Maintenance: decay, compaction, re-embed.
    P4Maintenance = 4,
}

/// Static description of a background job.
#[derive(Clone, Debug)]
pub struct JobProfile {
    pub name: &'static str,
    pub priority: Priority,
    /// Jobs sharing a key never run concurrently (single-flight, N3).
    pub single_flight_key: &'static str,
}

/// A registered background job.
#[async_trait]
pub trait BackgroundJob: Send + Sync {
    fn profile(&self) -> JobProfile;
    /// Execute one run. Should check `cancel` at chunk boundaries and return
    /// promptly when cancelled (checkpointed/resumable, N14).
    async fn run(&self, cancel: CancellationToken) -> MemoryResult<()>;
}

/// Observes machine resources to gate background work (§25 / 32.3).
pub trait ResourceMonitor: Send + Sync {
    fn on_battery(&self) -> bool;
    fn memory_pressure(&self) -> bool;
}

/// Default monitor: memory pressure via `sysinfo` (real); battery assumed AC
/// (platform battery probing is wired via config on desktop — documented).
pub struct DefaultResourceMonitor {
    sys: std::sync::Mutex<sysinfo::System>,
    min_available_mb: u64,
}

impl DefaultResourceMonitor {
    pub fn new(min_available_mb: u64) -> Self {
        Self {
            sys: std::sync::Mutex::new(sysinfo::System::new()),
            min_available_mb,
        }
    }
}

impl ResourceMonitor for DefaultResourceMonitor {
    fn on_battery(&self) -> bool {
        false // AC assumed on the dev laptop; config flag drives real policy.
    }
    fn memory_pressure(&self) -> bool {
        let mut sys = self.sys.lock().unwrap_or_else(|p| p.into_inner());
        sys.refresh_memory();
        sys.available_memory() / (1024 * 1024) < self.min_available_mb
    }
}

/// A fixed monitor for deterministic tests.
pub struct StaticResourceMonitor {
    pub on_battery: bool,
    pub memory_pressure: bool,
}
impl ResourceMonitor for StaticResourceMonitor {
    fn on_battery(&self) -> bool {
        self.on_battery
    }
    fn memory_pressure(&self) -> bool {
        self.memory_pressure
    }
}

/// The scheduler.
pub struct CognitiveScheduler {
    jobs: Vec<Arc<dyn BackgroundJob>>,
    monitor: Arc<dyn ResourceMonitor>,
    inflight: Arc<DashSet<&'static str>>,
    cancel: CancellationToken,
}

impl CognitiveScheduler {
    pub fn new(monitor: Arc<dyn ResourceMonitor>) -> Self {
        Self {
            jobs: Vec::new(),
            monitor,
            inflight: Arc::new(DashSet::new()),
            cancel: CancellationToken::new(),
        }
    }

    pub fn register(&mut self, job: Arc<dyn BackgroundJob>) {
        self.jobs.push(job);
    }

    /// The highest priority permitted to run right now given resources.
    /// On battery or under memory pressure, P3/P4 are suspended (§25).
    pub fn max_allowed_priority(&self) -> Priority {
        if self.monitor.on_battery() || self.monitor.memory_pressure() {
            Priority::P2Enrichment
        } else {
            Priority::P4Maintenance
        }
    }

    /// Run every registered job whose priority is currently permitted, honoring
    /// single-flight. Jobs run sequentially by priority (P1→P4); a real timer
    /// loop calls this on triggers. Returns the number of jobs run.
    pub async fn run_ready(&self) -> usize {
        let ceiling = self.max_allowed_priority();
        let mut ordered: Vec<&Arc<dyn BackgroundJob>> = self
            .jobs
            .iter()
            .filter(|j| j.profile().priority <= ceiling)
            .collect();
        ordered.sort_by_key(|j| j.profile().priority);

        let mut ran = 0usize;
        for job in ordered {
            if self.cancel.is_cancelled() {
                break;
            }
            let key = job.profile().single_flight_key;
            if !self.inflight.insert(key) {
                continue; // already running under this key
            }
            let result = job.run(self.cancel.child_token()).await;
            self.inflight.remove(key);
            match result {
                Ok(()) => ran += 1,
                Err(e) => {
                    tracing::warn!(job = job.profile().name, error = %e, "background job failed")
                }
            }
        }
        ran
    }

    /// Signal all jobs to stop at their next checkpoint (graceful shutdown).
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingJob {
        priority: Priority,
        key: &'static str,
        runs: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl BackgroundJob for CountingJob {
        fn profile(&self) -> JobProfile {
            JobProfile {
                name: "counting",
                priority: self.priority,
                single_flight_key: self.key,
            }
        }
        async fn run(&self, _cancel: CancellationToken) -> MemoryResult<()> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn runs_permitted_priorities() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut sched = CognitiveScheduler::new(Arc::new(StaticResourceMonitor {
            on_battery: false,
            memory_pressure: false,
        }));
        sched.register(Arc::new(CountingJob {
            priority: Priority::P4Maintenance,
            key: "maint",
            runs: runs.clone(),
        }));
        assert_eq!(sched.run_ready().await, 1);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn battery_suspends_p3_p4() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut sched = CognitiveScheduler::new(Arc::new(StaticResourceMonitor {
            on_battery: true,
            memory_pressure: false,
        }));
        sched.register(Arc::new(CountingJob {
            priority: Priority::P4Maintenance,
            key: "maint",
            runs: runs.clone(),
        }));
        sched.register(Arc::new(CountingJob {
            priority: Priority::P1Integrity,
            key: "integrity",
            runs: runs.clone(),
        }));
        // Only the P1 job runs on battery.
        assert_eq!(sched.run_ready().await, 1);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(sched.max_allowed_priority(), Priority::P2Enrichment);
    }

    #[tokio::test]
    async fn shutdown_stops_scheduling() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut sched = CognitiveScheduler::new(Arc::new(StaticResourceMonitor {
            on_battery: false,
            memory_pressure: false,
        }));
        sched.register(Arc::new(CountingJob {
            priority: Priority::P1Integrity,
            key: "integrity",
            runs: runs.clone(),
        }));
        sched.shutdown();
        assert_eq!(sched.run_ready().await, 0);
    }
}
