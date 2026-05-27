//! RFC 008: Service Orchestrator — spawns and supervises the Python vision
//! sidecar and the uinput daemon, with Drop-based cleanup.

use anyhow::{anyhow, Context, Result};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Liveness of a single managed service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceLiveness {
    /// Not yet started or stopped intentionally
    Stopped,
    /// Process spawned but health check has not yet passed
    Starting,
    /// Healthy and serving requests
    Running,
    /// Process crashed or health check failing
    Failed,
}

impl ServiceLiveness {
    pub fn is_healthy(self) -> bool {
        matches!(self, ServiceLiveness::Running)
    }
}

/// Combined status snapshot for both managed services.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceStatus {
    pub vision_sidecar: ServiceLiveness,
    pub uinput_daemon: ServiceLiveness,
    pub automation_enabled: bool,
    /// PID of vision sidecar process, if running.
    pub vision_pid: Option<u32>,
    /// PID of uinput daemon process, if running.
    pub uinput_pid: Option<u32>,
}

impl ServiceStatus {
    pub fn all_healthy(&self) -> bool {
        self.vision_sidecar.is_healthy() && self.uinput_daemon.is_healthy()
    }
}

/// Configuration for the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Workspace root (where `sidecars/` and `target/` live).
    pub workspace_root: PathBuf,
    /// Vision sidecar HTTP port (health endpoint at /health).
    pub vision_port: u16,
    /// Path the uinput daemon will listen on.
    pub uinput_socket_path: PathBuf,
    /// Path to the uinput daemon binary (release or debug).
    pub uinput_daemon_binary: PathBuf,
    /// How often to poll service health.
    pub health_check_interval: Duration,
    /// If true, spawn uinput daemon via `sudo`. False requires the binary
    /// to have appropriate uinput capabilities already granted.
    pub use_sudo_for_uinput: bool,
}

impl OrchestratorConfig {
    /// Auto-detect a reasonable config from `current_exe` and the workspace
    /// layout. Used in dev (cargo run) and production (Tauri bundle).
    pub fn auto_detect() -> Result<Self> {
        let workspace_root = detect_workspace_root()?;
        let uinput_daemon_binary = detect_uinput_binary(&workspace_root)?;
        Ok(Self {
            workspace_root,
            vision_port: 8080,
            uinput_socket_path: crate::agent::gui_services::default_uinput_socket_path(),
            uinput_daemon_binary,
            health_check_interval: Duration::from_secs(5),
            use_sudo_for_uinput: true,
        })
    }
}

/// Inner shared state — guarded by `RwLock`.
#[derive(Default)]
struct OrchestratorState {
    vision_child: Option<Child>,
    uinput_child: Option<Child>,
    vision_status: ServiceLiveness,
    uinput_status: ServiceLiveness,
    /// User-controlled master switch. When false, no spawn attempts and
    /// `GlobalSafetyHalt` is engaged.
    automation_enabled: bool,
    /// Actual port the vision sidecar bound to (0 = not yet assigned).
    vision_port: u16,
    /// Number of automatic restart attempts for the uinput daemon since last
    /// successful run. Reset to 0 whenever both services become healthy.
    uinput_restart_attempts: u8,
    /// Unix epoch seconds when the last restart attempt was issued. Used with
    /// `DAEMON_RESTART_BACKOFF_SECS` to enforce per-attempt backoff.
    uinput_last_restart_epoch_secs: u64,
    /// Number of automatic restart attempts for the vision sidecar.
    vision_restart_attempts: u8,
    /// Unix epoch seconds when the last vision sidecar restart was issued.
    vision_last_restart_epoch_secs: u64,
}

impl Default for ServiceLiveness {
    fn default() -> Self {
        ServiceLiveness::Stopped
    }
}

/// Maximum number of automatic restart attempts for the uinput daemon.
/// After this limit, KRIA stays halted until the user manually restarts.
const MAX_DAEMON_RESTART_ATTEMPTS: u8 = 3;

/// Minimum backoff seconds between successive uinput daemon restart attempts.
/// Index 0 = first retry, 1 = second, 2 = third. Beyond this the daemon stays
/// halted and requires a manual restart.
const DAEMON_RESTART_BACKOFF_SECS: [u64; 3] = [5, 15, 45];

/// Unified service orchestrator.
///
/// Use `Arc<ServiceOrchestrator>` and clone for sharing across Tauri command
/// handlers. The internal `Drop` impl cleans up children when the last Arc
/// is released.
pub struct ServiceOrchestrator {
    config: OrchestratorConfig,
    state: Arc<RwLock<OrchestratorState>>,
    cancellation: CancellationToken,
    /// Set to `true` after `start()` completes (health monitor is running).
    /// `Drop` only engages the global halt when this is true, so a failed
    /// startup attempt does not overwrite the real error reason.
    initialized: AtomicBool,
}

impl ServiceOrchestrator {
    /// Create a new orchestrator. Does NOT spawn services — call `start()`.
    pub fn new(config: OrchestratorConfig) -> Self {
        Self {
            config,
            state: Arc::new(RwLock::new(OrchestratorState {
                automation_enabled: true,
                ..Default::default()
            })),
            cancellation: CancellationToken::new(),
            initialized: AtomicBool::new(false),
        }
    }

    /// Spawn both child processes and begin health monitoring.
    /// Try to bind to a port starting from `start`. Returns the first free port
    /// in the range [start, start+100]. Panics if none are available.
    fn find_free_port(start: u16) -> u16 {
        for port in start..=start.saturating_add(100) {
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return port;
            }
        }
        panic!(
            "no free TCP port found in range {}–{}",
            start,
            start.saturating_add(100)
        );
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        tracing::info!(target: "orchestrator", "🚀 Starting KRIA service orchestrator");

        // Engage halt until both services are healthy
        crate::safety::engage_halt("orchestrator startup");

        // RFC v2 (F4): Probe display server type.
        Self::probe_display_server();

        // Best-effort spawns: individual failures are non-fatal here.
        // The health monitor (always started below) will retry on the next
        // tick using the bounded auto-restart logic in run_health_check.
        if let Err(e) = self.spawn_vision_sidecar().await {
            tracing::warn!(
                target: "orchestrator",
                error = %e,
                "vision sidecar failed initial spawn — health monitor will retry"
            );
            let mut state = self.state.write().await;
            state.vision_status = ServiceLiveness::Failed;
        }
        if let Err(e) = self.spawn_uinput_daemon().await {
            tracing::warn!(
                target: "orchestrator",
                error = %e,
                "uinput daemon failed initial spawn — health monitor will retry"
            );
            let mut state = self.state.write().await;
            state.uinput_status = ServiceLiveness::Failed;
        }

        // Always spawn the health monitor regardless of spawn outcomes.
        // This is what drives the bounded auto-restart for both services.
        let weak = Arc::downgrade(self);
        let cancel = self.cancellation.clone();
        let interval = self.config.health_check_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = cancel.cancelled() => break,
                }
                let Some(orch) = weak.upgrade() else { break };
                orch.run_health_check().await;
            }
            tracing::info!(target: "orchestrator", "health monitor exiting");
        });

        // Mark initialized so Drop knows the health monitor is running.
        self.initialized.store(true, Ordering::Relaxed);

        Ok(())
    }

    /// Spawn the Python vision sidecar (`python main.py` in `sidecars/kria-vision`).
    async fn spawn_vision_sidecar(&self) -> Result<()> {
        let sidecar_dir = self.config.workspace_root.join("sidecars/kria-vision");
        if !sidecar_dir.join("main.py").exists() {
            return Err(anyhow!(
                "vision sidecar main.py not found at {}",
                sidecar_dir.display()
            ));
        }

        // Prefer venv python if it exists, fall back to system python
        let python = self.detect_vision_python(&sidecar_dir);
        let skip_pids = self.current_child_pids().await;
        self.cleanup_stale_processes(
            "vision sidecar",
            &[python.clone(), "main.py".to_string()],
            &skip_pids,
        )
        .await;

        tracing::info!(
            target: "orchestrator",
            python = %python,
            sidecar = %sidecar_dir.display(),
            "spawning vision sidecar"
        );

        // Pick a free port so we don't collide with a leftover manual process
        let port = Self::find_free_port(self.config.vision_port);
        {
            let mut state = self.state.write().await;
            state.vision_port = port;
        }
        tracing::info!(target: "orchestrator", port = port, "selected free port for vision sidecar");

        let mut cmd = Command::new(&python);
        cmd.arg("main.py")
            .current_dir(&sidecar_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .env("KRIA_VISION_PORT", port.to_string())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn vision sidecar: {}", python))?;

        let pid = child.id();
        {
            let mut state = self.state.write().await;
            state.vision_child = Some(child);
            state.vision_status = ServiceLiveness::Starting;
        }

        tracing::info!(target: "orchestrator", pid = ?pid, "vision sidecar PID");
        Ok(())
    }

    /// RFC v2 (F4): Probe `XDG_SESSION_TYPE` and the presence of `xdotool`.
    ///
    /// - On `x11` sessions: silent (the happy path).
    /// - On `wayland` sessions: emit a high-visibility warning. xdotool is
    ///   x11-only and the daemon's modifier-release path will fail. The
    ///   primary input path (uinput via ydotool) still works because it
    ///   bypasses the display server.
    /// - On unknown / missing: log a debug note.
    ///
    /// This probe is intentionally non-fatal: the daemon may still operate
    /// usefully on Wayland for keyboard-only workflows; we just want loud
    /// operator notice when xdotool calls will silently fail.
    fn probe_display_server() {
        let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let has_xdotool = which::which("xdotool").is_ok();
        let has_ydotool = which::which("ydotool").is_ok();

        match session_type.as_str() {
            "wayland" => {
                tracing::warn!(
                    target: "orchestrator",
                    session_type = %session_type,
                    has_xdotool,
                    has_ydotool,
                    "RFC v2 (F4): running on Wayland — xdotool-based modifier \
                     release will not work. GUI automation will rely on the \
                     uinput daemon path only. For full functionality, log into \
                     an X11/Xorg session or install Xwayland with a working DISPLAY."
                );
            }
            "x11" => {
                if !has_xdotool {
                    tracing::warn!(
                        target: "orchestrator",
                        "RFC v2 (F4): X11 session detected but `xdotool` not on PATH; \
                         modifier-release will fail. Install xdotool."
                    );
                } else {
                    tracing::info!(
                        target: "orchestrator",
                        "Display server: X11 (xdotool available)"
                    );
                }
            }
            other => {
                tracing::debug!(
                    target: "orchestrator",
                    session_type = %other,
                    has_xdotool,
                    has_ydotool,
                    "Display server type unknown — assuming X11-compatible"
                );
            }
        }
    }

    /// Detect which python to use for the vision sidecar.
    fn detect_vision_python(&self, sidecar_dir: &Path) -> String {
        let venv_python = sidecar_dir.join("venv/bin/python");
        if venv_python.exists() {
            return venv_python.to_string_lossy().into_owned();
        }
        // Fallback to system python3
        "python3".to_string()
    }

    /// Spawn the uinput daemon. On Linux this typically requires sudo unless
    /// the binary has `CAP_DAC_OVERRIDE` + access to `/dev/uinput`.
    async fn spawn_uinput_daemon(&self) -> Result<()> {
        if !self.config.uinput_daemon_binary.exists() {
            return Err(anyhow!(
                "uinput daemon binary not found at {} — run `cargo build --release -p kria-uinput-daemon`",
                self.config.uinput_daemon_binary.display()
            ));
        }

        let skip_pids = self.current_child_pids().await;
        self.cleanup_stale_processes(
            "uinput daemon",
            &[
                self.config.uinput_daemon_binary.display().to_string(),
                self.config.uinput_socket_path.display().to_string(),
            ],
            &skip_pids,
        )
        .await;

        if let Some(parent) = self.config.uinput_socket_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("create uinput socket parent directory {}", parent.display())
            })?;
        }

        // Remove stale socket from a previous run.
        // The socket may be owned by root (created by the daemon via sudo), so a plain
        // `remove_file` will fail when KRIA runs as a non-root user.  Try the direct
        // removal first; if that fails and sudo is in use, attempt `sudo -n rm -f`.
        // The daemon's own `create_secure_socket` also removes the stale socket at
        // startup (it runs as root), so this is just a best-effort pre-clean.
        if self.config.uinput_socket_path.exists() {
            if let Err(_) = tokio::fs::remove_file(&self.config.uinput_socket_path).await {
                if self.config.use_sudo_for_uinput {
                    tracing::debug!(
                        target: "orchestrator",
                        socket = %self.config.uinput_socket_path.display(),
                        "non-root socket removal failed; trying sudo rm -f"
                    );
                    let _ = Command::new("sudo")
                        .arg("-n")
                        .arg("rm")
                        .arg("-f")
                        .arg(&self.config.uinput_socket_path)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .output()
                        .await;
                }
            }
        }

        // Collect display-related environment variables that the daemon needs for
        // xdotool / AT-SPI window queries.  Only forward variables that are actually
        // set in this process's environment.
        let display_vars: Vec<(String, String)> = [
            "DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "XDG_SESSION_TYPE",
        ]
        .iter()
        .filter_map(|key| std::env::var(key).ok().map(|val| (key.to_string(), val)))
        .collect();

        let mut cmd = if self.config.use_sudo_for_uinput {
            let mut c = Command::new("sudo");
            c.arg("-n"); // non-interactive: fail if sudo would prompt

            // Forward display vars via `sudo --preserve-env=KEY1,KEY2,... binary`.
            //
            // Previously this used `sudo -n env KEY=VAL ... binary`, which required
            // a separate NOPASSWD entry for /usr/bin/env in sudoers and caused silent
            // spawn failures when only the daemon binary was in the NOPASSWD list.
            //
            // `--preserve-env=LIST` (supported in sudo ≥ 1.8.5) forwards named vars
            // from sudo's own environment to the target command without needing
            // SETENV in sudoers.  sudo inherits those vars from KRIA's process, so
            // we also set them explicitly on the Command for robustness.
            if !display_vars.is_empty() {
                let var_names = display_vars
                    .iter()
                    .map(|(k, _)| k.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                c.arg(format!("--preserve-env={}", var_names));
                for (key, val) in &display_vars {
                    c.env(key, val);
                }
            }

            c.arg(&self.config.uinput_daemon_binary)
                .arg("--socket")
                .arg(&self.config.uinput_socket_path)
                .arg("--parent-pid")
                .arg(std::process::id().to_string());
            if let Some(start_time) = process_start_time_ticks(std::process::id()) {
                c.arg("--parent-start-time").arg(start_time.to_string());
            }
            c
        } else {
            let mut c = Command::new(&self.config.uinput_daemon_binary);
            c.arg("--socket")
                .arg(&self.config.uinput_socket_path)
                .arg("--parent-pid")
                .arg(std::process::id().to_string());
            if let Some(start_time) = process_start_time_ticks(std::process::id()) {
                c.arg("--parent-start-time").arg(start_time.to_string());
            }
            for (key, val) in &display_vars {
                c.env(key, val);
            }
            c
        };

        tracing::debug!(
            target: "orchestrator",
            display_vars = ?display_vars.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            "forwarding display env vars to uinput daemon"
        );

        cmd.stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        tracing::info!(
            target: "orchestrator",
            binary = %self.config.uinput_daemon_binary.display(),
            socket = %self.config.uinput_socket_path.display(),
            sudo = self.config.use_sudo_for_uinput,
            "spawning uinput daemon"
        );

        let mut child = cmd
            .spawn()
            .context("failed to spawn uinput daemon (passwordless sudo may not be configured)")?;

        let pid = child.id();

        // Brief liveness check: wait 250 ms and verify the process has not already
        // exited.  Immediate exits almost always mean sudo rejected the command
        // ("password required") rather than a daemon logic error.
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok(Some(status)) = child.try_wait() {
            return Err(anyhow!(
                "uinput daemon exited immediately after spawn (status: {status}). \
                 This usually means `sudo -n` requires a password for the daemon binary. \
                 Ensure your sudoers file contains: \
                 `{user} ALL=(ALL) NOPASSWD: {binary}`",
                user = std::env::var("USER").unwrap_or_else(|_| "<user>".into()),
                binary = self.config.uinput_daemon_binary.display(),
            ));
        }

        {
            let mut state = self.state.write().await;
            state.uinput_child = Some(child);
            state.uinput_status = ServiceLiveness::Starting;
        }

        tracing::info!(target: "orchestrator", pid = ?pid, "uinput daemon PID");
        Ok(())
    }

    /// Run a single health check pass over both services.
    async fn run_health_check(self: &Arc<Self>) {
        let vision_live = self.check_vision_health().await;
        let uinput_live = self.check_uinput_health().await;

        let attempt_restart;
        let attempt_vision_restart;
        {
            let mut state = self.state.write().await;
            state.vision_status = vision_live;
            state.uinput_status = uinput_live;

            // Update GlobalSafetyHalt based on combined health + user toggle
            let user_wants_enabled = state.automation_enabled;
            let both_healthy = vision_live.is_healthy() && uinput_live.is_healthy();

            match (user_wants_enabled, both_healthy) {
                (true, true) => {
                    crate::safety::release_halt("services healthy");
                    // Reset restart counter whenever both services are healthy.
                    state.uinput_restart_attempts = 0;
                }
                (true, false) => {
                    // Build a granular reason that names which service is not ready
                    let vision_word = match vision_live {
                        ServiceLiveness::Running => "ok",
                        ServiceLiveness::Starting => "starting",
                        ServiceLiveness::Failed => "FAILED",
                        ServiceLiveness::Stopped => "stopped",
                    };
                    let uinput_word = match uinput_live {
                        ServiceLiveness::Running => "ok",
                        ServiceLiveness::Starting => "starting",
                        ServiceLiveness::Failed => "FAILED",
                        ServiceLiveness::Stopped => "stopped",
                    };
                    crate::safety::engage_halt(&format!(
                        "service not ready (vision={vision_word}, uinput={uinput_word})"
                    ));
                }
                (false, _) => crate::safety::engage_halt("user disabled automation via UI"),
            }

            // ── Bounded uinput daemon auto-restart ───────────────────────────
            // If the daemon died and automation is still desired, attempt a
            // bounded restart with exponential backoff. We only restart when
            // `uinput_child` is None (confirmed dead or never started) and the
            // required backoff since the last attempt has elapsed.
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let backoff_required = DAEMON_RESTART_BACKOFF_SECS
                .get(state.uinput_restart_attempts as usize)
                .copied()
                .unwrap_or(60);
            let elapsed = now_secs.saturating_sub(state.uinput_last_restart_epoch_secs);

            attempt_restart = user_wants_enabled
                && !uinput_live.is_healthy()
                && state.uinput_child.is_none()
                && state.uinput_restart_attempts < MAX_DAEMON_RESTART_ATTEMPTS
                && elapsed >= backoff_required;

            if attempt_restart {
                tracing::info!(
                    target: "orchestrator",
                    attempt = state.uinput_restart_attempts + 1,
                    max = MAX_DAEMON_RESTART_ATTEMPTS,
                    backoff_secs = backoff_required,
                    "Attempting bounded uinput daemon auto-restart"
                );
                state.uinput_restart_attempts += 1;
                state.uinput_last_restart_epoch_secs = now_secs;
            }

            // ── Bounded vision sidecar auto-restart ──────────────────────────
            let vision_backoff_required = DAEMON_RESTART_BACKOFF_SECS
                .get(state.vision_restart_attempts as usize)
                .copied()
                .unwrap_or(60);
            let vision_elapsed = now_secs.saturating_sub(state.vision_last_restart_epoch_secs);
            attempt_vision_restart = user_wants_enabled
                && vision_live == ServiceLiveness::Failed
                && state.vision_child.is_none()
                && state.vision_restart_attempts < MAX_DAEMON_RESTART_ATTEMPTS
                && vision_elapsed >= vision_backoff_required;

            if attempt_vision_restart {
                tracing::info!(
                    target: "orchestrator",
                    attempt = state.vision_restart_attempts + 1,
                    max = MAX_DAEMON_RESTART_ATTEMPTS,
                    backoff_secs = vision_backoff_required,
                    "Attempting bounded vision sidecar auto-restart"
                );
                state.vision_restart_attempts += 1;
                state.vision_last_restart_epoch_secs = now_secs;
            }

            // Reset restart counters when both become healthy
            if both_healthy {
                state.vision_restart_attempts = 0;
                state.vision_last_restart_epoch_secs = 0;
            }
        } // Release write lock before spawning

        if attempt_restart {
            match self.spawn_uinput_daemon().await {
                Ok(()) => {
                    tracing::info!(
                        target: "orchestrator",
                        "uinput daemon auto-restart spawned — health check will confirm"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "orchestrator",
                        error = %e,
                        "uinput daemon auto-restart failed to spawn"
                    );
                }
            }
        }

        if attempt_vision_restart {
            match self.spawn_vision_sidecar().await {
                Ok(()) => {
                    tracing::info!(
                        target: "orchestrator",
                        "vision sidecar auto-restart spawned — health check will confirm"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "orchestrator",
                        error = %e,
                        "vision sidecar auto-restart failed to spawn"
                    );
                }
            }
        }
    }

    /// HTTP health check for the vision sidecar.
    async fn check_vision_health(&self) -> ServiceLiveness {
        // First, check if the process is still alive
        {
            let mut state = self.state.write().await;
            if let Some(child) = state.vision_child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::warn!(
                            target: "orchestrator",
                            ?status,
                            "vision sidecar exited"
                        );
                        state.vision_child = None;
                        return ServiceLiveness::Failed;
                    }
                    Ok(None) => {} // still running
                    Err(e) => {
                        tracing::error!(target: "orchestrator", error = %e, "try_wait failed");
                        return ServiceLiveness::Failed;
                    }
                }
            } else {
                return ServiceLiveness::Stopped;
            }
        }

        // HTTP health probe
        // Use the *actual* port the sidecar bound to, not the config default.
        let port = {
            let state = self.state.read().await;
            if state.vision_port == 0 {
                self.config.vision_port
            } else {
                state.vision_port
            }
        };
        let url = format!("http://127.0.0.1:{}/health", port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok();

        match client {
            Some(c) => match c.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => ServiceLiveness::Running,
                Ok(resp) => {
                    tracing::warn!(
                        target: "orchestrator",
                        status = resp.status().as_u16(),
                        "vision /health returned non-200"
                    );
                    ServiceLiveness::Starting
                }
                Err(e) => {
                    tracing::debug!(target: "orchestrator", error = %e, "vision /health unreachable");
                    ServiceLiveness::Starting
                }
            },
            None => ServiceLiveness::Failed,
        }
    }

    /// Socket-existence check for the uinput daemon.
    /// (Heartbeat ping is sent by the GUI executor itself.)
    async fn check_uinput_health(&self) -> ServiceLiveness {
        {
            let mut state = self.state.write().await;
            if let Some(child) = state.uinput_child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        tracing::warn!(
                            target: "orchestrator",
                            ?status,
                            "uinput daemon exited"
                        );
                        state.uinput_child = None;
                        return ServiceLiveness::Failed;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(target: "orchestrator", error = %e, "try_wait failed");
                        return ServiceLiveness::Failed;
                    }
                }
            } else {
                return ServiceLiveness::Stopped;
            }
        }

        if !self.config.uinput_socket_path.exists() {
            return ServiceLiveness::Starting;
        }

        ServiceLiveness::Running
    }

    /// Get a status snapshot.
    pub async fn status(&self) -> ServiceStatus {
        let state = self.state.read().await;
        ServiceStatus {
            vision_sidecar: state.vision_status,
            uinput_daemon: state.uinput_status,
            automation_enabled: state.automation_enabled,
            vision_pid: state.vision_child.as_ref().and_then(|c| c.id()),
            uinput_pid: state.uinput_child.as_ref().and_then(|c| c.id()),
        }
    }

    /// Set the master automation toggle. When disabled:
    ///   - GlobalSafetyHalt is engaged immediately
    ///   - Both child processes are SIGKILLed
    ///   - Stale socket files are removed
    pub async fn set_automation_enabled(self: &Arc<Self>, enabled: bool) -> Result<()> {
        tracing::info!(target: "orchestrator", enabled, "user toggled automation");

        {
            let mut state = self.state.write().await;
            state.automation_enabled = enabled;
        }

        if !enabled {
            crate::safety::engage_halt("user disabled automation via UI");
            self.kill_children().await;
        } else {
            // Re-spawn services. Halt stays engaged until health check passes.
            crate::safety::engage_halt("re-spawning services");
            if let Err(e) = self.spawn_vision_sidecar().await {
                tracing::error!(target: "orchestrator", error = %e, "vision respawn failed");
            }
            if let Err(e) = self.spawn_uinput_daemon().await {
                tracing::error!(target: "orchestrator", error = %e, "uinput respawn failed");
            }
        }

        Ok(())
    }

    /// SIGKILL both children and clean up sockets. Synchronous part of shutdown.
    async fn kill_children(&self) {
        let mut state = self.state.write().await;

        if let Some(mut child) = state.vision_child.take() {
            tracing::info!(target: "orchestrator", "killing vision sidecar");
            let _ = child.kill().await;
            state.vision_status = ServiceLiveness::Stopped;
        }

        if let Some(mut child) = state.uinput_child.take() {
            tracing::info!(target: "orchestrator", "killing uinput daemon");
            // Try graceful first via SIGTERM, fall back to SIGKILL via Tokio
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                // Brief grace period then force-kill
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let _ = child.kill().await;
            state.uinput_status = ServiceLiveness::Stopped;
        }

        // Remove socket files
        let _ = tokio::fs::remove_file(&self.config.uinput_socket_path).await;
    }

    /// Graceful shutdown. Cancels health monitor, kills children, cleans sockets.
    pub async fn shutdown(&self) {
        tracing::info!(target: "orchestrator", "🛑 shutting down service orchestrator");
        crate::safety::engage_halt("orchestrator shutdown");
        self.cancellation.cancel();
        self.kill_children().await;
    }
}

impl Drop for ServiceOrchestrator {
    fn drop(&mut self) {
        // Best-effort sync cleanup. The async `shutdown()` should have been
        // called from a Tauri ExitRequested handler; this is the safety net.
        //
        // Only engage the halt when the orchestrator was actually running
        // (initialized = true). A failed start() attempt drops the orchestrator
        // before initialization completes; in that case the real error reason
        // is already recorded and we must not overwrite it with "orchestrator Drop".
        if self.initialized.load(Ordering::Relaxed) {
            tracing::warn!(target: "orchestrator", "Drop: emergency cleanup (prefer shutdown())");
            crate::safety::engage_halt("orchestrator Drop");
        } else {
            tracing::warn!(target: "orchestrator", "Drop: cleaning up failed startup attempt");
        }
        self.cancellation.cancel();

        // Synchronous SIGKILL via libc — Tokio's Child::kill() requires async
        if let Ok(mut state) = self.state.try_write() {
            if let Some(child) = state.vision_child.as_mut() {
                if let Some(pid) = child.id() {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
            }
            if let Some(child) = state.uinput_child.as_mut() {
                if let Some(pid) = child.id() {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }
        }

        // Remove stale socket synchronously
        let _ = std::fs::remove_file(&self.config.uinput_socket_path);
    }
}

impl ServiceOrchestrator {
    async fn current_child_pids(&self) -> Vec<u32> {
        let state = self.state.read().await;
        [
            state.vision_child.as_ref().and_then(|child| child.id()),
            state.uinput_child.as_ref().and_then(|child| child.id()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    async fn cleanup_stale_processes(
        &self,
        label: &str,
        required_cmdline_parts: &[String],
        skip_pids: &[u32],
    ) {
        let stale = find_matching_processes(required_cmdline_parts, skip_pids);
        if stale.is_empty() {
            return;
        }

        tracing::warn!(
            target: "orchestrator",
            service = label,
            pids = ?stale,
            "found stale GUI service process(es) from prior KRIA run; terminating before spawn"
        );

        for pid in &stale {
            self.signal_process(*pid, libc::SIGTERM, label).await;
        }

        tokio::time::sleep(Duration::from_millis(300)).await;

        let survivors = find_matching_processes(required_cmdline_parts, skip_pids);
        for pid in survivors {
            tracing::warn!(
                target: "orchestrator",
                service = label,
                pid,
                "stale GUI service survived SIGTERM; sending SIGKILL"
            );
            self.signal_process(pid, libc::SIGKILL, label).await;
        }
    }

    async fn signal_process(&self, pid: u32, signal: i32, label: &str) {
        let rc = unsafe { libc::kill(pid as i32, signal) };
        if rc == 0 {
            return;
        }

        let err = std::io::Error::last_os_error();
        if err.kind() == ErrorKind::PermissionDenied && self.config.use_sudo_for_uinput {
            let signal_arg = if signal == libc::SIGKILL {
                "-KILL"
            } else {
                "-TERM"
            };
            match Command::new("sudo")
                .arg("-n")
                .arg("kill")
                .arg(signal_arg)
                .arg(pid.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
            {
                Ok(status) if status.success() => return,
                Ok(status) => {
                    tracing::warn!(
                        target: "orchestrator",
                        service = label,
                        pid,
                        signal,
                        status = %status,
                        "sudo kill failed for stale GUI service process"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        target: "orchestrator",
                        service = label,
                        pid,
                        signal,
                        error = %error,
                        "failed to invoke sudo kill for stale GUI service process"
                    );
                }
            }
        } else {
            tracing::warn!(
                target: "orchestrator",
                service = label,
                pid,
                signal,
                error = %err,
                "failed to signal stale GUI service process"
            );
        }
    }
}

fn find_matching_processes(required_cmdline_parts: &[String], skip_pids: &[u32]) -> Vec<u32> {
    let self_pid = std::process::id();
    let mut matches = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return matches;
    };

    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == self_pid || skip_pids.contains(&pid) {
            continue;
        }

        let cmdline_path = entry.path().join("cmdline");
        let Ok(raw) = std::fs::read(cmdline_path) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let cmdline = raw
            .split(|b| *b == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part))
            .collect::<Vec<_>>()
            .join(" ");

        if cmdline_matches_required_parts(&cmdline, required_cmdline_parts) {
            matches.push(pid);
        }
    }

    matches
}

fn cmdline_matches_required_parts(cmdline: &str, required_cmdline_parts: &[String]) -> bool {
    required_cmdline_parts
        .iter()
        .all(|part| !part.is_empty() && cmdline.contains(part))
}

fn process_start_time_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 2..)?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

// ============================================================================
// Helpers
// ============================================================================

fn detect_workspace_root() -> Result<PathBuf> {
    // 1. Honor explicit env var
    if let Ok(p) = std::env::var("KRIA_WORKSPACE_ROOT") {
        let p = PathBuf::from(p);
        if p.join("Cargo.toml").exists() {
            return Ok(p);
        }
    }

    // 2. Walk up from current_exe()
    let exe = std::env::current_exe().context("current_exe failed")?;
    let mut cur = exe.as_path();
    while let Some(parent) = cur.parent() {
        if parent.join("sidecars/kria-vision/main.py").exists() {
            return Ok(parent.to_path_buf());
        }
        cur = parent;
    }

    // 3. Walk up from current_dir()
    if let Ok(cwd) = std::env::current_dir() {
        let mut cur = cwd.as_path();
        loop {
            if cur.join("sidecars/kria-vision/main.py").exists() {
                return Ok(cur.to_path_buf());
            }
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
    }

    Err(anyhow!(
        "could not locate KRIA workspace root (set KRIA_WORKSPACE_ROOT)"
    ))
}

fn detect_uinput_binary(workspace_root: &Path) -> Result<PathBuf> {
    if let Ok(p) = std::env::var("KRIA_UINPUT_BINARY") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Ok(p);
        }
    }
    for candidate in [
        workspace_root.join("target/release/kria-uinput-daemon"),
        workspace_root.join("target/debug/kria-uinput-daemon"),
    ] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(anyhow!(
        "kria-uinput-daemon binary not found — run `cargo build --release -p kria-uinput-daemon`"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_liveness_serde() {
        let s = serde_json::to_string(&ServiceLiveness::Running).unwrap();
        assert_eq!(s, "\"running\"");
        let parsed: ServiceLiveness = serde_json::from_str("\"failed\"").unwrap();
        assert_eq!(parsed, ServiceLiveness::Failed);
    }

    #[test]
    fn status_all_healthy() {
        let s = ServiceStatus {
            vision_sidecar: ServiceLiveness::Running,
            uinput_daemon: ServiceLiveness::Running,
            automation_enabled: true,
            vision_pid: Some(1),
            uinput_pid: Some(2),
        };
        assert!(s.all_healthy());

        let s = ServiceStatus {
            vision_sidecar: ServiceLiveness::Running,
            uinput_daemon: ServiceLiveness::Failed,
            automation_enabled: true,
            vision_pid: Some(1),
            uinput_pid: None,
        };
        assert!(!s.all_healthy());
    }

    #[test]
    fn stale_process_match_requires_all_parts() {
        let cmdline = "/media/obaid/SSD/KRIA/target/release/kria-uinput-daemon --socket /run/user/1000/kria-uinput.sock";
        assert!(cmdline_matches_required_parts(
            cmdline,
            &[
                "target/release/kria-uinput-daemon".to_string(),
                "/run/user/1000/kria-uinput.sock".to_string(),
            ]
        ));
        assert!(!cmdline_matches_required_parts(
            cmdline,
            &[
                "target/release/kria-uinput-daemon".to_string(),
                "/tmp/other.sock".to_string(),
            ]
        ));
    }
}
