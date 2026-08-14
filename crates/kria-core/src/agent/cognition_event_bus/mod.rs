//! Batch 3 — Cognition Event Bus.
//!
//! # Core Mission
//!
//! A typed, bounded, broadcast event bus that unifies all operational cognition
//! signals into one observable runtime stream. Extends the lower-level
//! [`PerceptionBus`] (filesystem/D-Bus/network) with higher-level operational
//! cognition events: workflow lifecycle, browser, IDE, interruption, policy, and
//! operational suggestion events.
//!
//! # Architecture
//!
//! ```text
//! PerceptionBus (low-level OS events)
//!     │
//!     ▼
//! CognitionEventBus  ◄── higher-level operational events
//!     │  (tokio::broadcast, cap = 256)
//!     │
//!     ├── AmbientCognitionLoop   (subscriber)
//!     ├── DesktopAwarenessRuntime (subscriber)
//!     ├── OperationalContextTracker (subscriber)
//!     └── AgentLoop event fan-out (subscriber)
//! ```
//!
//! # Invariants
//!
//! 1. **Bounded queue.** Broadcast channel capacity is capped at
//!    [`BUS_CAPACITY`] (256). Slow receivers get `RecvError::Lagged` and must
//!    catch up — they are NEVER allowed to stall producers.
//! 2. **Flood protection.** Identical events (same `dedup_key()`) within a
//!    2-second window are coalesced by [`EventFloodGuard`] before emission.
//! 3. **No LLM calls.** The event bus is a pure data pipeline; classifiers and
//!    responders live in subscribers.
//! 4. **Send + Sync + Clone.** All event types derive the traits needed for
//!    async fan-out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::debug;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Broadcast channel capacity. Slow receivers get `Lagged` — never stall.
pub const BUS_CAPACITY: usize = 256;

/// Deduplication window for identical events.
pub const DEDUP_WINDOW: Duration = Duration::from_secs(2);

/// Maximum event rate per unique key (events per window).
pub const MAX_EVENTS_PER_KEY_PER_WINDOW: u32 = 10;

// ─── Event Types ──────────────────────────────────────────────────────────────

/// A workflow lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEvent {
    /// Stable session ID (matches `WorkflowSession::session_id`).
    pub session_id: String,
    /// Human-readable workflow description.
    pub description: String,
    /// What happened.
    pub kind: WorkflowEventKind,
}

/// Discriminant for workflow lifecycle transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowEventKind {
    /// A new workflow started.
    Started,
    /// A GoalTree stage completed successfully.
    StageCompleted {
        stage_label: String,
        stage_index: u32,
    },
    /// A workflow stage failed.
    StageFailed { stage_label: String, reason: String },
    /// A workflow was paused (persisted to checkpoint).
    Paused { reason: String },
    /// A paused workflow is available for resumption.
    ResumeAvailable { continuation_hint: String },
    /// A workflow completed successfully.
    Completed { duration_ms: u64 },
    /// A workflow failed terminally.
    Failed { reason: String },
    /// A recovery action was planned.
    RecoveryPlanned { action_summary: String, depth: u8 },
}

/// A browser cognition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserCognitionEvent {
    /// Current page URL (may be partial).
    pub url: String,
    /// Current page title.
    pub title: String,
    /// What changed.
    pub kind: BrowserEventKind,
}

/// Discriminant for browser cognition transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrowserEventKind {
    /// Navigated to a new URL.
    Navigated,
    /// A new tab was opened.
    TabOpened,
    /// A tab was closed.
    TabClosed,
    /// An authentication dialog appeared.
    AuthInterrupt { service_hint: String },
    /// Page loading completed.
    PageLoaded,
    /// Download completed.
    DownloadCompleted { filename: String },
}

/// An IDE cognition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCognitionEvent {
    /// Workspace root (if known).
    pub workspace_root: Option<String>,
    /// What changed.
    pub kind: IdeEventKind,
}

/// Discriminant for IDE cognition transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdeEventKind {
    /// Build succeeded.
    BuildSucceeded,
    /// Build failed (contains first-N error summaries, bounded).
    BuildFailed {
        error_count: usize,
        first_error: String,
    },
    /// New diagnostics are available (error count changed).
    DiagnosticsChanged {
        error_count: usize,
        warning_count: usize,
    },
    /// Active file changed.
    ActiveFileChanged { path: String },
    /// Runtime/test failure detected.
    RuntimeFailure { description: String },
}

/// A desktop/AT-SPI cognition event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopCognitionEvent {
    /// Application name that triggered this event.
    pub app_name: String,
    /// What changed.
    pub kind: DesktopCognitionEventKind,
}

/// Discriminant for desktop cognition transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopCognitionEventKind {
    /// Window focus changed to a different application.
    FocusChanged { from: Option<String>, to: String },
    /// A new window appeared (popup, dialog, or app).
    WindowAppeared { title: String, is_dialog: bool },
    /// A window was closed.
    WindowClosed { title: String },
    /// An application launched.
    AppLaunched,
    /// An application crashed.
    AppCrashed { pid: u32 },
    /// A dialog requires dismissal.
    DialogRequiresDismissal { message: String },
}

/// An interruption/continuation event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuationEvent {
    /// Session ID this event relates to.
    pub session_id: String,
    /// What happened.
    pub kind: ContinuationEventKind,
}

/// Discriminant for continuation lifecycle events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContinuationEventKind {
    /// An interruption was classified.
    InterruptionClassified { class_summary: String },
    /// A checkpoint was written to disk.
    CheckpointWritten { path: String },
    /// A checkpoint was read and context rehydrated.
    CheckpointResumed,
    /// Maximum recovery depth reached — escalating to HITL.
    MaxDepthReached,
}

/// A policy/safety event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyEvent {
    /// What triggered this event.
    pub trigger: String,
    /// What the policy decided.
    pub kind: PolicyEventKind,
}

/// Discriminant for policy decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEventKind {
    /// A HITL confirmation gate was triggered.
    HitlGateTriggered { risk_summary: String },
    /// An operation was blocked by policy.
    OperationBlocked { reason: String },
    /// Ambient cognition was paused by user or policy.
    AmbientCognitionPaused,
    /// Ambient cognition was resumed.
    AmbientCognitionResumed,
}

/// An operational suggestion event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionEvent {
    /// Stable suggestion ID.
    pub suggestion_id: String,
    /// Human-readable suggestion content.
    pub content: String,
    /// Why this was suggested.
    pub rationale: String,
    /// What kind of suggestion this is.
    pub kind: SuggestionKind,
}

/// Discriminant for operational suggestion types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionKind {
    /// Resume a paused workflow.
    ResumePausedWorkflow { session_id: String },
    /// Recover from a failed build.
    RecoverBuildFailure,
    /// Continue an interrupted browser session.
    ContinueBrowserSession { url_hint: String },
    /// Suggest the next step in an operational goal.
    NextGoalStep { goal_id: String },
    /// Suggest addressing open IDE diagnostics.
    AddressDiagnostics { error_count: usize },
}

// ─── Top-level Event Variant ──────────────────────────────────────────────────

/// A typed cognition event broadcast to all subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CognitionEvent {
    /// Workflow lifecycle event.
    Workflow(WorkflowEvent),
    /// Browser cognition event.
    Browser(BrowserCognitionEvent),
    /// IDE cognition event.
    Ide(IdeCognitionEvent),
    /// Desktop/AT-SPI event.
    Desktop(DesktopCognitionEvent),
    /// Interruption/continuation event.
    Continuation(ContinuationEvent),
    /// Policy/safety event.
    Policy(PolicyEvent),
    /// Operational suggestion event.
    Suggestion(SuggestionEvent),
}

impl CognitionEvent {
    /// Stable deduplication key for flood protection.
    ///
    /// Identical keys within the dedup window are coalesced. The key encodes
    /// the event category and the most stable discriminant fields. It does NOT
    /// include dynamic content like error messages.
    pub fn dedup_key(&self) -> String {
        match self {
            Self::Workflow(e) => format!(
                "wf::{}::{:?}",
                e.session_id,
                std::mem::discriminant(&e.kind)
            ),
            Self::Browser(e) => format!("br::{}::{:?}", e.url, std::mem::discriminant(&e.kind)),
            Self::Ide(e) => format!(
                "ide::{}::{:?}",
                e.workspace_root.as_deref().unwrap_or(""),
                std::mem::discriminant(&e.kind)
            ),
            Self::Desktop(e) => {
                format!("dt::{}::{:?}", e.app_name, std::mem::discriminant(&e.kind))
            }
            Self::Continuation(e) => {
                format!(
                    "cont::{}::{:?}",
                    e.session_id,
                    std::mem::discriminant(&e.kind)
                )
            }
            Self::Policy(e) => format!("pol::{}::{:?}", e.trigger, std::mem::discriminant(&e.kind)),
            Self::Suggestion(e) => format!("sug::{}", e.suggestion_id),
        }
    }

    /// Human-readable one-line summary for logging.
    pub fn summary(&self) -> String {
        match self {
            Self::Workflow(e) => format!("[Workflow] {} — {:?}", e.session_id, e.kind),
            Self::Browser(e) => format!("[Browser] {} — {:?}", e.url, e.kind),
            Self::Ide(e) => format!("[IDE] {:?}", e.kind),
            Self::Desktop(e) => format!("[Desktop] {} — {:?}", e.app_name, e.kind),
            Self::Continuation(e) => format!("[Continuation] {} — {:?}", e.session_id, e.kind),
            Self::Policy(e) => format!("[Policy] {} — {:?}", e.trigger, e.kind),
            Self::Suggestion(e) => format!("[Suggestion] {}: {}", e.kind_name(), e.content),
        }
    }
}

impl SuggestionEvent {
    fn kind_name(&self) -> &'static str {
        match self.kind {
            SuggestionKind::ResumePausedWorkflow { .. } => "resume",
            SuggestionKind::RecoverBuildFailure => "build-recovery",
            SuggestionKind::ContinueBrowserSession { .. } => "browser",
            SuggestionKind::NextGoalStep { .. } => "goal",
            SuggestionKind::AddressDiagnostics { .. } => "diagnostics",
        }
    }
}

// ─── Event Flood Guard ────────────────────────────────────────────────────────

/// Debounces and rate-limits events before they enter the bus.
///
/// For each unique `dedup_key()`:
/// - First emission within the window always passes.
/// - Subsequent identical events within `DEDUP_WINDOW` are suppressed.
/// - After `MAX_EVENTS_PER_KEY_PER_WINDOW` events, all further events for
///   that key are suppressed until the window resets.
pub struct EventFloodGuard {
    /// Maps dedup_key → (first_seen, count).
    state: Mutex<HashMap<String, (Instant, u32)>>,
}

impl EventFloodGuard {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if the event should be emitted; `false` if suppressed.
    pub fn should_emit(&self, event: &CognitionEvent) -> bool {
        let key = event.dedup_key();
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();

        // Prune stale entries (> 2× dedup window)
        state.retain(|_, (first_seen, _)| now.duration_since(*first_seen) < DEDUP_WINDOW * 2);

        match state.get_mut(&key) {
            Some((first_seen, count)) => {
                let elapsed = now.duration_since(*first_seen);
                if elapsed >= DEDUP_WINDOW {
                    // Window expired — reset
                    *first_seen = now;
                    *count = 1;
                    true
                } else if *count >= MAX_EVENTS_PER_KEY_PER_WINDOW {
                    // Flood protection
                    false
                } else {
                    *count += 1;
                    // Suppress: identical event within window (count > 1)
                    *count == 1
                }
            }
            None => {
                state.insert(key, (now, 1));
                true
            }
        }
    }
}

impl Default for EventFloodGuard {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Cognition Event Bus ──────────────────────────────────────────────────────

/// The Cognition Event Bus — typed broadcast channel for operational cognition.
///
/// Clone freely — internally `Arc`-backed. The sender is always alive as long
/// as any clone of `CognitionEventBus` is alive.
#[derive(Clone)]
pub struct CognitionEventBus {
    sender: broadcast::Sender<CognitionEvent>,
    flood_guard: Arc<EventFloodGuard>,
}

impl CognitionEventBus {
    /// Create a new bus with [`BUS_CAPACITY`] slots.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(BUS_CAPACITY);
        Self {
            sender,
            flood_guard: Arc::new(EventFloodGuard::new()),
        }
    }

    /// Subscribe to the event bus. Returns a receiver.
    ///
    /// Receivers are independent — each gets every event. Lagging receivers
    /// receive `RecvError::Lagged` and skip the missed events; they do NOT
    /// block the bus.
    pub fn subscribe(&self) -> broadcast::Receiver<CognitionEvent> {
        self.sender.subscribe()
    }

    /// Emit a cognition event.
    ///
    /// Events are flood-guarded before emission. Returns the number of active
    /// subscribers that received the event, or 0 if suppressed/no subscribers.
    pub fn emit(&self, event: CognitionEvent) -> usize {
        if !self.flood_guard.should_emit(&event) {
            debug!(
                target: "cognition_event_bus",
                key = %event.dedup_key(),
                "CognitionEvent suppressed by flood guard"
            );
            return 0;
        }

        debug!(
            target: "cognition_event_bus",
            summary = %event.summary(),
            "CognitionEvent emitted"
        );

        match self.sender.send(event) {
            Ok(n) => n,
            Err(_) => {
                // No active receivers — acceptable in idle state
                0
            }
        }
    }

    /// Emit without flood protection (for one-shot critical events like policy blocks).
    pub fn emit_critical(&self, event: CognitionEvent) -> usize {
        debug!(
            target: "cognition_event_bus",
            summary = %event.summary(),
            "CognitionEvent emitted (critical, flood-bypass)"
        );
        match self.sender.send(event) {
            Ok(n) => n,
            Err(_) => 0,
        }
    }

    /// Number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for CognitionEventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Handle ───────────────────────────────────────────────────────────────────

/// Cheaply-cloneable Arc handle to the cognition event bus.
///
/// Wrap `CognitionEventBus` in this for ergonomic injection across threads.
pub type CognitionEventBusHandle = Arc<CognitionEventBus>;

/// Construct a new handle.
pub fn new_bus() -> CognitionEventBusHandle {
    Arc::new(CognitionEventBus::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    

    fn wf_started(id: &str) -> CognitionEvent {
        CognitionEvent::Workflow(WorkflowEvent {
            session_id: id.to_string(),
            description: "test wf".to_string(),
            kind: WorkflowEventKind::Started,
        })
    }

    #[test]
    fn bus_emit_delivers_to_subscriber() {
        let bus = CognitionEventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(wf_started("s1"));
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, CognitionEvent::Workflow(_)));
    }

    #[test]
    fn flood_guard_suppresses_duplicate_in_window() {
        let guard = EventFloodGuard::new();
        let ev = wf_started("dup");
        assert!(guard.should_emit(&ev), "first emission must pass");
        // Second identical event within the 2s window is suppressed
        assert!(
            !guard.should_emit(&ev),
            "duplicate in window must be suppressed"
        );
    }

    #[test]
    fn flood_guard_passes_different_events() {
        let guard = EventFloodGuard::new();
        let ev1 = wf_started("a");
        let ev2 = wf_started("b");
        assert!(guard.should_emit(&ev1));
        assert!(guard.should_emit(&ev2), "different key must pass");
    }

    #[test]
    fn bus_no_subscribers_returns_zero() {
        let bus = CognitionEventBus::new();
        let n = bus.emit(wf_started("s2"));
        assert_eq!(n, 0);
    }

    #[test]
    fn dedup_key_is_stable() {
        let ev = CognitionEvent::Policy(PolicyEvent {
            trigger: "policy_test".to_string(),
            kind: PolicyEventKind::AmbientCognitionPaused,
        });
        let k1 = ev.dedup_key();
        let k2 = ev.dedup_key();
        assert_eq!(k1, k2);
    }

    #[test]
    fn emit_critical_bypasses_flood_guard() {
        let bus = CognitionEventBus::new();
        let _rx = bus.subscribe();
        let ev = wf_started("crit");
        // Emit once normally to seed the flood guard
        bus.emit(ev.clone());
        // emit_critical should still succeed even if guard would suppress
        let n = bus.emit_critical(ev);
        assert_eq!(n, 1);
    }
}
