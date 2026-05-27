//! Turn Ownership FSM — Deterministic conversational state machine.
//!
//! Defines who owns the conversational floor at any moment:
//! - Exactly ONE owner at all times
//! - Explicit transitions only
//! - Deterministic cancellation
//! - Bounded interruption
//! - Generation-safe invalidation
//!
//! ## States
//! ```text
//! Idle → Listening → Processing → Speaking → Idle
//!                                     ↓
//!                                 Interrupting → Listening (barge-in)
//!                                     ↓
//!                                 Cancelling → Idle (abort)
//! ```
//!
//! ## Invariants
//! - No overlapping owners
//! - No half-cancelled states
//! - Interruption MUST invalidate active generation
//! - Cancelled refinement MUST NOT apply
//! - Interrupted TTS MUST stop deterministically
//! - Stale sidecar partials MUST drop immediately
//! - Restart MUST flush pending state

use serde::{Deserialize, Serialize};
use std::time::Instant;

// ─── Turn Owner State ─────────────────────────────────────────────────────

/// Who owns the conversational floor right now.
///
/// **Exactly one owner at all times.** No overlapping states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOwner {
    /// No active conversation. System is idle.
    Idle,
    /// User is speaking. STT is active. Audio is flowing.
    Listening,
    /// STT complete. LLM/refinement processing.
    Processing,
    /// Assistant is speaking. TTS is active.
    Speaking,
    /// Barge-in detected. Cancelling TTS, transitioning to Listening.
    Interrupting,
    /// Hard cancel in progress. Flushing all state.
    Cancelling,
    /// Sidecar restarting. Flushing stale state.
    Restarting,
}

impl TurnOwner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Processing => "processing",
            Self::Speaking => "speaking",
            Self::Interrupting => "interrupting",
            Self::Cancelling => "cancelling",
            Self::Restarting => "restarting",
        }
    }

    /// Returns true if the user currently holds the floor.
    pub fn is_user_turn(self) -> bool {
        matches!(self, Self::Listening)
    }

    /// Returns true if the assistant currently holds the floor.
    pub fn is_assistant_turn(self) -> bool {
        matches!(self, Self::Speaking)
    }

    /// Returns true if the system is in a transitional state.
    pub fn is_transitional(self) -> bool {
        matches!(
            self,
            Self::Interrupting | Self::Cancelling | Self::Restarting
        )
    }

    /// Returns true if audio should be flowing to STT.
    pub fn accepts_audio(self) -> bool {
        matches!(self, Self::Listening)
    }

    /// Returns true if TTS output should be active.
    pub fn produces_audio(self) -> bool {
        matches!(self, Self::Speaking)
    }
}

// ─── Interruption Cause ───────────────────────────────────────────────────

/// What caused the interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptionCause {
    /// User started speaking while assistant was speaking (barge-in).
    BargeIn,
    /// User explicitly cancelled (button press, command).
    UserCancel,
    /// System abort (error, timeout, resource pressure).
    SystemAbort,
    /// Sidecar crashed, needs restart.
    SidecarCrash,
    /// Session ended.
    SessionEnd,
}

// ─── Transition Events ────────────────────────────────────────────────────

/// Events that trigger turn ownership transitions.
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// User started speaking (VAD SpeechStart or push-to-talk).
    SpeechStart,
    /// STT finalized (UtteranceCommitted).
    SttFinalized,
    /// LLM/refinement processing complete, TTS starting.
    TtsStarting,
    /// TTS completed normally.
    TtsCompleted,
    /// Barge-in detected (VAD SpeechStart while Speaking).
    BargeIn,
    /// User cancel (explicit abort).
    UserCancel,
    /// System abort (error/timeout).
    SystemAbort,
    /// Sidecar crashed.
    SidecarCrash,
    /// Interruption/cancellation complete, ready for next state.
    TransitionComplete,
    /// Session ended.
    SessionEnd,
}

// ─── Invalidation Actions ─────────────────────────────────────────────────

/// Actions that MUST be performed on interruption/cancellation.
///
/// The FSM emits these; the runtime executes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidationAction {
    /// Cancel the current turn's CancellationToken.
    CancelTurnToken,
    /// Increment generation (invalidates all stale messages).
    IncrementGeneration,
    /// Flush pending audio queue.
    FlushAudioQueue,
    /// Flush pending partial queue.
    FlushPartialQueue,
    /// Cancel pending refinement (if any).
    CancelPendingRefinement,
    /// Stop TTS playback immediately.
    StopTts,
    /// Stop LLM token stream.
    StopLlm,
    /// Notify sidecar of generation change.
    NotifySidecarGenerationChange,
    /// Flush transcript authority to S0.
    ResetTranscriptAuthority,
}

// ─── Transition Result ────────────────────────────────────────────────────

/// Result of processing a turn event.
#[derive(Debug, Clone)]
pub struct TurnTransitionResult {
    /// Whether a state transition occurred.
    pub transitioned: bool,
    /// Previous state (before transition).
    pub from: TurnOwner,
    /// Current state (after transition).
    pub to: TurnOwner,
    /// Actions that MUST be executed by the runtime.
    pub actions: Vec<InvalidationAction>,
    /// Cause of interruption (if applicable).
    pub cause: Option<InterruptionCause>,
}

// ─── Turn Ownership FSM ───────────────────────────────────────────────────

/// Deterministic turn ownership state machine.
///
/// **Exactly one owner at all times.** Transitions are explicit.
/// The FSM emits invalidation actions; the runtime executes them.
#[derive(Debug)]
pub struct TurnOwnershipFsm {
    state: TurnOwner,
    generation: u64,
    /// Timestamp of last state change.
    last_transition: Instant,
    /// Count of interruptions this session (telemetry).
    interruption_count: u64,
    /// Count of barge-ins this session (telemetry).
    barge_in_count: u64,
}

impl TurnOwnershipFsm {
    /// Create a new FSM in Idle state.
    pub fn new(generation: u64) -> Self {
        Self {
            state: TurnOwner::Idle,
            generation,
            last_transition: Instant::now(),
            interruption_count: 0,
            barge_in_count: 0,
        }
    }

    /// Current state.
    pub fn state(&self) -> TurnOwner {
        self.state
    }

    /// Current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Time since last transition.
    pub fn time_in_state(&self) -> std::time::Duration {
        self.last_transition.elapsed()
    }

    /// Total interruptions this session.
    pub fn interruption_count(&self) -> u64 {
        self.interruption_count
    }

    /// Total barge-ins this session.
    pub fn barge_in_count(&self) -> u64 {
        self.barge_in_count
    }

    /// Process an event and return the transition result.
    ///
    /// The caller MUST execute all actions in the result.
    pub fn process_event(&mut self, event: TurnEvent) -> TurnTransitionResult {
        let from = self.state;

        match (&self.state, &event) {
            // ─── Idle transitions ─────────────────────────────────────
            (TurnOwner::Idle, TurnEvent::SpeechStart) => {
                self.transition_to(TurnOwner::Listening);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }

            // ─── Listening transitions ────────────────────────────────
            (TurnOwner::Listening, TurnEvent::SttFinalized) => {
                self.transition_to(TurnOwner::Processing);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }
            (TurnOwner::Listening, TurnEvent::UserCancel) => {
                self.handle_cancel(from, InterruptionCause::UserCancel)
            }
            (TurnOwner::Listening, TurnEvent::SystemAbort) => {
                self.handle_cancel(from, InterruptionCause::SystemAbort)
            }
            (TurnOwner::Listening, TurnEvent::SidecarCrash) => self.handle_sidecar_crash(from),

            // ─── Processing transitions ───────────────────────────────
            (TurnOwner::Processing, TurnEvent::TtsStarting) => {
                self.transition_to(TurnOwner::Speaking);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }
            (TurnOwner::Processing, TurnEvent::UserCancel) => {
                self.handle_cancel(from, InterruptionCause::UserCancel)
            }
            (TurnOwner::Processing, TurnEvent::SystemAbort) => {
                self.handle_cancel(from, InterruptionCause::SystemAbort)
            }

            // ─── Speaking transitions ─────────────────────────────────
            (TurnOwner::Speaking, TurnEvent::TtsCompleted) => {
                self.transition_to(TurnOwner::Idle);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }
            (TurnOwner::Speaking, TurnEvent::BargeIn) => self.handle_barge_in(from),
            (TurnOwner::Speaking, TurnEvent::UserCancel) => {
                self.handle_cancel(from, InterruptionCause::UserCancel)
            }
            (TurnOwner::Speaking, TurnEvent::SystemAbort) => {
                self.handle_cancel(from, InterruptionCause::SystemAbort)
            }

            // ─── Interrupting transitions ─────────────────────────────
            (TurnOwner::Interrupting, TurnEvent::TransitionComplete) => {
                // Barge-in complete → back to Listening
                self.transition_to(TurnOwner::Listening);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }
            (TurnOwner::Interrupting, TurnEvent::UserCancel) => {
                self.handle_cancel(from, InterruptionCause::UserCancel)
            }

            // ─── Cancelling transitions ───────────────────────────────
            (TurnOwner::Cancelling, TurnEvent::TransitionComplete) => {
                self.transition_to(TurnOwner::Idle);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }

            // ─── Restarting transitions ───────────────────────────────
            (TurnOwner::Restarting, TurnEvent::TransitionComplete) => {
                self.transition_to(TurnOwner::Idle);
                TurnTransitionResult {
                    transitioned: true,
                    from,
                    to: self.state,
                    actions: vec![],
                    cause: None,
                }
            }

            // ─── Session end from any state ───────────────────────────
            (_, TurnEvent::SessionEnd) => self.handle_cancel(from, InterruptionCause::SessionEnd),

            // ─── Sidecar crash from any active state ──────────────────
            (_, TurnEvent::SidecarCrash) => self.handle_sidecar_crash(from),

            // ─── Invalid/no-op transitions ────────────────────────────
            _ => TurnTransitionResult {
                transitioned: false,
                from,
                to: self.state,
                actions: vec![],
                cause: None,
            },
        }
    }

    // ─── Internal Handlers ────────────────────────────────────────────────

    fn handle_barge_in(&mut self, from: TurnOwner) -> TurnTransitionResult {
        self.transition_to(TurnOwner::Interrupting);
        self.barge_in_count += 1;
        self.interruption_count += 1;
        self.generation = self.generation.wrapping_add(1);

        TurnTransitionResult {
            transitioned: true,
            from,
            to: self.state,
            actions: vec![
                InvalidationAction::CancelTurnToken,
                InvalidationAction::IncrementGeneration,
                InvalidationAction::StopTts,
                InvalidationAction::StopLlm,
                InvalidationAction::FlushPartialQueue,
                InvalidationAction::CancelPendingRefinement,
                InvalidationAction::NotifySidecarGenerationChange,
            ],
            cause: Some(InterruptionCause::BargeIn),
        }
    }

    fn handle_cancel(&mut self, from: TurnOwner, cause: InterruptionCause) -> TurnTransitionResult {
        self.transition_to(TurnOwner::Cancelling);
        self.interruption_count += 1;
        self.generation = self.generation.wrapping_add(1);

        TurnTransitionResult {
            transitioned: true,
            from,
            to: self.state,
            actions: vec![
                InvalidationAction::CancelTurnToken,
                InvalidationAction::IncrementGeneration,
                InvalidationAction::StopTts,
                InvalidationAction::StopLlm,
                InvalidationAction::FlushAudioQueue,
                InvalidationAction::FlushPartialQueue,
                InvalidationAction::CancelPendingRefinement,
                InvalidationAction::ResetTranscriptAuthority,
                InvalidationAction::NotifySidecarGenerationChange,
            ],
            cause: Some(cause),
        }
    }

    fn handle_sidecar_crash(&mut self, from: TurnOwner) -> TurnTransitionResult {
        self.transition_to(TurnOwner::Restarting);
        self.interruption_count += 1;
        self.generation = self.generation.wrapping_add(1);

        TurnTransitionResult {
            transitioned: true,
            from,
            to: self.state,
            actions: vec![
                InvalidationAction::IncrementGeneration,
                InvalidationAction::FlushAudioQueue,
                InvalidationAction::FlushPartialQueue,
                InvalidationAction::CancelPendingRefinement,
                InvalidationAction::NotifySidecarGenerationChange,
                InvalidationAction::ResetTranscriptAuthority,
            ],
            cause: Some(InterruptionCause::SidecarCrash),
        }
    }

    fn transition_to(&mut self, to: TurnOwner) {
        tracing::debug!(
            from = self.state.as_str(),
            to = to.as_str(),
            generation = self.generation,
            "turn ownership transition"
        );
        self.state = to;
        self.last_transition = Instant::now();
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsm_starts_idle() {
        let fsm = TurnOwnershipFsm::new(0);
        assert_eq!(fsm.state(), TurnOwner::Idle);
        assert_eq!(fsm.generation(), 0);
        assert_eq!(fsm.interruption_count(), 0);
        assert_eq!(fsm.barge_in_count(), 0);
    }

    #[test]
    fn idle_to_listening_on_speech_start() {
        let mut fsm = TurnOwnershipFsm::new(0);
        let result = fsm.process_event(TurnEvent::SpeechStart);
        assert!(result.transitioned);
        assert_eq!(result.from, TurnOwner::Idle);
        assert_eq!(result.to, TurnOwner::Listening);
        assert!(result.actions.is_empty());
        assert_eq!(fsm.state(), TurnOwner::Listening);
    }

    #[test]
    fn listening_to_processing_on_stt_finalized() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        let result = fsm.process_event(TurnEvent::SttFinalized);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Processing);
    }

    #[test]
    fn processing_to_speaking_on_tts_starting() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        let result = fsm.process_event(TurnEvent::TtsStarting);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Speaking);
    }

    #[test]
    fn speaking_to_idle_on_tts_completed() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        let result = fsm.process_event(TurnEvent::TtsCompleted);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Idle);
    }

    #[test]
    fn full_happy_path() {
        let mut fsm = TurnOwnershipFsm::new(0);
        assert_eq!(fsm.state(), TurnOwner::Idle);

        fsm.process_event(TurnEvent::SpeechStart);
        assert_eq!(fsm.state(), TurnOwner::Listening);

        fsm.process_event(TurnEvent::SttFinalized);
        assert_eq!(fsm.state(), TurnOwner::Processing);

        fsm.process_event(TurnEvent::TtsStarting);
        assert_eq!(fsm.state(), TurnOwner::Speaking);

        fsm.process_event(TurnEvent::TtsCompleted);
        assert_eq!(fsm.state(), TurnOwner::Idle);

        // No interruptions
        assert_eq!(fsm.interruption_count(), 0);
        assert_eq!(fsm.barge_in_count(), 0);
    }

    #[test]
    fn barge_in_during_speaking() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);

        let result = fsm.process_event(TurnEvent::BargeIn);
        assert!(result.transitioned);
        assert_eq!(result.from, TurnOwner::Speaking);
        assert_eq!(result.to, TurnOwner::Interrupting);
        assert_eq!(result.cause, Some(InterruptionCause::BargeIn));

        // Must emit invalidation actions
        assert!(result
            .actions
            .contains(&InvalidationAction::CancelTurnToken));
        assert!(result
            .actions
            .contains(&InvalidationAction::IncrementGeneration));
        assert!(result.actions.contains(&InvalidationAction::StopTts));
        assert!(result.actions.contains(&InvalidationAction::StopLlm));
        assert!(result
            .actions
            .contains(&InvalidationAction::CancelPendingRefinement));

        // Generation incremented
        assert_eq!(fsm.generation(), 1);
        assert_eq!(fsm.barge_in_count(), 1);
    }

    #[test]
    fn barge_in_completes_to_listening() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        fsm.process_event(TurnEvent::BargeIn);

        let result = fsm.process_event(TurnEvent::TransitionComplete);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Listening);
    }

    #[test]
    fn user_cancel_during_listening() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);

        let result = fsm.process_event(TurnEvent::UserCancel);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);
        assert_eq!(result.cause, Some(InterruptionCause::UserCancel));
        assert!(result
            .actions
            .contains(&InvalidationAction::ResetTranscriptAuthority));
    }

    #[test]
    fn user_cancel_during_speaking() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);

        let result = fsm.process_event(TurnEvent::UserCancel);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);
        assert!(result.actions.contains(&InvalidationAction::StopTts));
    }

    #[test]
    fn cancel_completes_to_idle() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::UserCancel);

        let result = fsm.process_event(TurnEvent::TransitionComplete);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Idle);
    }

    #[test]
    fn sidecar_crash_during_listening() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);

        let result = fsm.process_event(TurnEvent::SidecarCrash);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Restarting);
        assert_eq!(result.cause, Some(InterruptionCause::SidecarCrash));
        assert!(result
            .actions
            .contains(&InvalidationAction::FlushAudioQueue));
        assert!(result
            .actions
            .contains(&InvalidationAction::FlushPartialQueue));
    }

    #[test]
    fn sidecar_crash_completes_to_idle() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SidecarCrash);

        let result = fsm.process_event(TurnEvent::TransitionComplete);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Idle);
    }

    #[test]
    fn generation_increments_on_interruption() {
        let mut fsm = TurnOwnershipFsm::new(0);
        assert_eq!(fsm.generation(), 0);

        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        fsm.process_event(TurnEvent::BargeIn);
        assert_eq!(fsm.generation(), 1);

        fsm.process_event(TurnEvent::TransitionComplete);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        fsm.process_event(TurnEvent::UserCancel);
        assert_eq!(fsm.generation(), 2);
    }

    #[test]
    fn generation_wraps_safely() {
        let mut fsm = TurnOwnershipFsm::new(u64::MAX);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::UserCancel);
        assert_eq!(fsm.generation(), 0); // wrapped
    }

    #[test]
    fn no_op_transitions_dont_change_state() {
        let mut fsm = TurnOwnershipFsm::new(0);

        // BargeIn while Idle → no-op
        let result = fsm.process_event(TurnEvent::BargeIn);
        assert!(!result.transitioned);
        assert_eq!(fsm.state(), TurnOwner::Idle);

        // TtsCompleted while Idle → no-op
        let result = fsm.process_event(TurnEvent::TtsCompleted);
        assert!(!result.transitioned);
        assert_eq!(fsm.state(), TurnOwner::Idle);

        // SttFinalized while Idle → no-op
        let result = fsm.process_event(TurnEvent::SttFinalized);
        assert!(!result.transitioned);
        assert_eq!(fsm.state(), TurnOwner::Idle);
    }

    #[test]
    fn session_end_from_any_state() {
        // From Idle
        let mut fsm = TurnOwnershipFsm::new(0);
        let result = fsm.process_event(TurnEvent::SessionEnd);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);

        // From Speaking
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        let result = fsm.process_event(TurnEvent::SessionEnd);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);
        assert!(result.actions.contains(&InvalidationAction::StopTts));
    }

    #[test]
    fn rapid_barge_in_storm() {
        let mut fsm = TurnOwnershipFsm::new(0);

        for i in 0..10 {
            // Start speaking
            fsm.process_event(TurnEvent::SpeechStart);
            fsm.process_event(TurnEvent::SttFinalized);
            fsm.process_event(TurnEvent::TtsStarting);

            // Barge-in
            fsm.process_event(TurnEvent::BargeIn);
            assert_eq!(fsm.state(), TurnOwner::Interrupting);

            // Complete interruption
            fsm.process_event(TurnEvent::TransitionComplete);
            assert_eq!(fsm.state(), TurnOwner::Listening);

            assert_eq!(fsm.barge_in_count(), (i + 1) as u64);
        }

        assert_eq!(fsm.barge_in_count(), 10);
        assert_eq!(fsm.generation(), 10);
    }

    #[test]
    fn cancel_during_interruption() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);
        fsm.process_event(TurnEvent::BargeIn);

        // Cancel while interrupting
        let result = fsm.process_event(TurnEvent::UserCancel);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);
    }

    #[test]
    fn turn_owner_properties() {
        assert!(TurnOwner::Listening.is_user_turn());
        assert!(!TurnOwner::Speaking.is_user_turn());

        assert!(TurnOwner::Speaking.is_assistant_turn());
        assert!(!TurnOwner::Listening.is_assistant_turn());

        assert!(TurnOwner::Interrupting.is_transitional());
        assert!(TurnOwner::Cancelling.is_transitional());
        assert!(TurnOwner::Restarting.is_transitional());
        assert!(!TurnOwner::Idle.is_transitional());

        assert!(TurnOwner::Listening.accepts_audio());
        assert!(!TurnOwner::Speaking.accepts_audio());

        assert!(TurnOwner::Speaking.produces_audio());
        assert!(!TurnOwner::Listening.produces_audio());
    }

    #[test]
    fn invalidation_actions_on_barge_in_are_complete() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);

        let result = fsm.process_event(TurnEvent::BargeIn);

        // All required actions present
        let actions = &result.actions;
        assert!(actions.contains(&InvalidationAction::CancelTurnToken));
        assert!(actions.contains(&InvalidationAction::IncrementGeneration));
        assert!(actions.contains(&InvalidationAction::StopTts));
        assert!(actions.contains(&InvalidationAction::StopLlm));
        assert!(actions.contains(&InvalidationAction::FlushPartialQueue));
        assert!(actions.contains(&InvalidationAction::CancelPendingRefinement));
        assert!(actions.contains(&InvalidationAction::NotifySidecarGenerationChange));

        // Barge-in does NOT reset transcript authority (user is still talking)
        assert!(!actions.contains(&InvalidationAction::ResetTranscriptAuthority));
    }

    #[test]
    fn invalidation_actions_on_cancel_are_complete() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);
        fsm.process_event(TurnEvent::TtsStarting);

        let result = fsm.process_event(TurnEvent::UserCancel);

        // All required actions present
        let actions = &result.actions;
        assert!(actions.contains(&InvalidationAction::CancelTurnToken));
        assert!(actions.contains(&InvalidationAction::IncrementGeneration));
        assert!(actions.contains(&InvalidationAction::StopTts));
        assert!(actions.contains(&InvalidationAction::StopLlm));
        assert!(actions.contains(&InvalidationAction::FlushAudioQueue));
        assert!(actions.contains(&InvalidationAction::FlushPartialQueue));
        assert!(actions.contains(&InvalidationAction::CancelPendingRefinement));
        assert!(actions.contains(&InvalidationAction::ResetTranscriptAuthority));
        assert!(actions.contains(&InvalidationAction::NotifySidecarGenerationChange));
    }

    #[test]
    fn turn_owner_as_str() {
        assert_eq!(TurnOwner::Idle.as_str(), "idle");
        assert_eq!(TurnOwner::Listening.as_str(), "listening");
        assert_eq!(TurnOwner::Processing.as_str(), "processing");
        assert_eq!(TurnOwner::Speaking.as_str(), "speaking");
        assert_eq!(TurnOwner::Interrupting.as_str(), "interrupting");
        assert_eq!(TurnOwner::Cancelling.as_str(), "cancelling");
        assert_eq!(TurnOwner::Restarting.as_str(), "restarting");
    }

    #[test]
    fn system_abort_during_processing() {
        let mut fsm = TurnOwnershipFsm::new(0);
        fsm.process_event(TurnEvent::SpeechStart);
        fsm.process_event(TurnEvent::SttFinalized);

        let result = fsm.process_event(TurnEvent::SystemAbort);
        assert!(result.transitioned);
        assert_eq!(result.to, TurnOwner::Cancelling);
        assert_eq!(result.cause, Some(InterruptionCause::SystemAbort));
    }
}
