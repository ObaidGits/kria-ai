//! Unified, provider-neutral observability event stream for the CPP.
//!
//! Every stage of the capability flow emits a [`CapabilityEvent`] tagged with a
//! `correlation_id`, `provider_id`, and (where applicable) `capability_id`, so a
//! goal's whole lifecycle (discover → permission → plan → execute → recover →
//! learn) is reconstructable as an ordered timeline. This is the single event
//! surface the desktop timeline / logs and any tracing exporter consume.
//!
//! It is a lightweight broadcast bus (bounded, lossy on lag — observability must
//! never back-pressure execution). It complements, not replaces, the existing
//! `AuditLedger`/`ExecutionMetrics`; the platform emits here for live UI, and the
//! frozen audit/metrics remain the durable record.

use std::sync::Arc;

use tokio::sync::broadcast;

/// The stage of the capability flow an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Negotiate,
    Discover,
    Rank,
    Permission,
    Acquire,
    /// Capability synthesis / generation (spec R7, Wave 9): spec creation, IR
    /// generation + validation, golden smoke — distinct from `Acquire` (install)
    /// so the Generate UI can render a synthesis timeline.
    Synthesize,
    Plan,
    Execute,
    /// A bounded retry of a failed attempt (Wave 11, spec R12.1).
    Retry,
    /// A durable long-running job state transition (Wave 11, spec R28).
    Job,
    Recover,
    Learn,
    Failure,
    Cancel,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Negotiate => "negotiate",
            Self::Discover => "discover",
            Self::Rank => "rank",
            Self::Permission => "permission",
            Self::Acquire => "acquire",
            Self::Synthesize => "synthesize",
            Self::Plan => "plan",
            Self::Execute => "execute",
            Self::Retry => "retry",
            Self::Job => "job",
            Self::Recover => "recover",
            Self::Learn => "learn",
            Self::Failure => "failure",
            Self::Cancel => "cancel",
        }
    }
}

/// The outcome recorded on an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Started,
    Ok,
    Declined,
    Failed,
    Degraded,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Ok => "ok",
            Self::Declined => "declined",
            Self::Failed => "failed",
            Self::Degraded => "degraded",
        }
    }
}

/// One observability event.
#[derive(Debug, Clone)]
pub struct CapabilityEvent {
    pub correlation_id: String,
    pub provider_id: String,
    pub capability_id: Option<String>,
    pub stage: Stage,
    pub outcome: Outcome,
    pub detail: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl CapabilityEvent {
    pub fn new(
        correlation_id: impl Into<String>,
        provider_id: impl Into<String>,
        capability_id: Option<String>,
        stage: Stage,
        outcome: Outcome,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            provider_id: provider_id.into(),
            capability_id,
            stage,
            outcome,
            detail: detail.into(),
            timestamp: chrono::Utc::now(),
        }
    }
}

/// A bounded broadcast bus for [`CapabilityEvent`]s. Cloneable; subscribers get a
/// live stream. Lossy under lag (observability never blocks execution).
#[derive(Clone)]
pub struct CapabilityEventBus {
    tx: broadcast::Sender<CapabilityEvent>,
}

impl CapabilityEventBus {
    /// Create a bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity.max(16));
        Self { tx }
    }

    /// Subscribe to the live event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<CapabilityEvent> {
        self.tx.subscribe()
    }

    /// Emit an event (also mirrors to `tracing` for the durable log). Never
    /// blocks: if there are no subscribers or the buffer lagged, the event is
    /// dropped from the live stream but still traced.
    pub fn emit(&self, event: CapabilityEvent) {
        tracing::debug!(
            target: "capability_event",
            correlation_id = %event.correlation_id,
            provider = %event.provider_id,
            capability = ?event.capability_id,
            stage = %event.stage.as_str(),
            outcome = %event.outcome.as_str(),
            detail = %event.detail,
            "cpp event"
        );
        let _ = self.tx.send(event);
    }
}

impl Default for CapabilityEventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

/// Shared handle type used by the platform.
pub type SharedEventBus = Arc<CapabilityEventBus>;
