//! Runtime Bridge — Wires P2 FSMs into the live v2 pipeline.
//!
//! Connects:
//! - TranscriptAuthorityFsm (§6) → pipeline transcript flow
//! - TurnOwnershipFsm → pipeline state machine
//! - RuntimeTelemetry → pipeline metrics
//! - SidecarIpc → pipeline audio/partial transport
//!
//! ## Design
//!
//! The bridge is a **coordinator**, not an orchestrator. It does NOT
//! make decisions — it routes events between FSMs and the pipeline.
//! All decisions are made by the FSMs themselves.
//!
//! ## Invariants
//! - No hidden orchestration logic
//! - No speculative execution
//! - Preserves all P0-P2 invariants
//! - Bounded, deterministic, cancellation-correct

use std::sync::Arc;
use tokio::sync::Mutex;

use super::runtime_telemetry::{
    DegradationLevel, LatencyHistogram, QueueMonitor, QueuePressure, RuntimeLoadSnapshot,
    TtfaTracker, WorkerBudget,
};
use super::transcript_authority::{TranscriptAuthorityFsm, TranscriptEvent, TranscriptState};
use super::turn_ownership::{TurnEvent, TurnOwner, TurnOwnershipFsm, TurnTransitionResult};

// ─── Runtime Bridge ───────────────────────────────────────────────────────

/// Production runtime bridge connecting P2 FSMs to the live pipeline.
///
/// **Not an orchestrator.** Routes events, does not make decisions.
pub struct RuntimeBridge {
    /// Transcript authority FSM (§6).
    pub transcript: Arc<Mutex<TranscriptAuthorityFsm>>,
    /// Turn ownership FSM.
    pub turn: Arc<Mutex<TurnOwnershipFsm>>,
    /// TTFA tracker.
    pub ttfa: Arc<Mutex<TtfaTracker>>,
    /// Interrupt latency histogram.
    pub interrupt_latency: Arc<Mutex<LatencyHistogram>>,
    /// Cancel latency histogram.
    pub cancel_latency: Arc<Mutex<LatencyHistogram>>,
    /// Audio queue monitor.
    pub audio_queue: Arc<Mutex<QueueMonitor>>,
    /// Partial queue monitor.
    pub partial_queue: Arc<Mutex<QueueMonitor>>,
    /// Whisper worker budget (§9: max 1 concurrent).
    pub whisper_budget: Arc<Mutex<WorkerBudget>>,
    /// Current degradation level.
    pub degradation: Arc<Mutex<DegradationLevel>>,
    /// Total turns processed.
    pub total_turns: Arc<Mutex<u64>>,
}

impl RuntimeBridge {
    /// Create a new runtime bridge with default configuration.
    pub fn new(ttfa_budget_ms: u64) -> Self {
        Self {
            transcript: Arc::new(Mutex::new(TranscriptAuthorityFsm::new(0))),
            turn: Arc::new(Mutex::new(TurnOwnershipFsm::new(0))),
            ttfa: Arc::new(Mutex::new(TtfaTracker::new(ttfa_budget_ms, 256))),
            interrupt_latency: Arc::new(Mutex::new(LatencyHistogram::new(256))),
            cancel_latency: Arc::new(Mutex::new(LatencyHistogram::new(256))),
            audio_queue: Arc::new(Mutex::new(QueueMonitor::new("audio", 64))),
            partial_queue: Arc::new(Mutex::new(QueueMonitor::new("partial", 64))),
            whisper_budget: Arc::new(Mutex::new(WorkerBudget::new("whisper", 1))),
            degradation: Arc::new(Mutex::new(DegradationLevel::None)),
            total_turns: Arc::new(Mutex::new(0)),
        }
    }

    /// Process a turn event and return invalidation actions.
    pub async fn process_turn_event(&self, event: TurnEvent) -> TurnTransitionResult {
        let result = {
            let mut turn = self.turn.lock().await;
            turn.process_event(event)
        };

        if result.transitioned {
            tracing::info!(
                from = result.from.as_str(),
                to = result.to.as_str(),
                actions = result.actions.len(),
                "turn ownership transition"
            );
        }

        result
    }

    /// Process a transcript event.
    pub async fn process_transcript_event(&self, event: TranscriptEvent) -> bool {
        let mut transcript = self.transcript.lock().await;
        transcript.process_event(event)
    }

    /// Get the current user-visible transcript.
    pub async fn user_visible_transcript(&self) -> String {
        let transcript = self.transcript.lock().await;
        transcript.user_visible().to_string()
    }

    /// Get the committed transcript (for execution).
    pub async fn committed_transcript(&self) -> String {
        let transcript = self.transcript.lock().await;
        transcript.committed().to_string()
    }

    /// Get the current transcript state.
    pub async fn transcript_state(&self) -> TranscriptState {
        let transcript = self.transcript.lock().await;
        transcript.state()
    }

    /// Get the current turn owner.
    pub async fn turn_owner(&self) -> TurnOwner {
        let turn = self.turn.lock().await;
        turn.state()
    }

    /// Record a TTFA measurement.
    pub async fn record_ttfa(&self, ttfa_ms: u64) {
        let mut ttfa = self.ttfa.lock().await;
        ttfa.record(ttfa_ms);
    }

    /// Record an interrupt latency measurement.
    pub async fn record_interrupt_latency(&self, ms: u64) {
        let mut hist = self.interrupt_latency.lock().await;
        hist.record(ms);
    }

    /// Record a cancel latency measurement.
    pub async fn record_cancel_latency(&self, ms: u64) {
        let mut hist = self.cancel_latency.lock().await;
        hist.record(ms);
    }

    /// Update audio queue depth.
    pub async fn update_audio_queue(&self, depth: usize) -> QueuePressure {
        let mut monitor = self.audio_queue.lock().await;
        monitor.update(depth)
    }

    /// Update partial queue depth.
    pub async fn update_partial_queue(&self, depth: usize) -> QueuePressure {
        let mut monitor = self.partial_queue.lock().await;
        monitor.update(depth)
    }

    /// Try to acquire a Whisper worker slot.
    pub async fn try_acquire_whisper(&self) -> bool {
        let mut budget = self.whisper_budget.lock().await;
        budget.try_acquire()
    }

    /// Release a Whisper worker slot.
    pub async fn release_whisper(&self) {
        let mut budget = self.whisper_budget.lock().await;
        budget.release();
    }

    /// Check if refinement should be skipped (degradation).
    pub async fn should_skip_refinement(&self) -> bool {
        let level = self.degradation.lock().await;
        level.skip_refinement()
    }

    /// Update degradation level based on current signals.
    pub async fn update_degradation(&self) {
        let audio_pressure = {
            let monitor = self.audio_queue.lock().await;
            monitor.last_pressure
        };
        let ttfa_overrun = {
            let ttfa = self.ttfa.lock().await;
            ttfa.overrun_rate()
        };
        let whisper_util = {
            let budget = self.whisper_budget.lock().await;
            budget.utilization()
        };

        let new_level = DegradationLevel::from_signals(audio_pressure, ttfa_overrun, whisper_util);

        let mut current = self.degradation.lock().await;
        if *current != new_level {
            tracing::info!(
                from = ?*current,
                to = ?new_level,
                "degradation level changed"
            );
            *current = new_level;
        }
    }

    /// Increment total turns counter.
    pub async fn increment_turns(&self) {
        let mut turns = self.total_turns.lock().await;
        *turns += 1;
    }

    /// Get a runtime load snapshot for telemetry/diagnostics.
    pub async fn load_snapshot(&self) -> RuntimeLoadSnapshot {
        let ttfa = self.ttfa.lock().await;
        let interrupt = self.interrupt_latency.lock().await;
        let cancel = self.cancel_latency.lock().await;
        let audio = self.audio_queue.lock().await;
        let partial = self.partial_queue.lock().await;
        let whisper = self.whisper_budget.lock().await;
        let turn = self.turn.lock().await;
        let turns = self.total_turns.lock().await;

        RuntimeLoadSnapshot {
            ttfa_p50_ms: ttfa.histogram.p50(),
            ttfa_p95_ms: ttfa.histogram.p95(),
            ttfa_p99_ms: ttfa.histogram.p99(),
            ttfa_overrun_rate: ttfa.overrun_rate(),
            interrupt_latency_p50_ms: interrupt.p50(),
            interrupt_latency_p95_ms: interrupt.p95(),
            cancel_latency_p50_ms: cancel.p50(),
            cancel_latency_p95_ms: cancel.p95(),
            audio_queue_pressure: audio.last_pressure.as_str().to_string(),
            partial_queue_pressure: partial.last_pressure.as_str().to_string(),
            whisper_worker_utilization: whisper.utilization(),
            total_turns: *turns,
            total_interruptions: turn.interruption_count(),
            total_barge_ins: turn.barge_in_count(),
        }
    }

    /// Reset the bridge for a new session.
    pub async fn reset(&self, generation: u64) {
        *self.transcript.lock().await = TranscriptAuthorityFsm::new(generation);
        *self.turn.lock().await = TurnOwnershipFsm::new(generation);
        *self.total_turns.lock().await = 0;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bridge_creation() {
        let bridge = RuntimeBridge::new(500);
        assert_eq!(bridge.turn_owner().await, TurnOwner::Idle);
        assert_eq!(bridge.transcript_state().await, TranscriptState::S0Idle);
        assert_eq!(bridge.user_visible_transcript().await, "");
    }

    #[tokio::test]
    async fn bridge_turn_event_routing() {
        let bridge = RuntimeBridge::new(500);

        let result = bridge.process_turn_event(TurnEvent::SpeechStart).await;
        assert!(result.transitioned);
        assert_eq!(bridge.turn_owner().await, TurnOwner::Listening);
    }

    #[tokio::test]
    async fn bridge_transcript_event_routing() {
        let bridge = RuntimeBridge::new(500);

        let transitioned = bridge
            .process_transcript_event(TranscriptEvent::FirstPartial {
                text: "hello".to_string(),
                generation: 0,
            })
            .await;
        assert!(transitioned);
        assert_eq!(
            bridge.transcript_state().await,
            TranscriptState::S1Speculative
        );
        assert_eq!(bridge.user_visible_transcript().await, "hello");
    }

    #[tokio::test]
    async fn bridge_ttfa_tracking() {
        let bridge = RuntimeBridge::new(500);
        bridge.record_ttfa(300).await;
        bridge.record_ttfa(600).await;

        let snapshot = bridge.load_snapshot().await;
        assert!(snapshot.ttfa_p50_ms.is_some());
        assert!(snapshot.ttfa_overrun_rate > 0.0); // 1 of 2 exceeded 500ms
    }

    #[tokio::test]
    async fn bridge_queue_monitoring() {
        let bridge = RuntimeBridge::new(500);

        let pressure = bridge.update_audio_queue(10).await;
        assert_eq!(pressure, QueuePressure::Normal);

        let pressure = bridge.update_audio_queue(62).await;
        assert_eq!(pressure, QueuePressure::Critical); // 62/64 = 96.8%
    }

    #[tokio::test]
    async fn bridge_whisper_budget() {
        let bridge = RuntimeBridge::new(500);

        assert!(bridge.try_acquire_whisper().await);
        assert!(!bridge.try_acquire_whisper().await); // at capacity (1)

        bridge.release_whisper().await;
        assert!(bridge.try_acquire_whisper().await); // freed
    }

    #[tokio::test]
    async fn bridge_degradation_update() {
        let bridge = RuntimeBridge::new(500);

        // Normal state
        bridge.update_degradation().await;
        assert!(!bridge.should_skip_refinement().await);

        // Simulate high queue pressure
        bridge.update_audio_queue(62).await; // >95% of 64 = critical
        bridge.update_degradation().await;
        assert!(bridge.should_skip_refinement().await);
    }

    #[tokio::test]
    async fn bridge_load_snapshot() {
        let bridge = RuntimeBridge::new(500);
        bridge.record_ttfa(200).await;
        bridge.record_interrupt_latency(30).await;
        bridge.record_cancel_latency(10).await;
        bridge.increment_turns().await;

        let snapshot = bridge.load_snapshot().await;
        assert_eq!(snapshot.total_turns, 1);
        assert_eq!(snapshot.ttfa_p50_ms, Some(200));
        assert_eq!(snapshot.interrupt_latency_p50_ms, Some(30));
        assert_eq!(snapshot.cancel_latency_p50_ms, Some(10));
    }

    #[tokio::test]
    async fn bridge_reset() {
        let bridge = RuntimeBridge::new(500);
        bridge.process_turn_event(TurnEvent::SpeechStart).await;
        bridge.increment_turns().await;

        bridge.reset(5).await;
        assert_eq!(bridge.turn_owner().await, TurnOwner::Idle);
        assert_eq!(bridge.transcript_state().await, TranscriptState::S0Idle);

        let snapshot = bridge.load_snapshot().await;
        assert_eq!(snapshot.total_turns, 0);
    }

    #[tokio::test]
    async fn bridge_full_turn_lifecycle() {
        let bridge = RuntimeBridge::new(500);

        // Speech start
        bridge.process_turn_event(TurnEvent::SpeechStart).await;
        bridge
            .process_transcript_event(TranscriptEvent::FirstPartial {
                text: "hello".to_string(),
                generation: 0,
            })
            .await;

        // STT finalized
        bridge.process_turn_event(TurnEvent::SttFinalized).await;
        bridge
            .process_transcript_event(TranscriptEvent::UtteranceCommitted {
                text: "hello world".to_string(),
                generation: 0,
            })
            .await;

        // Processing → Speaking
        bridge.process_turn_event(TurnEvent::TtsStarting).await;
        assert_eq!(bridge.turn_owner().await, TurnOwner::Speaking);
        assert_eq!(bridge.committed_transcript().await, "hello world");

        // TTS complete
        bridge.process_turn_event(TurnEvent::TtsCompleted).await;
        assert_eq!(bridge.turn_owner().await, TurnOwner::Idle);

        bridge.increment_turns().await;
        bridge.record_ttfa(350).await;

        let snapshot = bridge.load_snapshot().await;
        assert_eq!(snapshot.total_turns, 1);
    }
}
