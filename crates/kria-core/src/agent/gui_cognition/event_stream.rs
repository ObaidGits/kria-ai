//! Task 10.1 (`gui_cog_stream_ux`, default OFF) — DURING-turn event streaming.
//!
//! Historically the GUI Cognition runtime emitted its `gui_cognition:event`
//! telemetry envelopes as a single BATCH at the end of the turn
//! ([`GuiTurnOutcome::events`](super::GuiTurnOutcome)). The desktop layer then
//! walked that batch and emitted each envelope to the frontend AFTER the turn
//! had already finished — so the UI could not render observe/plan/per-step
//! progress as it happened.
//!
//! This module adds an OPTIONAL streaming SINK ([`GuiEventStreamSink`]) so that,
//! as the runtime progresses (observe → plan → per-step execute/verify), each
//! envelope is pushed through an mpsc channel the moment it is produced. The
//! desktop layer drains the receiver and emits each envelope to the frontend
//! incrementally via the EXISTING `gui_cognition:event` Tauri event (the event
//! NAME is unchanged — it is a frontend/backend contract).
//!
//! The behavior is gated behind the `gui_cog_stream_ux` feature flag
//! ([`GuiStreamUxConfig`], env [`STREAM_UX_ENV_FLAG`]), default OFF. When the
//! flag is OFF no sink is attached: the runtime pushes ONLY to the in-turn
//! buffer and still returns the full batch in `GuiTurnOutcome.events`, so
//! behavior is byte-for-byte unchanged (existing T2 tests that read
//! `outcome.events` are unaffected). When the flag is ON the same envelopes are
//! ALSO forwarded to the sink as they are produced, and the streamed sequence is
//! exactly equal to the final batch.
//!
//! Runtime authority is preserved: the sink is a passive, append-only observer —
//! it never feeds anything back into the planner/executor, never reorders or
//! drops events, and never changes the turn's control flow. The runtime stays
//! the authoritative orchestrator; streaming is pure additive telemetry.

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Environment variable that enables the `gui_cog_stream_ux` flag (Task 10.1).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns DURING-turn streaming ON. Default
/// (unset or any other value) keeps it OFF, preserving the end-of-turn batch
/// behavior byte-for-byte. The wave gate (Task 10.7) flips the live/desktop path
/// to default ON.
pub const STREAM_UX_ENV_FLAG: &str = "KRIA_GUI_COG_STREAM_UX";

/// Parse a `gui_cog_stream_ux` env value as truthy (`1`/`true`/`yes`/`on`).
fn stream_ux_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_stream_ux` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn stream_ux_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_stream_ux` feature-flag bundle (default OFF) — Task 10.1.
///
/// When enabled (and a sink is attached) the runtime forwards each
/// `gui_cognition:event` envelope to the streaming sink as it is produced, so
/// the desktop layer can emit incremental observe/plan/per-step progress to the
/// frontend instead of waiting for the end-of-turn batch. While OFF the sink is
/// never attached and the end batch is emitted exactly as today.
///
/// Mirrors the established `GuiSafetyPolishConfig` flag pattern exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiStreamUxConfig {
    /// Whether DURING-turn event streaming is active.
    pub enabled: bool,
}

impl Default for GuiStreamUxConfig {
    fn default() -> Self {
        // Task 10.1: flag default OFF until the wave gate (Task 10.7) flips it.
        Self { enabled: false }
    }
}

impl GuiStreamUxConfig {
    /// Construct an explicitly-enabled stream-ux config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled stream-ux config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`STREAM_UX_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: stream_ux_flag_truthy(lookup(STREAM_UX_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (wave gate flip, Task 10.7). DURING-turn streaming is active unless
    /// [`STREAM_UX_ENV_FLAG`] is explicitly falsy (`0`/`false`/`no`/`off`/empty),
    /// which is the documented rollback switch. An absent env value keeps the
    /// flag ON.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`](Self::from_env_default_on) with
    /// an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enabled unless the operator explicitly opts out via a
        // falsy env value (the rollback switch). Absent (None) is NOT falsy.
        Self {
            enabled: !stream_ux_flag_falsy(lookup(STREAM_UX_ENV_FLAG).as_deref()),
        }
    }

    /// Whether DURING-turn event streaming should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A streaming sink that forwards `gui_cognition:event` envelopes through an
/// mpsc channel as they are produced during the turn (Task 10.1).
///
/// The sink is a thin, cloneable handle around an unbounded mpsc sender. The
/// channel is unbounded so a push from the synchronous runtime hot-path never
/// blocks or backpressures the orchestration loop — the runtime stays in
/// control of pacing, and a slow/absent consumer can never stall the turn. Send
/// failures (receiver dropped) are intentionally ignored: streaming is additive
/// telemetry and must never alter the turn's outcome.
#[derive(Debug, Clone)]
pub struct GuiEventStreamSink {
    sender: UnboundedSender<serde_json::Value>,
}

impl GuiEventStreamSink {
    /// Create a connected sink + receiver pair. The runtime is given the sink;
    /// the desktop/test layer drains the [`UnboundedReceiver`].
    pub fn channel() -> (Self, UnboundedReceiver<serde_json::Value>) {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }

    /// Wrap an existing unbounded sender as a sink (e.g. when the desktop layer
    /// owns the channel).
    pub fn from_sender(sender: UnboundedSender<serde_json::Value>) -> Self {
        Self { sender }
    }

    /// Forward a single envelope. Never blocks; a closed channel is ignored so
    /// streaming can never affect the turn's control flow or outcome.
    pub fn send(&self, event: serde_json::Value) {
        let _ = self.sender.send(event);
    }
}

/// The runtime's append-only event log for a single turn.
///
/// This is the single place every `gui_cognition:event` envelope flows through.
/// Each [`push`](Self::push) appends to the in-turn buffer (always) AND, when a
/// sink is attached, forwards the same envelope to the streaming sink in FIFO
/// order — so the streamed sequence is exactly equal to the final batch returned
/// in [`GuiTurnOutcome::events`](super::GuiTurnOutcome).
///
/// When no sink is attached (the `gui_cog_stream_ux` flag OFF) this behaves
/// exactly like the previous `Vec<serde_json::Value>`: push-only buffering,
/// byte-for-byte unchanged.
#[derive(Debug, Default)]
pub struct GuiEventStream {
    events: Vec<serde_json::Value>,
    sink: Option<GuiEventStreamSink>,
}

impl GuiEventStream {
    /// An event stream with no sink (end-of-turn batch only; flag OFF behavior).
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            sink: None,
        }
    }

    /// An event stream that ALSO forwards each envelope to `sink` as it is
    /// produced (flag ON behavior). `None` is identical to [`new`](Self::new).
    pub fn with_sink(sink: Option<GuiEventStreamSink>) -> Self {
        Self {
            events: Vec::new(),
            sink,
        }
    }

    /// Append an envelope to the in-turn buffer and, when a sink is attached,
    /// forward it to the streaming sink immediately (DURING the turn).
    pub fn push(&mut self, event: serde_json::Value) {
        if let Some(sink) = &self.sink {
            // Forward the envelope as it is produced. The clone keeps the buffer
            // copy authoritative for the returned batch while the sink receives
            // an identical envelope in the same FIFO order.
            sink.send(event.clone());
        }
        self.events.push(event);
    }

    /// Whether a streaming sink is attached for this turn.
    pub fn is_streaming(&self) -> bool {
        self.sink.is_some()
    }

    /// Number of envelopes buffered so far.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether no envelope has been buffered yet.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Borrow the buffered envelopes.
    pub fn as_slice(&self) -> &[serde_json::Value] {
        &self.events
    }

    /// Consume the stream and return the full end-of-turn batch (the value
    /// returned in `GuiTurnOutcome.events`).
    pub fn into_events(self) -> Vec<serde_json::Value> {
        self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_flag_defaults_off() {
        assert!(!GuiStreamUxConfig::default().is_enabled());
        assert!(GuiStreamUxConfig::enabled().is_enabled());
        assert!(!GuiStreamUxConfig::disabled().is_enabled());
    }

    #[test]
    fn t1_from_env_lookup_default_off_unless_truthy() {
        let off = GuiStreamUxConfig::from_env_lookup(|_| None);
        assert!(!off.is_enabled(), "absent env => OFF on the default-off path");
        for falsy in ["0", "false", "no", "off", "", "garbage"] {
            let cfg = GuiStreamUxConfig::from_env_lookup(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must be OFF");
        }
        for truthy in ["1", "true", "yes", "on", "ON", "  true  "] {
            let cfg = GuiStreamUxConfig::from_env_lookup(|_| Some(truthy.to_string()));
            assert!(cfg.is_enabled(), "{truthy:?} must be ON");
        }
    }

    #[test]
    fn t1_from_env_lookup_default_on_rollback_switch() {
        // Absent => ON (the wave-gate default, Task 10.7).
        assert!(GuiStreamUxConfig::from_env_lookup_default_on(|_| None).is_enabled());
        // Explicit falsy => the documented rollback switch (OFF).
        for falsy in ["0", "false", "no", "off", ""] {
            let cfg = GuiStreamUxConfig::from_env_lookup_default_on(|_| Some(falsy.to_string()));
            assert!(!cfg.is_enabled(), "{falsy:?} must roll back to OFF");
        }
        assert!(
            GuiStreamUxConfig::from_env_lookup_default_on(|_| Some("1".to_string())).is_enabled()
        );
    }

    #[test]
    fn t1_env_flag_const_is_stable() {
        assert_eq!(STREAM_UX_ENV_FLAG, "KRIA_GUI_COG_STREAM_UX");
    }

    #[test]
    fn t1_stream_without_sink_buffers_only() {
        let mut stream = GuiEventStream::new();
        assert!(!stream.is_streaming());
        stream.push(serde_json::json!({ "type": "TurnStarted" }));
        stream.push(serde_json::json!({ "type": "TurnCompleted" }));
        assert_eq!(stream.len(), 2);
        let batch = stream.into_events();
        assert_eq!(batch.len(), 2);
    }

    #[tokio::test]
    async fn t1_stream_with_sink_forwards_each_envelope_in_order() {
        let (sink, mut rx) = GuiEventStreamSink::channel();
        let mut stream = GuiEventStream::with_sink(Some(sink));
        assert!(stream.is_streaming());
        stream.push(serde_json::json!({ "type": "TurnStarted" }));
        stream.push(serde_json::json!({ "type": "ObservationStarted" }));
        stream.push(serde_json::json!({ "type": "TurnCompleted" }));

        let mut streamed = Vec::new();
        while let Ok(event) = rx.try_recv() {
            streamed.push(event);
        }
        let batch = stream.into_events();
        assert_eq!(streamed, batch, "streamed sequence must equal the batch");
        assert_eq!(streamed[0]["type"], "TurnStarted");
        assert_eq!(streamed[1]["type"], "ObservationStarted");
        assert_eq!(streamed[2]["type"], "TurnCompleted");
    }
}
