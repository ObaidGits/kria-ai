//! Production Runtime Manager with authoritative lifecycle state machine.
//!
//! Transforms OpenClaw from simple container executor into production runtime manager.
//! Implements one runtime, one lifecycle, one scheduler, one health model, one recovery model.
//!
//! # Container Lifecycle State Machine
//!
//! Created → Preparing → Ready → Reserved → Executing → Cooling → Idle → Recycled → Destroyed
//!                  ↘ Failed → Recovering ↗
//!
//! No duplicated state tracking. Every runtime uses this lifecycle.

use super::config::OpenClawConfig;
use super::types::ResourceClass;
use bollard::container::{
    Config as ContainerConfig, CreateContainerOptions, InspectContainerOptions,
    RemoveContainerOptions, StartContainerOptions,
};
use bollard::Docker;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Container lifecycle state - authoritative for all runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerState {
    /// Container created but not started
    Created,
    /// Container starting, installing deps, health checks  
    Preparing,
    /// Container healthy, available in warm pool
    Ready,
    /// Container checked out for execution
    Reserved,
    /// Container running skill code
    Executing,
    /// Container cleaning up after execution
    Cooling,
    /// Container available for reuse
    Idle,
    /// Container marked for destruction
    Recycled,
    /// Container removed from system
    Destroyed,
    /// Container unhealthy, needs recovery
    Failed,
    /// Container being repaired/restarted
    Recovering,
}

/// Container health status for continuous monitoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// Container operating normally
    Healthy,
    /// Container showing performance degradation
    Degraded,
    /// Container not responding to commands
    Hung,
    /// Container being recovered
    Recovering,
    /// Container completely failed
    Dead,
}

/// Priority level for runtime scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Real-time priority - Voice, LLM, Vision (never starved)
    Realtime = 0,
    /// Interactive user operations
    Interactive = 1,
    /// Background automation
    Background = 2,
    /// Batch processing
    Batch = 3,
    /// Low priority tasks
    Low = 4,
}

/// Runtime container with full lifecycle tracking.
#[derive(Debug, Clone)]
pub struct RuntimeContainer {
    pub container_id: String,
    pub state: ContainerState,
    pub health: HealthStatus,
    pub resource_class: ResourceClass,
    pub created_at: Instant,
    pub last_used: Instant,
    pub state_changed_at: Instant,
    pub reuse_count: u32,
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub execution_time: Duration,
    pub priority: Priority,
    pub heartbeat_failures: u32,
    pub recovery_attempts: u32,
}

impl RuntimeContainer {
    pub fn new(container_id: String, resource_class: ResourceClass) -> Self {
        let now = Instant::now();
        Self {
            container_id,
            state: ContainerState::Created,
            health: HealthStatus::Healthy,
            resource_class,
            created_at: now,
            last_used: now,
            state_changed_at: now,
            reuse_count: 0,
            cpu_usage: 0.0,
            memory_usage: 0,
            execution_time: Duration::ZERO,
            priority: Priority::Background,
            heartbeat_failures: 0,
            recovery_attempts: 0,
        }
    }

    pub fn transition_state(&mut self, new_state: ContainerState) -> bool {
        if self.is_valid_transition(new_state) {
            self.state = new_state;
            self.state_changed_at = Instant::now();
            true
        } else {
            false
        }
    }

    fn is_valid_transition(&self, new_state: ContainerState) -> bool {
        use ContainerState::*;
        match (self.state, new_state) {
            // Normal lifecycle flow
            (Created, Preparing) => true,
            (Preparing, Ready) => true,
            (Ready, Reserved) => true,
            (Reserved, Executing) => true,
            (Executing, Cooling) => true,
            (Cooling, Idle) => true,
            (Idle, Reserved) => true,
            (Idle, Recycled) => true,
            (Recycled, Destroyed) => true,

            // Failure transitions from any state
            (_, Failed) => true,
            (Failed, Recovering) => true,
            (Recovering, Ready) => true,

            // Emergency cleanup
            (_, Destroyed) => true,

            _ => false,
        }
    }

    pub fn set_health(&mut self, health: HealthStatus) {
        self.health = health;
        if matches!(health, HealthStatus::Dead | HealthStatus::Hung) {
            self.transition_state(ContainerState::Failed);
        }
    }

    pub fn increment_reuse(&mut self) {
        self.reuse_count += 1;
        self.last_used = Instant::now();
    }

    pub fn is_eligible_for_reuse(&self) -> bool {
        matches!(self.state, ContainerState::Idle | ContainerState::Ready)
            && matches!(self.health, HealthStatus::Healthy)
    }

    pub fn is_stale(&self, max_idle_duration: Duration) -> bool {
        matches!(self.state, ContainerState::Idle) && self.last_used.elapsed() > max_idle_duration
    }

    pub fn should_recycle(&self, max_reuse_count: u32) -> bool {
        self.reuse_count >= max_reuse_count
            || matches!(self.health, HealthStatus::Dead | HealthStatus::Degraded)
            || self.recovery_attempts > 3
    }

    /// Check if container is aging and needs recycling (A4.2)
    pub fn is_aging(&self, aging_threshold: Duration) -> bool {
        self.created_at.elapsed() > aging_threshold
    }

    /// Check if container shows fragmentation (A4.2)
    pub fn is_fragmented(&self, fragmentation_threshold: f64) -> bool {
        // Simple fragmentation check based on memory usage pattern
        self.memory_usage as f64 / (1024.0 * 1024.0 * 512.0) > fragmentation_threshold
    }

    /// Get container priority for warm pool optimization (A4.2)
    pub fn get_warmth_score(&self) -> u32 {
        let mut score = 0;

        // Recent usage gets higher score
        if self.last_used.elapsed() < Duration::from_secs(60) {
            score += 10;
        }

        // Lower reuse count gets higher score (fresher containers preferred)
        score += 100 - self.reuse_count.min(100);

        // Healthy containers get priority
        if matches!(self.health, HealthStatus::Healthy) {
            score += 50;
        }

        score
    }
}

/// Warm pool configuration - production-grade warm pool management (A4.2).
#[derive(Debug, Clone)]
pub struct WarmPoolConfig {
    /// Minimum containers per class - always maintained
    pub minimum_containers: usize,
    /// Maximum containers per class - hard limit
    pub maximum_containers: usize,
    /// Warm reserve for immediate use - ready containers
    pub warm_reserve: usize,
    /// Idle reserve for reuse - containers waiting for work
    pub idle_reserve: usize,
    /// Burst reserve for high load - extra capacity during spikes
    pub burst_reserve: usize,
    /// Max idle time before recycling - prevents container aging
    pub max_idle_duration: Duration,
    /// Max reuse count before recycling - prevents fragmentation
    pub max_reuse_count: u32,
    /// Health check interval - continuous monitoring
    pub health_check_interval: Duration,
    /// Recovery timeout - time to wait for recovery
    pub recovery_timeout: Duration,
    /// Container aging threshold - time before aging
    pub aging_threshold: Duration,
    /// Fragmentation detection threshold - memory fragmentation limit
    pub fragmentation_threshold: f64,
    /// Prewarming interval - how often to check and prewarm
    pub prewarming_interval: Duration,
    /// Cold creation timeout - timeout for creating new containers
    pub cold_creation_timeout: Duration,
}

impl Default for WarmPoolConfig {
    fn default() -> Self {
        Self {
            minimum_containers: 2,
            maximum_containers: 10,
            warm_reserve: 3,
            idle_reserve: 2,
            burst_reserve: 5,
            max_idle_duration: Duration::from_secs(300),
            max_reuse_count: 50,
            health_check_interval: Duration::from_secs(30),
            recovery_timeout: Duration::from_secs(60),
            aging_threshold: Duration::from_secs(600), // 10 minutes
            fragmentation_threshold: 0.8,              // 80% memory fragmentation
            prewarming_interval: Duration::from_secs(15), // Check every 15s
            cold_creation_timeout: Duration::from_secs(30),
        }
    }
}

/// Active runtime with lease tracking.
#[derive(Debug)]
pub struct ActiveRuntime {
    pub invocation_id: String,
    pub container_id: String,
    pub skill_id: String,
    pub workspace_path: String,
    pub priority: Priority,
    pub started_at: Instant,
    pub lease_duration: Duration,
    /// Semaphore permit - released on drop
    pub _permit: OwnedSemaphorePermit,
}

/// Runtime scheduler chooses containers based on priority and availability (A4.5).
#[derive(Debug)]
pub struct RuntimeScheduler {
    /// Queue of pending requests by priority (A4.5 queue path — reserved for backpressure).
    #[allow(dead_code)]
    pending_queue: HashMap<Priority, VecDeque<SchedulingRequest>>,
    /// Resource pressure from HRA integration
    resource_pressure: f64,
    /// Scheduling metrics (A4.7)
    metrics: SchedulingMetrics,
    /// Cancellation tracking (A4.10)
    active_cancellations: HashMap<String, CancellationTracker>,
}

/// Scheduling metrics for A4.7
#[derive(Debug, Default, Clone)]
pub struct SchedulingMetrics {
    pub total_requests: u64,
    pub warm_reuse_count: u64,
    pub cold_creation_count: u64,
    pub queue_rejections: u64,
    pub average_queue_wait_ms: f64,
    pub container_utilization: HashMap<ResourceClass, f64>,
}

/// Cancellation tracking for A4.10.
///
/// Tracks an in-flight invocation's current phase so an external cancel can
/// target it. `cancelled` is the authoritative flag — merely tracking a phase
/// must NOT mark the invocation cancelled (that inversion previously made every
/// warm checkout return `Cancelled`).
#[derive(Debug)]
pub struct CancellationTracker {
    pub invocation_id: String,
    pub requested_at: Instant,
    pub phase: CancellationPhase,
    /// True only when cancellation was explicitly requested.
    pub cancelled: bool,
}

/// Phases where cancellation can occur (A4.10)
#[derive(Debug)]
pub enum CancellationPhase {
    Prepare,
    Checkout,
    Execution,
    Rpc,
    Cleanup,
}

#[derive(Debug)]
#[allow(dead_code)] // A4.5 queued-scheduling path; fields consumed when queue is drained.
struct SchedulingRequest {
    resource_class: ResourceClass,
    priority: Priority,
    requested_at: Instant,
    responder: tokio::sync::oneshot::Sender<Result<String, SchedulingError>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulingError {
    #[error("no containers available for class {resource_class:?}")]
    NoContainersAvailable { resource_class: ResourceClass },
    #[error("resource pressure too high: {pressure}")]
    ResourcePressure { pressure: f64 },
    #[error("queue full for priority {priority:?}")]
    QueueFull { priority: Priority },
}

impl RuntimeScheduler {
    pub fn new() -> Self {
        Self {
            pending_queue: HashMap::new(),
            resource_pressure: 0.0,
            metrics: SchedulingMetrics::default(),
            active_cancellations: HashMap::new(),
        }
    }

    /// Schedule container with comprehensive priority handling (A4.5 + A4.7 + A4.9).
    pub async fn schedule_container(
        &mut self,
        containers: &HashMap<String, RuntimeContainer>,
        resource_class: ResourceClass,
        priority: Priority,
    ) -> Result<String, SchedulingError> {
        let start_time = Instant::now();
        self.metrics.total_requests += 1;

        // A4.9: Priority scheduling - never starve Voice/LLM/Vision
        if priority == Priority::Realtime {
            // Realtime always gets preference, even under pressure
        } else if self.resource_pressure > 0.8 && priority > Priority::Interactive {
            self.metrics.queue_rejections += 1;
            return Err(SchedulingError::ResourcePressure {
                pressure: self.resource_pressure,
            });
        }

        // A4.5: Scheduler decides warm → idle → recycle → cold → queue → reject

        // Step 1: Try warm containers (first choice)
        let warm_result = self.try_warm_containers(containers, resource_class, priority);
        if let Some(container_id) = warm_result {
            self.metrics.warm_reuse_count += 1;
            self.update_scheduling_metrics(start_time, resource_class);
            return Ok(container_id);
        }

        // Step 2: Try idle containers
        let idle_result = self.try_idle_containers(containers, resource_class);
        if let Some(container_id) = idle_result {
            self.metrics.warm_reuse_count += 1;
            self.update_scheduling_metrics(start_time, resource_class);
            return Ok(container_id);
        }

        // Step 3: Check if we can force recycle some containers for higher priority
        if priority <= Priority::Interactive {
            let recycle_result =
                self.try_recycle_for_priority(containers, resource_class, priority);
            if let Some(container_id) = recycle_result {
                self.update_scheduling_metrics(start_time, resource_class);
                return Ok(container_id);
            }
        }

        // Step 4: Need cold creation
        self.metrics.cold_creation_count += 1;
        self.update_scheduling_metrics(start_time, resource_class);

        // Signal cold creation needed
        Err(SchedulingError::NoContainersAvailable { resource_class })
    }

    /// Try to find warm containers (A4.5)
    fn try_warm_containers(
        &self,
        containers: &HashMap<String, RuntimeContainer>,
        resource_class: ResourceClass,
        priority: Priority,
    ) -> Option<String> {
        let mut warm_candidates: Vec<_> = containers
            .values()
            .filter(|c| {
                c.resource_class == resource_class
                    && c.is_eligible_for_reuse()
                    && c.priority <= priority
            })
            .collect();

        if !warm_candidates.is_empty() {
            // Sort by warmth score (prefer recently used, healthy, low reuse count)
            warm_candidates.sort_by_key(|c| std::cmp::Reverse(c.get_warmth_score()));

            warm_candidates.first().map(|c| c.container_id.clone())
        } else {
            None
        }
    }

    /// Try to find idle containers (A4.5)
    fn try_idle_containers(
        &self,
        containers: &HashMap<String, RuntimeContainer>,
        resource_class: ResourceClass,
    ) -> Option<String> {
        let mut idle_candidates: Vec<_> = containers
            .values()
            .filter(|c| {
                c.resource_class == resource_class
                    && matches!(c.state, ContainerState::Idle)
                    && matches!(c.health, HealthStatus::Healthy)
            })
            .collect();

        if !idle_candidates.is_empty() {
            // Sort by freshness (prefer newer containers)
            idle_candidates.sort_by_key(|c| c.created_at);
            idle_candidates.reverse();

            idle_candidates.first().map(|c| c.container_id.clone())
        } else {
            None
        }
    }

    /// Try to recycle lower priority containers for higher priority work (A4.5 + A4.9)
    fn try_recycle_for_priority(
        &self,
        containers: &HashMap<String, RuntimeContainer>,
        resource_class: ResourceClass,
        requesting_priority: Priority,
    ) -> Option<String> {
        // Find containers that can be preempted for higher priority work
        let mut preempt_candidates: Vec<_> = containers
            .values()
            .filter(|c| {
                c.resource_class == resource_class
                    && c.priority > requesting_priority
                    && matches!(
                        c.state,
                        ContainerState::Executing | ContainerState::Reserved
                    )
            })
            .collect();

        if !preempt_candidates.is_empty() {
            // Sort by priority (preempt lowest priority first)
            preempt_candidates.sort_by_key(|c| std::cmp::Reverse(c.priority));

            // Return container for preemption
            preempt_candidates.first().map(|c| c.container_id.clone())
        } else {
            None
        }
    }

    /// Update scheduling metrics (A4.7)
    fn update_scheduling_metrics(&mut self, start_time: Instant, resource_class: ResourceClass) {
        let wait_time_ms = start_time.elapsed().as_millis() as f64;

        // Update average queue wait time
        let total_requests = self.metrics.total_requests as f64;
        self.metrics.average_queue_wait_ms =
            (self.metrics.average_queue_wait_ms * (total_requests - 1.0) + wait_time_ms)
                / total_requests;

        // Update container utilization
        let current_utilization = self
            .metrics
            .container_utilization
            .get(&resource_class)
            .copied()
            .unwrap_or(0.0);

        // Simple utilization calculation (requests per minute)
        let new_utilization = (current_utilization * 0.9) + (1.0 * 0.1);
        self.metrics
            .container_utilization
            .insert(resource_class, new_utilization);
    }

    /// Track an in-flight invocation's phase (A4.10). Does NOT mark it cancelled —
    /// preserves an existing `cancelled` flag if the invocation is already tracked.
    pub fn register_cancellation(&mut self, invocation_id: String, phase: CancellationPhase) {
        let already_cancelled = self
            .active_cancellations
            .get(&invocation_id)
            .map(|t| t.cancelled)
            .unwrap_or(false);
        self.active_cancellations.insert(
            invocation_id.clone(),
            CancellationTracker {
                invocation_id,
                requested_at: Instant::now(),
                phase,
                cancelled: already_cancelled,
            },
        );
    }

    /// Explicitly request cancellation of a tracked invocation (A4.10).
    pub fn request_cancellation(&mut self, invocation_id: &str) {
        if let Some(t) = self.active_cancellations.get_mut(invocation_id) {
            t.cancelled = true;
        }
    }

    /// Complete cancellation (A4.10)
    pub fn complete_cancellation(&mut self, invocation_id: &str) -> Option<CancellationTracker> {
        self.active_cancellations.remove(invocation_id)
    }

    /// Check if invocation has an explicit cancellation request (A4.10)
    pub fn is_cancelled(&self, invocation_id: &str) -> bool {
        self.active_cancellations
            .get(invocation_id)
            .map(|t| t.cancelled)
            .unwrap_or(false)
    }

    pub fn update_resource_pressure(&mut self, pressure: f64) {
        self.resource_pressure = pressure;
    }
}

/// Health monitor continuously monitors container health (A4.3).
#[derive(Debug)]
#[allow(dead_code)] // A4.3 fields (failed_containers/disk_thresholds/heartbeat_timeout) reserved for extended checks.
pub struct HealthMonitor {
    check_interval: Duration,
    failed_containers: HashMap<String, Instant>,
    cpu_thresholds: CpuThresholds,
    memory_thresholds: MemoryThresholds,
    disk_thresholds: DiskThresholds,
    network_thresholds: NetworkThresholds,
    heartbeat_timeout: Duration,
    hung_detection_timeout: Duration,
}

/// CPU monitoring thresholds (A4.3)
#[derive(Debug, Clone)]
pub struct CpuThresholds {
    pub healthy_max: f64,    // 70% CPU usage
    pub degraded_max: f64,   // 90% CPU usage
    pub hung_threshold: f64, // 98% CPU usage (hung process)
}

/// Memory monitoring thresholds (A4.3)
#[derive(Debug, Clone)]
pub struct MemoryThresholds {
    pub healthy_max: f64,  // 70% memory usage
    pub degraded_max: f64, // 85% memory usage
    pub critical_max: f64, // 95% memory usage (near OOM)
}

/// Disk monitoring thresholds (A4.3)
#[derive(Debug, Clone)]
pub struct DiskThresholds {
    pub healthy_max: f64,  // 80% disk usage
    pub degraded_max: f64, // 90% disk usage
    pub critical_max: f64, // 98% disk usage
}

/// Network monitoring thresholds (A4.3)
#[derive(Debug, Clone)]
pub struct NetworkThresholds {
    pub timeout_ms: u64,          // Network timeout in ms
    pub max_dropped_packets: u32, // Max dropped packets before degraded
}

/// Container health metrics (A4.3)
#[derive(Debug, Clone)]
pub struct HealthMetrics {
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub disk_usage: u64,
    pub disk_limit: u64,
    pub network_rx_dropped: u32,
    pub network_tx_dropped: u32,
    pub process_count: u32,
    pub uptime: Duration,
    pub last_heartbeat: Option<Instant>,
    pub container_state: String,
    pub exit_code: Option<i64>,
}

impl HealthMonitor {
    pub fn new(check_interval: Duration) -> Self {
        Self {
            check_interval,
            failed_containers: HashMap::new(),
            cpu_thresholds: CpuThresholds {
                healthy_max: 0.7,
                degraded_max: 0.9,
                hung_threshold: 0.98,
            },
            memory_thresholds: MemoryThresholds {
                healthy_max: 0.7,
                degraded_max: 0.85,
                critical_max: 0.95,
            },
            disk_thresholds: DiskThresholds {
                healthy_max: 0.8,
                degraded_max: 0.9,
                critical_max: 0.98,
            },
            network_thresholds: NetworkThresholds {
                timeout_ms: 5000,
                max_dropped_packets: 100,
            },
            heartbeat_timeout: Duration::from_secs(30),
            hung_detection_timeout: Duration::from_secs(60),
        }
    }

    /// Start comprehensive health monitoring loop (A4.3).
    pub async fn start_monitoring(
        &self,
        docker: Docker,
        containers: Arc<RwLock<HashMap<String, RuntimeContainer>>>,
        shutdown: broadcast::Receiver<()>,
    ) {
        let mut shutdown = shutdown;
        let mut interval = tokio::time::interval(self.check_interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.comprehensive_health_check(&docker, &containers).await;
                }
                _ = shutdown.recv() => {
                    info!("Health monitoring shutting down");
                    break;
                }
            }
        }
    }

    /// Comprehensive health check covering all aspects (A4.3 + A4.4)
    async fn comprehensive_health_check(
        &self,
        docker: &Docker,
        containers: &Arc<RwLock<HashMap<String, RuntimeContainer>>>,
    ) {
        let container_ids: Vec<String> = { containers.read().await.keys().cloned().collect() };

        for container_id in container_ids {
            let health_status = self
                .check_container_comprehensive_health(docker, &container_id)
                .await;

            let mut containers_write = containers.write().await;
            if let Some(container) = containers_write.get_mut(&container_id) {
                let old_health = container.health;
                container.set_health(health_status);

                // A4.4: Trigger automatic recovery for degraded/dead containers
                if matches!(health_status, HealthStatus::Dead | HealthStatus::Hung)
                    && old_health != health_status
                {
                    let failure_type = match health_status {
                        HealthStatus::Dead => FailureType::ContainerCrash,
                        HealthStatus::Hung => FailureType::RpcTimeout,
                        _ => FailureType::ContainerCrash,
                    };

                    // Determine recovery system and trigger recovery
                    // Note: In production, this would be handled by the RuntimeManager
                    warn!(
                        container_id = %container_id,
                        old_health = ?old_health,
                        new_health = ?health_status,
                        failure_type = ?failure_type,
                        "Container health degraded - recovery needed"
                    );
                } else if old_health != health_status {
                    info!(
                        container_id = %container_id,
                        old_health = ?old_health,
                        new_health = ?health_status,
                        "Container health status changed"
                    );
                }
            }
        }
    }

    /// Check all health aspects of container (A4.3)
    async fn check_container_comprehensive_health(
        &self,
        docker: &Docker,
        container_id: &str,
    ) -> HealthStatus {
        // Get container inspect info
        let inspect_result = docker
            .inspect_container(container_id, None::<InspectContainerOptions>)
            .await;

        let inspect_info = match inspect_result {
            Ok(info) => info,
            Err(_) => return HealthStatus::Dead,
        };

        // Check if container is running
        let container_running = inspect_info
            .state
            .as_ref()
            .and_then(|s| s.running)
            .unwrap_or(false);

        if !container_running {
            return HealthStatus::Dead;
        }

        // Check exit code
        if let Some(state) = &inspect_info.state {
            if let Some(exit_code) = state.exit_code {
                if exit_code != 0 {
                    return HealthStatus::Dead;
                }
            }
        }

        // Get basic stats (simplified for now)
        let stats_stream = docker.stats(
            container_id,
            Some(bollard::container::StatsOptions {
                stream: false,
                one_shot: true,
            }),
        );

        // Try to get one stats sample
        use futures::StreamExt;
        let mut stats_stream = stats_stream;
        if let Some(stats_result) = stats_stream.next().await {
            match stats_result {
                Ok(stats) => {
                    // Basic health check based on available stats
                    let health_metrics = self.extract_basic_health_metrics(&stats, &inspect_info);
                    self.evaluate_health_status(&health_metrics)
                }
                Err(_) => HealthStatus::Degraded,
            }
        } else {
            HealthStatus::Degraded
        }
    }

    /// Extract basic health metrics (A4.3 - simplified)
    fn extract_basic_health_metrics(
        &self,
        stats: &bollard::container::Stats,
        inspect_info: &bollard::models::ContainerInspectResponse,
    ) -> HealthMetrics {
        // CPU usage calculation (simplified) - skip complex calculation for now
        let cpu_usage = 0.0; // TODO: Implement proper CPU calculation when bollard types are available

        // Memory usage
        let (memory_usage, memory_limit) = (
            stats.memory_stats.usage.unwrap_or(0),
            stats.memory_stats.limit.unwrap_or(0),
        );

        // Network stats (basic) - skip complex network analysis for now
        let (rx_dropped, tx_dropped) = (0u32, 0u32); // TODO: Extract from network stats

        // Process count - use basic value
        let process_count = stats.pids_stats.current.unwrap_or(0) as u32;

        // Container uptime (basic)
        let uptime = Duration::from_secs(60); // Simplified - assume running for at least 60s

        HealthMetrics {
            cpu_usage,
            memory_usage,
            memory_limit,
            disk_usage: 0, // TODO: Extract from blkio stats
            disk_limit: 0,
            network_rx_dropped: rx_dropped,
            network_tx_dropped: tx_dropped,
            process_count,
            uptime,
            last_heartbeat: Some(Instant::now()),
            container_state: inspect_info
                .state
                .as_ref()
                .and_then(|s| s.status.as_ref())
                .map(|status| format!("{:?}", status))
                .unwrap_or_else(|| "unknown".to_string()),
            exit_code: inspect_info.state.as_ref().and_then(|s| s.exit_code),
        }
    }

    /// Evaluate health status based on comprehensive metrics (A4.3)
    fn evaluate_health_status(&self, metrics: &HealthMetrics) -> HealthStatus {
        // Check for dead state
        if metrics.exit_code.is_some() || metrics.container_state == "exited" {
            return HealthStatus::Dead;
        }

        // Check for hung state (high CPU for extended time + no recent heartbeat)
        if metrics.cpu_usage > self.cpu_thresholds.hung_threshold {
            if let Some(last_heartbeat) = metrics.last_heartbeat {
                if last_heartbeat.elapsed() > self.hung_detection_timeout {
                    return HealthStatus::Hung;
                }
            }
        }

        // Check for degraded state
        let is_cpu_degraded = metrics.cpu_usage > self.cpu_thresholds.degraded_max;
        let is_memory_degraded = metrics.memory_limit > 0
            && (metrics.memory_usage as f64 / metrics.memory_limit as f64)
                > self.memory_thresholds.degraded_max;
        let is_network_degraded = metrics.network_rx_dropped
            > self.network_thresholds.max_dropped_packets
            || metrics.network_tx_dropped > self.network_thresholds.max_dropped_packets;

        if is_cpu_degraded || is_memory_degraded || is_network_degraded {
            return HealthStatus::Degraded;
        }

        // Check for critical resource usage (still healthy but monitored)
        let is_memory_critical = metrics.memory_limit > 0
            && (metrics.memory_usage as f64 / metrics.memory_limit as f64)
                > self.memory_thresholds.critical_max;

        if is_memory_critical {
            warn!(
                memory_usage = metrics.memory_usage,
                memory_limit = metrics.memory_limit,
                usage_percent = (metrics.memory_usage as f64 / metrics.memory_limit as f64) * 100.0,
                "Container approaching memory limit"
            );
        }

        HealthStatus::Healthy
    }
}

/// Recovery system handles automatic container recovery (A4.4).
#[derive(Debug)]
pub struct RecoverySystem {
    max_recovery_attempts: u32,
    #[allow(dead_code)] // A4.4 reserved: base backoff for recovery retries.
    recovery_backoff: Duration,
    recovery_strategies: RecoveryStrategies,
}

/// Recovery strategies for different failure types (A4.4)
#[derive(Debug, Clone)]
pub struct RecoveryStrategies {
    /// Container crash recovery
    pub container_crash: RecoveryStrategy,
    /// Bridge crash recovery
    pub bridge_crash: RecoveryStrategy,
    /// RPC timeout recovery
    pub rpc_timeout: RecoveryStrategy,
    /// OOM recovery
    pub oom_recovery: RecoveryStrategy,
    /// Panic recovery
    pub panic_recovery: RecoveryStrategy,
    /// Docker restart recovery
    pub docker_restart: RecoveryStrategy,
}

/// Recovery strategy for specific failure type (A4.4)
#[derive(Debug, Clone)]
pub struct RecoveryStrategy {
    pub max_attempts: u32,
    pub backoff_multiplier: f64,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub recovery_actions: Vec<RecoveryAction>,
}

/// Recovery actions to take (A4.4)
#[derive(Debug, Clone)]
pub enum RecoveryAction {
    /// Restart the container
    RestartContainer,
    /// Recreate the container with same config
    RecreateContainer,
    /// Kill and recreate container
    ForceRecreateContainer,
    /// Clear container state and restart
    ClearStateAndRestart,
    /// Release all leases and restart
    ReleaseLeaseAndRestart,
    /// Notify recovery system of failure
    NotifyRecoveryFailure,
}

impl RecoverySystem {
    pub fn new() -> Self {
        Self {
            max_recovery_attempts: 3,
            recovery_backoff: Duration::from_secs(10),
            recovery_strategies: RecoveryStrategies {
                container_crash: RecoveryStrategy {
                    max_attempts: 3,
                    backoff_multiplier: 2.0,
                    initial_delay: Duration::from_secs(5),
                    max_delay: Duration::from_secs(60),
                    recovery_actions: vec![
                        RecoveryAction::RestartContainer,
                        RecoveryAction::RecreateContainer,
                        RecoveryAction::ForceRecreateContainer,
                    ],
                },
                bridge_crash: RecoveryStrategy {
                    max_attempts: 2,
                    backoff_multiplier: 1.5,
                    initial_delay: Duration::from_secs(2),
                    max_delay: Duration::from_secs(30),
                    recovery_actions: vec![
                        RecoveryAction::ClearStateAndRestart,
                        RecoveryAction::RecreateContainer,
                    ],
                },
                rpc_timeout: RecoveryStrategy {
                    max_attempts: 2,
                    backoff_multiplier: 1.0,
                    initial_delay: Duration::from_secs(1),
                    max_delay: Duration::from_secs(10),
                    recovery_actions: vec![RecoveryAction::RestartContainer],
                },
                oom_recovery: RecoveryStrategy {
                    max_attempts: 1,
                    backoff_multiplier: 1.0,
                    initial_delay: Duration::from_secs(5),
                    max_delay: Duration::from_secs(30),
                    recovery_actions: vec![RecoveryAction::ForceRecreateContainer],
                },
                panic_recovery: RecoveryStrategy {
                    max_attempts: 2,
                    backoff_multiplier: 2.0,
                    initial_delay: Duration::from_secs(3),
                    max_delay: Duration::from_secs(60),
                    recovery_actions: vec![
                        RecoveryAction::ClearStateAndRestart,
                        RecoveryAction::ForceRecreateContainer,
                    ],
                },
                docker_restart: RecoveryStrategy {
                    max_attempts: 1,
                    backoff_multiplier: 1.0,
                    initial_delay: Duration::from_secs(10),
                    max_delay: Duration::from_secs(60),
                    recovery_actions: vec![RecoveryAction::ReleaseLeaseAndRestart],
                },
            },
        }
    }

    /// Attempt to recover failed container with comprehensive recovery (A4.4).
    pub async fn recover_container(
        &self,
        docker: &Docker,
        container: &mut RuntimeContainer,
        failure_type: FailureType,
    ) -> Result<(), RecoveryError> {
        if container.recovery_attempts >= self.max_recovery_attempts {
            return Err(RecoveryError::MaxAttemptsReached {
                container_id: container.container_id.clone(),
                attempts: container.recovery_attempts,
            });
        }

        container.transition_state(ContainerState::Recovering);
        container.recovery_attempts += 1;

        let strategy = self.get_recovery_strategy(&failure_type);
        let delay = self.calculate_backoff_delay(container.recovery_attempts, &strategy);

        info!(
            container_id = %container.container_id,
            failure_type = ?failure_type,
            attempt = container.recovery_attempts,
            delay_ms = delay.as_millis(),
            "Starting container recovery"
        );

        // Apply backoff delay
        sleep(delay).await;

        // Execute recovery actions in sequence
        for action in &strategy.recovery_actions {
            match self
                .execute_recovery_action(docker, container, action)
                .await
            {
                Ok(()) => {
                    info!(
                        container_id = %container.container_id,
                        action = ?action,
                        "Recovery action succeeded"
                    );
                    break; // Success - stop trying further actions
                }
                Err(e) => {
                    warn!(
                        container_id = %container.container_id,
                        action = ?action,
                        error = %e,
                        "Recovery action failed, trying next"
                    );
                    continue; // Try next recovery action
                }
            }
        }

        // Verify recovery success
        if self.verify_recovery_success(docker, container).await {
            container.transition_state(ContainerState::Ready);
            container.set_health(HealthStatus::Healthy);
            info!(
                container_id = %container.container_id,
                attempts = container.recovery_attempts,
                "Container recovery successful"
            );
            Ok(())
        } else {
            container.transition_state(ContainerState::Failed);
            Err(RecoveryError::RecoveryFailed {
                container_id: container.container_id.clone(),
                failure_type,
            })
        }
    }

    /// Get appropriate recovery strategy for failure type (A4.4)
    fn get_recovery_strategy(&self, failure_type: &FailureType) -> &RecoveryStrategy {
        match failure_type {
            FailureType::ContainerCrash => &self.recovery_strategies.container_crash,
            FailureType::BridgeCrash => &self.recovery_strategies.bridge_crash,
            FailureType::RpcTimeout => &self.recovery_strategies.rpc_timeout,
            FailureType::OutOfMemory => &self.recovery_strategies.oom_recovery,
            FailureType::Panic => &self.recovery_strategies.panic_recovery,
            FailureType::DockerRestart => &self.recovery_strategies.docker_restart,
        }
    }

    /// Calculate exponential backoff delay (A4.4)
    fn calculate_backoff_delay(&self, attempt: u32, strategy: &RecoveryStrategy) -> Duration {
        let delay_ms = (strategy.initial_delay.as_millis() as f64
            * strategy.backoff_multiplier.powi(attempt as i32 - 1)) as u64;

        Duration::from_millis(delay_ms.min(strategy.max_delay.as_millis() as u64))
    }

    /// Execute specific recovery action (A4.4)
    async fn execute_recovery_action(
        &self,
        docker: &Docker,
        container: &mut RuntimeContainer,
        action: &RecoveryAction,
    ) -> Result<(), RecoveryError> {
        match action {
            RecoveryAction::RestartContainer => {
                docker
                    .restart_container(&container.container_id, None)
                    .await
                    .map_err(|e| RecoveryError::ActionFailed {
                        action: format!("{:?}", action),
                        error: e.to_string(),
                    })?;
            }
            RecoveryAction::RecreateContainer => {
                // TODO: Implement container recreation with same config
                self.recreate_container_basic(docker, container).await?;
            }
            RecoveryAction::ForceRecreateContainer => {
                // Force kill and recreate
                let _ = docker
                    .kill_container::<String>(&container.container_id, None)
                    .await;
                let _ = docker
                    .remove_container(
                        &container.container_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
                self.recreate_container_basic(docker, container).await?;
            }
            RecoveryAction::ClearStateAndRestart => {
                // Clear any internal state and restart
                container.reuse_count = 0;
                container.heartbeat_failures = 0;
                docker
                    .restart_container(&container.container_id, None)
                    .await
                    .map_err(|e| RecoveryError::ActionFailed {
                        action: format!("{:?}", action),
                        error: e.to_string(),
                    })?;
            }
            RecoveryAction::ReleaseLeaseAndRestart => {
                // TODO: Implement lease release logic
                // For now, just restart
                docker
                    .restart_container(&container.container_id, None)
                    .await
                    .map_err(|e| RecoveryError::ActionFailed {
                        action: format!("{:?}", action),
                        error: e.to_string(),
                    })?;
            }
            RecoveryAction::NotifyRecoveryFailure => {
                // Notification only - always succeeds
                warn!(
                    container_id = %container.container_id,
                    "Recovery failure notified to monitoring system"
                );
            }
        }

        Ok(())
    }

    /// Basic container recreation (A4.4)
    async fn recreate_container_basic(
        &self,
        docker: &Docker,
        container: &mut RuntimeContainer,
    ) -> Result<(), RecoveryError> {
        // For now, just remove and let the system create a new one
        // TODO: Implement full recreation with preserved config
        let _ = docker
            .remove_container(
                &container.container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // Mark for recreation
        container.transition_state(ContainerState::Created);
        Ok(())
    }

    /// Verify recovery was successful (A4.4)
    async fn verify_recovery_success(&self, docker: &Docker, container: &RuntimeContainer) -> bool {
        match docker
            .inspect_container(&container.container_id, None::<InspectContainerOptions>)
            .await
        {
            Ok(info) => info.state.as_ref().and_then(|s| s.running).unwrap_or(false),
            Err(_) => false,
        }
    }
}

/// Types of failures that can trigger recovery (A4.4)
#[derive(Debug, Clone)]
pub enum FailureType {
    ContainerCrash,
    BridgeCrash,
    RpcTimeout,
    OutOfMemory,
    Panic,
    DockerRestart,
}

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error("max recovery attempts reached for container {container_id}: {attempts}")]
    MaxAttemptsReached { container_id: String, attempts: u32 },
    #[error("recovery failed for container {container_id}: {failure_type:?}")]
    RecoveryFailed {
        container_id: String,
        failure_type: FailureType,
    },
    #[error("recovery action failed: {action} - {error}")]
    ActionFailed { action: String, error: String },
}

/// Handle to checked-out runtime container.
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub invocation_id: String,
    pub container_id: String,
    pub workspace_path: String,
    pub resource_class: ResourceClass,
    pub priority: Priority,
}

/// Comprehensive runtime metrics (A4.7)
#[derive(Debug, Default)]
pub struct RuntimeMetrics {
    pub ready_containers: u64,
    pub idle_containers: u64,
    pub executing_containers: u64,
    pub failed_containers: u64,
    pub active_runtimes: u64,
    pub reuse_percentage: f64,
    pub scheduling: SchedulingMetrics,
    pub by_resource_class: HashMap<ResourceClass, ResourceClassMetrics>,
}

/// Per-resource-class metrics (A4.7)
#[derive(Debug, Default)]
pub struct ResourceClassMetrics {
    pub total_containers: u64,
    pub available_containers: u64,
    pub total_reuse_count: u64,
    pub average_execution_time_ms: f64,
}

/// Stress test results (A4.11)
#[derive(Debug, Default)]
pub struct StressTestResults {
    pub pool_exhaustion: TestResult,
    pub rapid_cancellation: TestResult,
    pub recovery_tests: TestResult,
    pub resource_pressure: TestResult,
    pub leak_detection: LeakAuditResult,
}

/// Individual test result (A4.11)
#[derive(Debug, Default)]
pub struct TestResult {
    pub passed: bool,
    pub details: String,
}

/// Leak audit results (A4.11)
#[derive(Debug, Default)]
pub struct LeakAuditResult {
    pub passed: bool,
    pub orphan_containers: usize,
    pub orphan_leases: usize,
    pub stale_containers: usize,
    pub total_containers: usize,
    pub total_active_runtimes: usize,
}

/// Production Runtime Manager - authoritative container lifecycle.
pub struct RuntimeManager {
    pub docker: Docker,
    pub config: OpenClawConfig,
    warm_config: WarmPoolConfig,
    /// All runtime containers by ID
    containers: Arc<RwLock<HashMap<String, RuntimeContainer>>>,
    /// Active runtime invocations
    active_runtimes: Arc<Mutex<HashMap<String, ActiveRuntime>>>,
    /// Runtime scheduler
    scheduler: Arc<Mutex<RuntimeScheduler>>,
    /// Health monitor
    health_monitor: Arc<HealthMonitor>,
    /// Recovery system
    recovery_system: Arc<RecoverySystem>,
    /// Concurrency limiter
    semaphore: Arc<Semaphore>,
    /// Generation counter for lifecycle tracking
    generation: AtomicU64,
    /// Shutdown signal
    shutdown: broadcast::Sender<()>,
    /// Background task handles. Wrapped in `Mutex<Option<...>>` (interior
    /// mutability) so `shutdown(&self)` — the signature every real caller
    /// across the codebase already depends on (`runtime.rs`, `pool.rs`,
    /// `providers.rs`, `runtime_status.rs`, live Docker tests) — can `.take()`
    /// and genuinely `.await` each handle with a bounded timeout, instead of
    /// requiring `&mut self` (which would be a breaking API change) or
    /// papering over the join with a fixed `sleep()` (which is not a real
    /// wait and was rejected during task-2 re-investigation).
    health_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    recycler_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Handle for the prewarming background task (A4.2). `start_prewarming_system`
    /// previously discarded its `JoinHandle` (`let _handle = ...`), so `shutdown()`
    /// had no way to wait for it. Stored here so shutdown can genuinely join it
    /// with a bounded timeout — see `shutdown()` doc for why this matters even
    /// though `RuntimeManagerSpawn::create_container` is currently a stub.
    prewarm_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("docker error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("container creation failed: {0}")]
    CreationFailed(String),
    #[error("max concurrent runtimes reached: {0}")]
    MaxConcurrent(usize),
    #[error("scheduling failed: {0}")]
    Scheduling(#[from] SchedulingError),
    #[error("recovery failed: {0}")]
    Recovery(#[from] RecoveryError),
    #[error("container not found: {container_id}")]
    ContainerNotFound { container_id: String },
    #[error("invalid state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: ContainerState,
        to: ContainerState,
    },
    #[error("runtime cancelled: {invocation_id}")]
    Cancelled { invocation_id: String },
    #[error("other error: {message}")]
    Other { message: String },
}

impl RuntimeManager {
    /// Create new production runtime manager.
    pub async fn new(
        config: OpenClawConfig,
        warm_config: WarmPoolConfig,
    ) -> Result<Self, RuntimeError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| RuntimeError::CreationFailed(format!("Docker connection: {}", e)))?;

        docker.ping().await?;

        let (shutdown, _) = broadcast::channel(1);
        let health_monitor = Arc::new(HealthMonitor::new(warm_config.health_check_interval));
        let recovery_system = Arc::new(RecoverySystem::new());
        let scheduler = Arc::new(Mutex::new(RuntimeScheduler::new()));
        let max_concurrent = config.max_concurrent_invocations;

        Ok(Self {
            docker,
            config,
            warm_config,
            containers: Arc::new(RwLock::new(HashMap::new())),
            active_runtimes: Arc::new(Mutex::new(HashMap::new())),
            scheduler,
            health_monitor,
            recovery_system,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            generation: AtomicU64::new(0),
            shutdown,
            health_task: Mutex::new(None),
            recycler_task: Mutex::new(None),
            prewarm_task: Mutex::new(None),
        })
    }

    /// Initialize runtime manager with production warm pool (A4.2).
    pub async fn initialize(&mut self) -> Result<(), RuntimeError> {
        info!("Initializing production runtime manager");

        // Reap any orphaned substrate containers left by a prior/crashed session so
        // they neither leak nor block new containers (zombie-container cleanup).
        self.reap_orphaned_containers().await;

        // Start background tasks
        self.start_health_monitoring().await;
        self.start_idle_recycling().await;
        self.start_prewarming_system().await;

        // Pre-warm containers for all resource classes
        for class in [
            ResourceClass::Light,
            ResourceClass::Medium,
            ResourceClass::Heavy,
        ] {
            self.ensure_minimum_warm_containers(class).await?;
            info!(
                resource_class = ?class,
                count = self.warm_config.minimum_containers,
                "Pre-warmed containers"
            );
        }

        Ok(())
    }

    /// Force-remove any Docker containers whose name starts with our substrate
    /// prefix but which this manager does not track (orphans from a prior/crashed
    /// KRIA session). Best-effort: failures are logged, never fatal.
    async fn reap_orphaned_containers(&self) {
        use bollard::container::ListContainersOptions;
        let mut filters = HashMap::new();
        // Match by name prefix; Docker name filter is a substring match.
        filters.insert("name".to_string(), vec![self.config.container_name.clone()]);
        let opts = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };
        let existing = match self.docker.list_containers(Some(opts)).await {
            Ok(list) => list,
            Err(e) => {
                warn!(error = %e, "reap: failed to list containers (skipping orphan cleanup)");
                return;
            }
        };
        let mut reaped = 0usize;
        for c in existing {
            let Some(id) = c.id else { continue };
            let _ = self
                .docker
                .remove_container(
                    &id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await;
            reaped += 1;
        }
        if reaped > 0 {
            info!(
                reaped,
                "reap: removed orphaned substrate containers from a prior session"
            );
        }
    }

    /// Ensure minimum warm containers for resource class (A4.2)
    async fn ensure_minimum_warm_containers(
        &self,
        resource_class: ResourceClass,
    ) -> Result<(), RuntimeError> {
        let current_count = self
            .count_containers_by_class_and_state(
                resource_class,
                &[ContainerState::Ready, ContainerState::Idle],
            )
            .await;

        let needed = self
            .warm_config
            .minimum_containers
            .saturating_sub(current_count);

        for _ in 0..needed {
            let container_id = self.create_container(resource_class).await?;
            info!(
                container_id = %container_id,
                resource_class = ?resource_class,
                "Created warm container"
            );
        }

        Ok(())
    }

    /// Count containers by class and states (A4.2)
    async fn count_containers_by_class_and_state(
        &self,
        resource_class: ResourceClass,
        states: &[ContainerState],
    ) -> usize {
        self.containers
            .read()
            .await
            .values()
            .filter(|c| c.resource_class == resource_class && states.contains(&c.state))
            .count()
    }

    /// Checkout container with comprehensive cancellation support (A4.10).
    pub async fn checkout_container(
        &self,
        resource_class: ResourceClass,
        skill_id: &str,
        priority: Priority,
    ) -> Result<ContainerHandle, RuntimeError> {
        // Acquire concurrency permit
        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| RuntimeError::MaxConcurrent(self.config.max_concurrent_invocations))?;

        let invocation_id = uuid::Uuid::new_v4().to_string();

        // A4.10: Cancellation can interrupt during prepare phase
        {
            let scheduler = self.scheduler.lock().await;
            if scheduler.is_cancelled(&invocation_id) {
                return Err(RuntimeError::Cancelled { invocation_id });
            }
        }

        // Schedule container through scheduler
        let container_id = {
            let containers = self.containers.read().await;
            let mut scheduler = self.scheduler.lock().await;

            // A4.10: Cancellation can interrupt during checkout phase
            scheduler.register_cancellation(invocation_id.clone(), CancellationPhase::Checkout);

            scheduler
                .schedule_container(&containers, resource_class, priority)
                .await
                .or_else(|_| -> Result<String, SchedulingError> {
                    // No warm container available - create cold
                    Ok("__create_new__".to_string())
                })?
        };

        let container_id = if container_id == "__create_new__" {
            // A4.10: Cancellation can interrupt during cold creation
            {
                let mut scheduler = self.scheduler.lock().await;
                if scheduler.is_cancelled(&invocation_id) {
                    scheduler.complete_cancellation(&invocation_id);
                    return Err(RuntimeError::Cancelled { invocation_id });
                }
            }
            self.create_container(resource_class).await?
        } else {
            container_id
        };

        // A4.10: Cancellation can interrupt during reservation
        {
            let mut scheduler = self.scheduler.lock().await;
            if scheduler.is_cancelled(&invocation_id) {
                scheduler.complete_cancellation(&invocation_id);
                return Err(RuntimeError::Cancelled { invocation_id });
            }
        }

        // Reserve container
        self.transition_container_state(&container_id, ContainerState::Reserved)
            .await?;

        // Create workspace
        let workspace_path = format!("/workspace/{}", invocation_id);

        // A4.10: Cancellation can interrupt during RPC
        {
            let mut scheduler = self.scheduler.lock().await;
            scheduler.register_cancellation(invocation_id.clone(), CancellationPhase::Rpc);
            if scheduler.is_cancelled(&invocation_id) {
                // Clean up reserved state
                self.transition_container_state(&container_id, ContainerState::Idle)
                    .await?;
                scheduler.complete_cancellation(&invocation_id);
                return Err(RuntimeError::Cancelled { invocation_id });
            }
        }

        self.exec_in_container(&container_id, &["mkdir", "-p", &workspace_path])
            .await?;

        // Track active runtime
        let active = ActiveRuntime {
            invocation_id: invocation_id.clone(),
            container_id: container_id.clone(),
            skill_id: skill_id.to_string(),
            workspace_path: "/tmp/workspace".to_string(), // TODO: Pass actual workspace path
            priority,
            started_at: Instant::now(),
            lease_duration: Duration::from_secs(300), // 5 minutes default
            _permit: permit,
        };

        self.active_runtimes
            .lock()
            .await
            .insert(invocation_id.clone(), active);

        // Transition to executing
        self.transition_container_state(&container_id, ContainerState::Executing)
            .await?;

        // Clear cancellation registration (execution started successfully)
        {
            let mut scheduler = self.scheduler.lock().await;
            scheduler.complete_cancellation(&invocation_id);
        }

        Ok(ContainerHandle {
            invocation_id,
            container_id,
            workspace_path,
            resource_class,
            priority,
        })
    }

    /// Cancel runtime execution (A4.10)
    pub async fn cancel_runtime(&self, invocation_id: &str) -> Result<(), RuntimeError> {
        // Request cancellation immediately so any in-flight checkout for this
        // invocation aborts at its next cancellation checkpoint.
        {
            let mut scheduler = self.scheduler.lock().await;
            scheduler.register_cancellation(invocation_id.to_string(), CancellationPhase::Cleanup);
            scheduler.request_cancellation(invocation_id);
        }

        // Find and clean up active runtime
        let active_runtime = self.active_runtimes.lock().await.remove(invocation_id);

        if let Some(runtime) = active_runtime {
            info!(
                invocation_id = %invocation_id,
                container_id = %runtime.container_id,
                "Cancelling runtime execution"
            );

            // A4.10: Every cancellation releases leases, containers, resources

            // Transition container to cooling for cleanup
            self.transition_container_state(&runtime.container_id, ContainerState::Cooling)
                .await?;

            // Clean workspace
            let _ = self
                .exec_in_container(
                    &runtime.container_id,
                    &["rm", "-rf", &runtime.workspace_path],
                )
                .await;

            // Transition to idle for reuse
            self.transition_container_state(&runtime.container_id, ContainerState::Idle)
                .await?;

            // Update container metrics
            {
                let mut containers = self.containers.write().await;
                if let Some(container) = containers.get_mut(&runtime.container_id) {
                    container.increment_reuse();
                }
            }

            // Complete cancellation tracking
            {
                let mut scheduler = self.scheduler.lock().await;
                scheduler.complete_cancellation(invocation_id);
            }

            info!(
                invocation_id = %invocation_id,
                "Runtime cancellation completed - all resources released"
            );

            Ok(())
        } else {
            // Not found - might already be cancelled or completed
            let mut scheduler = self.scheduler.lock().await;
            scheduler.complete_cancellation(invocation_id);

            Err(RuntimeError::ContainerNotFound {
                container_id: format!("invocation_{}", invocation_id),
            })
        }
    }

    /// Return container after execution - implements reuse-first policy.
    pub async fn checkin_container(&self, handle: ContainerHandle) -> Result<(), RuntimeError> {
        // Remove from active tracking
        self.active_runtimes
            .lock()
            .await
            .remove(&handle.invocation_id);

        // Transition through cooling to idle for reuse
        self.transition_container_state(&handle.container_id, ContainerState::Cooling)
            .await?;

        // Clean workspace
        let _ = self
            .exec_in_container(&handle.container_id, &["rm", "-rf", &handle.workspace_path])
            .await;

        // Check if should recycle or reuse
        let should_recycle = {
            let containers = self.containers.read().await;
            containers
                .get(&handle.container_id)
                .map(|c| c.should_recycle(self.warm_config.max_reuse_count))
                .unwrap_or(true)
        };

        if should_recycle {
            self.transition_container_state(&handle.container_id, ContainerState::Recycled)
                .await?;
            self.destroy_container(&handle.container_id).await?;

            // Create replacement
            tokio::spawn({
                let manager = self.clone_for_spawn();
                let resource_class = handle.resource_class;
                async move {
                    if let Err(e) = manager.create_container(resource_class).await {
                        warn!("Failed to create replacement container: {}", e);
                    }
                }
            });
        } else {
            // Transition to idle for reuse - first choice
            self.transition_container_state(&handle.container_id, ContainerState::Idle)
                .await?;

            {
                let mut containers = self.containers.write().await;
                if let Some(container) = containers.get_mut(&handle.container_id) {
                    container.increment_reuse();
                }
            }
        }

        Ok(())
    }

    async fn create_container(
        &self,
        resource_class: ResourceClass,
    ) -> Result<String, RuntimeError> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel);
        // Unique per-container suffix so names NEVER collide with containers left
        // over from a prior/crashed session (deterministic `-0`/`-1` names caused a
        // Docker 409 "name already in use" → pool init failure → "sometimes it
        // doesn't start"). A short uuid fragment keeps names readable + unique.
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let container_name = format!(
            "{}-{}-{}-{}",
            self.config.container_name,
            resource_class,
            generation,
            &unique[..8]
        );

        // Create container config
        let config = ContainerConfig {
            image: Some(self.config.image.clone()),
            hostname: Some(container_name.clone()),
            cmd: Some(vec!["/bin/sleep".to_string(), "infinity".to_string()]),
            working_dir: Some("/workspace".to_string()),
            // Add resource limits based on class
            // Add tmpfs mount for workspace
            // Add security constraints
            ..Default::default()
        };

        let container = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name,
                    ..Default::default()
                }),
                config,
            )
            .await?;

        let container_id = container.id;

        // Start container
        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await?;

        // Create runtime container with lifecycle tracking
        let mut runtime_container = RuntimeContainer::new(container_id.clone(), resource_class);
        runtime_container.transition_state(ContainerState::Preparing);

        // Wait for health check
        tokio::time::sleep(Duration::from_secs(2)).await;
        runtime_container.transition_state(ContainerState::Ready);

        self.containers
            .write()
            .await
            .insert(container_id.clone(), runtime_container);

        info!(
            container_id = %container_id,
            resource_class = ?resource_class,
            "Created container"
        );

        Ok(container_id)
    }

    /// Create + start a ONE-OFF bespoke container from a fully-materialized
    /// `ContainerConfig` (A3 materialization applied for real: the image,
    /// idle cmd, resource limits, security options, and any capability/skill
    /// bind mounts the caller baked into `config.host_config`). Returns a
    /// `ContainerHandle` the caller destroys after the single execution.
    ///
    /// This is the real backing for `ContainerPool::create_materialized`,
    /// which previously discarded the config and checked out a generic warm
    /// container — meaning materialized mounts (capability grants, and the
    /// bundle-execution skill mount) never actually reached a container. The
    /// unique `kria-openclaw`-prefixed name keeps it visible to leak
    /// detection + orphan reaping exactly like pooled containers.
    pub async fn create_bespoke_container(
        &self,
        mut config: ContainerConfig<String>,
        resource_class: ResourceClass,
    ) -> Result<ContainerHandle, RuntimeError> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel);
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let container_name = format!(
            "{}-{}-bespoke-{}-{}",
            self.config.container_name,
            resource_class,
            generation,
            &unique[..8]
        );

        // Ensure the image is set (materialize::build sets it, but be safe)
        // and the hostname is unique + short (Linux sethostname 64-byte cap).
        if config.image.is_none() {
            config.image = Some(self.config.image.clone());
        }
        config.hostname = Some(format!("oc-{}", &unique[..8]));

        let container = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name.clone(),
                    ..Default::default()
                }),
                config,
            )
            .await?;
        let container_id = container.id;

        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await?;

        let mut runtime_container = RuntimeContainer::new(container_id.clone(), resource_class);
        runtime_container.transition_state(ContainerState::Preparing);
        // Brief readiness wait (idle cmd is up immediately; the exec handshake
        // retries internally).
        tokio::time::sleep(Duration::from_millis(300)).await;
        runtime_container.transition_state(ContainerState::Ready);
        self.containers
            .write()
            .await
            .insert(container_id.clone(), runtime_container);

        info!(
            container_id = %container_id,
            resource_class = ?resource_class,
            "Created bespoke materialized container"
        );

        Ok(ContainerHandle {
            invocation_id: uuid::Uuid::new_v4().to_string(),
            container_id,
            workspace_path: "/workspace".to_string(),
            resource_class,
            priority: Priority::Interactive,
        })
    }

    /// Graceful shutdown (app exit): stop background tasks and destroy every
    /// tracked container so nothing leaks. Best-effort — errors are logged.
    ///
    /// Hardened (task-2 re-investigation): `shutdown()` previously fired the
    /// broadcast signal and immediately raced ahead to destroy/reap WITHOUT
    /// ever confirming the health/recycle/prewarm loops had actually stopped
    /// (their `JoinHandle`s were discarded or never joined). Harmless today
    /// only because the one background path capable of creating a container
    /// (`start_prewarming_system`) is wired to the stub
    /// `RuntimeManagerSpawn::create_container` — but that stub is explicitly
    /// marked `TODO: Implement full creation logic`, so the moment it is
    /// implemented this becomes a live, silent container leak on every
    /// shutdown that races a prewarm tick.
    ///
    /// `shutdown()` now GENUINELY joins every background task handle (via the
    /// `Mutex<Option<JoinHandle>>` fields, `.take()`n here) with a bounded
    /// per-task timeout, BEFORE the destroy/reap sweep — so no loop can still
    /// be mid-creation when the sweep runs. A stuck task is logged and
    /// shutdown proceeds regardless (never blocks forever / never deadlocks).
    /// Idempotent: a second call simply finds `None` in each slot (nothing to
    /// join) and still runs the destroy/reap sweep.
    pub async fn shutdown(&self) {
        // Signal background loops (health/recycle/prewarm) to stop.
        let _ = self.shutdown.send(());

        const BACKGROUND_TASK_JOIN_TIMEOUT: Duration = Duration::from_secs(5);
        join_background_task(
            "health_monitor",
            self.health_task.lock().await.take(),
            BACKGROUND_TASK_JOIN_TIMEOUT,
        )
        .await;
        join_background_task(
            "idle_recycling",
            self.recycler_task.lock().await.take(),
            BACKGROUND_TASK_JOIN_TIMEOUT,
        )
        .await;
        join_background_task(
            "prewarming",
            self.prewarm_task.lock().await.take(),
            BACKGROUND_TASK_JOIN_TIMEOUT,
        )
        .await;

        let ids: Vec<String> = self.containers.read().await.keys().cloned().collect();
        for id in &ids {
            if let Err(e) = self.destroy_container(id).await {
                warn!(container_id = %id, error = %e, "shutdown: failed to destroy container");
            }
        }
        // Sweep any stragglers (defensive) — runs AFTER every background task
        // has genuinely stopped (or been given up on after the timeout), so
        // any container a loop started before observing the shutdown signal
        // is caught here rather than leaking silently.
        self.reap_orphaned_containers().await;
        info!(
            destroyed = ids.len(),
            "runtime manager shutdown — containers released"
        );
    }

    /// Drain and re-warm the pool WITHOUT stopping background tasks (used by the
    /// UI "Restart Substrate" action). Destroys current containers, reaps orphans,
    /// then re-creates the warm minimum for each class.
    pub async fn rewarm(&self) -> Result<(), RuntimeError> {
        let ids: Vec<String> = self.containers.read().await.keys().cloned().collect();
        for id in &ids {
            let _ = self.destroy_container(id).await;
        }
        self.reap_orphaned_containers().await;
        for class in [
            ResourceClass::Light,
            ResourceClass::Medium,
            ResourceClass::Heavy,
        ] {
            self.ensure_minimum_warm_containers(class).await?;
        }
        info!("runtime manager re-warmed after restart");
        Ok(())
    }

    async fn transition_container_state(
        &self,
        container_id: &str,
        new_state: ContainerState,
    ) -> Result<(), RuntimeError> {
        let mut containers = self.containers.write().await;
        let container =
            containers
                .get_mut(container_id)
                .ok_or_else(|| RuntimeError::ContainerNotFound {
                    container_id: container_id.to_string(),
                })?;

        let old_state = container.state;
        if !container.transition_state(new_state) {
            return Err(RuntimeError::InvalidTransition {
                from: old_state,
                to: new_state,
            });
        }

        Ok(())
    }

    pub async fn destroy_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        // Transition to destroyed state
        self.transition_container_state(container_id, ContainerState::Destroyed)
            .await?;

        // Remove from Docker
        let _ = self
            .docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        // Remove from tracking
        self.containers.write().await.remove(container_id);

        info!(container_id = %container_id, "Destroyed container");

        Ok(())
    }

    async fn exec_in_container(
        &self,
        container_id: &str,
        cmd: &[&str],
    ) -> Result<(), RuntimeError> {
        use bollard::exec::{CreateExecOptions, StartExecOptions};

        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(cmd.to_vec()),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await?;

        self.docker
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await?;

        Ok(())
    }

    async fn start_health_monitoring(&mut self) {
        let health_monitor = self.health_monitor.clone();
        let docker = self.docker.clone();
        let containers = self.containers.clone();
        let shutdown = self.shutdown.subscribe();

        let handle = tokio::spawn(async move {
            health_monitor
                .start_monitoring(docker, containers, shutdown)
                .await;
        });

        *self.health_task.lock().await = Some(handle);
    }

    /// Bug found during task 2 re-investigation (regression
    /// `regr_r2_idle_recycling_overwrites_health_task_handle`): this fn wrote its
    /// handle into `self.health_task`, silently overwriting (not aborting —
    /// `JoinHandle` drop only detaches) the REAL health-monitor handle set by
    /// `start_health_monitoring`. Harmless only because nothing previously
    /// joined `health_task` on shutdown; now that `shutdown()` joins every
    /// background task handle, this MUST write to `recycler_task`.
    async fn start_idle_recycling(&mut self) {
        let containers = self.containers.clone();
        let warm_config = self.warm_config.clone();
        let shutdown = self.shutdown.subscribe();
        let manager = self.clone_for_spawn();

        let handle = tokio::spawn(async move {
            let mut shutdown = shutdown;
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // A4.6: Idle recycling - maintain warm pool quality
                        let recycling_candidates: Vec<String> = {
                            containers.read().await
                                .values()
                                .filter(|c| {
                                    // Recycle if stale, aging, fragmented, or unhealthy
                                    c.is_stale(warm_config.max_idle_duration)
                                        || c.is_aging(warm_config.aging_threshold)
                                        || c.is_fragmented(warm_config.fragmentation_threshold)
                                        || c.should_recycle(warm_config.max_reuse_count)
                                        || matches!(c.health, HealthStatus::Degraded | HealthStatus::Hung)
                                })
                                .map(|c| c.container_id.clone())
                                .collect()
                        };

                        for container_id in recycling_candidates {
                            // Check if recycling would breach minimum containers
                            let container_class = {
                                containers.read().await
                                    .get(&container_id)
                                    .map(|c| c.resource_class)
                            };

                            if let Some(class) = container_class {
                                let current_count = {
                                    containers.read().await
                                        .values()
                                        .filter(|c| {
                                            c.resource_class == class
                                                && !matches!(c.state, ContainerState::Destroyed | ContainerState::Recycled)
                                        })
                                        .count()
                                };

                                // Only recycle if we won't go below minimum
                                if current_count > warm_config.minimum_containers {
                                    if let Err(e) = manager.destroy_container(&container_id).await {
                                        warn!(
                                            container_id = %container_id,
                                            error = %e,
                                            "Failed to recycle container"
                                        );
                                    } else {
                                        info!(
                                            container_id = %container_id,
                                            resource_class = ?class,
                                            "Recycled aging/stale/fragmented container"
                                        );
                                    }
                                } else {
                                    info!(
                                        container_id = %container_id,
                                        resource_class = ?class,
                                        current_count = current_count,
                                        minimum = warm_config.minimum_containers,
                                        "Skipped recycling to maintain minimum containers"
                                    );
                                }
                            }
                        }
                    }
                    _ = shutdown.recv() => {
                        info!("Idle recycling system shutting down");
                        break;
                    }
                }
            }
        });

        *self.recycler_task.lock().await = Some(handle);
    }

    /// Trigger automatic recovery for failed container (A4.4)
    pub async fn trigger_recovery(
        &self,
        container_id: &str,
        failure_type: FailureType,
    ) -> Result<(), RuntimeError> {
        let mut containers = self.containers.write().await;
        let container =
            containers
                .get_mut(container_id)
                .ok_or_else(|| RuntimeError::ContainerNotFound {
                    container_id: container_id.to_string(),
                })?;

        info!(
            container_id = %container_id,
            failure_type = ?failure_type,
            "Triggering automatic recovery"
        );

        match self
            .recovery_system
            .recover_container(&self.docker, container, failure_type)
            .await
        {
            Ok(()) => {
                info!(
                    container_id = %container_id,
                    "Automatic recovery successful"
                );
                Ok(())
            }
            Err(recovery_error) => {
                warn!(
                    container_id = %container_id,
                    error = %recovery_error,
                    "Automatic recovery failed"
                );

                // If recovery fails completely, transition to destroyed and create replacement
                container.transition_state(ContainerState::Destroyed);
                drop(containers); // Release lock before async operations

                // Remove failed container
                let _ = self
                    .docker
                    .remove_container(
                        container_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;

                // Create replacement container
                let resource_class = {
                    let containers = self.containers.read().await;
                    containers
                        .get(container_id)
                        .map(|c| c.resource_class)
                        .unwrap_or(ResourceClass::Light)
                };

                // Remove failed container from tracking
                self.containers.write().await.remove(container_id);

                // Create replacement asynchronously
                let manager = self.clone_for_spawn();
                tokio::spawn(async move {
                    if let Err(e) = manager.create_container(resource_class).await {
                        warn!(
                            resource_class = ?resource_class,
                            error = %e,
                            "Failed to create replacement container after recovery failure"
                        );
                    } else {
                        info!(
                            resource_class = ?resource_class,
                            "Created replacement container after recovery failure"
                        );
                    }
                });

                Err(RuntimeError::Recovery(recovery_error))
            }
        }
    }

    async fn start_prewarming_system(&mut self) {
        let containers = self.containers.clone();
        let warm_config = self.warm_config.clone();
        let shutdown = self.shutdown.subscribe();
        let manager = self.clone_for_spawn();

        // Proactive background warm-create is deliberately disabled for leak-safety
        // (see `RuntimeManagerSpawn::create_container`): pool owners that drop
        // without awaiting `shutdown()` cannot reap background-created containers.
        // This is NOT a functional gap — `checkout_container` creates a container
        // on demand via the real `RuntimeManager::create_container` when no warm
        // one is available (verified). State it once at INFO so the previous
        // per-tick WARN spam ("Prewarming failed for container") no longer
        // masquerades as a production error.
        info!(
            "prewarming: proactive background warm-create is on-demand-only (containers are \
             created at checkout by the real runtime); background prewarm is idle until the \
             leak-safe background-create lifecycle lands"
        );

        let handle = tokio::spawn(async move {
            let mut shutdown = shutdown;
            let mut interval = tokio::time::interval(warm_config.prewarming_interval);
            // Log the deliberate-disabled outcome at most once (not every tick).
            let mut disabled_logged = false;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // A4.2: Continuous prewarming to maintain warm reserves.
                        // Check the shutdown signal BETWEEN each create so a
                        // tick that is mid-flight when shutdown fires stops
                        // promptly — otherwise a long tick can outrun
                        // shutdown()'s bounded task-join and create containers
                        // after the reap sweep (leak).
                        'tick: for class in [ResourceClass::Light, ResourceClass::Medium, ResourceClass::Heavy] {
                            let current_ready = {
                                containers.read().await
                                    .values()
                                    .filter(|c| {
                                        c.resource_class == class
                                            && matches!(c.state, ContainerState::Ready)
                                            && matches!(c.health, HealthStatus::Healthy)
                                    })
                                    .count()
                            };

                            let needed = warm_config.warm_reserve.saturating_sub(current_ready);

                            for _ in 0..needed {
                                // Abort the tick immediately if shutdown was signalled.
                                if !matches!(shutdown.try_recv(), Err(tokio::sync::broadcast::error::TryRecvError::Empty)) {
                                    break 'tick;
                                }
                                if let Err(e) = manager.create_container(class).await {
                                    // The deliberate leak-safety no-op (background
                                    // create disabled) is expected — log it once at
                                    // debug, never per-tick WARN. A genuinely
                                    // unexpected Docker error still WARNs.
                                    if e.to_string().contains("is not implemented against real Docker") {
                                        if !disabled_logged {
                                            debug!(
                                                resource_class = ?class,
                                                "prewarming: background create disabled (on-demand at checkout); suppressing further notices"
                                            );
                                            disabled_logged = true;
                                        }
                                    } else {
                                        warn!(
                                            resource_class = ?class,
                                            error = %e,
                                            "Prewarming failed for container"
                                        );
                                    }
                                    break 'tick; // Don't spam on failure; stop this whole tick.
                                } else {
                                    info!(
                                        resource_class = ?class,
                                        "Prewarmed container created"
                                    );
                                }
                            }
                        }
                    }
                    _ = shutdown.recv() => {
                        info!("Prewarming system shutting down");
                        break;
                    }
                }
            }
        });

        *self.prewarm_task.lock().await = Some(handle);
    }

    /// Get comprehensive runtime metrics (A4.7)
    pub async fn get_runtime_metrics(&self) -> RuntimeMetrics {
        let containers = self.containers.read().await;
        let active_runtimes = self.active_runtimes.lock().await;
        let scheduler = self.scheduler.lock().await;

        let mut metrics = RuntimeMetrics::default();

        // Container metrics by state
        for container in containers.values() {
            match container.state {
                ContainerState::Ready => metrics.ready_containers += 1,
                ContainerState::Idle => metrics.idle_containers += 1,
                ContainerState::Executing => metrics.executing_containers += 1,
                ContainerState::Failed => metrics.failed_containers += 1,
                _ => {}
            }

            // Resource class breakdown
            let class_metrics = metrics
                .by_resource_class
                .entry(container.resource_class)
                .or_insert_with(|| ResourceClassMetrics::default());
            class_metrics.total_containers += 1;
            class_metrics.total_reuse_count += container.reuse_count as u64;

            if container.is_eligible_for_reuse() {
                class_metrics.available_containers += 1;
            }
        }

        // Active runtime metrics
        metrics.active_runtimes = active_runtimes.len() as u64;

        // Scheduling metrics
        metrics.scheduling = scheduler.metrics.clone();

        // Calculate reuse percentage
        if scheduler.metrics.total_requests > 0 {
            metrics.reuse_percentage = (scheduler.metrics.warm_reuse_count as f64
                / scheduler.metrics.total_requests as f64)
                * 100.0;
        }

        metrics
    }

    /// Integrate with HRA (A4.8) - update resource pressure
    pub async fn update_resource_pressure(&self, pressure: f64) {
        let mut scheduler = self.scheduler.lock().await;
        scheduler.update_resource_pressure(pressure);

        info!(
            pressure = pressure,
            "Updated runtime manager resource pressure"
        );

        // A4.8: During high pressure, reduce warm pool and deny low priority
        if pressure > 0.8 {
            self.handle_resource_pressure(pressure).await;
        }
    }

    /// Handle resource pressure by reducing pool size (A4.8)
    async fn handle_resource_pressure(&self, pressure: f64) {
        info!(pressure = pressure, "Handling high resource pressure");

        // Identify containers that can be recycled to reduce pressure
        let recyclable_containers: Vec<String> = {
            let containers = self.containers.read().await;
            containers
                .values()
                .filter(|c| {
                    // Recycle idle containers with low priority during pressure
                    matches!(c.state, ContainerState::Idle)
                        && c.priority >= Priority::Background
                        && c.reuse_count > 10 // Prefer recycling heavily used containers
                })
                .map(|c| c.container_id.clone())
                .take(((pressure - 0.8) * 10.0) as usize) // Scale with pressure
                .collect()
        };

        for container_id in recyclable_containers {
            if let Ok(()) = self
                .transition_container_state(&container_id, ContainerState::Recycled)
                .await
            {
                let _ = self.destroy_container(&container_id).await;
                info!(
                    container_id = %container_id,
                    "Recycled container due to resource pressure"
                );
            }
        }
    }

    /// Run comprehensive stress tests (A4.11)
    pub async fn run_stress_tests(&self) -> StressTestResults {
        let mut results = StressTestResults::default();

        info!("Starting comprehensive OpenClaw stress tests");

        // Test 1: Pool exhaustion
        results.pool_exhaustion = self.test_pool_exhaustion().await;

        // Test 2: Rapid cancellation
        results.rapid_cancellation = self.test_rapid_cancellation().await;

        // Test 3: Container crash recovery
        results.recovery_tests = self.test_container_recovery().await;

        // Test 4: Resource pressure handling
        results.resource_pressure = self.test_resource_pressure().await;

        // Audit for leaks
        results.leak_detection = self.audit_for_leaks().await;

        info!("Completed OpenClaw stress tests");
        results
    }

    /// Test pool exhaustion scenario (A4.11)
    async fn test_pool_exhaustion(&self) -> TestResult {
        info!("Testing pool exhaustion scenario");

        let mut handles = Vec::new();
        let mut success_count = 0;
        let max_attempts = self.warm_config.maximum_containers + 5; // Try to exceed limit

        for i in 0..max_attempts {
            match self
                .checkout_container(ResourceClass::Light, "stress_test", Priority::Background)
                .await
            {
                Ok(handle) => {
                    handles.push(handle);
                    success_count += 1;
                }
                Err(RuntimeError::MaxConcurrent(_)) => {
                    info!(attempt = i, "Hit max concurrent limit as expected");
                    break;
                }
                Err(e) => {
                    warn!(attempt = i, error = %e, "Unexpected error during pool exhaustion test");
                    break;
                }
            }
        }

        // Clean up
        for handle in handles {
            let _ = self.checkin_container(handle).await;
        }

        TestResult {
            passed: success_count > 0 && success_count <= self.warm_config.maximum_containers,
            details: format!(
                "Successfully created {} containers before limit",
                success_count
            ),
        }
    }

    /// Test rapid cancellation (A4.11)
    async fn test_rapid_cancellation(&self) -> TestResult {
        info!("Testing rapid cancellation scenario");

        let mut cancelled_count = 0;
        let total_attempts = 10;

        for i in 0..total_attempts {
            let invocation_id = format!("stress_cancel_{}", i);

            // Register cancellation immediately
            {
                let mut scheduler = self.scheduler.lock().await;
                scheduler.register_cancellation(invocation_id.clone(), CancellationPhase::Prepare);
            }

            // Try to checkout - should be cancelled
            match self
                .checkout_container(ResourceClass::Light, "stress_test", Priority::Background)
                .await
            {
                Err(RuntimeError::Cancelled { .. }) => {
                    cancelled_count += 1;
                }
                Ok(handle) => {
                    // Unexpected success - clean up
                    let _ = self.checkin_container(handle).await;
                }
                Err(_) => {
                    // Other error - not what we're testing
                }
            }
        }

        TestResult {
            passed: cancelled_count >= total_attempts / 2, // At least 50% should be cancelled
            details: format!(
                "Successfully cancelled {}/{} operations",
                cancelled_count, total_attempts
            ),
        }
    }

    /// Test container recovery (A4.11)
    async fn test_container_recovery(&self) -> TestResult {
        info!("Testing container recovery scenario");

        // Create a container and simulate failure
        match self.create_container(ResourceClass::Light).await {
            Ok(container_id) => {
                // Simulate container failure
                if let Ok(()) = self
                    .trigger_recovery(&container_id, FailureType::ContainerCrash)
                    .await
                {
                    TestResult {
                        passed: true,
                        details: "Container recovery successful".to_string(),
                    }
                } else {
                    TestResult {
                        passed: false,
                        details: "Container recovery failed".to_string(),
                    }
                }
            }
            Err(e) => TestResult {
                passed: false,
                details: format!("Failed to create test container: {}", e),
            },
        }
    }

    /// Test resource pressure handling (A4.11)
    async fn test_resource_pressure(&self) -> TestResult {
        info!("Testing resource pressure handling");

        // Simulate high pressure
        self.update_resource_pressure(0.9).await;

        // Try to create low priority work - should be rejected
        match self
            .checkout_container(ResourceClass::Light, "stress_test", Priority::Low)
            .await
        {
            Err(RuntimeError::Scheduling(SchedulingError::ResourcePressure { .. })) => {
                // Reset pressure
                self.update_resource_pressure(0.1).await;

                TestResult {
                    passed: true,
                    details: "Resource pressure correctly rejected low priority work".to_string(),
                }
            }
            Ok(handle) => {
                let _ = self.checkin_container(handle).await;
                self.update_resource_pressure(0.1).await;

                TestResult {
                    passed: false,
                    details: "Resource pressure did not reject low priority work".to_string(),
                }
            }
            Err(e) => {
                self.update_resource_pressure(0.1).await;

                TestResult {
                    passed: false,
                    details: format!("Unexpected error: {}", e),
                }
            }
        }
    }

    /// Audit for resource leaks (A4.11)
    async fn audit_for_leaks(&self) -> LeakAuditResult {
        info!("Auditing for resource leaks");

        let containers = self.containers.read().await;
        let active_runtimes = self.active_runtimes.lock().await;

        let mut audit = LeakAuditResult::default();

        // Check for orphaned containers
        for container in containers.values() {
            if matches!(
                container.state,
                ContainerState::Destroyed | ContainerState::Recycled
            ) {
                audit.orphan_containers += 1;
            }
        }

        // Check for orphaned leases (active runtimes without containers)
        for runtime in active_runtimes.values() {
            if !containers.contains_key(&runtime.container_id) {
                audit.orphan_leases += 1;
            }
        }

        // Check for stale containers (created long ago but never used)
        let stale_threshold = Duration::from_secs(3600); // 1 hour
        for container in containers.values() {
            if container.created_at.elapsed() > stale_threshold && container.reuse_count == 0 {
                audit.stale_containers += 1;
            }
        }

        audit.total_containers = containers.len();
        audit.total_active_runtimes = active_runtimes.len();
        audit.passed = audit.orphan_containers == 0 && audit.orphan_leases == 0;

        if audit.passed {
            info!("Leak audit passed - no resource leaks detected");
        } else {
            warn!(
                orphan_containers = audit.orphan_containers,
                orphan_leases = audit.orphan_leases,
                stale_containers = audit.stale_containers,
                "Leak audit failed - resource leaks detected"
            );
        }

        audit
    }

    fn clone_for_spawn(&self) -> RuntimeManagerSpawn {
        RuntimeManagerSpawn {
            docker: self.docker.clone(),
            config: self.config.clone(),
            containers: self.containers.clone(),
        }
    }
}

// Helper struct for spawning async tasks
#[derive(Clone)]
struct RuntimeManagerSpawn {
    docker: Docker,
    #[allow(dead_code)] // reserved for a future leak-safe background create_container
    config: OpenClawConfig,
    containers: Arc<RwLock<HashMap<String, RuntimeContainer>>>,
}

impl RuntimeManagerSpawn {
    /// Background-task container creation (prewarming loop, `checkin_container`
    /// recycle-replacement, `trigger_recovery` replacement).
    ///
    /// NOT implemented against real Docker: returns an honest error rather than
    /// creating a container. This was briefly implemented to make the warm pool
    /// prewarm continuously, but `ContainerPool::new` eagerly starts this
    /// prewarm loop for EVERY pool, and any owner that drops a pool without
    /// awaiting `shutdown()` (common in tests and short-lived callers) cannot
    /// async-reap the background-created containers on `Drop` → real container
    /// leaks (confirmed: the eval leak-baseline suite regressed). Enabling
    /// background creation therefore requires a deterministic
    /// "stop-loop-then-reap" lifecycle guarantee at every pool owner, which is
    /// a separate, deliberate change. Until then this stays an honest error
    /// (never a fabricated container id, R15 honesty invariant); the
    /// boot-time warm pool is still created by `RuntimeManager::create_container`
    /// (the real impl) during `initialize()`.
    async fn create_container(
        &self,
        resource_class: ResourceClass,
    ) -> Result<String, RuntimeError> {
        Err(RuntimeError::Other {
            message: format!(
                "RuntimeManagerSpawn::create_container is not implemented against real Docker \
                 (resource_class={resource_class:?}); refusing to fabricate a container id"
            ),
        })
    }

    async fn destroy_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        // Remove from tracking
        self.containers.write().await.remove(container_id);

        // Remove from Docker
        let _ = self
            .docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        Ok(())
    }
}

/// Genuinely await a background task handle with a bounded timeout, logging
/// (never panicking/blocking forever) if it does not stop in time. Used by
/// `RuntimeManager::shutdown()` to guarantee no background loop can still be
/// mid-container-creation when the destroy/reap sweep runs.
async fn join_background_task(
    name: &str,
    handle: Option<tokio::task::JoinHandle<()>>,
    timeout: Duration,
) {
    let Some(handle) = handle else {
        return;
    };
    match tokio::time::timeout(timeout, handle).await {
        Ok(Ok(())) => {
            info!(task = name, "background task joined cleanly on shutdown");
        }
        Ok(Err(join_error)) => {
            warn!(task = name, error = %join_error, "background task panicked/was cancelled during shutdown");
        }
        Err(_timed_out) => {
            warn!(
                task = name,
                timeout_secs = timeout.as_secs(),
                "background task did not stop within the shutdown timeout — proceeding with reap sweep anyway"
            );
        }
    }
}

/// Replace old ContainerPool with RuntimeManager.
pub type ContainerPool = RuntimeManager;

#[cfg(test)]
mod spawn_prewarm_tests {
    use super::*;

    /// `RuntimeManagerSpawn::create_container` is deliberately an honest error
    /// (NOT a fabricated container id), because `ContainerPool::new` starts the
    /// prewarm loop for every pool and pool owners that drop without awaiting
    /// `shutdown()` cannot async-reap background-created containers on `Drop`
    /// (real leak — the eval leak-baseline suite regressed when this created
    /// real containers). A leak-safe background create needs a deterministic
    /// stop-loop-then-reap guarantee at every pool owner first. This asserts
    /// the honest-error contract so a future re-implementation is a conscious
    /// change with the leak-safety work done.
    #[tokio::test]
    async fn spawn_create_container_is_honest_error_until_leak_safe() {
        // No Docker needed — the method returns without touching Docker.
        let manager =
            match RuntimeManager::new(OpenClawConfig::default(), WarmPoolConfig::default()).await {
                Ok(m) => m,
                Err(_) => {
                    eprintln!("[SKIP] Docker not available for RuntimeManager::new");
                    return;
                }
            };
        let spawn = manager.clone_for_spawn();
        let result = spawn.create_container(ResourceClass::Light).await;
        assert!(
            result.is_err(),
            "spawn create_container must return an honest error (no fabricated id, no leaky real create)"
        );
    }
}
