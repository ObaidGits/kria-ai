//! Event-Driven Perception — debounced system event aggregation.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
//! │  inotify     │   │  D-Bus      │   │  Netlink    │
//! │  (notify     │   │  (zbus)     │   │  (polling)  │
//! │   crate)     │   │             │   │             │
//! └──────┬───────┘   └──────┬───────┘   └──────┬───────┘
//!        │                  │                  │
//!        └──────────┬───────┴──────────────────┘
//!                   │
//!           ┌───────▼────────┐
//!           │ EventDebouncer │  ← 2-second window
//!           │ (key-based     │    groups identical events
//!           │  aggregation)  │
//!           └───────┬────────┘
//!                   │
//!           ┌───────▼────────┐
//!           │ PerceptionBus  │  ← tokio::sync::broadcast
//!           │ (fan-out)      │
//!           └───────┬────────┘
//!                   │
//!        ┌──────────┴──────────┐
//!        ▼                     ▼
//! ┌──────────────┐     ┌──────────────┐
//! │ CuriosityLoop│     │  UI / Logs   │
//! └──────────────┘     └──────────────┘
//! ```
//!
//! # Debouncing Strategy
//!
//! Identical events (same key = source + event_kind + primary_path) arriving
//! within a 2-second window are collapsed into a single `PerceptionEvent`
//! with a `count` field. This prevents event floods from overwhelming the
//! CuriosityLoop (e.g., a `git checkout` touching 500 files → 1 aggregated event).
//!
//! # Thread Safety
//!
//! All event sources run as independent Tokio tasks. The debouncer is
//! `Send + 'static` and owns its receiver. The broadcast sender is
//! cloneable and cheaply shared.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

// ─── Perception Events ──────────────────────────────────────────────────────

/// A debounced perception event broadcast to all subscribers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PerceptionEvent {
    /// What kind of event this is.
    pub kind: EventKind,
    /// Aggregation key (source + kind + path).
    pub key: String,
    /// Primary path or object affected.
    pub primary_path: Option<String>,
    /// Number of identical events collapsed in the debounce window.
    pub count: u32,
    /// Human-readable summary.
    pub summary: String,
    /// Event severity (for prioritization).
    pub severity: EventSeverity,
    /// Epoch milliseconds when the event was first observed.
    pub first_seen_epoch_ms: u64,
    /// Epoch milliseconds when the event was finalized (debounce window closed).
    pub finalized_epoch_ms: u64,
}

/// The kind of perception event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EventKind {
    /// Filesystem change (create, modify, delete).
    Filesystem(FilesystemOp),
    /// D-Bus signal (service started/stopped, device added/removed).
    DbusSignal(String),
    /// Network state change (interface up/down, connectivity change).
    NetworkChange(String),
    /// System health threshold breach (disk, RAM, battery, thermal).
    HealthBreach(String),
    /// Process lifecycle event (started, crashed, exited).
    ProcessLifecycle(String),
}

/// Filesystem operation type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FilesystemOp {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// Event severity for prioritization.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum EventSeverity {
    /// Informational, no action needed.
    Info,
    /// Notable, may trigger curiosity investigation.
    Notable,
    /// Warning, should be investigated.
    Warning,
    /// Critical, immediate attention needed.
    Critical,
}

// ─── Debouncer ──────────────────────────────────────────────────────────────

/// Pending event waiting in the debounce window.
#[derive(Debug, Clone)]
struct PendingEvent {
    kind: EventKind,
    primary_path: Option<String>,
    summary: String,
    severity: EventSeverity,
    count: u32,
    first_seen: Instant,
}

/// Time-windowed event debouncer/aggregator.
///
/// Groups identical events (same key) within a configurable window,
/// then emits a single aggregated `PerceptionEvent` when the window closes.
pub struct EventDebouncer {
    /// How long to wait before finalizing an event batch.
    window: Duration,
    /// Pending events keyed by aggregation key.
    pending: HashMap<String, PendingEvent>,
    /// Channel to send finalized events into the perception bus.
    event_tx: broadcast::Sender<PerceptionEvent>,
}

impl EventDebouncer {
    /// Create a new debouncer with the given window duration and broadcast sender.
    pub fn new(window: Duration, event_tx: broadcast::Sender<PerceptionEvent>) -> Self {
        Self {
            window,
            pending: HashMap::new(),
            event_tx,
        }
    }

    /// Ingest a raw event. If an event with the same key is already pending,
    /// increment its count. Otherwise, insert it.
    pub fn ingest(
        &mut self,
        kind: EventKind,
        primary_path: Option<String>,
        summary: String,
        severity: EventSeverity,
    ) {
        let key = Self::make_key(&kind, &primary_path);
        let now = Instant::now();

        if let Some(entry) = self.pending.get_mut(&key) {
            entry.count += 1;
            // Keep the highest severity seen.
            if severity > entry.severity {
                entry.severity = severity;
            }
            // Update summary to reflect aggregation.
            entry.summary = format!("{} ({}x)", summary, entry.count + 1);
        } else {
            self.pending.insert(
                key,
                PendingEvent {
                    kind,
                    primary_path,
                    summary,
                    severity,
                    count: 1,
                    first_seen: now,
                },
            );
        }
    }

    /// Tick the debouncer. Call this periodically (e.g., every 500ms).
    /// Finalizes all events whose debounce window has expired.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let mut expired_keys = Vec::new();

        for (key, entry) in &self.pending {
            if now.duration_since(entry.first_seen) >= self.window {
                expired_keys.push(key.clone());
            }
        }

        for key in expired_keys {
            if let Some(entry) = self.pending.remove(&key) {
                let event = PerceptionEvent {
                    kind: entry.kind,
                    key,
                    primary_path: entry.primary_path,
                    count: entry.count,
                    summary: entry.summary,
                    severity: entry.severity,
                    first_seen_epoch_ms: epoch_millis_from_instant(entry.first_seen),
                    finalized_epoch_ms: epoch_millis_now(),
                };
                // Ignore send error (no subscribers is fine).
                let _ = self.event_tx.send(event);
            }
        }
    }

    /// Force-finalize ALL pending events (e.g., on shutdown).
    pub fn flush_all(&mut self) {
        let keys: Vec<String> = self.pending.keys().cloned().collect();
        for key in keys {
            if let Some(entry) = self.pending.remove(&key) {
                let event = PerceptionEvent {
                    kind: entry.kind,
                    key,
                    primary_path: entry.primary_path,
                    count: entry.count,
                    summary: entry.summary,
                    severity: entry.severity,
                    first_seen_epoch_ms: epoch_millis_from_instant(entry.first_seen),
                    finalized_epoch_ms: epoch_millis_now(),
                };
                let _ = self.event_tx.send(event);
            }
        }
    }

    /// Number of events currently pending in the debounce window.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Build an aggregation key from event kind and path.
    fn make_key(kind: &EventKind, path: &Option<String>) -> String {
        let kind_str = match kind {
            EventKind::Filesystem(op) => format!("fs:{:?}", op),
            EventKind::DbusSignal(sig) => format!("dbus:{}", sig),
            EventKind::NetworkChange(detail) => format!("net:{}", detail),
            EventKind::HealthBreach(detail) => format!("health:{}", detail),
            EventKind::ProcessLifecycle(detail) => format!("proc:{}", detail),
        };
        match path {
            Some(p) => format!("{}:{}", kind_str, p),
            None => kind_str,
        }
    }
}

// ─── Perception Bus ─────────────────────────────────────────────────────────

/// The central perception bus — fan-out broadcast channel.
///
/// All perception events flow through here. Subscribers receive a
/// `broadcast::Receiver<PerceptionEvent>` and can process events
/// independently.
pub struct PerceptionBus {
    /// Broadcast sender (cloneable, cheap to share).
    tx: broadcast::Sender<PerceptionEvent>,
}

impl PerceptionBus {
    /// Create a new perception bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Get a clone of the sender (for feeding events from debouncer).
    pub fn sender(&self) -> broadcast::Sender<PerceptionEvent> {
        self.tx.clone()
    }

    /// Subscribe to the perception bus.
    pub fn subscribe(&self) -> broadcast::Receiver<PerceptionEvent> {
        self.tx.subscribe()
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

// ─── Filesystem Source ──────────────────────────────────────────────────────

/// Filesystem event source using the `notify` crate.
///
/// Watches configured directories and feeds events into the debouncer.
pub struct FilesystemSource {
    watched_paths: Vec<std::path::PathBuf>,
}

impl FilesystemSource {
    pub fn new(watched_paths: Vec<std::path::PathBuf>) -> Self {
        Self { watched_paths }
    }

    /// Run the filesystem watcher. Events are sent to `event_sink` as
    /// raw (kind, path, summary, severity) tuples for debouncer ingestion.
    ///
    /// This is a long-running task — call it via `tokio::spawn`.
    pub async fn run(
        self,
        event_sink: tokio::sync::mpsc::UnboundedSender<(
            EventKind,
            Option<String>,
            String,
            EventSeverity,
        )>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        use notify::{Event, RecursiveMode, Watcher};
        use std::sync::{mpsc, Arc, Mutex};

        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
        let rx = Arc::new(Mutex::new(rx));

        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Failed to create filesystem watcher: {}", e);
                return;
            }
        };

        for path in &self.watched_paths {
            if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
                tracing::warn!("Failed to watch {}: {}", path.display(), e);
            }
        }

        tracing::info!(
            "FilesystemSource watching {} paths",
            self.watched_paths.len()
        );

        loop {
            if cancel.is_cancelled() {
                break;
            }

            // Poll the notify channel with timeout to allow cancellation checks.
            // Use Arc<Mutex<Receiver>> because spawn_blocking requires 'static.
            let rx_clone = rx.clone();
            let recv_result = tokio::task::spawn_blocking(move || {
                let rx = rx_clone.lock().unwrap();
                rx.recv_timeout(std::time::Duration::from_millis(500))
            })
            .await;

            match recv_result {
                Ok(Ok(Ok(event))) => {
                    let (kind, path, summary) = classify_fs_event(event);
                    let _ = event_sink.send((kind, path, summary, EventSeverity::Notable));
                }
                Ok(Ok(Err(e))) => {
                    tracing::warn!("Filesystem watch error: {}", e);
                }
                Ok(Err(mpsc::RecvTimeoutError::Timeout)) => {
                    // Normal timeout, loop back to check cancellation.
                }
                Ok(Err(mpsc::RecvTimeoutError::Disconnected)) => {
                    tracing::warn!("Filesystem watcher channel disconnected");
                    break;
                }
                Err(e) => {
                    tracing::error!("Filesystem watcher task panicked: {}", e);
                    break;
                }
            }
        }

        tracing::info!("FilesystemSource shut down");
    }
}

/// Classify a `notify::Event` into our event kind.
fn classify_fs_event(event: notify::Event) -> (EventKind, Option<String>, String) {
    use notify::EventKind as NKind;

    let path = event.paths.first().map(|p| p.display().to_string());
    let path_label = path.clone().unwrap_or_else(|| "unknown".to_string());

    match event.kind {
        NKind::Create(_) => (
            EventKind::Filesystem(FilesystemOp::Created),
            path,
            format!("File created: {}", path_label),
        ),
        NKind::Modify(_) => (
            EventKind::Filesystem(FilesystemOp::Modified),
            path,
            format!("File modified: {}", path_label),
        ),
        NKind::Remove(_) => (
            EventKind::Filesystem(FilesystemOp::Deleted),
            path,
            format!("File deleted: {}", path_label),
        ),
        NKind::Access(_) => (
            EventKind::Filesystem(FilesystemOp::Modified),
            path,
            format!("File accessed: {}", path_label),
        ),
        other => (
            EventKind::Filesystem(FilesystemOp::Modified),
            path,
            format!("Filesystem event {:?}: {}", other, path_label),
        ),
    }
}

// ─── D-Bus Source ───────────────────────────────────────────────────────────

/// D-Bus signal source.
///
/// Listens for system D-Bus signals (service state changes, device events)
/// and feeds them into the perception bus.
///
/// Requires the `dbus-perception` feature flag.
pub struct DbusSource;

impl DbusSource {
    /// Run the D-Bus listener. This is a long-running task.
    ///
    /// Currently a stub — D-Bus monitoring requires `zbus` with the
    /// `dbus-perception` feature. When disabled, this returns immediately.
    pub async fn run(
        event_sink: tokio::sync::mpsc::UnboundedSender<(
            EventKind,
            Option<String>,
            String,
            EventSeverity,
        )>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        #[cfg(feature = "dbus-perception")]
        {
            Self::run_zbus(event_sink, cancel).await;
        }

        #[cfg(not(feature = "dbus-perception"))]
        {
            tracing::info!("DbusSource: zbus feature not enabled, skipping D-Bus monitoring");
            let _ = event_sink; // suppress unused warning
            cancel.cancelled().await;
        }
    }

    #[cfg(feature = "dbus-perception")]
    async fn run_zbus(
        event_sink: tokio::sync::mpsc::UnboundedSender<(
            EventKind,
            Option<String>,
            String,
            EventSeverity,
        )>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        use futures::StreamExt;
        use zbus::Connection;

        let connection = match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to connect to system D-Bus: {}", e);
                return;
            }
        };

        // Monitor systemd unit state changes.
        let mut stream = match connection
            .add_match(
                "type='signal',interface='org.freedesktop.systemd1.Manager',member='UnitNew'",
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to add D-Bus match rule: {}", e);
                return;
            }
        };

        tracing::info!("DbusSource: monitoring systemd unit signals");

        loop {
            if cancel.is_cancelled() {
                break;
            }

            tokio::select! {
                _ = cancel.cancelled() => break,
                msg = stream.next() => {
                    if let Some(msg) = msg {
                        let member = msg.member().map(|m| m.to_string()).unwrap_or_default();
                        let path = msg.path().map(|p| p.to_string());
                        let summary = format!("D-Bus signal: {}", member);
                        let _ = event_sink.send((
                            EventKind::DbusSignal(member),
                            path,
                            summary,
                            EventSeverity::Notable,
                        ));
                    }
                }
            }
        }

        tracing::info!("DbusSource shut down");
    }
}

// ─── Perception Loop ───────────────────────────────────────────────────────

/// Configuration for the perception loop.
#[derive(Debug, Clone)]
pub struct PerceptionConfig {
    /// Paths to watch for filesystem changes.
    pub watch_paths: Vec<std::path::PathBuf>,
    /// Debounce window duration.
    pub debounce_window: Duration,
    /// Broadcast channel capacity.
    pub bus_capacity: usize,
    /// Debouncer tick interval (how often to check for expired windows).
    pub tick_interval: Duration,
}

impl Default for PerceptionConfig {
    fn default() -> Self {
        Self {
            watch_paths: vec![
                std::path::PathBuf::from("/tmp"),
                dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/home")),
            ],
            debounce_window: Duration::from_secs(2),
            bus_capacity: 1024,
            tick_interval: Duration::from_millis(500),
        }
    }
}

/// The main perception loop — orchestrates event sources and debouncing.
pub struct PerceptionLoop {
    config: PerceptionConfig,
    bus: PerceptionBus,
}

impl PerceptionLoop {
    pub fn new(config: PerceptionConfig) -> Self {
        let bus = PerceptionBus::new(config.bus_capacity);
        Self { config, bus }
    }

    /// Get a reference to the perception bus (for subscribing).
    pub fn bus(&self) -> &PerceptionBus {
        &self.bus
    }

    /// Run the perception loop. Returns when `cancel` is triggered.
    ///
    /// This spawns internal tasks for each event source and runs the
    /// debouncer tick loop on the calling task.
    pub async fn run(self, cancel: tokio_util::sync::CancellationToken) {
        let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<(
            EventKind,
            Option<String>,
            String,
            EventSeverity,
        )>();

        let mut debouncer = EventDebouncer::new(self.config.debounce_window, self.bus.sender());

        // Spawn filesystem source.
        let fs_source = FilesystemSource::new(self.config.watch_paths.clone());
        let fs_cancel = cancel.clone();
        let fs_sink = raw_tx.clone();
        let fs_handle = tokio::spawn(async move {
            fs_source.run(fs_sink, fs_cancel).await;
        });

        // Spawn D-Bus source.
        let dbus_cancel = cancel.clone();
        let dbus_sink = raw_tx.clone();
        let dbus_handle = tokio::spawn(async move {
            DbusSource::run(dbus_sink, dbus_cancel).await;
        });

        tracing::info!("PerceptionLoop started");

        // Main tick loop: drain raw events into debouncer, tick debouncer.
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("PerceptionLoop received shutdown signal");
                    break;
                }
                Some((kind, path, summary, severity)) = raw_rx.recv() => {
                    debouncer.ingest(kind, path, summary, severity);
                }
                _ = tokio::time::sleep(self.config.tick_interval) => {
                    debouncer.tick();
                }
            }
        }

        // Flush remaining events on shutdown.
        debouncer.flush_all();

        // Wait for source tasks.
        let _ = fs_handle.await;
        let _ = dbus_handle.await;

        tracing::info!("PerceptionLoop shut down");
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Get current time as epoch milliseconds.
fn epoch_millis_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Convert an `Instant` to epoch milliseconds (approximate, via wall clock delta).
fn epoch_millis_from_instant(instant: Instant) -> u64 {
    let elapsed = instant.elapsed();
    epoch_millis_now() - elapsed.as_millis() as u64
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debouncer_collapses_identical_events() {
        let bus = PerceptionBus::new(64);
        let mut debouncer = EventDebouncer::new(Duration::from_millis(100), bus.sender());

        // Ingest 10 identical events rapidly.
        for _ in 0..10 {
            debouncer.ingest(
                EventKind::Filesystem(FilesystemOp::Modified),
                Some("/tmp/test.txt".to_string()),
                "File modified: /tmp/test.txt".to_string(),
                EventSeverity::Notable,
            );
        }

        // All 10 should be collapsed into 1 pending entry.
        assert_eq!(debouncer.pending_count(), 1);

        // Before the window expires, no events should be broadcast.
        let mut rx = bus.subscribe();
        debouncer.tick();
        assert!(rx.try_recv().is_err());

        // Wait for the window to expire.
        std::thread::sleep(Duration::from_millis(150));
        debouncer.tick();

        // Now we should receive 1 aggregated event with count=10.
        let event = rx.try_recv().expect("Expected one aggregated event");
        assert_eq!(event.count, 10);
        assert_eq!(event.kind, EventKind::Filesystem(FilesystemOp::Modified));
        assert_eq!(event.primary_path, Some("/tmp/test.txt".to_string()));
    }

    #[test]
    fn test_debouncer_separates_different_events() {
        let bus = PerceptionBus::new(64);
        let mut debouncer = EventDebouncer::new(Duration::from_millis(100), bus.sender());

        debouncer.ingest(
            EventKind::Filesystem(FilesystemOp::Modified),
            Some("/tmp/a.txt".to_string()),
            "File modified: /tmp/a.txt".to_string(),
            EventSeverity::Notable,
        );
        debouncer.ingest(
            EventKind::Filesystem(FilesystemOp::Created),
            Some("/tmp/b.txt".to_string()),
            "File created: /tmp/b.txt".to_string(),
            EventSeverity::Notable,
        );
        debouncer.ingest(
            EventKind::HealthBreach("disk_low".to_string()),
            None,
            "Disk space below threshold".to_string(),
            EventSeverity::Warning,
        );

        // Three distinct keys → three pending entries.
        assert_eq!(debouncer.pending_count(), 3);
    }

    #[test]
    fn test_debouncer_flush_all() {
        let bus = PerceptionBus::new(64);
        let mut debouncer = EventDebouncer::new(Duration::from_secs(60), bus.sender());
        let mut rx = bus.subscribe();

        debouncer.ingest(
            EventKind::Filesystem(FilesystemOp::Created),
            Some("/tmp/flush.txt".to_string()),
            "File created".to_string(),
            EventSeverity::Info,
        );
        debouncer.ingest(
            EventKind::NetworkChange("eth0_down".to_string()),
            None,
            "Network interface down".to_string(),
            EventSeverity::Critical,
        );

        // Window is 60s, so tick won't finalize anything.
        debouncer.tick();
        assert!(rx.try_recv().is_err());

        // Flush all forces finalization.
        debouncer.flush_all();
        assert_eq!(debouncer.pending_count(), 0);

        let e1 = rx.try_recv().expect("First event");
        let e2 = rx.try_recv().expect("Second event");
        assert_eq!(e1.count, 1);
        assert_eq!(e2.count, 1);
        // Two different events flushed.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_debouncer_severity_escalation() {
        let bus = PerceptionBus::new(64);
        let mut debouncer = EventDebouncer::new(Duration::from_millis(100), bus.sender());
        let mut rx = bus.subscribe();

        // First event is Info.
        debouncer.ingest(
            EventKind::HealthBreach("cpu_high".to_string()),
            None,
            "CPU usage high".to_string(),
            EventSeverity::Info,
        );
        // Second event with same key escalates to Warning.
        debouncer.ingest(
            EventKind::HealthBreach("cpu_high".to_string()),
            None,
            "CPU usage critical".to_string(),
            EventSeverity::Warning,
        );
        // Third event escalates to Critical.
        debouncer.ingest(
            EventKind::HealthBreach("cpu_high".to_string()),
            None,
            "CPU usage extreme".to_string(),
            EventSeverity::Critical,
        );

        std::thread::sleep(Duration::from_millis(150));
        debouncer.tick();

        let event = rx.try_recv().expect("Expected aggregated event");
        assert_eq!(event.count, 3);
        assert_eq!(event.severity, EventSeverity::Critical);
    }

    #[test]
    fn test_perception_bus_fan_out() {
        let bus = PerceptionBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        assert_eq!(bus.subscriber_count(), 2);

        let event = PerceptionEvent {
            kind: EventKind::ProcessLifecycle("nginx_crashed".to_string()),
            key: "proc:nginx_crashed".to_string(),
            primary_path: None,
            count: 1,
            summary: "nginx process crashed".to_string(),
            severity: EventSeverity::Critical,
            first_seen_epoch_ms: epoch_millis_now(),
            finalized_epoch_ms: epoch_millis_now(),
        };

        bus.sender().send(event.clone()).unwrap();

        let received1 = rx1.try_recv().unwrap();
        let received2 = rx2.try_recv().unwrap();

        assert_eq!(received1.key, "proc:nginx_crashed");
        assert_eq!(received2.key, "proc:nginx_crashed");
        assert_eq!(received1.severity, EventSeverity::Critical);
    }

    #[tokio::test]
    async fn test_event_flood_debouncing() {
        // Simulate a massive event flood (e.g., git checkout touching 500 files).
        let bus = PerceptionBus::new(1024);
        let mut debouncer = EventDebouncer::new(Duration::from_millis(200), bus.sender());
        let mut rx = bus.subscribe();

        // 500 rapid filesystem events on different files.
        for i in 0..500 {
            debouncer.ingest(
                EventKind::Filesystem(FilesystemOp::Modified),
                Some(format!("/repo/file_{}.rs", i)),
                format!("File modified: /repo/file_{}.rs", i),
                EventSeverity::Notable,
            );
        }

        // Each file has a unique key → 500 pending entries.
        assert_eq!(debouncer.pending_count(), 500);

        // Wait for debounce window.
        tokio::time::sleep(Duration::from_millis(250)).await;
        debouncer.tick();

        // Should receive 500 individual events (each file is unique).
        let mut count = 0;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 500);

        // Now simulate 500 events on the SAME file.
        for _ in 0..500 {
            debouncer.ingest(
                EventKind::Filesystem(FilesystemOp::Modified),
                Some("/repo/hot_file.rs".to_string()),
                "File modified: /repo/hot_file.rs".to_string(),
                EventSeverity::Notable,
            );
        }

        // All 500 collapsed into 1 pending entry.
        assert_eq!(debouncer.pending_count(), 1);

        tokio::time::sleep(Duration::from_millis(250)).await;
        debouncer.tick();

        let event = rx.try_recv().expect("Expected aggregated flood event");
        assert_eq!(event.count, 500);
        assert_eq!(event.primary_path, Some("/repo/hot_file.rs".to_string()));
    }

    #[test]
    fn test_event_severity_ordering() {
        assert!(EventSeverity::Info < EventSeverity::Notable);
        assert!(EventSeverity::Notable < EventSeverity::Warning);
        assert!(EventSeverity::Warning < EventSeverity::Critical);
    }

    #[test]
    fn test_make_key_uniqueness() {
        let key1 = EventDebouncer::make_key(
            &EventKind::Filesystem(FilesystemOp::Modified),
            &Some("/tmp/a.txt".to_string()),
        );
        let key2 = EventDebouncer::make_key(
            &EventKind::Filesystem(FilesystemOp::Modified),
            &Some("/tmp/b.txt".to_string()),
        );
        let key3 = EventDebouncer::make_key(
            &EventKind::Filesystem(FilesystemOp::Created),
            &Some("/tmp/a.txt".to_string()),
        );
        let key4 =
            EventDebouncer::make_key(&EventKind::HealthBreach("disk_low".to_string()), &None);

        // All keys should be unique.
        assert_ne!(key1, key2);
        assert_ne!(key1, key3);
        assert_ne!(key1, key4);
        assert_ne!(key2, key3);
        assert_ne!(key2, key4);
        assert_ne!(key3, key4);
    }
}
