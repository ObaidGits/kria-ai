//! Sidecar session runtime — socket lifecycle, heartbeat, supervision (§5, §10, §11)
//!
//! **Bounded, deterministic sidecar session management.**
//!
//! ## Session Lifecycle
//! - One session_id per session (P2-R1)
//! - Generation host-owned, monotonic (P2-R2)
//! - Stale generations dropped (P2-R3)
//!
//! ## Heartbeat
//! - Ping every 5s (P2-R7)
//! - Pong within 1s (P2-R7)
//! - 3 consecutive missing pongs → restart
//!
//! ## Supervision
//! - Exponential backoff: 100ms base, 5s cap (§10)
//! - Max 5 restarts per 60s window (§10)
//! - After max: disable 120s, fallback Whisper-only

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::sidecar_ipc::{
    read_frame, unlink_stale_socket, write_frame, IpcMessage, HEARTBEAT_PING_INTERVAL_MS,
    HEARTBEAT_PONG_TIMEOUT_MS, MAX_AUDIO_BUFFER_BYTES, MAX_PENDING_MESSAGES,
};

// ─── Supervision Constants (§10) ──────────────────────────────────────────

/// Restart backoff base (100ms per §10)
const RESTART_BACKOFF_BASE_MS: u64 = 100;

/// Restart backoff cap (5s per §10)
const RESTART_BACKOFF_CAP_MS: u64 = 5_000;

/// Max restarts per window (5 per §10)
const MAX_RESTARTS_PER_WINDOW: usize = 5;

/// Restart window duration (60s per §10)
const RESTART_WINDOW_DURATION_MS: u64 = 60_000;

/// Disable duration after max restarts (120s per §10)
const DISABLE_DURATION_MS: u64 = 120_000;

/// Consecutive missing pongs before restart (3 per §5.4)
const MAX_MISSING_PONGS: usize = 3;

// ─── Session State ────────────────────────────────────────────────────────

/// Sidecar session state.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub session_id: String,
    pub generation: u64,
    pub sample_rate: u32,
    pub connected: bool,
    pub last_pong: Option<Instant>,
    pub missing_pongs: usize,
}

impl SessionState {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            session_id: Uuid::new_v4().to_string(),
            generation: 0,
            sample_rate,
            connected: false,
            last_pong: None,
            missing_pongs: 0,
        }
    }

    /// Increment generation (wrapping add per P1 pattern).
    pub fn increment_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Check if a message generation is stale.
    pub fn is_stale(&self, msg_generation: u64) -> bool {
        msg_generation != self.generation
    }
}

// ─── Restart Tracker (§10) ────────────────────────────────────────────────

/// Tracks restart attempts with exponential backoff and window limits.
#[derive(Debug)]
pub struct RestartTracker {
    pub attempts: Vec<Instant>,
    pub backoff_count: usize,
    disabled_until: Option<Instant>,
}

impl RestartTracker {
    pub fn new() -> Self {
        Self {
            attempts: Vec::new(),
            backoff_count: 0,
            disabled_until: None,
        }
    }

    /// Check if restarts are currently disabled.
    pub fn is_disabled(&self) -> bool {
        if let Some(until) = self.disabled_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Record a restart attempt. Returns true if restart is allowed.
    pub fn record_restart(&mut self) -> bool {
        let now = Instant::now();

        // Check if disabled
        if self.is_disabled() {
            return false;
        }

        // Clean old attempts outside window
        let window_start = now - Duration::from_millis(RESTART_WINDOW_DURATION_MS);
        self.attempts.retain(|&t| t > window_start);

        // Check if at capacity
        if self.attempts.len() >= MAX_RESTARTS_PER_WINDOW {
            tracing::warn!(
                attempts = self.attempts.len(),
                window_ms = RESTART_WINDOW_DURATION_MS,
                "max restarts per window exceeded, disabling sidecar"
            );
            self.disabled_until = Some(now + Duration::from_millis(DISABLE_DURATION_MS));
            return false;
        }

        // Record attempt
        self.attempts.push(now);
        self.backoff_count += 1;

        true
    }

    /// Get the current backoff duration (exponential with cap).
    pub fn backoff_duration(&self) -> Duration {
        let backoff_ms = RESTART_BACKOFF_BASE_MS
            .saturating_mul(2u64.saturating_pow(self.backoff_count as u32))
            .min(RESTART_BACKOFF_CAP_MS);
        Duration::from_millis(backoff_ms)
    }

    /// Reset backoff counter on successful connection.
    pub fn reset_backoff(&mut self) {
        self.backoff_count = 0;
    }
}

// ─── Bounded Message Queue ────────────────────────────────────────────────

/// Bounded message queue with backpressure telemetry.
pub struct BoundedMessageQueue {
    tx: mpsc::Sender<IpcMessage>,
    rx: mpsc::Receiver<IpcMessage>,
    audio_bytes: Arc<Mutex<usize>>,
}

impl BoundedMessageQueue {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(MAX_PENDING_MESSAGES);
        Self {
            tx,
            rx,
            audio_bytes: Arc::new(Mutex::new(0)),
        }
    }

    /// Send a message with backpressure handling.
    pub async fn send(&self, msg: IpcMessage) -> Result<()> {
        // Track audio bytes
        if let IpcMessage::Audio { ref pcm, .. } = msg {
            let bytes = pcm.len() * std::mem::size_of::<f32>();
            let mut audio_bytes = self.audio_bytes.lock().await;
            *audio_bytes += bytes;

            if *audio_bytes > MAX_AUDIO_BUFFER_BYTES {
                tracing::warn!(
                    audio_bytes = *audio_bytes,
                    max = MAX_AUDIO_BUFFER_BYTES,
                    "audio buffer exceeded, backpressure"
                );
                // Emit backpressure telemetry (§8)
                // In production, this would emit a structured event
            }
        }

        // Try send with timeout (§5.5)
        let timeout = Duration::from_millis(super::sidecar_ipc::BACKPRESSURE_TIMEOUT_MS);
        match tokio::time::timeout(timeout, self.tx.send(msg)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => bail!("message queue closed"),
            Err(_) => {
                tracing::warn!("message send timed out after {}ms", timeout.as_millis());
                bail!("message send timeout")
            }
        }
    }

    /// Receive a message.
    pub async fn recv(&mut self) -> Option<IpcMessage> {
        let msg = self.rx.recv().await;

        // Update audio bytes
        if let Some(IpcMessage::Audio { ref pcm, .. }) = msg {
            let bytes = pcm.len() * std::mem::size_of::<f32>();
            let mut audio_bytes = self.audio_bytes.lock().await;
            *audio_bytes = audio_bytes.saturating_sub(bytes);
        }

        msg
    }

    pub fn sender(&self) -> mpsc::Sender<IpcMessage> {
        self.tx.clone()
    }
}

// ─── Socket Helpers ───────────────────────────────────────────────────────

/// Create a secure Unix socket listener (§5.1).
pub async fn create_listener(socket_path: &PathBuf) -> Result<UnixListener> {
    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create socket directory: {:?}", parent))?;
    }

    // Unlink stale socket
    unlink_stale_socket(socket_path)?;

    // Bind listener
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("failed to bind Unix socket: {:?}", socket_path))?;

    tracing::info!(path = ?socket_path, "sidecar listener bound");

    Ok(listener)
}

/// Connect to a Unix socket with timeout.
pub async fn connect_with_timeout(socket_path: &PathBuf, timeout: Duration) -> Result<UnixStream> {
    tokio::time::timeout(timeout, UnixStream::connect(socket_path))
        .await
        .context("connection timeout")?
        .with_context(|| format!("failed to connect to socket: {:?}", socket_path))
}

// ─── Session Handshake ────────────────────────────────────────────────────

/// Perform hello handshake (§5.3).
pub async fn handshake_hello<S>(stream: &mut S, state: &SessionState) -> Result<IpcMessage>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Send hello
    let hello = IpcMessage::Hello {
        proto: "0.1".to_string(),
        session_id: state.session_id.clone(),
        sample_rate: state.sample_rate,
        generation: state.generation,
    };

    write_frame(stream, &hello).await?;
    tracing::debug!(
        session_id = %state.session_id,
        generation = state.generation,
        "sent hello"
    );

    // Wait for hello_ack
    let response = read_frame(stream).await?;

    match response {
        IpcMessage::HelloAck { .. } => {
            tracing::info!(
                session_id = %state.session_id,
                generation = state.generation,
                "handshake complete"
            );
            Ok(response)
        }
        IpcMessage::Error { code, fatal } => {
            bail!("handshake error: {} (fatal: {})", code, fatal);
        }
        _ => {
            bail!("unexpected handshake response: {:?}", response);
        }
    }
}

/// Perform bye shutdown (§5.3).
pub async fn handshake_bye<S>(stream: &mut S, state: &SessionState) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let bye = IpcMessage::Bye {
        session_id: state.session_id.clone(),
        generation: state.generation,
    };

    write_frame(stream, &bye).await?;
    tracing::debug!(
        session_id = %state.session_id,
        generation = state.generation,
        "sent bye"
    );

    // Wait for bye_ack with timeout (500ms per §5.6)
    let timeout = Duration::from_millis(500);
    match tokio::time::timeout(timeout, read_frame(stream)).await {
        Ok(Ok(IpcMessage::ByeAck)) => {
            tracing::info!(
                session_id = %state.session_id,
                "bye acknowledged"
            );
            Ok(())
        }
        Ok(Ok(msg)) => {
            tracing::warn!(?msg, "unexpected bye response");
            Ok(())
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "bye read error");
            Ok(())
        }
        Err(_) => {
            tracing::warn!("bye_ack timeout");
            Ok(())
        }
    }
}

// ─── Heartbeat Task ───────────────────────────────────────────────────────

/// Spawn heartbeat task (ping every 5s, detect missing pongs).
pub fn spawn_heartbeat_task<S>(
    mut stream: S,
    state: Arc<Mutex<SessionState>>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let ping_interval = Duration::from_millis(HEARTBEAT_PING_INTERVAL_MS);
        let pong_timeout = Duration::from_millis(HEARTBEAT_PONG_TIMEOUT_MS);
        let mut interval = tokio::time::interval(ping_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::debug!("heartbeat task cancelled");
                    break;
                }
                _ = interval.tick() => {
                    // Send ping
                    let ts_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64;

                    let ping = IpcMessage::Ping { ts_ms };

                    if let Err(e) = write_frame(&mut stream, &ping).await {
                        tracing::warn!(error = %e, "failed to send ping");
                        break;
                    }

                    // Wait for pong
                    match tokio::time::timeout(pong_timeout, read_frame(&mut stream)).await {
                        Ok(Ok(IpcMessage::Pong { .. })) => {
                            // Pong received
                            let mut state = state.lock().await;
                            state.last_pong = Some(Instant::now());
                            state.missing_pongs = 0;
                            tracing::trace!("pong received");
                        }
                        Ok(Ok(msg)) => {
                            tracing::warn!(?msg, "unexpected heartbeat response");
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "heartbeat read error");
                            let mut state = state.lock().await;
                            state.missing_pongs += 1;
                            if state.missing_pongs >= MAX_MISSING_PONGS {
                                tracing::error!(
                                    missing = state.missing_pongs,
                                    "max missing pongs exceeded, sidecar dead"
                                );
                                break;
                            }
                        }
                        Err(_) => {
                            // Pong timeout
                            let mut state = state.lock().await;
                            state.missing_pongs += 1;
                            tracing::warn!(
                                missing = state.missing_pongs,
                                max = MAX_MISSING_PONGS,
                                "pong timeout"
                            );
                            if state.missing_pongs >= MAX_MISSING_PONGS {
                                tracing::error!(
                                    missing = state.missing_pongs,
                                    "max missing pongs exceeded, sidecar dead"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("heartbeat task exiting");
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_state_generation_increment() {
        let mut state = SessionState::new(16000);
        assert_eq!(state.generation, 0);

        state.increment_generation();
        assert_eq!(state.generation, 1);

        // Test wrapping
        state.generation = u64::MAX;
        state.increment_generation();
        assert_eq!(state.generation, 0);
    }

    #[test]
    fn session_state_stale_detection() {
        let state = SessionState::new(16000);
        assert!(!state.is_stale(0));
        assert!(state.is_stale(1));
    }

    #[test]
    fn restart_tracker_backoff() {
        let mut tracker = RestartTracker::new();

        // Initial: backoff_count=0, duration = 100 * 2^0 = 100ms
        assert_eq!(tracker.backoff_duration().as_millis(), 100);

        // First restart: backoff_count=1, duration = 100 * 2^1 = 200ms
        assert!(tracker.record_restart());
        assert_eq!(tracker.backoff_duration().as_millis(), 200);

        // Second restart: backoff_count=2, duration = 100 * 2^2 = 400ms
        assert!(tracker.record_restart());
        assert_eq!(tracker.backoff_duration().as_millis(), 400);

        // Third restart: backoff_count=3, duration = 100 * 2^3 = 800ms
        assert!(tracker.record_restart());
        assert_eq!(tracker.backoff_duration().as_millis(), 800);

        // Fourth restart: backoff_count=4, duration = 100 * 2^4 = 1600ms
        assert!(tracker.record_restart());
        assert_eq!(tracker.backoff_duration().as_millis(), 1600);

        // Fifth restart: backoff_count=5, duration = 100 * 2^5 = 3200ms
        // This is the last one before hitting the window limit
        assert!(tracker.record_restart());
        assert_eq!(tracker.backoff_duration().as_millis(), 3200);

        // Sixth restart would exceed window limit (5 per 60s)
        assert!(!tracker.record_restart());

        // Reset and test cap
        tracker = RestartTracker::new();
        for _ in 0..6 {
            tracker.backoff_count += 1;
        }
        // backoff_count=6, duration = 100 * 2^6 = 6400ms, capped at 5000ms
        assert_eq!(tracker.backoff_duration().as_millis(), 5000);
    }

    #[test]
    fn restart_tracker_window_limit() {
        let mut tracker = RestartTracker::new();

        // Fill window
        for _ in 0..MAX_RESTARTS_PER_WINDOW {
            assert!(tracker.record_restart());
        }

        // Next restart should be rejected
        assert!(!tracker.record_restart());
        assert!(tracker.is_disabled());
    }

    #[test]
    fn restart_tracker_reset_backoff() {
        let mut tracker = RestartTracker::new();
        tracker.record_restart();
        tracker.record_restart();
        assert_eq!(tracker.backoff_count, 2);

        tracker.reset_backoff();
        assert_eq!(tracker.backoff_count, 0);
        assert_eq!(tracker.backoff_duration().as_millis(), 100);
    }

    #[tokio::test]
    async fn bounded_queue_send_recv() {
        let mut queue = BoundedMessageQueue::new();

        let msg = IpcMessage::Ping { ts_ms: 1000 };
        queue.send(msg.clone()).await.unwrap();

        let received = queue.recv().await.unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn bounded_queue_audio_tracking() {
        let queue = BoundedMessageQueue::new();

        let audio = IpcMessage::Audio {
            session_id: "test".to_string(),
            generation: 0,
            seq: 0,
            pcm: vec![0.0; 1000],
        };

        queue.send(audio).await.unwrap();

        let audio_bytes = *queue.audio_bytes.lock().await;
        assert_eq!(audio_bytes, 1000 * std::mem::size_of::<f32>());
    }

    #[test]
    fn constants_match_spec() {
        assert_eq!(RESTART_BACKOFF_BASE_MS, 100, "§10");
        assert_eq!(RESTART_BACKOFF_CAP_MS, 5_000, "§10");
        assert_eq!(MAX_RESTARTS_PER_WINDOW, 5, "§10");
        assert_eq!(RESTART_WINDOW_DURATION_MS, 60_000, "§10");
        assert_eq!(DISABLE_DURATION_MS, 120_000, "§10");
        assert_eq!(MAX_MISSING_PONGS, 3, "§5.4");
    }
}
