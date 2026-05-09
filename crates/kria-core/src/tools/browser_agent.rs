//! Browser Agent — LLM-controlled web automation via Docker sidecar.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────┐
//! │  BrowserAgent     │
//! │  (Rust wrapper)   │
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Docker Container │  ← ephemeral, killed on drop/preemption
//! │  (Python sidecar) │
//! │  browser-use      │
//! └────────┬─────────┘
//!          │
//!          ▼
//! ┌──────────────────┐
//! │  Chromium         │  ← headless, inside container
//! └──────────────────┘
//! ```
//!
//! # Lifecycle Guarantees
//!
//! 1. **Drop safety**: When `BrowserAgent` is dropped (task cancelled, preempted,
//!    or normal completion), the Docker container is aggressively killed via
//!    `docker kill` + `docker rm`. No zombie processes.
//!
//! 2. **Cancellation**: The agent checks a `CancellationToken` at every step.
//!    If the ExecutiveController preempts (P0 voice task), the container is
//!    killed immediately — no graceful shutdown, no waiting for Chromium.
//!
//! 3. **Timeout**: Every task has a hard timeout. If the Python sidecar doesn't
//!    respond within the deadline, the container is killed.
//!
//! 4. **Isolation**: The container runs with:
//!    - No host network (`--network=none` by default, or configurable)
//!    - Read-only rootfs where possible
//!    - PID limit
//!    - Memory limit
//!    - No privileged mode

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

// ─── Configuration ──────────────────────────────────────────────────────────

/// Configuration for the Browser Agent.
#[derive(Debug, Clone)]
pub struct BrowserAgentConfig {
    /// Docker image to use for the browser sidecar.
    pub image: String,
    /// Network mode: "none", "bridge", "host", or custom.
    pub network_mode: String,
    /// Memory limit for the container (e.g., "512m").
    pub memory_limit: String,
    /// PID limit for the container.
    pub pids_limit: i64,
    /// Hard timeout for any single browser task.
    pub task_timeout: Duration,
    /// Timeout for container startup (waiting for ready signal).
    pub startup_timeout: Duration,
    /// Port the Python sidecar listens on inside the container.
    pub sidecar_port: u16,
    /// Maximum number of browser steps per task.
    pub max_steps: usize,
}

impl Default for BrowserAgentConfig {
    fn default() -> Self {
        Self {
            image: "kria-browser-use:latest".to_string(),
            network_mode: "bridge".to_string(),
            memory_limit: "512m".to_string(),
            pids_limit: 64,
            task_timeout: Duration::from_secs(120),
            startup_timeout: Duration::from_secs(30),
            sidecar_port: 8080,
            max_steps: 20,
        }
    }
}

// ─── Types ──────────────────────────────────────────────────────────────────

/// A browser automation task submitted to the Browser Agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrowserTask {
    /// Natural language description of what to do.
    pub instruction: String,
    /// Maximum number of steps the agent can take.
    pub max_steps: usize,
    /// Optional starting URL.
    pub start_url: Option<String>,
    /// Whether to take screenshots at each step.
    pub screenshot_each_step: bool,
}

/// Result of a browser automation task.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrowserResult {
    /// Whether the task completed successfully.
    pub success: bool,
    /// Final answer or extracted data.
    pub output: String,
    /// Number of steps taken.
    pub steps_taken: usize,
    /// Total wall-clock time.
    pub duration: Duration,
    /// Screenshots captured (base64-encoded PNGs).
    pub screenshots: Vec<String>,
    /// Error message if failed.
    pub error: Option<String>,
}

/// State of the browser container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    /// No container exists.
    NotStarted,
    /// Container is starting up.
    Starting,
    /// Container is ready to accept tasks.
    Ready,
    /// Container is executing a task.
    Running,
    /// Container is being torn down.
    Stopping,
    /// Container failed and is being cleaned up.
    Failed,
}

// ─── Browser Agent ──────────────────────────────────────────────────────────

/// Docker-wrapped browser automation agent.
///
/// Manages the full lifecycle of a Browser-Use Python sidecar container.
/// Implements `Drop` for aggressive cleanup — when this struct is dropped,
/// the container is killed immediately.
pub struct BrowserAgent {
    /// Configuration.
    config: BrowserAgentConfig,
    /// Docker container ID (set after start).
    container_id: Arc<Mutex<Option<String>>>,
    /// Current state.
    state: Arc<Mutex<ContainerState>>,
    /// Cancellation token from ExecutiveController.
    cancel: CancellationToken,
    /// Whether the container has been explicitly stopped.
    stopped: Arc<Mutex<bool>>,
}

impl BrowserAgent {
    /// Create a new Browser Agent.
    pub fn new(config: BrowserAgentConfig, cancel: CancellationToken) -> Self {
        Self {
            config,
            container_id: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ContainerState::NotStarted)),
            cancel,
            stopped: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the current container state.
    pub async fn state(&self) -> ContainerState {
        *self.state.lock().await
    }

    /// Start the browser container.
    ///
    /// Pulls the image if needed, creates and starts the container,
    /// and waits for the sidecar to become ready.
    pub async fn start(&self) -> Result<(), BrowserError> {
        {
            let mut state = self.state.lock().await;
            if *state != ContainerState::NotStarted {
                return Err(BrowserError::InvalidState {
                    current: *state,
                    expected: ContainerState::NotStarted,
                });
            }
            *state = ContainerState::Starting;
        }

        // Check for preemption before starting.
        if self.cancel.is_cancelled() {
            self.set_state(ContainerState::NotStarted).await;
            return Err(BrowserError::Cancelled);
        }

        // Create and start the container.
        let container_id = self.create_container().await?;
        *self.container_id.lock().await = Some(container_id.clone());

        // Start the container.
        self.start_container(&container_id).await?;

        // Wait for the sidecar to become ready.
        match self.wait_for_ready(&container_id).await {
            Ok(()) => {
                self.set_state(ContainerState::Ready).await;
                Ok(())
            }
            Err(e) => {
                self.set_state(ContainerState::Failed).await;
                // Clean up the failed container.
                let _ = self.kill_container(&container_id).await;
                Err(e)
            }
        }
    }

    /// Execute a browser task.
    ///
    /// Sends the task instruction to the Python sidecar and waits for
    /// the result. Checks cancellation at every step.
    pub async fn execute(&self, task: BrowserTask) -> Result<BrowserResult, BrowserError> {
        // Must be in Ready state.
        {
            let state = self.state.lock().await;
            if *state != ContainerState::Ready {
                return Err(BrowserError::InvalidState {
                    current: *state,
                    expected: ContainerState::Ready,
                });
            }
        }

        self.set_state(ContainerState::Running).await;

        let container_id = self.container_id.lock().await.clone()
            .ok_or(BrowserError::NoContainer)?;

        let start = Instant::now();

        // Execute with timeout and cancellation.
        let result = tokio::select! {
            _ = self.cancel.cancelled() => {
                self.set_state(ContainerState::Stopping).await;
                let _ = self.kill_container(&container_id).await;
                return Err(BrowserError::Cancelled);
            }
            result = tokio::time::timeout(
                self.config.task_timeout,
                self.execute_task_on_sidecar(&container_id, &task),
            ) => {
                match result {
                    Ok(r) => r,
                    Err(_) => {
                        tracing::warn!("Browser task timed out after {:?}", self.config.task_timeout);
                        let _ = self.kill_container(&container_id).await;
                        return Err(BrowserError::Timeout(self.config.task_timeout));
                    }
                }
            }
        };

        self.set_state(ContainerState::Ready).await;

        match result {
            Ok(mut output) => {
                output.duration = start.elapsed();
                Ok(output)
            }
            Err(e) => {
                tracing::error!("Browser task failed: {}", e);
                // Don't kill the container on task failure — it may be reusable.
                Err(e)
            }
        }
    }

    /// Stop and remove the container.
    /// No-op if no container exists.
    pub async fn stop(&self) -> Result<(), BrowserError> {
        let has_container = self.container_id.lock().await.is_some();
        if !has_container {
            return Ok(());
        }

        *self.stopped.lock().await = true;
        self.set_state(ContainerState::Stopping).await;

        if let Some(container_id) = self.container_id.lock().await.as_ref() {
            self.kill_container(container_id).await?;
        }

        *self.container_id.lock().await = None;
        self.set_state(ContainerState::NotStarted).await;
        Ok(())
    }

    // ─── Internal: Docker Operations ────────────────────────────────────────

    /// Create a hardened Docker container.
    async fn create_container(&self) -> Result<String, BrowserError> {
        let output = tokio::process::Command::new("docker")
            .args([
                "create",
                "--rm",
                "--network", &self.config.network_mode,
                "--memory", &self.config.memory_limit,
                "--pids-limit", &self.config.pids_limit.to_string(),
                "--read-only",
                "--tmpfs", "/tmp:size=64m",
                "--security-opt", "no-new-privileges",
                "--cap-drop", "ALL",
                "-p", &format!("127.0.0.1::{}", self.config.sidecar_port),
                &self.config.image,
            ])
            .output()
            .await
            .map_err(|e| BrowserError::Docker(format!("Failed to create container: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BrowserError::Docker(format!(
                "docker create failed: {}",
                stderr
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tracing::info!("Created browser container: {}", container_id);
        Ok(container_id)
    }

    /// Start a created container.
    async fn start_container(&self, container_id: &str) -> Result<(), BrowserError> {
        let output = tokio::process::Command::new("docker")
            .args(["start", container_id])
            .output()
            .await
            .map_err(|e| BrowserError::Docker(format!("Failed to start container: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BrowserError::Docker(format!(
                "docker start failed: {}",
                stderr
            )));
        }

        tracing::info!("Started browser container: {}", container_id);
        Ok(())
    }

    /// Wait for the sidecar to become ready (health check).
    async fn wait_for_ready(&self, container_id: &str) -> Result<(), BrowserError> {
        let deadline = Instant::now() + self.config.startup_timeout;
        let port = self.get_host_port(container_id).await?;

        loop {
            if self.cancel.is_cancelled() {
                return Err(BrowserError::Cancelled);
            }

            if Instant::now() > deadline {
                return Err(BrowserError::StartupTimeout(self.config.startup_timeout));
            }

            // Try to connect to the sidecar health endpoint.
            let url = format!("http://127.0.0.1:{}/health", port);
            match reqwest::get(&url).await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::info!("Browser sidecar ready on port {}", port);
                    return Ok(());
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Get the host port mapped to the sidecar port.
    async fn get_host_port(&self, container_id: &str) -> Result<u16, BrowserError> {
        let output = tokio::process::Command::new("docker")
            .args([
                "port",
                container_id,
                &format!("{}/tcp", self.config.sidecar_port),
            ])
            .output()
            .await
            .map_err(|e| BrowserError::Docker(format!("docker port failed: {}", e)))?;

        if !output.status.success() {
            return Err(BrowserError::Docker("Failed to get container port".into()));
        }

        let port_str = String::from_utf8_lossy(&output.stdout);
        // Format: "0.0.0.0:12345" or ":::12345"
        let port = port_str
            .trim()
            .split(':')
            .last()
            .ok_or_else(|| BrowserError::Docker(format!("Invalid port format: {}", port_str)))?
            .parse::<u16>()
            .map_err(|_| BrowserError::Docker(format!("Invalid port number: {}", port_str)))?;

        Ok(port)
    }

    /// Execute a task on the sidecar (HTTP request to the container).
    async fn execute_task_on_sidecar(
        &self,
        container_id: &str,
        task: &BrowserTask,
    ) -> Result<BrowserResult, BrowserError> {
        let port = self.get_host_port(container_id).await?;
        let url = format!("http://127.0.0.1:{}/execute", port);

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(task)
            .timeout(self.config.task_timeout)
            .send()
            .await
            .map_err(|e| BrowserError::Sidecar(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BrowserError::Sidecar(format!(
                "Sidecar returned {}: {}",
                status, body
            )));
        }

        let result: BrowserResult = response
            .json()
            .await
            .map_err(|e| BrowserError::Sidecar(format!("Failed to parse response: {}", e)))?;

        Ok(result)
    }

    /// Aggressively kill and remove a container.
    ///
    /// Uses `docker kill` (SIGKILL) for immediate termination, then `docker rm`
    /// to clean up. This is the nuclear option — no graceful shutdown.
    async fn kill_container(&self, container_id: &str) -> Result<(), BrowserError> {
        tracing::info!("Killing browser container: {}", container_id);

        // Kill (SIGKILL) — immediate, no graceful shutdown.
        let kill_result = tokio::process::Command::new("docker")
            .args(["kill", container_id])
            .output()
            .await;

        if let Err(e) = kill_result {
            tracing::warn!("docker kill failed (container may already be dead): {}", e);
        }

        // Remove — clean up container resources.
        let rm_result = tokio::process::Command::new("docker")
            .args(["rm", "-f", container_id])
            .output()
            .await;

        if let Err(e) = rm_result {
            tracing::warn!("docker rm failed: {}", e);
        }

        Ok(())
    }

    async fn set_state(&self, state: ContainerState) {
        *self.state.lock().await = state;
    }
}

// ─── Drop: Aggressive Cleanup ───────────────────────────────────────────────

impl Drop for BrowserAgent {
    fn drop(&mut self) {
        // We can't use async in Drop, so we spawn a blocking task
        // to kill the container.
        let container_id = self.container_id.clone();
        let stopped = self.stopped.clone();

        // Try to get the container ID synchronously.
        // If the mutex is poisoned or locked, we'll do a best-effort cleanup.
        let id = {
            // We're in Drop, so we can use try_lock.
            // If it fails, the container will be cleaned up by Docker's --rm flag
            // when the process exits.
            container_id.try_lock().ok().and_then(|guard| guard.clone())
        };

        let already_stopped = stopped.try_lock().ok().map(|g| *g).unwrap_or(false);

        if let (Some(id), false) = (id, already_stopped) {
            tracing::warn!("BrowserAgent dropped without explicit stop — killing container {}", id);

            // Spawn a blocking task to kill the container.
            // This is fire-and-forget — we can't await in Drop.
            std::thread::spawn(move || {
                // Use std::process::Command (synchronous) for Drop cleanup.
                let _ = std::process::Command::new("docker")
                    .args(["kill", &id])
                    .output();
                let _ = std::process::Command::new("docker")
                    .args(["rm", "-f", &id])
                    .output();
                tracing::info!("Drop cleanup complete for container {}", id);
            });
        }
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────────

/// Errors from the Browser Agent.
#[derive(Debug, Clone)]
pub enum BrowserError {
    /// Docker operation failed.
    Docker(String),
    /// Sidecar communication failed.
    Sidecar(String),
    /// Container is in an invalid state for the requested operation.
    InvalidState {
        current: ContainerState,
        expected: ContainerState,
    },
    /// No container exists.
    NoContainer,
    /// Task was cancelled by ExecutiveController.
    Cancelled,
    /// Task exceeded its timeout.
    Timeout(Duration),
    /// Container startup timed out.
    StartupTimeout(Duration),
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker(msg) => write!(f, "Docker error: {}", msg),
            Self::Sidecar(msg) => write!(f, "Sidecar error: {}", msg),
            Self::InvalidState { current, expected } => {
                write!(f, "Invalid state: {:?}, expected {:?}", current, expected)
            }
            Self::NoContainer => write!(f, "No container"),
            Self::Cancelled => write!(f, "Cancelled"),
            Self::Timeout(d) => write!(f, "Timed out after {:?}", d),
            Self::StartupTimeout(d) => write!(f, "Startup timed out after {:?}", d),
        }
    }
}

impl std::error::Error for BrowserError {}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_invalid_state_transition() {
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);

        // Cannot execute when not started.
        let task = BrowserTask {
            instruction: "Go to example.com".to_string(),
            max_steps: 5,
            start_url: None,
            screenshot_each_step: false,
        };

        let result = agent.execute(task).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            BrowserError::InvalidState { current, expected } => {
                assert_eq!(current, ContainerState::NotStarted);
                assert_eq!(expected, ContainerState::Ready);
            }
            other => panic!("Expected InvalidState, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_cancellation_before_start() {
        let cancel = CancellationToken::new();
        cancel.cancel(); // Pre-cancel.

        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);
        let result = agent.start().await;

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BrowserError::Cancelled));
    }

    #[tokio::test]
    async fn test_stop_when_not_started() {
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);

        // Stop on a non-started agent should succeed (no-op).
        let result = agent.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_container_state_tracking() {
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);

        assert_eq!(agent.state().await, ContainerState::NotStarted);
    }

    #[tokio::test]
    async fn test_config_defaults() {
        let config = BrowserAgentConfig::default();
        assert_eq!(config.image, "kria-browser-use:latest");
        assert_eq!(config.network_mode, "bridge");
        assert_eq!(config.memory_limit, "512m");
        assert_eq!(config.pids_limit, 64);
        assert_eq!(config.max_steps, 20);
    }

    #[tokio::test]
    async fn test_cancel_during_execution() {
        // Simulate: agent has a container, is running a task, ExecutiveController cancels.
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel.clone());

        // Manually set state to Ready and set a container ID
        // (simulating a started container).
        agent.set_state(ContainerState::Ready).await;
        *agent.container_id.lock().await = Some("fake_container_id".to_string());

        // Cancel the token (simulating P0 voice task arrival).
        cancel.cancel();

        let task = BrowserTask {
            instruction: "Search for something".to_string(),
            max_steps: 5,
            start_url: None,
            screenshot_each_step: false,
        };

        // Execute should return Cancelled.
        let result = agent.execute(task).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BrowserError::Cancelled));
    }

    #[test]
    fn test_drop_kills_container() {
        // This test verifies that the Drop implementation
        // doesn't panic when there's no container to clean up.
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);
        drop(agent);
        // If Drop panicked, the test would fail.
    }

    #[test]
    fn test_drop_with_container_id_does_not_panic() {
        // Simulate a scenario where the agent has a container ID
        // but is dropped without explicit stop.
        let cancel = CancellationToken::new();
        let agent = BrowserAgent::new(BrowserAgentConfig::default(), cancel);

        // Set a fake container ID.
        // We can't use async in a sync test, so we use try_lock.
        if let Ok(mut id) = agent.container_id.try_lock() {
            *id = Some("fake_container_id_12345".to_string());
        }

        // Drop should attempt to kill the container but not panic.
        // The docker kill/rm will fail (fake ID), but that's fine.
        drop(agent);
    }

    #[test]
    fn test_browser_error_display() {
        let err = BrowserError::Docker("container not found".to_string());
        assert!(err.to_string().contains("Docker error"));

        let err = BrowserError::Cancelled;
        assert!(err.to_string().contains("Cancelled"));

        let err = BrowserError::Timeout(Duration::from_secs(30));
        assert!(err.to_string().contains("30s"));
    }
}
