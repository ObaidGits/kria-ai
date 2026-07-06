//! SkillEvent — the single observability stream for OpenClaw execution (event-contract INV-7).
//!
//! Every execution emits `SkillEvent`s through one process-global broadcast channel. UI, audit,
//! telemetry, and analytics are all projections of this stream — there is no parallel logging
//! system. Events are also mirrored to `tracing` for logs.

use super::types::ExecutionSource;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tokio::sync::broadcast;

/// Closed lifecycle stage set (event-contract §2). Maps 1:1 to execution-contract phases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Started,
    Preparing,
    Waiting,
    Running,
    Streaming,
    Completed,
    Cancelled,
    Preempted,
    Failed,
    Retrying,
    Recovered,
}

/// Closed failure taxonomy (event-contract §6). Free-form text lives in `FailureInfo.message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    AdmissionDenied,
    Timeout,
    Oom,
    NetworkDenied,
    CapabilityViolation,
    UnknownTool,
    HandlerError,
    RuntimeCrash,
    WorkerUnreachable,
    PolicyDenied,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureInfo {
    pub kind: FailureKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
}

/// Actual resource usage sampled at cleanup (resource-contract §6 / RES-5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_ms: u64,
    pub peak_mem_bytes: u64,
    pub gpu_ms: u64,
    pub storage_peak_bytes: u64,
}

/// Capability lifecycle actions (A3.10). Carried on the SAME event stream (one bus) via the
/// optional `capability` field — no duplicate event bus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAction {
    Requested,
    Granted,
    Denied,
    Revoked,
    Expired,
    Escalated,
    Reduced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEventInfo {
    pub action: CapabilityAction,
    /// Approval hash identifying the exact granted set (security-contract).
    pub capability_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The single event every execution emits (event-contract §1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEvent {
    pub correlation_id: String,
    pub execution_id: String,
    pub skill_id: String,
    pub version: String,
    pub source: String,
    pub runtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_ref: Option<String>,
    pub stage: Stage,
    pub ts: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_wait_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureInfo>,
    /// Capability lifecycle payload (A3.10) — same stream, additive field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<CapabilityEventInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl SkillEvent {
    /// Construct a minimally-valid event for a stage. Callers enrich stage-specific fields.
    pub fn new(
        correlation_id: impl Into<String>,
        execution_id: impl Into<String>,
        skill_id: impl Into<String>,
        source: ExecutionSource,
        runtime: &str,
        stage: Stage,
    ) -> Self {
        Self {
            correlation_id: correlation_id.into(),
            execution_id: execution_id.into(),
            skill_id: skill_id.into(),
            version: String::new(),
            source: source.as_str().to_string(),
            runtime: runtime.to_string(),
            instance_ref: None,
            stage,
            ts: chrono::Utc::now(),
            resource: None,
            latency_ms: None,
            queue_wait_ms: None,
            failure: None,
            capability: None,
            reason: None,
        }
    }

    pub fn with_instance(mut self, instance_ref: impl Into<String>) -> Self {
        self.instance_ref = Some(instance_ref.into());
        self
    }

    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = Some(latency_ms);
        self
    }

    pub fn with_failure(mut self, failure: FailureInfo) -> Self {
        self.failure = Some(failure);
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_capability(mut self, info: CapabilityEventInfo) -> Self {
        self.capability = Some(info);
        self
    }
}

/// Emit a capability lifecycle event on the single stream (A3.10).
#[allow(clippy::too_many_arguments)]
pub fn emit_capability(
    correlation_id: &str,
    execution_id: &str,
    skill_id: &str,
    action: CapabilityAction,
    capability_hash: &str,
    publisher: Option<String>,
    risk: Option<String>,
    detail: Option<String>,
) {
    let stage = match action {
        CapabilityAction::Requested => Stage::Preparing,
        CapabilityAction::Denied => Stage::Failed,
        CapabilityAction::Revoked => Stage::Cancelled,
        _ => Stage::Running,
    };
    let ev = SkillEvent::new(
        correlation_id,
        execution_id,
        skill_id,
        ExecutionSource::OpenClaw,
        "docker",
        stage,
    )
    .with_capability(CapabilityEventInfo {
        action,
        capability_hash: capability_hash.to_string(),
        publisher,
        risk,
        detail,
    });
    emit(ev);
}

/// Process-global event bus. Lazily created; capacity bounded (lagging subscribers drop oldest).
static EVENT_BUS: OnceLock<broadcast::Sender<SkillEvent>> = OnceLock::new();

fn sender() -> &'static broadcast::Sender<SkillEvent> {
    EVENT_BUS.get_or_init(|| {
        let (tx, _rx) = broadcast::channel(256);
        tx
    })
}

/// Subscribe to the OpenClaw skill-event stream (UI / telemetry / audit projections).
pub fn subscribe() -> broadcast::Receiver<SkillEvent> {
    sender().subscribe()
}

/// Emit a skill event to the single stream + mirror to tracing. Never blocks; drops if no
/// subscribers (broadcast semantics).
pub fn emit(event: SkillEvent) {
    tracing::info!(
        target: "openclaw::event",
        correlation_id = %event.correlation_id,
        execution_id = %event.execution_id,
        skill_id = %event.skill_id,
        runtime = %event.runtime,
        stage = ?event.stage,
        latency_ms = ?event.latency_ms,
        failure = ?event.failure.as_ref().map(|f| &f.kind),
        "openclaw skill event"
    );
    let _ = sender().send(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_are_broadcast_to_subscribers() {
        let mut rx = subscribe();
        emit(SkillEvent::new(
            "corr-1",
            "exec-1",
            "oc_calculator",
            ExecutionSource::OpenClaw,
            "docker",
            Stage::Started,
        ));
        let ev = rx.recv().await.expect("event received");
        assert_eq!(ev.skill_id, "oc_calculator");
        assert_eq!(ev.stage, Stage::Started);
        assert_eq!(ev.runtime, "docker");
    }

    #[test]
    fn event_serializes_compactly() {
        let ev = SkillEvent::new(
            "c",
            "e",
            "oc_x",
            ExecutionSource::OpenClaw,
            "docker",
            Stage::Completed,
        )
        .with_latency(42);
        let j = serde_json::to_value(&ev).unwrap();
        assert_eq!(j["stage"], "completed");
        assert_eq!(j["latency_ms"], 42);
        // Optional None fields are omitted.
        assert!(j.get("failure").is_none());
    }
}
