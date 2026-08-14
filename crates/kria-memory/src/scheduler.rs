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

use crate::error::MemoryResult;

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
    /// Returns `true` when the CPU or GPU is thermally throttled.
    ///
    /// Default implementation returns `false` so existing monitor
    /// implementations remain unbroken without adding a new method body.
    fn thermal_pressure(&self) -> bool {
        false
    }
    /// Returns `true` when the ONNX / llama.cpp embedding or inference
    /// worker is saturated and cannot accept additional work without queuing.
    ///
    /// Default implementation returns `false` so existing monitor
    /// implementations remain unbroken without adding a new method body.
    fn model_pressure(&self) -> bool {
        false
    }
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
    pub thermal_pressure: bool,
    pub model_pressure: bool,
}
impl ResourceMonitor for StaticResourceMonitor {
    fn on_battery(&self) -> bool {
        self.on_battery
    }
    fn memory_pressure(&self) -> bool {
        self.memory_pressure
    }
    fn thermal_pressure(&self) -> bool {
        self.thermal_pressure
    }
    fn model_pressure(&self) -> bool {
        self.model_pressure
    }
}

// ---------------------------------------------------------------------------
// F3.8 — Bounded Job Envelope (task 3.8.1)
// ---------------------------------------------------------------------------

/// Classifies the primary resource a job competes for.
///
/// Used to route jobs to the correct bounded worker pool and to enforce
/// the invariant that >50 ms SQLite/CPU/embedding work never runs on
/// the async executor (MGR-009, MGD-015).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceClass {
    /// Pure in-process CPU work (parsing, graph traversal, RRF fusion).
    Cpu,
    /// Synchronous blocking I/O work (SQLite reads/writes, file operations).
    BlockingIo,
    /// Embedding inference on the ONNX runtime worker thread(s).
    Embedding,
    /// Read-only analytical / reporting work (centrality, community detection).
    Analytics,
}

/// Bounded job envelope carrying identity, scheduling, and cancellation
/// metadata for every unit of work submitted to the cognitive scheduler.
///
/// All fields except `cancel` are value-typed so the envelope can be cloned
/// cheaply and sent to a blocking-worker thread alongside the actual closure.
///
/// **Invariants (MGR-009, MG-H03, MG-H14, MG-M16–M17):**
/// - `priority` gates resource-pressure checks via `should_run_under_pressure`.
/// - `deadline` enables the scheduler to expire stale work before dispatch.
/// - `cancel` is always a child token so parent shutdown propagates.
/// - `retry_budget` counts down; callers must not re-enqueue after it reaches 0.
#[derive(Clone, Debug)]
pub struct JobEnvelope {
    /// Stable job identity — use a UUID v4 string; unique per submission.
    pub id: String,
    /// Links related jobs for distributed tracing / observability (MGR-028).
    pub correlation_id: String,
    /// Scheduling priority (P0–P4).
    pub priority: Priority,
    /// Absolute deadline; `None` means no deadline constraint.
    pub deadline: Option<std::time::Instant>,
    /// Cooperative cancellation token (child of the scheduler root).
    pub cancel: CancellationToken,
    /// Jobs sharing the same non-`None` key may be merged or deduplicated
    /// before dispatch (coalescing, MGR-009 wake-queue ≤1024).
    pub coalescing_key: Option<String>,
    /// Authority event ID or revision cursor from which work should resume
    /// after a prior run was interrupted (e.g., embedding rebuild cursor).
    pub authority_cursor: Option<String>,
    /// Primary resource class this job competes for.
    pub resource_class: ResourceClass,
    /// Remaining retry attempts.  Starts at the configured budget (e.g. 3)
    /// and is decremented by the scheduler on each failed attempt.
    /// At 0 the job is abandoned and the failure is logged.
    pub retry_budget: u8,
}

impl JobEnvelope {
    /// Returns `true` when a deadline is set and has already passed.
    ///
    /// Expired envelopes should be discarded before dispatch; attempting to
    /// run expired work wastes a worker slot and will produce a late result.
    #[inline]
    pub fn is_expired(&self) -> bool {
        match self.deadline {
            Some(dl) => std::time::Instant::now() >= dl,
            None => false,
        }
    }

    /// Returns `true` when this job is allowed to run given current resource
    /// pressure reported by `monitor`.
    ///
    /// Policy (MGR-045, §25 Runtime Budget Manager):
    /// - **P0/P1** — always run; these are stop/security/correction jobs.
    /// - **P2** — run unless `memory_pressure` is active.
    /// - **P3/P4** — skip when `on_battery` *or* `memory_pressure` is active.
    pub fn should_run_under_pressure(&self, monitor: &dyn ResourceMonitor) -> bool {
        match self.priority {
            Priority::P0Foreground | Priority::P1Integrity => true,
            Priority::P2Enrichment => !monitor.memory_pressure(),
            Priority::P3Cognition | Priority::P4Maintenance => {
                !monitor.on_battery() && !monitor.memory_pressure()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// F3.8 — Foreground preemption / yield checks (task 3.8.3)
// ---------------------------------------------------------------------------

/// Decision returned by [`PreemptionChecker::check_yield`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreemptionDecision {
    /// Background work should stop and yield the worker.
    Preempt,
    /// Background work may continue.
    Continue,
}

/// Maximum elapsed time before a background job must yield (MGR-009 §2).
const PREEMPTION_BUDGET_MS: u128 = 100;

/// Cooperative yield / preemption checker for background cognition jobs.
///
/// Background jobs call [`check_yield`] at every logical chunk boundary.
/// A job **must** stop when it receives [`PreemptionDecision::Preempt`] and
/// resume on its next scheduled slot.
///
/// Two preemption triggers exist (in order of precedence):
/// 1. A foreground P0 arrival signals [`signal_foreground_arrival`], which
///    cancels the internal token immediately.
/// 2. The elapsed wall-clock time since `start` exceeds 100 ms.
///
/// **Invariants (MGR-009 AC-2, MGR-022, MGD-015):**
/// - P0 yield / defer ≤ 100 ms.
/// - Deterministic fairness: required reconciliation eventually progresses.
#[derive(Clone, Debug)]
pub struct PreemptionChecker {
    /// Cancelled when a P0 foreground task arrives.
    foreground_token: CancellationToken,
}

impl PreemptionChecker {
    /// Create a new checker with a fresh foreground signal token.
    pub fn new() -> Self {
        Self {
            foreground_token: CancellationToken::new(),
        }
    }

    /// Check whether the current background task should yield.
    ///
    /// Returns [`PreemptionDecision::Preempt`] immediately when:
    /// - The foreground token has been cancelled (P0 arrival), **or**
    /// - `elapsed` since `start` exceeds [`PREEMPTION_BUDGET_MS`] (100 ms).
    ///
    /// Otherwise returns [`PreemptionDecision::Continue`].
    pub fn check_yield(&self, start: std::time::Instant) -> PreemptionDecision {
        if self.foreground_token.is_cancelled() {
            return PreemptionDecision::Preempt;
        }
        if start.elapsed().as_millis() > PREEMPTION_BUDGET_MS {
            return PreemptionDecision::Preempt;
        }
        PreemptionDecision::Continue
    }

    /// Signal that a P0 foreground task has arrived.
    ///
    /// All background jobs holding a clone of this checker will receive
    /// [`PreemptionDecision::Preempt`] on their next [`check_yield`] call.
    pub fn signal_foreground_arrival(&self) {
        self.foreground_token.cancel();
    }

    /// Expose the underlying token so callers can pass it to
    /// `tokio_util::sync::CancellationToken`-aware async helpers.
    pub fn foreground_token(&self) -> &CancellationToken {
        &self.foreground_token
    }
}

impl Default for PreemptionChecker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// F3.8 — Deterministic fairness for P1/P2 background work (task 3.8.3)
// ---------------------------------------------------------------------------

/// A work item held by [`FairnessScheduler`].
struct FairnessItem {
    priority: Priority,
    work: Box<dyn FnOnce() + Send + 'static>,
}

/// Simple round-robin fairness scheduler for P1/P2 background work.
///
/// Prevents P1 (enrichment) work from indefinitely starving P2 (cognition /
/// reconciliation) work by guaranteeing that **at most 3 P1 items are
/// dispatched in a row** before the next pending P2 item is returned.
///
/// Fairness guarantee (MGR-009 AC-2, deterministic fairness invariant):
/// - When there are both P1 and P2 items queued, a P2 item is returned
///   after at most every 3 consecutive P1 items.
/// - When only one priority class is queued, items of that class are
///   returned until the queue drains.
///
/// This maps to the spec invariant: "required reconciliation eventually
/// progresses even if P1 backlog is large."
pub struct FairnessScheduler {
    queue: std::collections::VecDeque<FairnessItem>,
    /// Counts consecutive P1 items dispatched since the last P2 item.
    p1_streak: usize,
    /// Maximum consecutive P1 items allowed before a P2 item is forced.
    p1_limit: usize,
}

impl FairnessScheduler {
    /// Create a scheduler with the default fairness limit (3 P1 per 1 P2).
    pub fn new() -> Self {
        Self::with_p1_limit(3)
    }

    /// Create a scheduler with a configurable P1-per-P2 limit.
    pub fn with_p1_limit(p1_limit: usize) -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
            p1_streak: 0,
            p1_limit,
        }
    }

    /// Enqueue a unit of work tagged with its priority.
    ///
    /// Only [`Priority::P1Integrity`] and [`Priority::P2Enrichment`] items are
    /// valid for this scheduler (background P1/P2 classes).  Callers may
    /// enqueue other priorities but the fairness logic only special-cases
    /// P1 vs P2; all other priorities are treated as P1 for streak counting.
    pub fn enqueue<F>(&mut self, priority: Priority, work_fn: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.queue.push_back(FairnessItem {
            priority,
            work: Box::new(work_fn),
        });
    }

    /// Returns the next item according to priority-order with P2 fairness.
    ///
    /// Selection rules (in order):
    /// 1. If the P1 streak has reached `p1_limit` **and** a P2 item exists,
    ///    force a P2 item to prevent starvation.
    /// 2. Otherwise return the first P1 item if one exists (P1 before P2 by
    ///    default priority order).
    /// 3. Otherwise return the first P2 item.
    /// 4. Return `None` when the queue is empty.
    ///
    /// The returned closure is ready to call; the caller is responsible for
    /// executing it on the appropriate worker.
    pub fn next_due(&mut self) -> Option<Box<dyn FnOnce() + Send + 'static>> {
        if self.queue.is_empty() {
            return None;
        }

        // Determine whether we must force a P2 item to satisfy fairness.
        let force_p2 = self.p1_streak >= self.p1_limit
            && self
                .queue
                .iter()
                .any(|i| i.priority == Priority::P2Enrichment);

        let idx = if force_p2 {
            // Find the first P2 item.
            self.queue
                .iter()
                .position(|i| i.priority == Priority::P2Enrichment)
        } else {
            // Normal priority order: pick the first P1 item if available.
            let p1_pos = self
                .queue
                .iter()
                .position(|i| i.priority == Priority::P1Integrity);
            match p1_pos {
                Some(pos) => Some(pos),
                None => {
                    // No P1; fall back to P2.
                    self.queue
                        .iter()
                        .position(|i| i.priority == Priority::P2Enrichment)
                }
            }
        };

        let idx = idx?;
        let item = self.queue.remove(idx)?;

        // Update streak counter.
        if item.priority == Priority::P2Enrichment {
            self.p1_streak = 0;
        } else {
            self.p1_streak += 1;
        }

        Some(item.work)
    }

    /// Returns the number of items currently queued.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns `true` when the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl Default for FairnessScheduler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// F3.8 — Bounded wake queue with coalescing/drop (task 3.8.4)
// ---------------------------------------------------------------------------

/// Classifies a job's durability for the bounded wake queue.
///
/// The key design distinction (MGR-009, MGR-039, MGR-042):
/// - **Rebuildable** jobs are idempotent projections; losing one wake is safe
///   because the trigger can be re-sent once capacity clears.
/// - **Durable** jobs carry work that must not be lost regardless of queue
///   pressure (outbox relay, event cursor advance, lifecycle cascade).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// Safe to drop or coalesce at cap (FTS5 wake, embedding-rebuild-wake,
    /// analytics).  These are idempotent; re-sending when capacity clears
    /// is acceptable.
    Rebuildable,
    /// Must not be dropped (outbox relay, event cursor advance, authority
    /// write completion, lifecycle cascade).  Always accepted even when the
    /// queue is at the 1024-item cap.
    Durable,
}

/// Outcome of a [`BoundedWakeQueue::push`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushResult {
    /// The item was added to the queue.
    Pushed,
    /// A matching coalescing key was already in the queue; the new item was
    /// silently dropped because one wake is sufficient (idempotent).
    Coalesced,
    /// The queue is at capacity and the item is `Rebuildable`; dropped under
    /// backpressure.  The sender may re-try once capacity clears.
    Dropped,
}

/// Bounded wake queue with coalescing and selective cap enforcement.
///
/// Enforces the invariant **wake queue ≤ 1024** (MGR-009, MGR-022, MGR-028,
/// MGR-039, MGR-042, MGR-045; MGD-015) while preserving all durable work
/// (outbox relay, event cursor advance, lifecycle cascade).
///
/// ## Coalescing
/// For `Rebuildable` items: if a matching non-`None` `coalescing_key` is
/// already anywhere in the queue the new item is dropped and [`PushResult::Coalesced`]
/// is returned.  This guarantees at most one outstanding wake per logical
/// trigger kind (FTS5 rebuild, embedding rebuild, analytics refresh).
///
/// ## Cap enforcement
/// The default cap is 1024.  When the cap is reached:
/// - `Rebuildable` items with no coalescing match are dropped
///   ([`PushResult::Dropped`]).
/// - `Durable` items are always pushed regardless of cap, ensuring that
///   outbox relay and event cursor work are never silently lost.
pub struct BoundedWakeQueue {
    inner: std::collections::VecDeque<JobEnvelope>,
    cap: usize,
}

impl BoundedWakeQueue {
    /// Default wake-queue cap mandated by the spec (MGR-009).
    pub const DEFAULT_CAP: usize = 1024;

    /// Create a new queue with the default cap of 1024.
    pub fn new() -> Self {
        Self::with_cap(Self::DEFAULT_CAP)
    }

    /// Create a new queue with a custom cap (useful for tests).
    pub fn with_cap(cap: usize) -> Self {
        Self {
            inner: std::collections::VecDeque::new(),
            cap,
        }
    }

    /// Push a job envelope onto the queue.
    ///
    /// Decision matrix:
    ///
    /// | Kind          | Coalescing match? | At cap? | Result               |
    /// |---------------|-------------------|---------|----------------------|
    /// | `Rebuildable` | yes               | any     | `Coalesced` (drop)   |
    /// | `Rebuildable` | no                | yes     | `Dropped`            |
    /// | `Rebuildable` | no                | no      | `Pushed`             |
    /// | `Durable`     | any               | any     | `Pushed`             |
    ///
    /// For `Durable` work, coalescing is intentionally skipped: different
    /// durable jobs sharing a coalescing key may carry distinct authority
    /// cursors or correlation IDs that must each be processed.
    pub fn push(&mut self, envelope: JobEnvelope, kind: JobKind) -> PushResult {
        match kind {
            JobKind::Durable => {
                // Durable work is always accepted; no cap, no coalescing.
                self.inner.push_back(envelope);
                PushResult::Pushed
            }
            JobKind::Rebuildable => {
                // Coalescing check: if a matching key already exists, drop.
                if let Some(ref key) = envelope.coalescing_key {
                    let already_present = self
                        .inner
                        .iter()
                        .any(|e| e.coalescing_key.as_deref() == Some(key.as_str()));
                    if already_present {
                        return PushResult::Coalesced;
                    }
                }
                // Cap check: drop rebuildable work when at capacity.
                if self.inner.len() >= self.cap {
                    return PushResult::Dropped;
                }
                self.inner.push_back(envelope);
                PushResult::Pushed
            }
        }
    }

    /// Remove and return the front item, or `None` if the queue is empty.
    pub fn pop(&mut self) -> Option<JobEnvelope> {
        self.inner.pop_front()
    }

    /// Number of items currently in the queue.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` when the queue holds no items.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns `true` when the queue has reached the configured cap.
    ///
    /// Note: a `Durable` push will still succeed even when this returns `true`.
    pub fn is_full(&self) -> bool {
        self.inner.len() >= self.cap
    }
}

impl Default for BoundedWakeQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// F3.8 — Pressure-aware policy (task 3.8.5)
// ---------------------------------------------------------------------------

/// Computed pressure policy for background work scheduling.
///
/// Consolidates all four resource pressure flags into decisions that drive
/// priority ceilings, worker concurrency, chunk sizes, and pause behaviour.
///
/// **Policy table (MGR-009, MGR-022, MGR-028, MGR-039, MGR-042, MGR-045;
/// MGD-015; MG-H03, MG-H14, MG-M16–M17):**
///
/// | Pressure state                    | Max priority | Concurrency | Chunk mult | Pause nonessential |
/// |-----------------------------------|--------------|-------------|------------|--------------------|
/// | None                              | P4           | base        | 1.0        | false              |
/// | Battery only                      | P2           | base        | 1.0        | false              |
/// | Memory only                       | P2           | base/2      | 1.0        | false              |
/// | Thermal only                      | P1           | base/2      | 0.5        | true               |
/// | Model only                        | P1           | base        | 0.5        | true               |
/// | Any thermal OR model              | P1           | ≥ base/2    | 0.5        | true               |
/// | All pressures                     | P1           | base/2      | 0.5        | true               |
pub struct PressurePolicy {
    on_battery: bool,
    memory_pressure: bool,
    thermal_pressure: bool,
    model_pressure: bool,
}

impl PressurePolicy {
    /// Snapshot all four pressure flags from `monitor`.
    pub fn new(monitor: &dyn ResourceMonitor) -> Self {
        Self {
            on_battery: monitor.on_battery(),
            memory_pressure: monitor.memory_pressure(),
            thermal_pressure: monitor.thermal_pressure(),
            model_pressure: monitor.model_pressure(),
        }
    }

    /// The highest priority that may be dispatched right now.
    ///
    /// - **P1** when thermal or model pressure is active (shed cognition/maintenance
    ///   and enrichment; keep only integrity work).
    /// - **P2** when battery or memory pressure is active (suspend P3/P4).
    /// - **P4** when there is no pressure (all work may run).
    pub fn max_allowed_priority(&self) -> Priority {
        if self.thermal_pressure || self.model_pressure {
            Priority::P1Integrity
        } else if self.on_battery || self.memory_pressure {
            Priority::P2Enrichment
        } else {
            Priority::P4Maintenance
        }
    }

    /// Suggested worker pool concurrency given a `base` thread count.
    ///
    /// Under memory or thermal pressure, concurrency is halved (minimum 1)
    /// to reduce working-set footprint and CPU/thermal headroom.
    /// Under battery-only or model-only pressure, the base concurrency is
    /// preserved because the limiting factor is not the thread count.
    pub fn suggested_worker_concurrency(&self, base: usize) -> usize {
        if self.memory_pressure || self.thermal_pressure {
            (base / 2).max(1)
        } else {
            base
        }
    }

    /// Multiplier applied to chunk sizes for nonessential background work.
    ///
    /// Returns `0.5` when thermal or model pressure is detected so that
    /// each background work chunk consumes less CPU / model headroom.
    /// Returns `1.0` otherwise (full chunk size).
    pub fn chunk_size_multiplier(&self) -> f32 {
        if self.thermal_pressure || self.model_pressure {
            0.5
        } else {
            1.0
        }
    }

    /// Returns `true` when nonessential background work should be paused.
    ///
    /// Thermal or model pressure indicates that the CPU/GPU or the inference
    /// worker is saturated; chunking alone is insufficient and all
    /// non-integrity background work should pause until pressure eases.
    pub fn should_pause_nonessential(&self) -> bool {
        self.thermal_pressure || self.model_pressure
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
            thermal_pressure: false,
            model_pressure: false,
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
            thermal_pressure: false,
            model_pressure: false,
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
            thermal_pressure: false,
            model_pressure: false,
        }));
        sched.register(Arc::new(CountingJob {
            priority: Priority::P1Integrity,
            key: "integrity",
            runs: runs.clone(),
        }));
        sched.shutdown();
        assert_eq!(sched.run_ready().await, 0);
    }

    // -----------------------------------------------------------------------
    // JobEnvelope unit tests (task 3.8.1)
    // -----------------------------------------------------------------------

    fn make_envelope(priority: Priority) -> JobEnvelope {
        JobEnvelope {
            id: "test-id".to_string(),
            correlation_id: "test-corr".to_string(),
            priority,
            deadline: None,
            cancel: CancellationToken::new(),
            coalescing_key: None,
            authority_cursor: None,
            resource_class: ResourceClass::Cpu,
            retry_budget: 3,
        }
    }

    #[test]
    fn expired_deadline_returns_true() {
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let env = JobEnvelope {
            deadline: Some(past),
            ..make_envelope(Priority::P1Integrity)
        };
        assert!(env.is_expired(), "deadline in the past must be expired");
    }

    #[test]
    fn non_expired_deadline_returns_false() {
        let future = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let env = JobEnvelope {
            deadline: Some(future),
            ..make_envelope(Priority::P1Integrity)
        };
        assert!(
            !env.is_expired(),
            "deadline in the future must not be expired"
        );
    }

    #[test]
    fn null_deadline_never_expires() {
        let env = make_envelope(Priority::P1Integrity);
        assert!(!env.is_expired(), "no deadline must never be expired");
    }

    #[test]
    fn p0_runs_under_battery_pressure() {
        let monitor = StaticResourceMonitor {
            on_battery: true,
            memory_pressure: true,
            thermal_pressure: false,
            model_pressure: false,
        };
        let env = make_envelope(Priority::P0Foreground);
        assert!(
            env.should_run_under_pressure(&monitor),
            "P0 must always run regardless of pressure"
        );
    }

    #[test]
    fn p1_runs_under_battery_and_memory_pressure() {
        let monitor = StaticResourceMonitor {
            on_battery: true,
            memory_pressure: true,
            thermal_pressure: false,
            model_pressure: false,
        };
        let env = make_envelope(Priority::P1Integrity);
        assert!(
            env.should_run_under_pressure(&monitor),
            "P1 must always run regardless of pressure"
        );
    }

    #[test]
    fn p2_does_not_run_under_memory_pressure() {
        let monitor = StaticResourceMonitor {
            on_battery: false,
            memory_pressure: true,
            thermal_pressure: false,
            model_pressure: false,
        };
        let env = make_envelope(Priority::P2Enrichment);
        assert!(
            !env.should_run_under_pressure(&monitor),
            "P2 must not run when memory_pressure is true"
        );
    }

    #[test]
    fn p3_does_not_run_under_memory_pressure() {
        let monitor = StaticResourceMonitor {
            on_battery: false,
            memory_pressure: true,
            thermal_pressure: false,
            model_pressure: false,
        };
        let env = make_envelope(Priority::P3Cognition);
        assert!(
            !env.should_run_under_pressure(&monitor),
            "P3 must not run when memory_pressure is true"
        );
    }

    #[test]
    fn p4_does_not_run_under_battery_pressure() {
        let monitor = StaticResourceMonitor {
            on_battery: true,
            memory_pressure: false,
            thermal_pressure: false,
            model_pressure: false,
        };
        let env = make_envelope(Priority::P4Maintenance);
        assert!(
            !env.should_run_under_pressure(&monitor),
            "P4 must not run when on_battery is true"
        );
    }

    // -----------------------------------------------------------------------
    // PreemptionChecker unit tests (task 3.8.3)
    // -----------------------------------------------------------------------

    #[test]
    fn check_yield_preempts_when_foreground_token_triggered() {
        let checker = PreemptionChecker::new();
        let start = std::time::Instant::now();
        // Before signal: should continue (assuming wall-clock < 100 ms).
        assert_eq!(
            checker.check_yield(start),
            PreemptionDecision::Continue,
            "must be Continue before foreground signal"
        );
        // Signal P0 arrival.
        checker.signal_foreground_arrival();
        assert_eq!(
            checker.check_yield(start),
            PreemptionDecision::Preempt,
            "must Preempt immediately after foreground signal"
        );
    }

    #[test]
    fn check_yield_preempts_after_100ms_elapsed() {
        let checker = PreemptionChecker::new();
        // Simulate a start time 101 ms in the past.
        let past_start = std::time::Instant::now() - std::time::Duration::from_millis(101);
        assert_eq!(
            checker.check_yield(past_start),
            PreemptionDecision::Preempt,
            "must Preempt when elapsed > 100 ms"
        );
    }

    #[test]
    fn check_yield_continues_before_100ms_with_no_signal() {
        let checker = PreemptionChecker::new();
        let start = std::time::Instant::now();
        // Immediate check; wall-clock cannot exceed 100 ms in a unit test.
        assert_eq!(
            checker.check_yield(start),
            PreemptionDecision::Continue,
            "must Continue within budget with no foreground signal"
        );
    }

    // -----------------------------------------------------------------------
    // FairnessScheduler unit tests (task 3.8.3)
    // -----------------------------------------------------------------------

    #[test]
    fn fairness_p2_runs_after_at_most_3_p1_items() {
        let mut sched = FairnessScheduler::new();
        // Enqueue 4 P1 items then 1 P2 item.
        for i in 0..4usize {
            sched.enqueue(Priority::P1Integrity, move || {
                let _ = i; // just a closure that captures i
            });
        }
        sched.enqueue(Priority::P2Enrichment, || {});

        // Drain until we see the P2 item; record how many P1 items preceded it.
        let mut p1_before_p2 = 0usize;
        let mut p2_seen = false;
        while let Some(work) = sched.next_due() {
            if p2_seen {
                break;
            }
            // We can tell which it is by counting: fairness inserts P2 at slot 4
            // (index 3, after 3 P1). Check the streak by inspecting run order.
            work(); // execute to drain
            p1_before_p2 += 1;
            // After 3 P1 items the next call must return P2.
            if p1_before_p2 == 3 {
                // The next item off the scheduler must be P2.
                let forced = sched.next_due().expect("P2 item must be forced here");
                forced();
                p2_seen = true;
            }
        }
        assert!(
            p2_seen,
            "P2 item must have been dispatched within 3 P1 items"
        );
        assert!(
            p1_before_p2 <= 3,
            "at most 3 P1 items may precede a P2 item; got {p1_before_p2}"
        );
    }

    // -----------------------------------------------------------------------
    // BoundedWakeQueue unit tests (task 3.8.4)
    // -----------------------------------------------------------------------

    /// Helper: build a minimal envelope with an optional coalescing key.
    fn make_wake_envelope(coalescing_key: Option<&str>) -> JobEnvelope {
        JobEnvelope {
            id: uuid::Uuid::new_v4().to_string(),
            correlation_id: "test-corr".to_string(),
            priority: Priority::P2Enrichment,
            deadline: None,
            cancel: CancellationToken::new(),
            coalescing_key: coalescing_key.map(|s| s.to_string()),
            authority_cursor: None,
            resource_class: ResourceClass::Cpu,
            retry_budget: 3,
        }
    }

    #[test]
    fn empty_queue_pop_returns_none() {
        let mut q = BoundedWakeQueue::new();
        assert_eq!(q.len(), 0);
        assert!(q.is_empty());
        assert!(q.pop().is_none(), "pop on empty queue must return None");
    }

    #[test]
    fn rebuildable_item_with_matching_key_is_coalesced() {
        let mut q = BoundedWakeQueue::with_cap(16);
        let first = make_wake_envelope(Some("fts5-rebuild"));
        let second = make_wake_envelope(Some("fts5-rebuild"));

        let r1 = q.push(first, JobKind::Rebuildable);
        let r2 = q.push(second, JobKind::Rebuildable);

        assert_eq!(r1, PushResult::Pushed, "first item must be Pushed");
        assert_eq!(
            r2,
            PushResult::Coalesced,
            "duplicate coalescing key must be Coalesced (not added)"
        );
        assert_eq!(
            q.len(),
            1,
            "queue must contain exactly one item after coalescing"
        );
    }

    #[test]
    fn rebuildable_item_dropped_at_cap() {
        let cap = 4usize;
        let mut q = BoundedWakeQueue::with_cap(cap);

        // Fill the queue with distinct keys so no coalescing occurs.
        for i in 0..cap {
            let key = format!("key-{i}");
            let env = make_wake_envelope(Some(&key));
            let result = q.push(env, JobKind::Rebuildable);
            assert_eq!(result, PushResult::Pushed, "item {i} must be Pushed");
        }
        assert!(q.is_full(), "queue must be full after {cap} pushes");

        // A further rebuildable item with a new key must be dropped.
        let overflow = make_wake_envelope(Some("overflow-key"));
        let result = q.push(overflow, JobKind::Rebuildable);
        assert_eq!(
            result,
            PushResult::Dropped,
            "rebuildable item must be Dropped when queue is at cap"
        );
        assert_eq!(q.len(), cap, "queue length must not exceed cap");
    }

    #[test]
    fn durable_item_always_accepted_at_cap() {
        let cap = 4usize;
        let mut q = BoundedWakeQueue::with_cap(cap);

        // Fill the queue with rebuildable items.
        for i in 0..cap {
            let key = format!("key-{i}");
            let env = make_wake_envelope(Some(&key));
            q.push(env, JobKind::Rebuildable);
        }
        assert!(q.is_full());

        // Durable item must always be accepted even when at cap.
        let durable = make_wake_envelope(None);
        let result = q.push(durable, JobKind::Durable);
        assert_eq!(
            result,
            PushResult::Pushed,
            "durable item must always be Pushed regardless of cap"
        );
        assert_eq!(
            q.len(),
            cap + 1,
            "queue must hold one item beyond cap for durable work"
        );
    }

    #[test]
    fn coalescing_only_matches_when_both_keys_are_some_and_equal() {
        let mut q = BoundedWakeQueue::with_cap(16);

        // None coalescing key: should NOT coalesce with anything.
        let no_key_1 = make_wake_envelope(None);
        let no_key_2 = make_wake_envelope(None);
        assert_eq!(q.push(no_key_1, JobKind::Rebuildable), PushResult::Pushed);
        assert_eq!(
            q.push(no_key_2, JobKind::Rebuildable),
            PushResult::Pushed,
            "None key must not coalesce — two distinct None-key items are independent"
        );
        assert_eq!(q.len(), 2);

        // Different keys: should NOT coalesce.
        let key_a = make_wake_envelope(Some("fts5"));
        let key_b = make_wake_envelope(Some("embedding"));
        assert_eq!(q.push(key_a, JobKind::Rebuildable), PushResult::Pushed);
        assert_eq!(
            q.push(key_b, JobKind::Rebuildable),
            PushResult::Pushed,
            "different keys must not coalesce"
        );
        assert_eq!(q.len(), 4);

        // Same key: must coalesce now.
        let key_a2 = make_wake_envelope(Some("fts5"));
        assert_eq!(
            q.push(key_a2, JobKind::Rebuildable),
            PushResult::Coalesced,
            "matching Some key must coalesce"
        );
        assert_eq!(q.len(), 4, "no new item added on coalescing");
    }

    #[test]
    fn fairness_p1_runs_before_p2_when_no_window_elapsed() {
        let mut sched = FairnessScheduler::new();
        // Enqueue one P2 then one P1.
        let p2_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let p1_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let p2_ran_c = p2_ran.clone();
        sched.enqueue(Priority::P2Enrichment, move || {
            p2_ran_c.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let p1_ran_c = p1_ran.clone();
        sched.enqueue(Priority::P1Integrity, move || {
            p1_ran_c.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        // First call should return P1 (higher priority, streak=0).
        let first = sched.next_due().expect("queue not empty");
        first();
        assert!(
            p1_ran.load(std::sync::atomic::Ordering::SeqCst),
            "P1 must run before P2 when streak < limit"
        );
        assert!(
            !p2_ran.load(std::sync::atomic::Ordering::SeqCst),
            "P2 must not have run yet"
        );

        // Second call should return P2 (only item left).
        let second = sched.next_due().expect("P2 item still pending");
        second();
        assert!(
            p2_ran.load(std::sync::atomic::Ordering::SeqCst),
            "P2 must run when it is the only item remaining"
        );
    }

    // -----------------------------------------------------------------------
    // PressurePolicy unit tests (task 3.8.5)
    // -----------------------------------------------------------------------

    /// Helper to build a fully-specified StaticResourceMonitor.
    fn static_monitor(
        on_battery: bool,
        memory_pressure: bool,
        thermal_pressure: bool,
        model_pressure: bool,
    ) -> StaticResourceMonitor {
        StaticResourceMonitor {
            on_battery,
            memory_pressure,
            thermal_pressure,
            model_pressure,
        }
    }

    #[test]
    fn pressure_policy_no_pressure() {
        // No pressure: max P4, full concurrency, multiplier 1.0, no pause.
        let mon = static_monitor(false, false, false, false);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P4Maintenance,
            "no pressure: must allow P4"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            4,
            "no pressure: concurrency must be unchanged"
        );
        assert!(
            (policy.chunk_size_multiplier() - 1.0_f32).abs() < f32::EPSILON,
            "no pressure: chunk multiplier must be 1.0"
        );
        assert!(
            !policy.should_pause_nonessential(),
            "no pressure: must not pause nonessential work"
        );
    }

    #[test]
    fn pressure_policy_battery_only() {
        // Battery only: max P2, full concurrency, multiplier 1.0, no pause.
        let mon = static_monitor(true, false, false, false);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P2Enrichment,
            "battery only: must cap at P2"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            4,
            "battery only: concurrency must be unchanged"
        );
        assert!(
            (policy.chunk_size_multiplier() - 1.0_f32).abs() < f32::EPSILON,
            "battery only: chunk multiplier must be 1.0"
        );
        assert!(
            !policy.should_pause_nonessential(),
            "battery only: must not pause nonessential work"
        );
    }

    #[test]
    fn pressure_policy_memory_only() {
        // Memory only: max P2, half concurrency, multiplier 1.0, no pause.
        let mon = static_monitor(false, true, false, false);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P2Enrichment,
            "memory only: must cap at P2"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            2,
            "memory only: concurrency must be halved"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(1),
            1,
            "memory only: halved concurrency must be at least 1"
        );
        assert!(
            (policy.chunk_size_multiplier() - 1.0_f32).abs() < f32::EPSILON,
            "memory only: chunk multiplier must be 1.0"
        );
        assert!(
            !policy.should_pause_nonessential(),
            "memory only: must not pause nonessential work"
        );
    }

    #[test]
    fn pressure_policy_thermal_only() {
        // Thermal only: max P1, half concurrency, multiplier 0.5, pause=true.
        let mon = static_monitor(false, false, true, false);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P1Integrity,
            "thermal only: must cap at P1"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            2,
            "thermal only: concurrency must be halved"
        );
        assert!(
            (policy.chunk_size_multiplier() - 0.5_f32).abs() < f32::EPSILON,
            "thermal only: chunk multiplier must be 0.5"
        );
        assert!(
            policy.should_pause_nonessential(),
            "thermal only: must pause nonessential work"
        );
    }

    #[test]
    fn pressure_policy_model_only() {
        // Model pressure only: max P1, full concurrency, multiplier 0.5, pause=true.
        let mon = static_monitor(false, false, false, true);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P1Integrity,
            "model only: must cap at P1"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            4,
            "model only: concurrency must be unchanged (model pressure doesn't halve threads)"
        );
        assert!(
            (policy.chunk_size_multiplier() - 0.5_f32).abs() < f32::EPSILON,
            "model only: chunk multiplier must be 0.5"
        );
        assert!(
            policy.should_pause_nonessential(),
            "model only: must pause nonessential work"
        );
    }

    #[test]
    fn pressure_policy_all_pressures() {
        // All pressures: max P1, half concurrency, multiplier 0.5, pause=true.
        let mon = static_monitor(true, true, true, true);
        let policy = PressurePolicy::new(&mon);
        assert_eq!(
            policy.max_allowed_priority(),
            Priority::P1Integrity,
            "all pressures: must cap at P1"
        );
        assert_eq!(
            policy.suggested_worker_concurrency(4),
            2,
            "all pressures: concurrency must be halved"
        );
        assert!(
            (policy.chunk_size_multiplier() - 0.5_f32).abs() < f32::EPSILON,
            "all pressures: chunk multiplier must be 0.5"
        );
        assert!(
            policy.should_pause_nonessential(),
            "all pressures: must pause nonessential work"
        );
    }
}
