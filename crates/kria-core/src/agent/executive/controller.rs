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

use std::collections::BinaryHeap;
use std::cmp::Reverse;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
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
    CancelTask { task_id: uuid::Uuid },
}

pub struct ExecutiveController {
    /// Configuration.
    config: ExecutiveConfig,
    /// Ingress channel: receives tasks from all subsystems.
    rx: mpsc::UnboundedReceiver<TaskRequest>,
    /// Command channel: receives control commands (cancel, etc.).
    cmd_rx: mpsc::UnboundedReceiver<ControllerCommand>,
    /// Public sender for submitting tasks.
    #[allow(dead_code)]
    tx: mpsc::UnboundedSender<TaskRequest>,
    /// Priority queue for pending tasks (BinaryHeap with Reverse for min-heap behavior).
    queue: BinaryHeap<Reverse<QueuedTask>>,
    /// Currently running foreground task (max 1).
    foreground: Option<TaskHandle>,
    /// Background task pool.
    background: JoinSet<(uuid::Uuid, TaskResult)>,
    /// GPU lease manager (existing).
    gpu_lease: Arc<GpuLeaseManager>,
    /// Policy gate for command evaluation.
    policy_gate: Arc<dyn PolicyGate>,
    /// Preemption manager.
    preemption: PreemptionManager,
    /// Event broadcast (for UI observability).
    event_tx: watch::Sender<Option<ControllerEvent>>,
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
        let (event_tx, _event_rx) = watch::channel(None);
        let shutdown = CancellationToken::new();

        let sender = ExecutiveSender {
            tx: tx.clone(),
            cmd_tx,
            cancel: shutdown.clone(),
        };

        let preemption = PreemptionManager::new(std::time::Duration::from_millis(
            config.preemption_grace_ms,
        ));

        let controller = Self {
            config,
            rx,
            cmd_rx,
            tx,
            queue: BinaryHeap::new(),
            foreground: None,
            background: JoinSet::new(),
            gpu_lease,
            policy_gate,
            preemption,
            event_tx,
            shutdown,
            last_foreground_completed: None,
        };

        (controller, sender)
    }

    /// Subscribe to controller events (for UI).
    pub fn subscribe_events(&self) -> watch::Receiver<Option<ControllerEvent>> {
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
                tx: _,
                queue: _,
                foreground,
                background,
                gpu_lease: _,
                policy_gate: _,
                preemption: _,
                event_tx: _,
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

            // After any event, try to drain queued tasks
            self.try_drain_queue().await;
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
            ScheduleDecision::Preempt { victim_id, replacement } => {
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
                if self.foreground.is_some() || !self.background.is_empty() || !self.queue.is_empty() {
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
        let requires_gpu = task.requires_gpu;
        let cancel = task.cancel.clone();
        let started_at = std::time::Instant::now();

        self.emit(ControllerEvent::TaskStarted {
            task_id,
            priority,
            source: source.clone(),
        });

        if priority.is_foreground() {
            // Foreground task: spawn and track as the active foreground.
            let gpu_lease = self.gpu_lease.clone();
            let policy_gate = self.policy_gate.clone();
            let cancel_clone = cancel.clone();

            let join = tokio::spawn(async move {
                let result = Self::execute_task(task, gpu_lease, policy_gate, cancel_clone).await;
                result
            });

            self.foreground = Some(TaskHandle {
                id: task_id,
                priority,
                source,
                cancel,
                join,
                started_at,
                requires_gpu,
            });
        } else {
            // Background task: spawn into JoinSet.
            let gpu_lease = self.gpu_lease.clone();
            let policy_gate = self.policy_gate.clone();
            let cancel_clone = cancel.clone();

            self.background.spawn(async move {
                let result = Self::execute_task(task, gpu_lease, policy_gate, cancel_clone).await;
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

    /// Try to drain queued tasks when resources become available.
    /// This is a no-op future that resolves when there's capacity.
    async fn try_drain_queue(&mut self) {
        // Only drain if we have capacity
        if self.foreground.is_some() && self.background.len() >= self.config.max_background_tasks {
            // No capacity — this branch will never resolve, which is fine
            // (tokio::select! will pick other branches)
            std::future::pending::<()>().await;
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

                // Cancel the victim
                fg.cancel.cancel();

                // Wait for graceful shutdown (up to grace period)
                self.preemption.wait_for_grace(&mut fg.join).await;

                // Drop the old handle (aborts if still running)
                self.foreground = None;

                self.emit(ControllerEvent::TaskPreempted {
                    victim_id,
                    replacement_id: replacement.id,
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
            tracing::info!(
                task_id = %fg.id,
                priority = %fg.priority,
                result = %result,
                "Foreground task completed"
            );

            self.emit(ControllerEvent::TaskCompleted {
                task_id: fg.id,
                result_summary: result.to_string(),
                duration_ms,
            });

            // Release GPU lease if held
            if fg.requires_gpu {
                self.emit(ControllerEvent::GpuLeaseReleased { task_id: fg.id });
            }

            self.last_foreground_completed = Some(std::time::Instant::now());
        }
    }

    /// Handle background task completion.
    fn on_background_complete(&mut self, result: Result<(uuid::Uuid, TaskResult), tokio::task::JoinError>) {
        match result {
            Ok((task_id, task_result)) => {
                tracing::debug!(
                    task_id = %task_id,
                    result = %task_result,
                    "Background task completed"
                );
                self.emit(ControllerEvent::TaskCompleted {
                    task_id,
                    result_summary: task_result.to_string(),
                    duration_ms: 0, // Background tasks track their own duration
                });
            }
            Err(e) => {
                tracing::error!("Background task panicked: {}", e);
            }
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
    ) -> TaskResult {
        let start = std::time::Instant::now();

        // Acquire GPU lease if needed
        let _gpu_guard = if task.requires_gpu && task.priority.can_acquire_gpu() {
            let is_foreground = task.priority.is_foreground();
            let turn_id = task.id.to_string();
            match gpu_lease
                .acquire_lease(GpuOwner::L1Worker, turn_id, is_foreground)
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

        // Execute based on payload type
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                TaskResult::Cancelled {
                    reason: "Cancelled by ExecutiveController".into(),
                }
            }
            result = Self::run_payload(task.payload, cancel.clone()) => {
                result
            }
        }
    }

    /// Run the actual task payload. This is where the work happens.
    async fn run_payload(payload: TaskPayload, _cancel: CancellationToken) -> TaskResult {
        let start = std::time::Instant::now();

        match payload {
            TaskPayload::UserTurn { text, is_voice, session_id } => {
                // TODO: Wire into AgentLoop.run()
                tracing::info!(
                    text = %text,
                    is_voice = is_voice,
                    session_id = %session_id,
                    "Processing user turn"
                );
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(format!("Processed: {}", text)),
                }
            }

            TaskPayload::ExecuteCommand { command } => {
                // TODO: Wire into SubprocessExecutor.execute()
                tracing::info!(command = ?command, "Executing command");
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(format!("Executed: {} {}", command.binary, command.args.join(" "))),
                }
            }

            TaskPayload::BackgroundDiagnostics { commands } => {
                tracing::info!(count = commands.len(), "Running background diagnostics");
                // TODO: Execute each command via SubprocessExecutor
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(format!("Ran {} diagnostic commands", commands.len())),
                }
            }

            TaskPayload::GatherEvidence { commands } => {
                tracing::info!(count = commands.len(), "Gathering evidence");
                // TODO: Execute each command via SubprocessExecutor
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(format!("Gathered evidence from {} commands", commands.len())),
                }
            }

            TaskPayload::CompileSkill { plan_json: _ } => {
                tracing::info!("Compiling skill from plan");
                // TODO: Wire into SkillCompiler
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some("Skill compiled".into()),
                }
            }

            TaskPayload::VramMaintenanceRefresh { reason } => {
                tracing::info!(reason = %reason, "Starting VRAM maintenance refresh");
                // TODO: Wire into Orchestrator.evict_to_ram() → reload
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some("VRAM refreshed".into()),
                }
            }

            TaskPayload::HitlResponse { request_id, approved } => {
                tracing::info!(
                    request_id = %request_id,
                    approved = approved,
                    "HITL response received"
                );
                // TODO: Wire into HitlGateway.respond()
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(format!("HITL response: {}", if approved { "approved" } else { "rejected" })),
                }
            }

            TaskPayload::Maintenance { description } => {
                tracing::info!(description = %description, "Running maintenance task");
                TaskResult::Success {
                    total_duration: start.elapsed(),
                    output: Some(description),
                }
            }
        }
    }

    /// Emit a controller event (non-blocking, best-effort).
    fn emit(&self, event: ControllerEvent) {
        let _ = self.event_tx.send(Some(event));
    }

    /// Handle an internal command (cancel, etc.).
    fn handle_command(&mut self, cmd: ControllerCommand) {
        match cmd {
            ControllerCommand::CancelTask { task_id } => {
                // Check foreground first
                if let Some(ref fg) = self.foreground {
                    if fg.id == task_id {
                        tracing::info!(task_id = %task_id, "Cancelling foreground task via command");
                        fg.cancel.cancel();
                        return;
                    }
                }

                // Check queued tasks — remove and cancel matching entry
                let mut remaining = BinaryHeap::new();
                while let Some(Reverse(queued)) = self.queue.pop() {
                    if queued.request.id == task_id {
                        tracing::info!(task_id = %task_id, "Cancelling queued task via command");
                        queued.request.cancel.cancel();
                        // Don't re-insert; task is cancelled
                    } else {
                        remaining.push(Reverse(queued));
                    }
                }
                self.queue = remaining;

                // Background tasks are in a JoinSet — we can't cancel by ID
                // directly, but the task's own CancellationToken was already
                // cloned into the spawned future. If the caller has access to
                // the TaskRequest they can cancel via that token; otherwise the
                // task will complete normally.
                tracing::debug!(task_id = %task_id, "Cancel command processed (background tasks cancel via their own token)");
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
