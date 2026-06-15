//! LlamaServerManager — manages the llama-server process lifecycle.
//!
//! Key design decisions:
//! - AtomicU8 for lock-free server state (V7: no RwLock deadlock)
//! - CancellationToken for non-blocking stream abort (V13)
//! - Ephemeral ports via --port 0 + stderr parsing (V5, V14)
//! - ChildGuard RAII: SIGTERM→SIGKILL ladder + prctl/setsid (Phase 2)

use crate::config::{ModelProfile, OrchestratorConfig};
use crate::infra::event_bus::EventBus;
use crate::llm::orchestrator::child_guard::{self, ChildGuard};
use crate::llm::orchestrator::vision_strategy::VisionMode;
use crate::platform::os;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

/// Server states (stored in AtomicU8).
pub const STATE_STOPPED: u8 = 0;
pub const STATE_STARTING: u8 = 1;
pub const STATE_READY: u8 = 2;
pub const STATE_SWAPPING: u8 = 3;
pub const STATE_ERROR: u8 = 4;

const ROUTER_MODE_UNKNOWN: u8 = 0;
const ROUTER_MODE_SUPPORTED: u8 = 1;
const ROUTER_MODE_UNSUPPORTED: u8 = 2;

pub(crate) const ACTIVE_SLOT_ID: u32 = 0;
pub(crate) const ACTIVE_SLOT_FILENAME: &str = "kria_active_slot.bin";

#[derive(Debug, Clone, Copy)]
struct LaunchTuning {
    batch_size: u32,
    ubatch_size: Option<u32>,
    parallel: Option<u32>,
    no_warmup: bool,
}

fn launch_tuning(config_batch_size: u32, enable_vision: bool) -> LaunchTuning {
    let configured = config_batch_size.max(1);

    if enable_vision {
        // Vision inference on 6GB-class GPUs is unstable with auto parallel
        // slots + warmup. Use a conservative profile to avoid segfault/OOM
        // while keeping the endpoint responsive.
        let safe_batch = configured.clamp(1, 128);
        return LaunchTuning {
            batch_size: safe_batch,
            ubatch_size: Some(safe_batch),
            parallel: Some(1),
            no_warmup: true,
        };
    }

    LaunchTuning {
        batch_size: configured,
        ubatch_size: None,
        parallel: None,
        no_warmup: false,
    }
}

/// Whether the vision projector (clip/mmproj) weights should be kept in system
/// RAM instead of being offloaded to the GPU.
///
/// Default: `true` (CPU-resident clip). This avoids the VRAM OOM abort during
/// clip load on small (6GB-class) GPUs where the LLM weights + KV cache already
/// consume most of the device memory.
///
/// Opt back into GPU offload (old behavior) by setting `KRIA_MMPROJ_GPU_OFFLOAD`
/// to a truthy value (`1`, `true`, `yes`, `on`). When enabled, no
/// `--no-mmproj-offload` flag is passed and the spawn command is byte-for-byte
/// identical to the pre-fix behavior.
fn mmproj_cpu_only() -> bool {
    match std::env::var("KRIA_MMPROJ_GPU_OFFLOAD") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            // Truthy => allow GPU offload => NOT cpu-only.
            !matches!(v.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => true,
    }
}

fn v1_models_endpoint(base_url: &str, action: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/models/{action}")
    } else {
        format!("{base}/v1/models/{action}")
    }
}

/// Manages a single llama-server process with atomic state tracking.
pub struct LlamaServerManager {
    config: OrchestratorConfig,
    model_path: String,
    mmproj_path: Option<String>,
    /// Lock-free server state — readable from any task without blocking.
    state: AtomicU8,
    /// Current GPU layers.
    current_ngl: AtomicU32,
    /// Current context window.
    current_ctx: AtomicU32,
    /// Whether the current runtime was spawned with vision/mmproj enabled.
    current_vision: AtomicBool,
    /// Last GPU layer count before an API-level unload. Used to restore
    /// `current_ngl` on API load success.
    pre_api_unload_ngl: AtomicU32,
    /// Last vision flag before an API-level unload.
    pre_api_unload_vision: AtomicBool,
    /// Router Mode capability cache.
    ///
    /// `ROUTER_MODE_UNKNOWN` => not yet probed for this process lifetime
    /// `ROUTER_MODE_SUPPORTED` => unload/load endpoints responded successfully
    /// `ROUTER_MODE_UNSUPPORTED` => endpoint returned 404/501 (or launch is not router-mode)
    router_mode_capability: AtomicU8,
    /// The actual API URL (updated after port discovery).
    api_url: tokio::sync::RwLock<String>,
    /// The child process wrapped in a ChildGuard for safe lifecycle management.
    child: Mutex<Option<ChildGuard>>,
    /// CancellationToken — cancelled during swap to abort in-flight streams.
    /// Guarded by a std::sync::Mutex so `cancel_streams()` can atomically
    /// cancel the current token and mint a fresh replacement. Using std::sync
    /// (not tokio) because the critical section is non-async and sub-μs.
    cancel_token: std::sync::Mutex<CancellationToken>,
    /// Token for the stderr reader task.
    reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Notified when a swap finishes (STATE_STOPPED or STATE_READY).
    /// Callers waiting for a swap to complete use this instead of busy-polling.
    swap_done: Arc<Notify>,
}

impl LlamaServerManager {
    pub fn new(
        config: OrchestratorConfig,
        model_path: String,
        mmproj_path: Option<String>,
    ) -> Self {
        Self {
            config,
            model_path,
            mmproj_path,
            state: AtomicU8::new(STATE_STOPPED),
            current_ngl: AtomicU32::new(0),
            current_ctx: AtomicU32::new(0),
            current_vision: AtomicBool::new(false),
            pre_api_unload_ngl: AtomicU32::new(0),
            pre_api_unload_vision: AtomicBool::new(false),
            router_mode_capability: AtomicU8::new(ROUTER_MODE_UNKNOWN),
            api_url: tokio::sync::RwLock::new(String::new()),
            child: Mutex::new(None),
            cancel_token: std::sync::Mutex::new(CancellationToken::new()),
            reader_handle: Mutex::new(None),
            swap_done: Arc::new(Notify::new()),
        }
    }

    /// Current server state (lock-free read).
    pub fn state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }

    /// Whether the server is ready to accept requests.
    pub fn is_healthy(&self) -> bool {
        self.state() == STATE_READY
    }

    /// Whether the underlying process appears alive right now.
    pub async fn has_live_process(&self) -> bool {
        let mut child_lock = self.child.lock().await;
        let Some(guard) = child_lock.as_mut() else {
            return false;
        };

        // Guard with inner child=None means the process was already reaped.
        if !guard.is_alive() {
            *child_lock = None;
            return false;
        }

        match guard.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                *child_lock = None;
                false
            }
        }
    }

    /// Whether a swap is in progress (streams should be cancelled).
    pub fn is_swapping(&self) -> bool {
        self.state() == STATE_SWAPPING
    }

    /// Current (ngl, context) parameters.
    pub fn current_params(&self) -> (u32, u32) {
        (
            self.current_ngl.load(Ordering::Acquire),
            self.current_ctx.load(Ordering::Acquire),
        )
    }

    /// Whether the current runtime can accept image input.
    pub fn current_vision_enabled(&self) -> bool {
        self.current_vision.load(Ordering::Acquire)
    }

    /// Whether vision is configured well enough to request a vision spawn.
    pub fn vision_configured(&self) -> bool {
        self.mmproj_path
            .as_ref()
            .map(|p| !p.trim().is_empty() && Path::new(p).exists())
            .unwrap_or(false)
    }

    /// Router-mode `--models-dir` value inferred from `model_path`.
    fn router_models_dir(&self) -> Option<PathBuf> {
        let path = Path::new(&self.model_path);
        let parent = path.parent()?;
        if parent.as_os_str().is_empty() {
            None
        } else {
            Some(parent.to_path_buf())
        }
    }

    /// Model identifier used for router mode API (`/v1/models/load|unload`)
    /// and `--model` argument when `--models-dir` is enabled.
    fn router_model_id(&self) -> String {
        let path = Path::new(&self.model_path);
        if self.router_models_dir().is_some() {
            path.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| self.model_path.clone())
        } else {
            self.model_path.clone()
        }
    }

    fn router_mode_supported_cached(&self) -> Option<bool> {
        match self.router_mode_capability.load(Ordering::Acquire) {
            ROUTER_MODE_SUPPORTED => Some(true),
            ROUTER_MODE_UNSUPPORTED => Some(false),
            _ => None,
        }
    }

    fn mark_router_mode_supported(&self) {
        self.router_mode_capability
            .store(ROUTER_MODE_SUPPORTED, Ordering::Release);
    }

    fn mark_router_mode_unsupported(&self) -> bool {
        // Returns true only on the first transition to unsupported.
        self.router_mode_capability
            .swap(ROUTER_MODE_UNSUPPORTED, Ordering::AcqRel)
            != ROUTER_MODE_UNSUPPORTED
    }

    /// Get the current API URL.
    pub fn api_url(&self) -> String {
        // Use try_read to avoid blocking; fall back to empty if locked
        self.api_url
            .try_read()
            .map(|u| u.clone())
            .unwrap_or_default()
    }

    /// Current orchestrator model profile (used by vision pre-flight budgeting).
    pub fn model_profile(&self) -> ModelProfile {
        self.config.model_profile.clone()
    }

    /// Current orchestrator VRAM safety margin (MB).
    pub fn safety_margin_mb(&self) -> u64 {
        self.config.safety_margin_mb
    }

    /// Test helper: force the API URL without spawning a real child process.
    #[doc(hidden)]
    pub async fn set_api_url_for_testing(&self, api_url: String) {
        let mut lock = self.api_url.write().await;
        *lock = api_url;
    }

    /// Get a CancellationToken that streams should select! on.
    ///
    /// Returns a clone of the *current* (non-cancelled) token. Each stream
    /// captures this at creation time, so only streams started before the
    /// next `cancel_streams()` call will be aborted.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.lock().unwrap().clone()
    }

    /// Cancel all in-flight streams, then mint a fresh token for new streams.
    ///
    /// This is the fix for the non-renewable CancellationToken bug: previously
    /// the token was created once and never replaced, so after the first swap
    /// every new stream would inherit an already-cancelled token.
    pub fn cancel_streams(&self) {
        let mut guard = self.cancel_token.lock().unwrap();
        guard.cancel();
        *guard = CancellationToken::new();
    }

    /// Returns an `Arc<Notify>` that is notified every time a swap finishes
    /// (on both success and failure paths). Callers can `notified().await`
    /// instead of busy-polling `is_swapping()`.  The Arc is stable for the
    /// lifetime of the manager.
    pub fn swap_done_notify(&self) -> Arc<Notify> {
        self.swap_done.clone()
    }

    /// Await swap completion with a timeout.
    /// Returns `true` if the swap finished before the deadline, `false` if
    /// the timeout was hit (server is still swapping or stuck).
    pub async fn wait_for_swap_done(&self, timeout: Duration) -> bool {
        // Fast path: not swapping right now.
        if !self.is_swapping() {
            return true;
        }
        // Subscribe before the is_swapping fast-path re-check so we can't
        // miss a notify that fires between the two checks.
        let notified = self.swap_done.notified();
        if !self.is_swapping() {
            return true;
        }
        tokio::time::timeout(timeout, notified).await.is_ok()
    }

    /// Spawn a new llama-server with the given parameters.
    ///
    /// - Uses `--port 0` for ephemeral port assignment
    /// - Parses stderr for the actual port
    /// - Waits for /health to report ready
    pub async fn spawn(
        &self,
        ngl: u32,
        context: u32,
        vision_mode: VisionMode,
        _event_bus: Arc<EventBus>,
    ) -> anyhow::Result<()> {
        // Derive booleans from the vision mode for backward compat
        let vision_requested = vision_mode.load_mmproj();
        let vision_enabled = vision_requested && self.vision_configured();
        if vision_requested && !vision_enabled {
            tracing::warn!(
                requested = vision_requested,
                ?vision_mode,
                mmproj_path = ?self.mmproj_path,
                "server_manager: vision requested but mmproj is missing/unavailable; starting text-only runtime"
            );
        }

        self.state.store(STATE_STARTING, Ordering::Release);
        self.current_vision.store(false, Ordering::Release);

        // Resolve binary: check ~/.kria/bin/ first, then config path (with .exe on Windows)
        let binary = os::resolve_binary("llama-server", &self.config.llama_server_binary);

        // Build llama-server command
        let mut cmd = tokio::process::Command::new(&binary);
        let tuning = launch_tuning(self.config.batch_size, vision_enabled);

        // Configure prctl(PR_SET_PDEATHSIG=SIGKILL) + setsid() in pre_exec
        // so the child is killed if Kria panics or is force-quit.
        child_guard::configure_child_command(&mut cmd);

        let router_models_dir = self.router_models_dir();
        let model_id = self.router_model_id();
        if let Some(models_dir) = router_models_dir.as_ref() {
            // Router mode: this is required for `/v1/models/unload|load` to work.
            //
            // Important compatibility note:
            // Some llama-server builds expose `--models-dir` but still expect
            // `--model` to be a resolvable filesystem path at process start.
            // Passing only the basename here can make startup fail before the
            // listening port is reported ("failed to open GGUF file ...").
            cmd.arg("--models-dir").arg(models_dir);
            cmd.arg("--model").arg(&self.model_path);
        } else {
            // Fallback for bare relative paths where parent dir is unavailable.
            cmd.arg("--model").arg(&self.model_path);
        }
        cmd.arg("--port").arg("0"); // Ephemeral port (V5)
        cmd.arg("--ctx-size").arg(context.to_string());
        cmd.arg("--n-gpu-layers").arg(ngl.to_string());
        cmd.arg("--batch-size").arg(tuning.batch_size.to_string());

        // --slot-save-path enables `/slots/{id}?action=save|restore`, used
        // by the Tier B drop-and-swap path to persist conversational KV
        // cache across the hard process restart that frees VRAM. We always
        // pass it (modern llama.cpp versions ignore it harmlessly when the
        // endpoint is unused).
        let slot_dir = self.resolve_slot_save_path();
        if let Err(e) = std::fs::create_dir_all(&slot_dir) {
            tracing::warn!(?e, path = %slot_dir.display(),
                "server_manager: failed to create slot-save dir; KV restore across swaps may fail");
        }
        cmd.arg("--slot-save-path").arg(&slot_dir);

        if let Some(ubatch) = tuning.ubatch_size {
            cmd.arg("--ubatch-size").arg(ubatch.to_string());
        }

        if let Some(parallel) = tuning.parallel {
            cmd.arg("--parallel").arg(parallel.to_string());
        }

        if tuning.no_warmup {
            cmd.arg("--no-warmup");
        }

        if self.config.flash_attention {
            cmd.arg("--flash-attn").arg("on");
        }
        // mlock pre-flight: even if config requests mlock, never pass --mlock
        // to llama-server when the OS doesn't have enough headroom to pin the
        // model file plus a 2 GB safety buffer. Doing so on a low-RAM box is a
        // guaranteed system freeze.
        if self.config.mlock {
            if mlock_is_safe(&self.model_path) {
                cmd.arg("--mlock");
            } else {
                tracing::warn!(
                    model_path = %self.model_path,
                    "server_manager: mlock requested but free RAM is insufficient — \
                     dropping --mlock to avoid OOM/freeze"
                );
            }
        }

        // Vision projector (mmproj)
        // CpuVision mode: load mmproj even at ngl=0. The projector weights
        // live in system RAM, not VRAM. This keeps the LLM sighted.
        if vision_enabled {
            if let Some(ref mmproj) = self.mmproj_path {
                cmd.arg("--mmproj").arg(mmproj);

                // Keep the vision projector (clip) weights in system RAM by
                // default. On 6GB-class GPUs the LLM weights + KV cache already
                // fill VRAM, so offloading the ~2GB worst-case mmproj causes a
                // hard `ggml_backend_buffer_set_usage` OOM abort during clip
                // load ("exited before reporting listening port"). Keeping clip
                // on CPU is also the documented design intent above.
                //
                // Kill-switch / opt-in: set KRIA_MMPROJ_GPU_OFFLOAD=1 (or true)
                // on machines with VRAM headroom to restore GPU offload. When
                // set, the spawn command is byte-for-byte the pre-fix behavior.
                if mmproj_cpu_only() {
                    cmd.arg("--no-mmproj-offload");
                }

                tracing::info!(
                    ?vision_mode,
                    ngl,
                    mmproj_cpu_only = mmproj_cpu_only(),
                    "server_manager: loading mmproj (vision_mode={vision_mode})"
                );
            }
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        tracing::info!(
            binary = %binary,
            ngl,
            ctx = context,
            router_mode = router_models_dir.is_some(),
            model_id = %model_id,
            models_dir = ?router_models_dir,
            vision_requested,
            vision_enabled,
            batch_size = tuning.batch_size,
            ubatch_size = ?tuning.ubatch_size,
            parallel = ?tuning.parallel,
            no_warmup = tuning.no_warmup,
            "server_manager: spawning llama-server"
        );

        let child = cmd.spawn().map_err(|e| {
            self.state.store(STATE_ERROR, Ordering::Release);
            anyhow::anyhow!("failed to spawn llama-server: {}", e)
        })?;
        let mut guard = ChildGuard::new(child);

        // Parse stdout/stderr for port discovery and log forwarding.
        // Some llama.cpp builds print the listening line to stdout instead
        // of stderr, so we consume both streams.
        let stderr = guard.take_stderr();
        let stdout = guard.take_stdout();
        let port = match self.discover_port(stderr, stdout).await {
            Ok(p) => p,
            Err(e) => {
                // Kill the child before returning the error so we don't leak it.
                guard.force_kill().await;
                return Err(e);
            }
        };

        let url = format!("http://127.0.0.1:{}/v1", port);
        tracing::info!(port, url = %url, "server_manager: discovered ephemeral port");

        // Update API URL before storing the guard so callers can read it.
        {
            let mut lock = self.api_url.write().await;
            *lock = url.clone();
        }

        // Store the child guard.
        {
            let mut lock = self.child.lock().await;
            *lock = Some(guard);
        }

        // Wait for the health endpoint to report ready.
        // On failure, take the guard back and force-kill to avoid leaving a
        // zombie process that holds the port.
        if let Err(e) = self.wait_for_health(&url).await {
            if let Some(mut g) = self.child.lock().await.take() {
                g.force_kill().await;
            }
            self.state.store(STATE_ERROR, Ordering::Release);
            return Err(e);
        }

        // Update state and params atomically
        self.current_ngl.store(ngl, Ordering::Release);
        self.current_ctx.store(context, Ordering::Release);
        self.current_vision.store(vision_enabled, Ordering::Release);
        self.state.store(STATE_READY, Ordering::Release);

        tracing::info!(
            ngl,
            ctx = context,
            port,
            "server_manager: llama-server is ready"
        );

        Ok(())
    }

    /// Discover the ephemeral port from llama-server output.
    /// llama-server prints something like: "main: server is listening on http://127.0.0.1:PORT"
    async fn discover_port(
        &self,
        stderr: Option<ChildStderr>,
        stdout: Option<ChildStdout>,
    ) -> anyhow::Result<u16> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut stream_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

        if let Some(stderr) = stderr {
            let tx = tx.clone();
            stream_tasks.push(tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "llama-server", "{}", line);
                    let _ = tx.send(line);
                }
            }));
        }

        if let Some(stdout) = stdout {
            let tx = tx.clone();
            stream_tasks.push(tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "llama-server", "{}", line);
                    let _ = tx.send(line);
                }
            }));
        }

        drop(tx);

        if stream_tasks.is_empty() {
            return Err(anyhow::anyhow!("no stdout/stderr from llama-server"));
        }

        let port_timeout_secs = self.config.port_discovery_timeout_secs.max(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(port_timeout_secs);

        let discovered = loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(line)) => {
                    // llama-server log format varies but port is usually in
                    // "... listening ... :PORT" or "..., port: PORT ..."
                    if let Some(port) = Self::extract_port_from_line(&line) {
                        break Ok(port);
                    }
                }
                Ok(None) => {
                    break Err(anyhow::anyhow!(
                        "llama-server exited before reporting listening port"
                    ));
                }
                Err(_) => {
                    break Err(anyhow::anyhow!(
                        "timed out waiting for llama-server to report listening port after {}s",
                        port_timeout_secs
                    ));
                }
            }
        };

        match discovered {
            Ok(port) => {
                // Keep draining process logs in background after discovery.
                let handle = tokio::spawn(async move {
                    for task in stream_tasks {
                        let _ = task.await;
                    }
                });
                *self.reader_handle.lock().await = Some(handle);
                Ok(port)
            }
            Err(e) => {
                for task in stream_tasks {
                    task.abort();
                }
                Err(e)
            }
        }
    }

    /// Extract port number from a llama-server log line.
    fn extract_port_from_line(line: &str) -> Option<u16> {
        // Match patterns like "127.0.0.1:8080" or "0.0.0.0:12345"
        // The port appears after the last colon in the address
        if !line.to_ascii_lowercase().contains("listening") {
            return None;
        }

        // Prefer explicit `port` token if present (covers modern llama.cpp logs
        // like `..., port: 44123, n_threads_http: 31`).
        if let Some(port_idx) = line.to_ascii_lowercase().rfind("port") {
            let tail = &line[port_idx + 4..];
            let digits: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(port) = digits.parse::<u16>() {
                if port >= 1024 {
                    return Some(port);
                }
            }
        }

        // Fallback for older formats that end with host:port.
        for segment in line.rsplit(':') {
            let trimmed = segment.trim_start();
            let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = digits.parse::<u16>() {
                if port >= 1024 {
                    return Some(port);
                }
            }
        }

        None
    }

    /// Wait for /health to return 200.
    /// Uses exponential backoff: 50 → 100 → 200 → 400 → 800ms (cap).
    async fn wait_for_health(&self, api_url: &str) -> anyhow::Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?;

        let health_url = api_url.replace("/v1", "/health");
        let health_timeout_secs = self.config.health_check_timeout_secs.max(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(health_timeout_secs);
        let mut backoff_ms: u64 = 50;

        loop {
            if tokio::time::Instant::now() > deadline {
                self.state.store(STATE_ERROR, Ordering::Release);
                return Err(anyhow::anyhow!(
                    "llama-server health check timed out after {}s",
                    health_timeout_secs
                ));
            }

            match client.get(&health_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return Ok(());
                }
                Ok(resp) => {
                    tracing::debug!(
                        status = %resp.status(),
                        "server_manager: health check not ready yet"
                    );
                }
                Err(e) => {
                    tracing::debug!(?e, "server_manager: health check connection error");
                }
            }

            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(800);
        }
    }

    // ─── API-Level Model Swap (Router Mode) ──────────────────────────────────
    //
    // These methods use llama.cpp Router Mode endpoints (`/v1/models/unload`
    // and `/v1/models/load`) to flush or restore model weights in VRAM without
    // killing the server process. The server stays alive on its ephemeral port
    // and no port re-discovery is needed.
    //
    // Prerequisites:
    // - llama-server must be built from llama.cpp b5291+ (Router Mode support)
    // - Server must be started in Router Mode (`--models-dir` instead of `--model`)
    //
    // When Router Mode is unavailable (older builds or single-model launch),
    // these methods return Err and the caller falls back to process restart.

    /// Unload the current model from VRAM via the Router Mode API.
    ///
    /// On success: model weights are flushed from GPU, VRAM is freed,
    /// server process stays alive, port stays stable.
    ///
    /// On failure (501/404 = Router Mode not available): returns Err.
    /// Caller should fall back to `graceful_stop` + `spawn`.
    pub async fn api_unload_model(&self) -> anyhow::Result<()> {
        let base_url = self.api_url();
        if base_url.is_empty() {
            tracing::error!(
                "server_manager: api_unload_model called but no API URL is set — \
                             server is not running. Caller will fall back to process kill."
            );
            anyhow::bail!("api_unload_model: no API URL (server not running)");
        }

        if self.router_models_dir().is_none() {
            anyhow::bail!(
                "api_unload_model: Router Mode not active (--models-dir unavailable). \
                 Falling back to process restart."
            );
        }

        if matches!(self.router_mode_supported_cached(), Some(false)) {
            anyhow::bail!(
                "api_unload_model: Router Mode not supported (cached HTTP 404/501). \
                 Falling back to process restart."
            );
        }

        // Router Mode endpoint (OpenAI-compatible prefix included).
        let models_url = v1_models_endpoint(&base_url, "unload");

        let request_timeout_secs = self.config.health_check_timeout_secs.clamp(1, 30);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(request_timeout_secs))
            .build()
            .unwrap_or_default();

        let model_id = self.router_model_id();
        let payload = serde_json::json!({ "model": model_id });

        tracing::debug!(
            url = %models_url,
            model = %self.router_model_id(),
            request_timeout_secs,
            "server_manager: API-level model unload requested"
        );

        self.state.store(STATE_SWAPPING, Ordering::Release);
        self.cancel_streams();

        let request_start = std::time::Instant::now();
        let resp = client
            .post(&models_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let elapsed_ms = request_start.elapsed().as_millis();
                self.state.store(STATE_READY, Ordering::Release);
                let timed_out = e.is_timeout();
                tracing::error!(
                    elapsed_ms,
                    timed_out,
                    error = %e,
                    url = %models_url,
                    "server_manager: api_unload_model HTTP request FAILED before fallback. \
                     This means the HTTP call never reached the server or timed out. \
                     Caller will now fall back to SIGTERM/SIGKILL process restart."
                );
                anyhow::anyhow!("api_unload_model: transport error after {elapsed_ms}ms: {e}")
            })?;

        let elapsed_ms = request_start.elapsed().as_millis();
        let status = resp.status();

        if status.as_u16() == 501 || status.as_u16() == 404 {
            // Router Mode not available — revert state and let caller fall back.
            // This is the EXPECTED path for single-model llama-server builds.
            let body = resp.text().await.unwrap_or_default();
            self.state.store(STATE_READY, Ordering::Release);
            let first_unsupported = self.mark_router_mode_unsupported();
            if first_unsupported {
                tracing::debug!(
                    status = status.as_u16(),
                    elapsed_ms,
                    url = %models_url,
                    body = %body,
                    "server_manager: api_unload_model received HTTP {} (Router Mode not supported). \
                     Falling back to legacy restart path. \
                     To enable zero-downtime swaps, upgrade llama-server to b5291+ and use --models-dir.",
                    status.as_u16()
                );
            } else {
                tracing::debug!(
                    status = status.as_u16(),
                    elapsed_ms,
                    "server_manager: api_unload_model skipping zero-downtime path (Router Mode unsupported, cached)"
                );
            }
            anyhow::bail!(
                "api_unload_model: Router Mode not supported (HTTP {status}). \
                 Falling back to process restart."
            );
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            self.state.store(STATE_READY, Ordering::Release);
            tracing::error!(
                status = status.as_u16(),
                elapsed_ms,
                body = %body,
                "server_manager: api_unload_model failed with unexpected HTTP status"
            );
            anyhow::bail!("api_unload_model: failed (HTTP {status}): {body}");
        }

        // Model unloaded — GPU is clear. Keep process alive.
        self.pre_api_unload_ngl
            .store(self.current_ngl.load(Ordering::Acquire), Ordering::Release);
        self.pre_api_unload_vision.store(
            self.current_vision.load(Ordering::Acquire),
            Ordering::Release,
        );
        self.current_ngl.store(0, Ordering::Release);
        self.current_vision.store(false, Ordering::Release);
        self.mark_router_mode_supported();
        // Stay in SWAPPING state — caller will transition to READY after reload
        self.swap_done.notify_waiters();

        tracing::info!(
            elapsed_ms,
            "server_manager: model unloaded via API (VRAM freed, process alive)"
        );
        Ok(())
    }

    /// Reload the model into VRAM via the Router Mode API.
    ///
    /// Inverse of `api_unload_model`. Pulls weights back onto GPU.
    /// The server process must still be alive (not killed).
    pub async fn api_load_model(&self) -> anyhow::Result<()> {
        let base_url = self.api_url();
        if base_url.is_empty() {
            anyhow::bail!("api_load_model: no API URL (server not running)");
        }

        if self.router_models_dir().is_none() {
            anyhow::bail!(
                "api_load_model: Router Mode not active (--models-dir unavailable). \
                 Falling back to process restart."
            );
        }

        if matches!(self.router_mode_supported_cached(), Some(false)) {
            anyhow::bail!(
                "api_load_model: Router Mode not supported (cached HTTP 404/501). \
                 Falling back to process restart."
            );
        }

        let models_url = v1_models_endpoint(&base_url, "load");

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120)) // Loading can take a while
            .build()
            .unwrap_or_default();

        let model_id = self.router_model_id();
        let payload = serde_json::json!({ "model": model_id });

        tracing::debug!(
            url = %models_url,
            model = %self.router_model_id(),
            "server_manager: API-level model load requested"
        );

        let resp = client
            .post(&models_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("api_load_model: transport error: {e}"))?;

        let status = resp.status();
        if status.as_u16() == 501 || status.as_u16() == 404 {
            let body = resp.text().await.unwrap_or_default();
            self.state.store(STATE_READY, Ordering::Release);
            let first_unsupported = self.mark_router_mode_unsupported();
            if first_unsupported {
                tracing::debug!(
                    status = status.as_u16(),
                    url = %models_url,
                    body = %body,
                    "server_manager: api_load_model received HTTP {} (Router Mode not supported). \
                     Falling back to legacy restart path.",
                    status.as_u16()
                );
            } else {
                tracing::debug!(
                    status = status.as_u16(),
                    "server_manager: api_load_model skipping zero-downtime path (Router Mode unsupported, cached)"
                );
            }
            anyhow::bail!(
                "api_load_model: Router Mode not supported (HTTP {status}). \
                 Falling back to process restart."
            );
        }

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            self.state.store(STATE_ERROR, Ordering::Release);
            anyhow::bail!("api_load_model: failed (HTTP {status}): {body}");
        }

        // Wait for health before marking ready
        let health_url = base_url.replace("/v1", "/health");
        if let Err(e) = self.wait_for_health(&health_url).await {
            self.state.store(STATE_ERROR, Ordering::Release);
            return Err(e);
        }

        // Restore logical runtime params that were intentionally set to
        // `ngl=0/vision=false` at unload time.
        let restored_ngl = self.pre_api_unload_ngl.load(Ordering::Acquire);
        let restored_vision = self.pre_api_unload_vision.load(Ordering::Acquire);
        if restored_ngl > 0 {
            self.current_ngl.store(restored_ngl, Ordering::Release);
        }
        self.mark_router_mode_supported();
        self.current_vision
            .store(restored_vision, Ordering::Release);
        self.state.store(STATE_READY, Ordering::Release);
        self.swap_done.notify_waiters();

        tracing::info!("server_manager: model reloaded via API (GPU-resident)");
        Ok(())
    }

    pub(crate) async fn run_warmup_completion(&self) -> anyhow::Result<()> {
        let base = self.api_url();
        if base.is_empty() {
            return Ok(());
        }

        let endpoint = format!("{}/chat/completions", base.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| anyhow::anyhow!("warmup client build failed: {e}"))?;

        let payload = serde_json::json!({
            "model": self.router_model_id(),
            "messages": [
                { "role": "system", "content": "kria-internal-warmup" },
                { "role": "user", "content": "hello" }
            ],
            "max_tokens": 1,
            "temperature": 0.0,
            "stream": false
        });

        let resp = client
            .post(&endpoint)
            .json(&payload)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("warmup transport error: {e}"))?;

        let status = resp.status();
        if status.is_success() {
            tracing::debug!("server_manager: warmup completion succeeded");
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("warmup completion failed (HTTP {}): {}", status, body);
        }
    }

    /// Graceful stop: send interrupt signal, wait for drain, then kill.
    pub async fn graceful_stop(&self) {
        self.graceful_stop_with_timeout(Duration::from_secs(
            self.config.graceful_stop_timeout_secs.max(1),
        ))
        .await;
    }

    /// Graceful stop with explicit timeout override.
    /// SIGTERM → wait(timeout) → SIGKILL → wait (via ChildGuard).
    pub async fn graceful_stop_with_timeout(&self, timeout: Duration) {
        self.state.store(STATE_SWAPPING, Ordering::Release);
        self.cancel_streams();

        // Drain the reader task before killing so we get any final log lines.
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }

        if let Some(mut guard) = self.child.lock().await.take() {
            guard.terminate(timeout).await;
        }

        self.state.store(STATE_STOPPED, Ordering::Release);
        self.current_vision.store(false, Ordering::Release);
        self.swap_done.notify_waiters();
    }

    /// Immediate kill (emergency path): SIGKILL → reap.
    pub async fn kill(&self) {
        self.state.store(STATE_SWAPPING, Ordering::Release);
        self.cancel_streams();

        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }

        if let Some(mut guard) = self.child.lock().await.take() {
            guard.force_kill().await;
        }

        self.state.store(STATE_STOPPED, Ordering::Release);
        self.current_vision.store(false, Ordering::Release);
        self.swap_done.notify_waiters();
    }

    /// Resolve the configured `--slot-save-path` to an absolute directory.
    /// Empty / unset config falls back to `<system_tmp>/kria_llama_slots`.
    pub fn resolve_slot_save_path(&self) -> std::path::PathBuf {
        let raw = self.config.slot_save_path.trim();
        if raw.is_empty() {
            std::env::temp_dir().join("kria_llama_slots")
        } else {
            std::path::PathBuf::from(raw)
        }
    }

    pub(crate) async fn save_active_slot(&self) -> bool {
        self.save_slot_kv(ACTIVE_SLOT_ID, ACTIVE_SLOT_FILENAME)
            .await
    }

    pub(crate) async fn restore_active_slot(&self) -> bool {
        self.restore_slot_kv(ACTIVE_SLOT_ID, ACTIVE_SLOT_FILENAME)
            .await
    }

    /// Best-effort: persist a single slot's KV cache to disk via
    /// `POST /slots/{id_slot}?action=save` (llama.cpp HTTP API).
    ///
    /// Used by the Tier B swap to preserve conversational context across
    /// the hard process restart that frees VRAM. Returns `true` on HTTP 200,
    /// `false` on any failure (logged at debug level — this is an
    /// optimisation, never a correctness requirement).
    pub async fn save_slot_kv(&self, slot_id: u32, filename: &str) -> bool {
        let api_url = self.api_url();
        if api_url.is_empty() {
            return false;
        }
        // api_url ends in "/v1" — slots endpoints are on the server root.
        let root = api_url.strip_suffix("/v1").unwrap_or(&api_url);
        let url = format!("{}/slots/{}?action=save", root, slot_id);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        match client
            .post(&url)
            .json(&serde_json::json!({ "filename": filename }))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(slot_id, filename, "server_manager: slot KV cache saved");
                true
            }
            Ok(resp) => {
                tracing::debug!(
                    slot_id,
                    filename,
                    status = %resp.status(),
                    "server_manager: slot save returned non-200 (continuing without snapshot)"
                );
                false
            }
            Err(e) => {
                tracing::debug!(?e, slot_id, "server_manager: slot save request failed");
                false
            }
        }
    }

    /// Best-effort counterpart to `save_slot_kv`. Restores the KV cache from
    /// disk into slot `slot_id` after the server has been respawned. Failure
    /// only means the user's prior turn context is lost — the next prompt
    /// will simply re-prefill from scratch.
    pub async fn restore_slot_kv(&self, slot_id: u32, filename: &str) -> bool {
        let api_url = self.api_url();
        if api_url.is_empty() {
            return false;
        }
        let root = api_url.strip_suffix("/v1").unwrap_or(&api_url);
        let url = format!("{}/slots/{}?action=restore", root, slot_id);
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        match client
            .post(&url)
            .json(&serde_json::json!({ "filename": filename }))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!(slot_id, filename, "server_manager: slot KV cache restored");
                true
            }
            Ok(resp) => {
                tracing::debug!(
                    slot_id,
                    filename,
                    status = %resp.status(),
                    "server_manager: slot restore returned non-200"
                );
                false
            }
            Err(e) => {
                tracing::debug!(?e, slot_id, "server_manager: slot restore request failed");
                false
            }
        }
    }
}

/// Pre-flight check: is it safe to pass `--mlock` to llama-server?
///
/// Returns `true` only when the host has at least
/// `model_size_bytes + 2 GiB` of *available* RAM (i.e. unused + reclaimable
/// page cache). On any I/O error or detection failure we fail-closed and
/// return `false`, so the worst case is "model loads without mlock" rather
/// than "system freezes".
fn mlock_is_safe(model_path: &str) -> bool {
    let model_size = match std::fs::metadata(model_path) {
        Ok(m) => m.len(),
        Err(e) => {
            tracing::debug!(?e, "mlock_is_safe: cannot stat model file");
            return false;
        }
    };

    // sysinfo::System::available_memory() returns bytes (sysinfo 0.32+).
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let available_bytes = sys.available_memory();

    let headroom: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
    let needed = model_size.saturating_add(headroom);
    let safe = available_bytes >= needed;

    tracing::debug!(
        model_size_mb = model_size / (1024 * 1024),
        available_mb = available_bytes / (1024 * 1024),
        needed_mb = needed / (1024 * 1024),
        safe,
        "mlock_is_safe: pre-flight RAM check"
    );
    safe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_port_from_standard_line() {
        let line = "main: server is listening on http://127.0.0.1:43567";
        assert_eq!(
            LlamaServerManager::extract_port_from_line(line),
            Some(43567)
        );
    }

    #[test]
    fn extract_port_from_plain_line() {
        let line = "server is listening on 0.0.0.0:8080";
        assert_eq!(LlamaServerManager::extract_port_from_line(line), Some(8080));
    }

    #[test]
    fn extract_port_from_line_with_space_after_colon() {
        let line = "srv  listen: HTTP server is listening, hostname: 127.0.0.1, port: 44123, n_threads_http: 31";
        assert_eq!(
            LlamaServerManager::extract_port_from_line(line),
            Some(44123)
        );
    }

    #[test]
    fn extract_port_from_port_word_format() {
        let line = "main: server is listening, host 127.0.0.1, port 45321";
        assert_eq!(
            LlamaServerManager::extract_port_from_line(line),
            Some(45321)
        );
    }

    #[test]
    fn no_port_from_unrelated_line() {
        let line = "model loaded successfully in 2.3s";
        assert_eq!(LlamaServerManager::extract_port_from_line(line), None);
    }

    #[test]
    fn state_transitions() {
        let config = OrchestratorConfig::default();
        let mgr = LlamaServerManager::new(config, "/tmp/model.gguf".into(), None);
        assert_eq!(mgr.state(), STATE_STOPPED);
        mgr.state.store(STATE_READY, Ordering::Release);
        assert!(mgr.is_healthy());
        mgr.state.store(STATE_SWAPPING, Ordering::Release);
        assert!(mgr.is_swapping());
        assert!(!mgr.is_healthy());
    }

    #[test]
    fn launch_tuning_uses_conservative_profile_for_vision() {
        let tuning = launch_tuning(512, true);
        assert_eq!(tuning.batch_size, 128);
        assert_eq!(tuning.ubatch_size, Some(128));
        assert_eq!(tuning.parallel, Some(1));
        assert!(tuning.no_warmup);
    }

    #[test]
    fn launch_tuning_preserves_config_for_non_vision() {
        let tuning = launch_tuning(256, false);
        assert_eq!(tuning.batch_size, 256);
        assert_eq!(tuning.ubatch_size, None);
        assert_eq!(tuning.parallel, None);
        assert!(!tuning.no_warmup);
    }

    #[test]
    fn mmproj_cpu_only_branches() {
        // Serialize env mutation within this single test to avoid cross-test
        // interference (no other test touches KRIA_MMPROJ_GPU_OFFLOAD).
        let key = "KRIA_MMPROJ_GPU_OFFLOAD";
        let prev = std::env::var(key).ok();

        // Default (unset) => clip stays on CPU (safe default).
        std::env::remove_var(key);
        assert!(mmproj_cpu_only(), "unset must default to CPU-resident clip");

        // Truthy values => allow GPU offload => NOT cpu-only.
        for truthy in ["1", "true", "TRUE", "Yes", " on "] {
            std::env::set_var(key, truthy);
            assert!(
                !mmproj_cpu_only(),
                "{truthy:?} must opt into GPU offload (not cpu-only)"
            );
        }

        // Falsy / unrecognized values => keep clip on CPU.
        for falsy in ["0", "false", "no", "off", ""] {
            std::env::set_var(key, falsy);
            assert!(
                mmproj_cpu_only(),
                "{falsy:?} must keep clip CPU-resident"
            );
        }

        // Restore prior environment.
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}
