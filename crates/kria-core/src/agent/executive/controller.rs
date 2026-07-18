//! ExecutiveController — Thin Brain dispatcher.
//!
//! Architecture:
//! ```text
//!   Voice Pipeline ──┐
//!   Text Chat    ────┤
//!   HITL Gateway ────┤   ┌──────────────────────┐   ┌─────────────────┐
//!   Curiosity    ────┼──→│  Ingress MPSC channel │──→│ Fast Dispatcher │
//!   Maintenance  ────┤   │  (unbounded, lockfree)│   │   (<2ms loop)   │
//!   Skill Comp   ────┘   └──────────────────────┘   └────────┬────────┘
//!                                                             │
//!                              ┌───────────────────────────────┤
//!                              ↓                               ↓
//!                     ┌─────────────────┐           ┌──────────────────┐
//!                     │ Foreground Slot │           │  Background Pool │
//!                     │ (1 task max)    │           │  (JoinSet, max N)│
//!                     │ Voice/Interactive│          │  Curiosity/Skill │
//!                     └─────────────────┘           └──────────────────┘
//! ```
//!
//! Key invariant: The main dispatch loop NEVER blocks on I/O.
//! All heavy work (command execution, audit logging, HITL waits)
//! happens inside spawned worker tasks.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::preemption::PreemptionManager;
use super::types::*;
use crate::resource::gpu_lease::{GpuLeaseManager, GpuOwner};
use crate::safety::policy_gate::PolicyGate;

/// Configuration for the ExecutiveController.
#[derive(Debug, Clone)]
pub struct ExecutiveConfig {
    /// Maximum concurrent background tasks.
    pub max_background_tasks: usize,
    /// Grace period before force-killing a preempted task.
    pub preemption_grace_ms: u64,
    /// Enable VRAM maintenance GC loop.
    pub vram_maintenance_enabled: bool,
    /// Idle duration before triggering VRAM maintenance.
    pub vram_idle_threshold_secs: u64,
}

impl Default for ExecutiveConfig {
    fn default() -> Self {
        Self {
            max_background_tasks: 3,
            preemption_grace_ms: 500,
            vram_maintenance_enabled: true,
            vram_idle_threshold_secs: 1800, // 30 minutes
        }
    }
}

/// The Executive Controller — KRIA's central brain.
///
/// Owns the main dispatch loop. Receives `TaskRequest`s via MPSC,
/// makes scheduling decisions, and dispatches to worker pools.
/// All I/O happens in spawned workers, never in the dispatch loop.
/// Internal command sent to the controller's dispatch loop.
enum ControllerCommand {
    /// Cancel a task by ID (foreground, background, or queued).
    CancelTask {
        task_id: uuid::Uuid,
    },
    GpuLeaseAcquired {
        task_id: uuid::Uuid,
    },
    GpuLeaseReleased {
        task_id: uuid::Uuid,
    },
}

pub struct ExecutiveController {
    /// Configuration.
    config: ExecutiveConfig,
    /// Ingress channel: receives tasks from all subsystems.
    rx: mpsc::UnboundedReceiver<TaskRequest>,
    /// Command channel: receives control commands (cancel, etc.).
    cmd_rx: mpsc::UnboundedReceiver<ControllerCommand>,
    cmd_tx: mpsc::UnboundedSender<ControllerCommand>,
    /// Public sender for submitting tasks.
    #[allow(dead_code)]
    tx: mpsc::UnboundedSender<TaskRequest>,
    /// Priority queue for pending tasks (BinaryHeap with Reverse for min-heap behavior).
    queue: BinaryHeap<Reverse<QueuedTask>>,
    /// Currently running foreground task (max 1).
    foreground: Option<TaskHandle>,
    /// Background task pool.
    background: JoinSet<(uuid::Uuid, TaskResult)>,
    /// Metadata + cancellation handles for running background tasks.
    background_tasks: HashMap<uuid::Uuid, ExecutiveTaskSnapshot>,
    background_cancellations: HashMap<uuid::Uuid, CancellationToken>,
    /// GPU lease manager (existing).
    gpu_lease: Arc<GpuLeaseManager>,
    /// Policy gate for command evaluation.
    policy_gate: Arc<dyn PolicyGate>,
    /// Preemption manager.
    preemption: PreemptionManager,
    /// Event broadcast (for UI observability; bounded, every event retained until consumed).
    event_tx: broadcast::Sender<ControllerEvent>,
    /// Latest bounded read model used by desktop/server snapshot commands.
    snapshot_tx: watch::Sender<ExecutiveSnapshot>,
    gpu_lease_holder: Option<uuid::Uuid>,
    total_completed: u64,
    total_failed: u64,
    /// Shutdown signal.
    shutdown: CancellationToken,
    /// Tracks when the last foreground task completed (for idle detection).
    last_foreground_completed: Option<std::time::Instant>,
}

/// A task in the priority queue, ordered by priority then submission time.
#[derive(Debug)]
struct QueuedTask {
    priority: TaskPriority,
    submitted_at: std::time::Instant,
    request: TaskRequest,
}

// BinaryHeap is a max-heap. We use Reverse() to get min-heap behavior
// (lowest priority number = highest priority = dequeued first).
impl PartialEq for QueuedTask {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.submitted_at == other.submitted_at
    }
}

impl Eq for QueuedTask {}

impl PartialOrd for QueuedTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.submitted_at.cmp(&self.submitted_at)) // earlier = higher priority
    }
}

impl ExecutiveController {
    /// Create a new ExecutiveController.
    ///
    /// Returns the controller and a sender handle that subsystems use to submit tasks.
    pub fn new(
        config: ExecutiveConfig,
        gpu_lease: Arc<GpuLeaseManager>,
        policy_gate: Arc<dyn PolicyGate>,
    ) -> (Self, ExecutiveSender) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(256);
        let (snapshot_tx, snapshot_rx) = watch::channel(ExecutiveSnapshot::default());
        let shutdown = CancellationToken::new();

        let sender = ExecutiveSender {
            tx: tx.clone(),
            cmd_tx: cmd_tx.clone(),
            event_tx: event_tx.clone(),
            snapshot_rx,
            cancel: shutdown.clone(),
        };

        let preemption =
            PreemptionManager::new(std::time::Duration::from_millis(config.preemption_grace_ms));

        let controller = Self {
            config,
            rx,
            cmd_rx,
            cmd_tx,
            tx,
            queue: BinaryHeap::new(),
            foreground: None,
            background: JoinSet::new(),
            background_tasks: HashMap::new(),
            background_cancellations: HashMap::new(),
            gpu_lease,
            policy_gate,
            preemption,
            event_tx,
            snapshot_tx,
            gpu_lease_holder: None,
            total_completed: 0,
            total_failed: 0,
            shutdown,
            last_foreground_completed: None,
        };

        (controller, sender)
    }

    /// Subscribe to every controller event (for UI/runtime bridges).
    pub fn subscribe_events(&self) -> broadcast::Receiver<ControllerEvent> {
        self.event_tx.subscribe()
    }

    /// Main dispatch loop. Runs until shutdown.
    ///
    /// This loop is intentionally fast — it NEVER blocks on I/O.
    /// All heavy work is dispatched to worker tasks.
    pub async fn run(&mut self) {
        tracing::info!("ExecutiveController started");

        loop {
            // Destructure self to avoid multiple borrows in tokio::select!
            let Self {
                config: _,
                rx,
                cmd_rx,
                cmd_tx: _,
                tx: _,
                queue: _,
                foreground,
                background,
                background_tasks: _,
                background_cancellations: _,
                gpu_lease: _,
                policy_gate: _,
                preemption: _,
                event_tx: _,
                snapshot_tx: _,
                gpu_lease_holder: _,
                total_completed: _,
                total_failed: _,
                shutdown,
                last_foreground_completed: _,
            } = self;

            let has_foreground = foreground.is_some();

            tokio::select! {
                biased;  // Priority: check completions first, then new tasks

                // 1. Check if foreground task completed
                fg_result = Self::poll_foreground_static(foreground), if has_foreground => {
                    if let Some(result) = fg_result {
                        self.on_foreground_complete(result).await;
                    }
                }

                // 2. Check if any background task completed
                Some(result) = background.join_next() => {
                    self.on_background_complete(result);
                }

                // 3. Receive new task from ingress channel
                Some(task) = rx.recv() => {
                    self.dispatch(task).await;
                }

                // 4. Receive control commands (cancel, etc.)
                Some(cmd) = cmd_rx.recv() => {
                    self.handle_command(cmd);
                }

                // 5. Shutdown signal
                _ = shutdown.cancelled() => {
                    tracing::info!("ExecutiveController shutting down");
                    self.graceful_shutdown().await;
                    break;
                }
            }

            // After any event, drain available capacity and publish one bounded read model.
            self.try_drain_queue().await;
            self.publish_snapshot();
        }
    }

    /// Fast dispatch decision (<2ms). No I/O.
    async fn dispatch(&mut self, task: TaskRequest) {
        let decision = self.schedule(task);
        match decision {
            ScheduleDecision::Execute(task) => {
                self.spawn_task(task).await;
            }
            ScheduleDecision::Enqueue(task) => {
                self.enqueue(task);
            }
            ScheduleDecision::Preempt {
                victim_id,
                replacement,
            } => {
                self.preempt_and_replace(victim_id, replacement).await;
            }
            ScheduleDecision::Reject { task_id, reason } => {
                tracing::warn!(task_id = %task_id, reason = %reason, "Task rejected");
                self.emit(ControllerEvent::TaskRejected { task_id, reason });
            }
        }
    }

    /// Pure scheduling logic. No I/O, no blocking.
    fn schedule(&self, task: TaskRequest) -> ScheduleDecision {
        match task.priority {
            TaskPriority::Voice => {
                // Voice is always P0. Preempt foreground if needed.
                if let Some(ref fg) = self.foreground {
                    if fg.priority == TaskPriority::Voice {
                        // Already processing a voice task — queue briefly
                        ScheduleDecision::Enqueue(task)
                    } else {
                        // Foreground is interactive/HITL — voice preempts
                        ScheduleDecision::Preempt {
                            victim_id: fg.id,
                            replacement: task,
                        }
                    }
                } else {
                    ScheduleDecision::Execute(task)
                }
            }

            TaskPriority::Interactive => {
                if self.foreground.is_some() {
                    ScheduleDecision::Enqueue(task)
                } else {
                    ScheduleDecision::Execute(task)
                }
            }

            TaskPriority::HitlResponse => {
                // HITL responses bypass the queue — they unblock blocked tasks.
                // But we still check if foreground slot is free.
                if self.foreground.is_some() {
                    // Queue with high priority (will be dequeued before Background)
                    ScheduleDecision::Enqueue(task)
                } else {
                    ScheduleDecision::Execute(task)
                }
            }

            TaskPriority::Background => {
                if self.background.len() >= self.config.max_background_tasks {
                    ScheduleDecision::Reject {
                        task_id: task.id,
                        reason: format!(
                            "Background task limit reached ({})",
                            self.config.max_background_tasks
                        ),
                    }
                } else {
                    // Background tasks are spawned directly (no foreground slot needed)
                    ScheduleDecision::Execute(task)
                }
            }

            TaskPriority::Maintenance => {
                // Maintenance tasks only run when system is truly idle
                // (no foreground, no background, queue is empty).
                if self.foreground.is_some()
                    || !self.background.is_empty()
                    || !self.queue.is_empty()
                {
                    ScheduleDecision::Enqueue(task)
                } else {
                    ScheduleDecision::Execute(task)
                }
            }
        }
    }

    /// Spawn a task into the appropriate worker pool.
    async fn spawn_task(&mut self, task: TaskRequest) {
        let task_id = task.id;
        let priority = task.priority;
        let source = task.source.clone();
        let description = task.payload.description();
        let submitted_at = task.submitted_at;
        let submitted_at_utc = Self::instant_timestamp(submitted_at);
        let requires_gpu = task.requires_gpu;
        let cancel = task.cancel.clone();
        let started_at = std::time::Instant::now();
        let started_at_utc = chrono::Utc::now().to_rfc3339();

        self.emit(ControllerEvent::TaskStarted {
            task_id,
            priority,
            source: source.clone(),
            description: description.clone(),
            ts: started_at_utc.clone(),
        });

        let gpu_lease = self.gpu_lease.clone();
        let policy_gate = self.policy_gate.clone();
        let cancel_clone = cancel.clone();
        let cmd_tx = self.cmd_tx.clone();

        if priority.is_foreground() {
            let join = tokio::spawn(async move {
                Self::execute_task(task, gpu_lease, policy_gate, cancel_clone, cmd_tx).await
            });

            self.foreground = Some(TaskHandle {
                id: task_id,
                priority,
                source,
                description,
                submitted_at,
                started_at_utc,
                cancel,
                join,
                started_at,
                requires_gpu,
            });
        } else {
            self.background_tasks.insert(
                task_id,
                ExecutiveTaskSnapshot {
                    id: task_id,
                    priority,
                    source,
                    state: TaskState::Running,
                    description,
                    submitted_at: submitted_at_utc,
                    started_at: Some(started_at_utc),
                    completed_at: None,
                    duration_ms: None,
                    error: None,
                    requires_gpu,
                },
            );
            self.background_cancellations.insert(task_id, cancel);
            self.background.spawn(async move {
                let result =
                    Self::execute_task(task, gpu_lease, policy_gate, cancel_clone, cmd_tx).await;
                (task_id, result)
            });
        }
    }

    /// Enqueue a task in the priority queue.
    fn enqueue(&mut self, task: TaskRequest) {
        self.queue.push(Reverse(QueuedTask {
            priority: task.priority,
            submitted_at: task.submitted_at,
            request: task,
        }));
    }

    /// Drain queued tasks while matching foreground/background capacity exists.
    async fn try_drain_queue(&mut self) {
        if self.foreground.is_some() && self.background.len() >= self.config.max_background_tasks {
            return;
        }

        // Drain as many queued tasks as we can
        while let Some(Reverse(task)) = self.queue.peek() {
            let can_execute = if task.priority.is_foreground() {
                self.foreground.is_none()
            } else if task.priority == TaskPriority::Maintenance {
                self.foreground.is_none() && self.background.is_empty() && self.queue.len() <= 1
            } else {
                self.background.len() < self.config.max_background_tasks
            };

            if can_execute {
                let Reverse(task) = self.queue.pop().unwrap();
                self.spawn_task(task.request).await;
            } else {
                break;
            }
        }
    }

    /// Preempt the current foreground task and spawn the replacement.
    async fn preempt_and_replace(&mut self, victim_id: uuid::Uuid, replacement: TaskRequest) {
        if let Some(ref mut fg) = self.foreground {
            if fg.id == victim_id {
                tracing::info!(
                    victim = %victim_id,
                    replacement = %replacement.id,
                    "Preempting foreground task"
                );

                let victim_priority = fg.priority;
                let replacement_id = replacement.id;
                let replacement_priority = replacement.priority;

                // Cancel the victim
                fg.cancel.cancel();

                // Wait for graceful shutdown (up to grace period)
                self.preemption.wait_for_grace(&mut fg.join).await;

                // Drop the old handle (aborts if still running)
                self.foreground = None;

                self.emit(ControllerEvent::TaskPreempted {
                    victim_id,
                    victim_priority,
                    replacement_id,
                    replacement_priority,
                    ts: chrono::Utc::now().to_rfc3339(),
                });

                // Spawn the replacement
                self.spawn_task(replacement).await;
            }
        }
    }

    /// Handle foreground task completion.
    async fn on_foreground_complete(&mut self, result: TaskResult) {
        if let Some(fg) = self.foreground.take() {
            let duration_ms = fg.elapsed().as_millis() as u64;
            let (success, output_summary, error) = Self::result_details(&result);
            self.record_outcome(&result);
            tracing::info!(
                task_id = %fg.id,
                priority = %fg.priority,
                result = %result,
                "Foreground task completed"
            );

            self.emit(ControllerEvent::TaskCompleted {
                task_id: fg.id,
                success,
                duration_ms,
                output_summary,
                error,
                ts: chrono::Utc::now().to_rfc3339(),
            });
            self.last_foreground_completed = Some(std::time::Instant::now());
        }
    }

    /// Handle background task completion.
    fn on_background_complete(
        &mut self,
        result: Result<(uuid::Uuid, TaskResult), tokio::task::JoinError>,
    ) {
        match result {
            Ok((task_id, task_result)) => {
                let duration_ms = Self::result_duration_ms(&task_result);
                let (success, output_summary, error) = Self::result_details(&task_result);
                self.background_tasks.remove(&task_id);
                self.background_cancellations.remove(&task_id);
                self.record_outcome(&task_result);
                tracing::debug!(
                    task_id = %task_id,
                    result = %task_result,
                    "Background task completed"
                );
                self.emit(ControllerEvent::TaskCompleted {
                    task_id,
                    success,
                    duration_ms,
                    output_summary,
                    error,
                    ts: chrono::Utc::now().to_rfc3339(),
                });
            }
            Err(error) => {
                tracing::error!(%error, "Background task panicked");
                self.total_failed = self.total_failed.saturating_add(1);
            }
        }
    }

    fn result_details(result: &TaskResult) -> (bool, Option<String>, Option<String>) {
        match result {
            TaskResult::Success { output, .. } => (true, output.clone(), None),
            TaskResult::Failed { reason, .. } => (false, None, Some(reason.clone())),
            TaskResult::Cancelled { reason } => (false, None, Some(reason.clone())),
            TaskResult::TimedOut { timeout } => (
                false,
                None,
                Some(format!("Timed out after {}ms", timeout.as_millis())),
            ),
        }
    }

    fn result_duration_ms(result: &TaskResult) -> u64 {
        match result {
            TaskResult::Success { total_duration, .. }
            | TaskResult::Failed { total_duration, .. } => total_duration.as_millis() as u64,
            TaskResult::TimedOut { timeout } => timeout.as_millis() as u64,
            TaskResult::Cancelled { .. } => 0,
        }
    }

    fn record_outcome(&mut self, result: &TaskResult) {
        match result {
            TaskResult::Success { .. } => {
                self.total_completed = self.total_completed.saturating_add(1)
            }
            TaskResult::Failed { .. } | TaskResult::TimedOut { .. } => {
                self.total_failed = self.total_failed.saturating_add(1)
            }
            TaskResult::Cancelled { .. } => {}
        }
    }

    /// Poll the foreground task for completion (static version for borrow splitting).
    async fn poll_foreground_static(foreground: &mut Option<TaskHandle>) -> Option<TaskResult> {
        if let Some(ref mut fg) = foreground {
            match (&mut fg.join).await {
                Ok(result) => Some(result),
                Err(e) => Some(TaskResult::Failed {
                    reason: format!("Foreground task panicked: {}", e),
                    total_duration: fg.elapsed(),
                }),
            }
        } else {
            None
        }
    }

    /// Execute a task. This runs in a spawned worker, NOT in the dispatch loop.
    async fn execute_task(
        task: TaskRequest,
        gpu_lease: Arc<GpuLeaseManager>,
        _policy_gate: Arc<dyn PolicyGate>,
        cancel: CancellationToken,
        cmd_tx: mpsc::UnboundedSender<ControllerCommand>,
    ) -> TaskResult {
        let start = std::time::Instant::now();
        let task_id = task.id;

        // Acquire GPU lease if needed. Routes through HRA admission (single runtime authority);
        // priority is derived from the owner class (L1Worker → InteractiveFg).
        let gpu_guard = if task.requires_gpu && task.priority.can_acquire_gpu() {
            let turn_id = task.id.to_string();
            match gpu_lease
                .acquire_guard_gated(GpuOwner::L1Worker, turn_id, None, 0)
                .await
            {
                Ok(guard) => Some(guard),
                Err(e) => {
                    return TaskResult::Failed {
                        reason: format!("GPU lease failed: {}", e),
                        total_duration: start.elapsed(),
                    };
                }
            }
        } else {
            None
        };

        if gpu_guard.is_some() {
            let _ = cmd_tx.send(ControllerCommand::GpuLeaseAcquired { task_id });
        }
        let (work, on_cancel) = task.payload.into_parts();
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                if let Some(cancel_work) = on_cancel {
                    cancel_work();
                }
                TaskResult::Cancelled {
                    reason: "Cancelled by ExecutiveController".into(),
                }
            }
            result = work => result,
        };
        drop(gpu_guard);
        if task.requires_gpu && task.priority.can_acquire_gpu() {
            let _ = cmd_tx.send(ControllerCommand::GpuLeaseReleased { task_id });
        }
        result
    }

    fn instant_timestamp(instant: std::time::Instant) -> String {
        let elapsed = instant.elapsed();
        chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::from_std(elapsed).unwrap_or_default())
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339()
    }

    fn publish_snapshot(&self) {
        const QUEUED_SNAPSHOT_CAP: usize = 256;
        let active_foreground = self.foreground.as_ref().map(|task| ExecutiveTaskSnapshot {
            id: task.id,
            priority: task.priority,
            source: task.source.clone(),
            state: TaskState::Running,
            description: task.description.clone(),
            submitted_at: Self::instant_timestamp(task.submitted_at),
            started_at: Some(task.started_at_utc.clone()),
            completed_at: None,
            duration_ms: None,
            error: None,
            requires_gpu: task.requires_gpu,
        });
        let mut active_background = self.background_tasks.values().cloned().collect::<Vec<_>>();
        active_background.sort_by(|left, right| left.started_at.cmp(&right.started_at));

        let mut queued_tasks = self.queue.iter().map(|item| &item.0).collect::<Vec<_>>();
        queued_tasks.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.submitted_at.cmp(&right.submitted_at))
        });
        let queued = queued_tasks
            .into_iter()
            .take(QUEUED_SNAPSHOT_CAP)
            .map(|queued| ExecutiveTaskSnapshot {
                id: queued.request.id,
                priority: queued.request.priority,
                source: queued.request.source.clone(),
                state: TaskState::Queued,
                description: queued.request.payload.description(),
                submitted_at: Self::instant_timestamp(queued.request.submitted_at),
                started_at: None,
                completed_at: None,
                duration_ms: None,
                error: None,
                requires_gpu: queued.request.requires_gpu,
            })
            .collect();

        self.snapshot_tx.send_replace(ExecutiveSnapshot {
            active_foreground,
            active_background,
            queued,
            gpu_lease_holder: self.gpu_lease_holder,
            gpu_lease_remaining_ms: None,
            total_completed: self.total_completed,
            total_failed: self.total_failed,
        });
    }

    /// Emit a controller event (non-blocking, best-effort).
    fn emit(&self, event: ControllerEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Handle an internal command (cancel + GPU lifecycle).
    fn handle_command(&mut self, cmd: ControllerCommand) {
        match cmd {
            ControllerCommand::CancelTask { task_id } => {
                if let Some(ref foreground) = self.foreground {
                    if foreground.id == task_id {
                        tracing::info!(%task_id, "Cancelling foreground executive task");
                        foreground.cancel.cancel();
                        return;
                    }
                }

                if let Some(cancel) = self.background_cancellations.get(&task_id) {
                    tracing::info!(%task_id, "Cancelling background executive task");
                    cancel.cancel();
                    return;
                }

                let mut remaining = BinaryHeap::new();
                let mut found = false;
                while let Some(Reverse(queued)) = self.queue.pop() {
                    if queued.request.id == task_id {
                        found = true;
                        queued.request.cancel.cancel();
                    } else {
                        remaining.push(Reverse(queued));
                    }
                }
                self.queue = remaining;
                if found {
                    tracing::info!(%task_id, "Cancelled queued executive task");
                } else {
                    tracing::debug!(%task_id, "Executive cancel was idempotent; task not active");
                }
            }
            ControllerCommand::GpuLeaseAcquired { task_id } => {
                self.gpu_lease_holder = Some(task_id);
                self.emit(ControllerEvent::GpuLeaseAcquired { task_id });
            }
            ControllerCommand::GpuLeaseReleased { task_id } => {
                if self.gpu_lease_holder == Some(task_id) {
                    self.gpu_lease_holder = None;
                }
                self.emit(ControllerEvent::GpuLeaseReleased { task_id });
            }
        }
    }

    /// Graceful shutdown: cancel all tasks, wait briefly, then exit.
    async fn graceful_shutdown(&mut self) {
        // Cancel all background tasks
        self.background.abort_all();

        // Cancel foreground
        if let Some(fg) = self.foreground.take() {
            fg.cancel.cancel();
            let _ = fg.join.await;
        }

        // Drain remaining queue
        while let Some(Reverse(task)) = self.queue.pop() {
            task.request.cancel.cancel();
        }

        tracing::info!("ExecutiveController shutdown complete");
    }
}

/// Public handle for submitting tasks to the ExecutiveController.
/// This is what subsystems (voice pipeline, text chat, etc.) hold.
#[derive(Clone)]
pub struct ExecutiveSender {
    tx: mpsc::UnboundedSender<TaskRequest>,
    cmd_tx: mpsc::UnboundedSender<ControllerCommand>,
    event_tx: broadcast::Sender<ControllerEvent>,
    snapshot_rx: watch::Receiver<ExecutiveSnapshot>,
    cancel: CancellationToken,
}

impl ExecutiveSender {
    /// Submit a task to the ExecutiveController.
    /// Returns `Err` if the controller has shut down.
    pub fn submit(&self, task: TaskRequest) -> Result<(), TaskRequest> {
        self.tx.send(task).map_err(|e| e.0)
    }

    /// Request cancellation of a task by ID.
    /// Returns `Ok(())` even if the task ID is not found (idempotent).
    pub fn cancel_task(&self, task_id: uuid::Uuid) -> Result<(), String> {
        self.cmd_tx
            .send(ControllerCommand::CancelTask { task_id })
            .map_err(|_| "ExecutiveController has shut down".to_string())
    }

    /// Subscribe to controller lifecycle events without lossy polling.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ControllerEvent> {
        self.event_tx.subscribe()
    }

    /// Return latest bounded controller-owned read model without blocking the dispatcher.
    pub fn snapshot(&self) -> ExecutiveSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    /// Check if the controller is still running.
    pub fn is_alive(&self) -> bool {
        !self.cancel.is_cancelled()
    }

    /// Signal the controller to shut down gracefully.
    /// The controller will drain its queue and exit the run loop.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}
