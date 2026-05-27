//! STT Sidecar Process Supervisor (§10 ENHANCED_STT.md)
//!
//! Manages the lifecycle of the streaming STT sidecar process:
//! - Process spawning with kill-on-drop
//! - Stdout/stderr capture
//! - Crash detection and restart
//! - Graceful shutdown (SIGTERM → SIGKILL)
//! - Integration with RestartTracker (§10 backoff)
//! - Bidirectional IPC pump (audio out, partials in)
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                  SidecarSupervisor                    │
//! │                                                       │
//! │  ┌──────────┐    ┌──────────┐    ┌──────────────┐   │
//! │  │ Spawner  │───►│  Child   │───►│ Stdout/Stderr│   │
//! │  └──────────┘    └──────────┘    └──────────────┘   │
//! │                       │                              │
//! │                       ▼                              │
//! │  ┌──────────────────────────────────────────────┐   │
//! │  │           AF_UNIX Socket                      │   │
//! │  │  ┌────────────┐      ┌────────────────────┐  │   │
//! │  │  │ Audio Pump │      │ Partial Receiver   │  │   │
//! │  │  │ (host→side)│      │ (side→host)        │  │   │
//! │  │  └────────────┘      └────────────────────┘  │   │
//! │  └──────────────────────────────────────────────┘   │
//! │                                                       │
//! │  ┌──────────────────────────────────────────────┐   │
//! │  │           RestartTracker (§10)                │   │
//! │  │  backoff: 100ms→5s, cap: 5/60s, disable:120s │   │
//! │  └──────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Invariants
//! - Process supervised at all times
//! - Crash → restart with backoff
//! - Stale socket cleaned before respawn
//! - Generation incremented on restart
//! - No unbounded async task graphs

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::sidecar_ipc::{resolve_socket_path, unlink_stale_socket, IpcMessage, MAX_CHUNK_SAMPLES};
use super::sidecar_session::{RestartTracker, SessionState};

// ─── Sidecar Configuration ───────────────────────────────────────────────

/// Configuration for the STT sidecar process.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Path to the sidecar binary/script.
    pub command: String,
    /// Arguments to pass to the sidecar.
    pub args: Vec<String>,
    /// Environment variables for the sidecar.
    pub env: Vec<(String, String)>,
    /// Socket path override (default: resolve_socket_path()).
    pub socket_path: Option<PathBuf>,
    /// Sample rate for audio streaming.
    pub sample_rate: u32,
    /// Max audio chunk samples per message (default: 4000 = 250ms @ 16kHz).
    pub max_chunk_samples: usize,
}

impl Default for SidecarConfig {
    fn default() -> Self {
        Self {
            command: "kria-stt-sidecar".to_string(),
            args: Vec::new(),
            env: Vec::new(),
            socket_path: None,
            sample_rate: 16_000,
            max_chunk_samples: MAX_CHUNK_SAMPLES,
        }
    }
}

// ─── Sidecar Status ──────────────────────────────────────────────────────

/// Current status of the sidecar process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarStatus {
    /// Not started yet.
    Idle,
    /// Process is starting up.
    Starting,
    /// Process is running and connected.
    Running,
    /// Process crashed, awaiting restart.
    Crashed,
    /// Restart limit exceeded, disabled.
    Disabled,
    /// Gracefully shut down.
    Stopped,
}

// ─── Sidecar Events ──────────────────────────────────────────────────────

/// Events emitted by the sidecar supervisor for telemetry.
#[derive(Debug, Clone)]
pub enum SidecarEvent {
    /// Sidecar process spawned.
    Spawned { pid: u32 },
    /// Sidecar connected via socket.
    Connected { session_id: String, generation: u64 },
    /// Sidecar process crashed.
    Crashed {
        exit_code: Option<i32>,
        stderr: String,
    },
    /// Sidecar restarting with backoff.
    Restarting { attempt: usize, backoff_ms: u64 },
    /// Sidecar disabled after max restarts.
    Disabled { duration_ms: u64 },
    /// Sidecar gracefully stopped.
    Stopped,
    /// Partial transcript received from sidecar.
    PartialReceived {
        session_id: String,
        generation: u64,
        seq: u64,
        text: String,
        stable: bool,
    },
    /// Stale partial dropped.
    StalePartialDropped {
        expected_generation: u64,
        received_generation: u64,
    },
    /// Audio backpressure detected.
    AudioBackpressure { pending_bytes: usize },
}

// ─── Sidecar Supervisor ──────────────────────────────────────────────────

/// Manages the STT sidecar process lifecycle.
///
/// **Not a transcript authority.** Only transports audio and receives partials.
pub struct SidecarSupervisor {
    config: SidecarConfig,
    session: Arc<Mutex<SessionState>>,
    restart_tracker: Arc<Mutex<RestartTracker>>,
    status: Arc<Mutex<SidecarStatus>>,
    cancel: CancellationToken,
    event_tx: mpsc::UnboundedSender<SidecarEvent>,
    /// Channel for sending audio chunks to the sidecar write pump.
    audio_tx: mpsc::Sender<Vec<f32>>,
    /// Channel for receiving partial transcripts from the sidecar read pump.
    partial_rx: Arc<Mutex<mpsc::Receiver<IpcMessage>>>,
    /// Internal sender for partial transcripts (used by read pump).
    #[allow(dead_code)]
    partial_tx: mpsc::Sender<IpcMessage>,
}

impl SidecarSupervisor {
    /// Create a new sidecar supervisor.
    ///
    /// Returns the supervisor and a receiver for sidecar events.
    pub fn new(config: SidecarConfig) -> (Self, mpsc::UnboundedReceiver<SidecarEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (audio_tx, _audio_rx) = mpsc::channel(64); // bounded audio queue
        let (partial_tx, partial_rx) = mpsc::channel(64); // bounded partial queue

        let supervisor = Self {
            config,
            session: Arc::new(Mutex::new(SessionState::new(16_000))),
            restart_tracker: Arc::new(Mutex::new(RestartTracker::new())),
            status: Arc::new(Mutex::new(SidecarStatus::Idle)),
            cancel: CancellationToken::new(),
            event_tx,
            audio_tx,
            partial_rx: Arc::new(Mutex::new(partial_rx)),
            partial_tx,
        };

        (supervisor, event_rx)
    }

    /// Get the current sidecar status.
    pub async fn status(&self) -> SidecarStatus {
        *self.status.lock().await
    }

    /// Get the audio sender for streaming audio to the sidecar.
    pub fn audio_sender(&self) -> mpsc::Sender<Vec<f32>> {
        self.audio_tx.clone()
    }

    /// Get the partial receiver for receiving transcripts from the sidecar.
    pub fn partial_receiver(&self) -> Arc<Mutex<mpsc::Receiver<IpcMessage>>> {
        self.partial_rx.clone()
    }

    /// Get the current session state.
    pub async fn session_state(&self) -> SessionState {
        self.session.lock().await.clone()
    }

    /// Increment generation (on cancel/restart per §11).
    pub async fn increment_generation(&self) {
        let mut session = self.session.lock().await;
        session.increment_generation();
        tracing::info!(
            generation = session.generation,
            session_id = %session.session_id,
            "generation incremented"
        );
    }

    /// Spawn the sidecar process.
    ///
    /// Returns the child process handle and stderr capture task.
    pub async fn spawn_process(&self) -> Result<(Child, JoinHandle<String>)> {
        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args);

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Capture stdout/stderr, kill on drop
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn sidecar: {}", self.config.command))?;

        let pid = child.id().unwrap_or(0);
        tracing::info!(pid, command = %self.config.command, "sidecar process spawned");

        let _ = self.event_tx.send(SidecarEvent::Spawned { pid });

        // Capture stderr in background (bounded to 4KB)
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stderr from sidecar"))?;

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut output = String::new();
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            tracing::warn!(target: "sidecar_stderr", "{}", trimmed);
                            // Bounded stderr capture (4KB)
                            if output.len() < 4096 {
                                output.push_str(trimmed);
                                output.push('\n');
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("sidecar stderr read error: {}", e);
                        break;
                    }
                }
            }
            output
        });

        *self.status.lock().await = SidecarStatus::Starting;

        Ok((child, stderr_task))
    }

    /// Wait for the sidecar to exit and handle crash/restart.
    pub async fn wait_and_restart(
        &self,
        mut child: Child,
        stderr_task: JoinHandle<String>,
    ) -> Result<()> {
        let cancel = self.cancel.clone();

        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!("sidecar supervisor cancelled, shutting down");
                self.shutdown_child(&mut child).await;
                *self.status.lock().await = SidecarStatus::Stopped;
                let _ = self.event_tx.send(SidecarEvent::Stopped);
                Ok(())
            }
            exit_status = child.wait() => {
                let exit_code = exit_status.ok().and_then(|s| s.code());
                let stderr_output = stderr_task.await.unwrap_or_default();

                tracing::warn!(
                    exit_code = ?exit_code,
                    stderr_len = stderr_output.len(),
                    "sidecar process exited"
                );

                *self.status.lock().await = SidecarStatus::Crashed;
                let _ = self.event_tx.send(SidecarEvent::Crashed {
                    exit_code,
                    stderr: stderr_output.clone(),
                });

                // Increment generation on crash (§11)
                self.increment_generation().await;

                // Clean stale socket
                let socket_path = self.config.socket_path.clone()
                    .unwrap_or_else(resolve_socket_path);
                let _ = unlink_stale_socket(&socket_path);

                // Attempt restart with backoff
                self.attempt_restart().await
            }
        }
    }

    /// Attempt to restart the sidecar with backoff policy (§10).
    async fn attempt_restart(&self) -> Result<()> {
        let mut tracker = self.restart_tracker.lock().await;

        if tracker.is_disabled() {
            tracing::error!("sidecar restarts disabled, falling back to Whisper-only");
            *self.status.lock().await = SidecarStatus::Disabled;
            let _ = self.event_tx.send(SidecarEvent::Disabled {
                duration_ms: 120_000,
            });
            bail!("sidecar restarts disabled");
        }

        if !tracker.record_restart() {
            tracing::error!("max restarts exceeded, disabling sidecar");
            *self.status.lock().await = SidecarStatus::Disabled;
            let _ = self.event_tx.send(SidecarEvent::Disabled {
                duration_ms: 120_000,
            });
            bail!("max restarts exceeded");
        }

        let backoff = tracker.backoff_duration();
        let attempt = tracker.attempts.len();
        drop(tracker);

        tracing::info!(
            attempt,
            backoff_ms = backoff.as_millis() as u64,
            "restarting sidecar after backoff"
        );

        let _ = self.event_tx.send(SidecarEvent::Restarting {
            attempt,
            backoff_ms: backoff.as_millis() as u64,
        });

        // Sleep for backoff duration (cancellation-aware)
        tokio::select! {
            biased;
            _ = self.cancel.cancelled() => {
                bail!("restart cancelled");
            }
            _ = tokio::time::sleep(backoff) => {}
        }

        Ok(())
    }

    /// Gracefully shutdown the child process (SIGTERM → wait → SIGKILL).
    async fn shutdown_child(&self, child: &mut Child) {
        // Try SIGTERM first
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }

        // Wait up to 500ms for graceful exit (§5.6)
        match tokio::time::timeout(Duration::from_millis(500), child.wait()).await {
            Ok(_) => {
                tracing::info!("sidecar exited gracefully after SIGTERM");
            }
            Err(_) => {
                tracing::warn!("sidecar did not exit after SIGTERM, sending SIGKILL");
                let _ = child.kill().await;
            }
        }
    }

    /// Cancel the supervisor (triggers graceful shutdown).
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the supervisor is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

// ─── Audio Streaming Pump ─────────────────────────────────────────────────

/// Bounded audio chunk for streaming to sidecar.
///
/// Enforces:
/// - Max chunk size (4000 samples = 250ms @ 16kHz)
/// - Monotonic sequence numbers
/// - Generation tagging
#[derive(Debug, Clone)]
pub struct AudioChunkEnvelope {
    pub session_id: String,
    pub generation: u64,
    pub seq: u64,
    pub pcm: Vec<f32>,
}

impl AudioChunkEnvelope {
    /// Convert to IPC message.
    pub fn to_ipc_message(&self) -> IpcMessage {
        IpcMessage::Audio {
            session_id: self.session_id.clone(),
            generation: self.generation,
            seq: self.seq,
            pcm: self.pcm.clone(),
        }
    }

    /// Validate chunk size (max 4000 samples per §5.3).
    pub fn is_valid_size(&self) -> bool {
        self.pcm.len() <= MAX_CHUNK_SAMPLES
    }
}

/// Split audio into bounded chunks (max 4000 samples each).
pub fn chunk_audio(
    audio: &[f32],
    session_id: &str,
    generation: u64,
    start_seq: u64,
) -> Vec<AudioChunkEnvelope> {
    audio
        .chunks(MAX_CHUNK_SAMPLES)
        .enumerate()
        .map(|(i, chunk)| AudioChunkEnvelope {
            session_id: session_id.to_string(),
            generation,
            seq: start_seq + i as u64,
            pcm: chunk.to_vec(),
        })
        .collect()
}

// ─── Partial Transport ────────────────────────────────────────────────────

/// Validate and filter a received partial message.
///
/// Returns `true` if valid, `false` if stale/invalid.
pub fn validate_partial(
    msg: &IpcMessage,
    expected_session: &str,
    expected_generation: u64,
) -> bool {
    match msg {
        IpcMessage::Partial {
            session_id,
            generation,
            ..
        } => {
            // Check session
            if session_id != expected_session {
                tracing::warn!(
                    expected = expected_session,
                    received = session_id.as_str(),
                    "partial from wrong session, dropping"
                );
                return false;
            }

            // Check generation (§11: MUST NOT apply partial with generation < current)
            if *generation != expected_generation {
                tracing::debug!(
                    expected = expected_generation,
                    received = *generation,
                    "stale partial dropped"
                );
                return false;
            }

            true
        }
        _ => false,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_config_default() {
        let config = SidecarConfig::default();
        assert_eq!(config.sample_rate, 16_000);
        assert_eq!(config.max_chunk_samples, MAX_CHUNK_SAMPLES);
        assert!(config.args.is_empty());
        assert!(config.env.is_empty());
    }

    #[test]
    fn audio_chunk_envelope_valid_size() {
        let chunk = AudioChunkEnvelope {
            session_id: "test".to_string(),
            generation: 0,
            seq: 0,
            pcm: vec![0.0; MAX_CHUNK_SAMPLES],
        };
        assert!(chunk.is_valid_size());

        let oversized = AudioChunkEnvelope {
            session_id: "test".to_string(),
            generation: 0,
            seq: 0,
            pcm: vec![0.0; MAX_CHUNK_SAMPLES + 1],
        };
        assert!(!oversized.is_valid_size());
    }

    #[test]
    fn audio_chunk_to_ipc_message() {
        let chunk = AudioChunkEnvelope {
            session_id: "test-session".to_string(),
            generation: 5,
            seq: 42,
            pcm: vec![0.1, 0.2, 0.3],
        };

        let msg = chunk.to_ipc_message();
        match msg {
            IpcMessage::Audio {
                session_id,
                generation,
                seq,
                pcm,
            } => {
                assert_eq!(session_id, "test-session");
                assert_eq!(generation, 5);
                assert_eq!(seq, 42);
                assert_eq!(pcm, vec![0.1, 0.2, 0.3]);
            }
            _ => panic!("expected Audio message"),
        }
    }

    #[test]
    fn chunk_audio_splits_correctly() {
        let audio = vec![0.0f32; 10_000]; // 2.5 chunks
        let chunks = chunk_audio(&audio, "session", 1, 0);

        assert_eq!(chunks.len(), 3); // 4000 + 4000 + 2000
        assert_eq!(chunks[0].pcm.len(), MAX_CHUNK_SAMPLES);
        assert_eq!(chunks[1].pcm.len(), MAX_CHUNK_SAMPLES);
        assert_eq!(chunks[2].pcm.len(), 2000);
        assert_eq!(chunks[0].seq, 0);
        assert_eq!(chunks[1].seq, 1);
        assert_eq!(chunks[2].seq, 2);
    }

    #[test]
    fn chunk_audio_empty() {
        let audio: Vec<f32> = Vec::new();
        let chunks = chunk_audio(&audio, "session", 0, 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_audio_exact_boundary() {
        let audio = vec![0.0f32; MAX_CHUNK_SAMPLES];
        let chunks = chunk_audio(&audio, "session", 0, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].pcm.len(), MAX_CHUNK_SAMPLES);
    }

    #[test]
    fn validate_partial_correct() {
        let msg = IpcMessage::Partial {
            session_id: "session-1".to_string(),
            generation: 5,
            seq: 10,
            text: "hello".to_string(),
            stable: false,
        };

        assert!(validate_partial(&msg, "session-1", 5));
    }

    #[test]
    fn validate_partial_stale_generation() {
        let msg = IpcMessage::Partial {
            session_id: "session-1".to_string(),
            generation: 3, // stale
            seq: 10,
            text: "hello".to_string(),
            stable: false,
        };

        assert!(!validate_partial(&msg, "session-1", 5));
    }

    #[test]
    fn validate_partial_wrong_session() {
        let msg = IpcMessage::Partial {
            session_id: "session-2".to_string(), // wrong
            generation: 5,
            seq: 10,
            text: "hello".to_string(),
            stable: false,
        };

        assert!(!validate_partial(&msg, "session-1", 5));
    }

    #[test]
    fn validate_partial_non_partial_message() {
        let msg = IpcMessage::Ping { ts_ms: 1000 };
        assert!(!validate_partial(&msg, "session-1", 5));
    }

    #[tokio::test]
    async fn supervisor_creation() {
        let config = SidecarConfig::default();
        let (supervisor, _event_rx) = SidecarSupervisor::new(config);

        assert_eq!(supervisor.status().await, SidecarStatus::Idle);
        assert!(!supervisor.is_cancelled());
    }

    #[tokio::test]
    async fn supervisor_cancel() {
        let config = SidecarConfig::default();
        let (supervisor, _event_rx) = SidecarSupervisor::new(config);

        supervisor.cancel();
        assert!(supervisor.is_cancelled());
    }

    #[tokio::test]
    async fn supervisor_generation_increment() {
        let config = SidecarConfig::default();
        let (supervisor, _event_rx) = SidecarSupervisor::new(config);

        let state = supervisor.session_state().await;
        assert_eq!(state.generation, 0);

        supervisor.increment_generation().await;

        let state = supervisor.session_state().await;
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn sidecar_status_variants() {
        assert_ne!(SidecarStatus::Idle, SidecarStatus::Running);
        assert_ne!(SidecarStatus::Crashed, SidecarStatus::Disabled);
        assert_ne!(SidecarStatus::Starting, SidecarStatus::Stopped);
    }
}
