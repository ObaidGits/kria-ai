//! Transcript Authority FSM — `ENHANCED_STT.md` §6 (normative)
//!
//! Implements the single-source-of-truth transcript lifecycle:
//!
//! ```text
//! S0_Idle → S1_Speculative → S2_Stabilizing → S3_Committed → S4_RefinedFinal
//! ```
//!
//! ## Ownership Rules (MUST hold)
//! - Execution ALWAYS uses S3 committed transcript
//! - Refinement MAY improve UI transcript (S4) only
//! - Partials are advisory, NOT authoritative
//! - Whisper MUST NOT become transcript authority
//!
//! ## Authority Transitions (§6.2)
//! - S1→S2: stable=true from sidecar OR prefix hold rule (2 consecutive, ≥120ms)
//! - S2→S3: VAD EndCandidate + tail padding → UtteranceCommitted
//! - S3→S4: Reconciliation engine (§7) mutates visible string; atomic UI swap
//!
//! ## Invariants
//! - Visible string owned by exactly one state at a time (R4)
//! - No display of Whisper text before S4 unless §7 pass-through
//! - Generation mismatch → drop all partials
//! - Rollback caps (§7.1) always enforced

use serde::{Deserialize, Serialize};
use std::time::Instant;

use super::reconcile::reconcile_ts_whisper;

// ─── Transcript Authority State (§6.1) ────────────────────────────────────

/// Transcript authority state per §6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptState {
    /// No active session. Visible string is empty.
    S0Idle,
    /// Streamer partial text only. First partial received.
    S1Speculative,
    /// Streamer frozen prefix + volatile tail. Stabilized.
    S2Stabilizing,
    /// Streamer snapshot Ts frozen. UtteranceCommitted.
    S3Committed,
    /// Reconciled string per §7. Refinement applied.
    S4RefinedFinal,
}

impl TranscriptState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S0Idle => "s0_idle",
            Self::S1Speculative => "s1_speculative",
            Self::S2Stabilizing => "s2_stabilizing",
            Self::S3Committed => "s3_committed",
            Self::S4RefinedFinal => "s4_refined_final",
        }
    }
}

// ─── Transition Events ────────────────────────────────────────────────────

/// Events that trigger state transitions.
#[derive(Debug, Clone)]
pub enum TranscriptEvent {
    /// First partial received from sidecar.
    FirstPartial { text: String, generation: u64 },
    /// Partial update from sidecar.
    PartialUpdate {
        text: String,
        generation: u64,
        seq: u64,
        stable: bool,
    },
    /// Prefix hold rule satisfied (2 consecutive same-prefix, ≥120ms apart).
    PrefixHoldSatisfied,
    /// VAD EndCandidate + tail padding complete → UtteranceCommitted.
    UtteranceCommitted { text: String, generation: u64 },
    /// Whisper refinement result available.
    RefinementAvailable {
        whisper_text: String,
        generation: u64,
    },
    /// New turn started (generation++).
    NewTurn { generation: u64 },
    /// User undo refinement.
    UndoRefine,
    /// Cancel/abort.
    Cancel { generation: u64 },
}

// ─── Transcript Authority FSM ─────────────────────────────────────────────

/// Prefix hold tracker for S1→S2 transition (§6.2 rule 1).
#[derive(Debug, Clone)]
struct PrefixHoldTracker {
    /// Last partial text for prefix comparison.
    last_text: String,
    /// Count of consecutive partials with same word-prefix.
    consecutive_prefix_count: usize,
    /// Timestamp of first matching prefix partial.
    first_match_at: Option<Instant>,
}

impl PrefixHoldTracker {
    fn new() -> Self {
        Self {
            last_text: String::new(),
            consecutive_prefix_count: 0,
            first_match_at: None,
        }
    }

    /// Check if a new partial satisfies the prefix hold rule.
    /// Rule: same word-prefix for 2 consecutive partials ≥120ms apart.
    fn check(&mut self, text: &str) -> bool {
        let words_new: Vec<&str> = text.split_whitespace().collect();
        let words_old: Vec<&str> = self.last_text.split_whitespace().collect();

        // Check if new text starts with old text's word-prefix
        let has_prefix = if words_old.is_empty() {
            false
        } else {
            words_new.len() >= words_old.len() && words_new[..words_old.len()] == words_old[..]
        };

        if has_prefix {
            self.consecutive_prefix_count += 1;
            if self.first_match_at.is_none() {
                self.first_match_at = Some(Instant::now());
            }

            // Check time constraint: ≥120ms since first match
            if self.consecutive_prefix_count >= 2 {
                if let Some(first) = self.first_match_at {
                    if first.elapsed().as_millis() >= 120 {
                        return true;
                    }
                }
            }
        } else {
            // Reset on non-prefix
            self.consecutive_prefix_count = 0;
            self.first_match_at = None;
        }

        self.last_text = text.to_string();
        false
    }

    fn reset(&mut self) {
        self.last_text.clear();
        self.consecutive_prefix_count = 0;
        self.first_match_at = None;
    }
}

// ─── Telemetry ────────────────────────────────────────────────────────────

/// Telemetry event emitted on state transitions.
#[derive(Debug, Clone)]
pub struct TranscriptTransition {
    pub from: TranscriptState,
    pub to: TranscriptState,
    pub generation: u64,
    pub trigger: &'static str,
}

// ─── The FSM ──────────────────────────────────────────────────────────────

/// Transcript Authority FSM (§6).
///
/// **Single source of truth** for the visible transcript string.
/// Enforces ownership rules, rollback caps, and generation safety.
#[derive(Debug)]
pub struct TranscriptAuthorityFsm {
    state: TranscriptState,
    generation: u64,
    /// Current visible text (owned by exactly one state at a time).
    user_visible: String,
    /// Committed transcript (S3). Used for execution. NEVER mutated after commit.
    committed: String,
    /// Frozen prefix (S2). Stable portion of the partial.
    frozen_prefix: String,
    /// Volatile tail (S2). Unstable portion after frozen prefix.
    volatile_tail: String,
    /// Prefix hold tracker for S1→S2 transition.
    prefix_tracker: PrefixHoldTracker,
    /// Transition log for telemetry.
    transitions: Vec<TranscriptTransition>,
}

impl TranscriptAuthorityFsm {
    /// Create a new FSM in S0_Idle state.
    pub fn new(generation: u64) -> Self {
        Self {
            state: TranscriptState::S0Idle,
            generation,
            user_visible: String::new(),
            committed: String::new(),
            frozen_prefix: String::new(),
            volatile_tail: String::new(),
            prefix_tracker: PrefixHoldTracker::new(),
            transitions: Vec::new(),
        }
    }

    /// Current state.
    pub fn state(&self) -> TranscriptState {
        self.state
    }

    /// Current generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The user-visible transcript string (owned by current state).
    pub fn user_visible(&self) -> &str {
        &self.user_visible
    }

    /// The committed transcript (S3). Used for execution.
    /// Returns empty string if not yet committed.
    pub fn committed(&self) -> &str {
        &self.committed
    }

    /// The frozen prefix (S2 only).
    pub fn frozen_prefix(&self) -> &str {
        &self.frozen_prefix
    }

    /// Drain transition log for telemetry emission.
    pub fn drain_transitions(&mut self) -> Vec<TranscriptTransition> {
        std::mem::take(&mut self.transitions)
    }

    // ─── State Transitions ────────────────────────────────────────────────

    /// Process an event and advance the FSM.
    ///
    /// Returns `true` if a state transition occurred.
    pub fn process_event(&mut self, event: TranscriptEvent) -> bool {
        match event {
            TranscriptEvent::FirstPartial { text, generation } => {
                self.handle_first_partial(&text, generation)
            }
            TranscriptEvent::PartialUpdate {
                text,
                generation,
                stable,
                ..
            } => self.handle_partial_update(&text, generation, stable),
            TranscriptEvent::PrefixHoldSatisfied => self.handle_prefix_hold(),
            TranscriptEvent::UtteranceCommitted { text, generation } => {
                self.handle_utterance_committed(&text, generation)
            }
            TranscriptEvent::RefinementAvailable {
                whisper_text,
                generation,
            } => self.handle_refinement(&whisper_text, generation),
            TranscriptEvent::NewTurn { generation } => self.handle_new_turn(generation),
            TranscriptEvent::UndoRefine => self.handle_undo_refine(),
            TranscriptEvent::Cancel { generation } => self.handle_cancel(generation),
        }
    }

    /// S0 → S1: First partial received.
    fn handle_first_partial(&mut self, text: &str, generation: u64) -> bool {
        if self.state != TranscriptState::S0Idle {
            return false;
        }
        if generation != self.generation {
            tracing::debug!(
                expected = self.generation,
                received = generation,
                "stale first partial, ignoring"
            );
            return false;
        }

        self.transition_to(TranscriptState::S1Speculative, "first_partial");
        self.user_visible = text.to_string();
        self.prefix_tracker.reset();
        self.prefix_tracker.last_text = text.to_string();
        true
    }

    /// S1 partial update: check for prefix hold → S2.
    fn handle_partial_update(&mut self, text: &str, generation: u64, stable: bool) -> bool {
        // Generation check (§4 R2)
        if generation != self.generation {
            tracing::debug!(
                expected = self.generation,
                received = generation,
                "stale partial, dropping"
            );
            return false;
        }

        match self.state {
            TranscriptState::S0Idle => {
                // Treat as first partial
                self.handle_first_partial(text, generation)
            }
            TranscriptState::S1Speculative => {
                self.user_visible = text.to_string();

                // Check stable flag from sidecar (§6.2 rule 1)
                if stable {
                    self.transition_to(TranscriptState::S2Stabilizing, "stable_flag");
                    self.frozen_prefix = text.to_string();
                    self.volatile_tail.clear();
                    return true;
                }

                // Check prefix hold rule (§6.2 rule 1)
                if self.prefix_tracker.check(text) {
                    self.transition_to(TranscriptState::S2Stabilizing, "prefix_hold");
                    self.frozen_prefix = text.to_string();
                    self.volatile_tail.clear();
                    return true;
                }

                false
            }
            TranscriptState::S2Stabilizing => {
                // Update volatile tail only; frozen prefix stays
                if text.starts_with(&self.frozen_prefix) {
                    self.volatile_tail = text[self.frozen_prefix.len()..].to_string();
                } else {
                    // Prefix changed — this shouldn't happen in S2
                    // but handle gracefully
                    self.volatile_tail.clear();
                }
                self.user_visible = text.to_string();
                false
            }
            // In S3/S4, partials are ignored (committed transcript is authoritative)
            TranscriptState::S3Committed | TranscriptState::S4RefinedFinal => false,
        }
    }

    /// External prefix hold trigger → S2.
    fn handle_prefix_hold(&mut self) -> bool {
        if self.state != TranscriptState::S1Speculative {
            return false;
        }
        self.transition_to(TranscriptState::S2Stabilizing, "prefix_hold_external");
        self.frozen_prefix = self.user_visible.clone();
        self.volatile_tail.clear();
        true
    }

    /// S2 → S3: UtteranceCommitted (§6.2 rule 2).
    fn handle_utterance_committed(&mut self, text: &str, generation: u64) -> bool {
        if generation != self.generation {
            tracing::warn!(
                expected = self.generation,
                received = generation,
                "stale UtteranceCommitted, ignoring"
            );
            return false;
        }

        // Allow commit from S1 or S2 (VAD may fire before prefix stabilizes)
        match self.state {
            TranscriptState::S1Speculative | TranscriptState::S2Stabilizing => {
                self.transition_to(TranscriptState::S3Committed, "utterance_committed");
                self.committed = text.to_string();
                self.user_visible = text.to_string();
                self.frozen_prefix.clear();
                self.volatile_tail.clear();
                true
            }
            _ => false,
        }
    }

    /// S3 → S4: Refinement available (§6.2 rule 3).
    ///
    /// Applies reconciliation (§7) with rollback caps (§7.1).
    /// **MUST NOT** alter committed transcript (execution uses S3).
    fn handle_refinement(&mut self, whisper_text: &str, generation: u64) -> bool {
        if self.state != TranscriptState::S3Committed {
            tracing::debug!(
                state = self.state.as_str(),
                "refinement received in wrong state, ignoring"
            );
            return false;
        }

        if generation != self.generation {
            tracing::warn!(
                expected = self.generation,
                received = generation,
                "stale refinement, ignoring"
            );
            return false;
        }

        // Apply reconciliation (§7) — bounded, deterministic
        let outcome = reconcile_ts_whisper(&self.committed, whisper_text);

        tracing::info!(
            kind = outcome.kind.as_trace_str(),
            committed_len = self.committed.chars().count(),
            whisper_len = whisper_text.chars().count(),
            user_visible_len = outcome.user_visible.chars().count(),
            "reconciliation applied"
        );

        // Transition to S4 with reconciled text
        self.transition_to(TranscriptState::S4RefinedFinal, "refinement_applied");
        self.user_visible = outcome.user_visible;

        // CRITICAL: committed transcript is NEVER mutated
        // Execution always uses self.committed (S3)

        true
    }

    /// New turn: reset to S0, increment generation.
    fn handle_new_turn(&mut self, generation: u64) -> bool {
        self.transition_to(TranscriptState::S0Idle, "new_turn");
        self.generation = generation;
        self.user_visible.clear();
        self.committed.clear();
        self.frozen_prefix.clear();
        self.volatile_tail.clear();
        self.prefix_tracker.reset();
        true
    }

    /// Undo refinement: S4 → S3 (revert to committed).
    fn handle_undo_refine(&mut self) -> bool {
        if self.state != TranscriptState::S4RefinedFinal {
            return false;
        }
        self.transition_to(TranscriptState::S3Committed, "undo_refine");
        self.user_visible = self.committed.clone();
        true
    }

    /// Cancel: reset to S0, increment generation.
    fn handle_cancel(&mut self, generation: u64) -> bool {
        self.transition_to(TranscriptState::S0Idle, "cancel");
        self.generation = generation;
        self.user_visible.clear();
        self.committed.clear();
        self.frozen_prefix.clear();
        self.volatile_tail.clear();
        self.prefix_tracker.reset();
        true
    }

    // ─── Internal ─────────────────────────────────────────────────────────

    fn transition_to(&mut self, to: TranscriptState, trigger: &'static str) {
        let from = self.state;
        self.state = to;
        self.transitions.push(TranscriptTransition {
            from,
            to,
            generation: self.generation,
            trigger,
        });
        tracing::debug!(
            from = from.as_str(),
            to = to.as_str(),
            generation = self.generation,
            trigger,
            "transcript authority transition"
        );
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsm_starts_idle() {
        let fsm = TranscriptAuthorityFsm::new(0);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
        assert_eq!(fsm.generation(), 0);
        assert_eq!(fsm.user_visible(), "");
        assert_eq!(fsm.committed(), "");
    }

    #[test]
    fn s0_to_s1_on_first_partial() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        let transitioned = fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S1Speculative);
        assert_eq!(fsm.user_visible(), "hello");
    }

    #[test]
    fn stale_first_partial_rejected() {
        let mut fsm = TranscriptAuthorityFsm::new(5);
        let transitioned = fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 3, // stale
        });
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
    }

    #[test]
    fn s1_to_s2_on_stable_flag() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "hello world".to_string(),
            generation: 0,
            seq: 1,
            stable: true,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S2Stabilizing);
        assert_eq!(fsm.frozen_prefix(), "hello world");
    }

    #[test]
    fn s1_to_s2_on_prefix_hold_external() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::PrefixHoldSatisfied);
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S2Stabilizing);
        assert_eq!(fsm.frozen_prefix(), "hello");
    }

    #[test]
    fn s2_to_s3_on_utterance_committed() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "hello world".to_string(),
            generation: 0,
            seq: 1,
            stable: true,
        });

        let transitioned = fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
        assert_eq!(fsm.committed(), "hello world");
        assert_eq!(fsm.user_visible(), "hello world");
    }

    #[test]
    fn s3_to_s4_on_refinement() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        // Drive to S3
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello world".to_string(),
            generation: 0,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);
        // Committed unchanged (execution uses S3)
        assert_eq!(fsm.committed(), "hello world");
    }

    #[test]
    fn refinement_does_not_mutate_committed() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "helo".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "helo wrld".to_string(),
            generation: 0,
        });

        // Whisper provides better text
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello world".to_string(),
            generation: 0,
        });

        // Committed MUST remain unchanged (execution uses S3)
        assert_eq!(fsm.committed(), "helo wrld");
        // User visible may be updated by reconciliation
        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);
    }

    #[test]
    fn stale_refinement_rejected() {
        let mut fsm = TranscriptAuthorityFsm::new(5);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 5,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 5,
        });

        let transitioned = fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello world".to_string(),
            generation: 3, // stale
        });
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
    }

    #[test]
    fn undo_refine_reverts_to_committed() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello world!".to_string(),
            generation: 0,
        });
        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);

        let transitioned = fsm.process_event(TranscriptEvent::UndoRefine);
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
        assert_eq!(fsm.user_visible(), "hello world");
    }

    #[test]
    fn new_turn_resets_to_idle() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::NewTurn { generation: 1 });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
        assert_eq!(fsm.generation(), 1);
        assert_eq!(fsm.user_visible(), "");
        assert_eq!(fsm.committed(), "");
    }

    #[test]
    fn cancel_resets_to_idle() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::Cancel { generation: 1 });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
        assert_eq!(fsm.generation(), 1);
    }

    #[test]
    fn stale_partial_in_s1_dropped() {
        let mut fsm = TranscriptAuthorityFsm::new(5);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 5,
        });

        let transitioned = fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "stale text".to_string(),
            generation: 3, // stale
            seq: 1,
            stable: false,
        });
        assert!(!transitioned);
        assert_eq!(fsm.user_visible(), "hello"); // unchanged
    }

    #[test]
    fn partials_ignored_in_s3() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "something else".to_string(),
            generation: 0,
            seq: 99,
            stable: true,
        });
        assert!(!transitioned);
        assert_eq!(fsm.user_visible(), "hello world"); // committed unchanged
    }

    #[test]
    fn partials_ignored_in_s4() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "something else".to_string(),
            generation: 0,
            seq: 99,
            stable: true,
        });
        assert!(!transitioned);
    }

    #[test]
    fn s1_commit_allowed_without_s2() {
        // VAD may fire before prefix stabilizes
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitioned = fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
    }

    #[test]
    fn transition_log_recorded() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 0,
        });

        let transitions = fsm.drain_transitions();
        assert_eq!(transitions.len(), 2);
        assert_eq!(transitions[0].from, TranscriptState::S0Idle);
        assert_eq!(transitions[0].to, TranscriptState::S1Speculative);
        assert_eq!(transitions[1].from, TranscriptState::S1Speculative);
        assert_eq!(transitions[1].to, TranscriptState::S3Committed);
    }

    #[test]
    fn reconciliation_rollback_cap_enforced() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 0,
        });

        // Whisper provides completely different text → should be rejected by §7
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "completely different text that is very long".to_string(),
            generation: 0,
        });

        // Even in S4, committed is preserved
        assert_eq!(fsm.committed(), "hello");
        // User visible should be the committed text (rejected by reconciliation)
        assert_eq!(fsm.user_visible(), "hello");
    }

    #[test]
    fn reconciliation_prefix_extend_applied() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        let committed_text = "hello world this is a test sentence";
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: committed_text.to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: committed_text.to_string(),
            generation: 0,
        });

        // Whisper extends with a few more words
        let whisper = format!("{} and more", committed_text);
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: whisper.clone(),
            generation: 0,
        });

        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);
        // Committed unchanged
        assert_eq!(fsm.committed(), committed_text);
        // User visible updated by reconciliation (prefix extend)
        assert!(fsm.user_visible().starts_with(committed_text));
    }

    #[test]
    fn reconciliation_bounded_replace_applied() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "a b c d".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "a b c d".to_string(),
            generation: 0,
        });

        // Whisper provides small correction (1 word different)
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "a b c e".to_string(),
            generation: 0,
        });

        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);
        assert_eq!(fsm.committed(), "a b c d"); // unchanged
        assert_eq!(fsm.user_visible(), "a b c e"); // bounded replace
    }

    #[test]
    fn full_lifecycle_s0_s1_s2_s3_s4() {
        let mut fsm = TranscriptAuthorityFsm::new(0);

        // S0 → S1
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hel".to_string(),
            generation: 0,
        });
        assert_eq!(fsm.state(), TranscriptState::S1Speculative);

        // S1 → S2 (stable flag)
        fsm.process_event(TranscriptEvent::PartialUpdate {
            text: "hello wor".to_string(),
            generation: 0,
            seq: 1,
            stable: true,
        });
        assert_eq!(fsm.state(), TranscriptState::S2Stabilizing);

        // S2 → S3 (commit)
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello world".to_string(),
            generation: 0,
        });
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
        assert_eq!(fsm.committed(), "hello world");

        // S3 → S4 (refinement)
        fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello world".to_string(),
            generation: 0,
        });
        assert_eq!(fsm.state(), TranscriptState::S4RefinedFinal);
    }

    #[test]
    fn generation_safety_across_turns() {
        let mut fsm = TranscriptAuthorityFsm::new(0);

        // Turn 1
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "turn one".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "turn one".to_string(),
            generation: 0,
        });

        // New turn
        fsm.process_event(TranscriptEvent::NewTurn { generation: 1 });
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
        assert_eq!(fsm.generation(), 1);

        // Old generation partial rejected
        let transitioned = fsm.process_event(TranscriptEvent::FirstPartial {
            text: "stale".to_string(),
            generation: 0, // old generation
        });
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);

        // Current generation works
        let transitioned = fsm.process_event(TranscriptEvent::FirstPartial {
            text: "turn two".to_string(),
            generation: 1,
        });
        assert!(transitioned);
        assert_eq!(fsm.state(), TranscriptState::S1Speculative);
    }

    #[test]
    fn stale_utterance_committed_rejected() {
        let mut fsm = TranscriptAuthorityFsm::new(5);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 5,
        });

        let transitioned = fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 3, // stale
        });
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S1Speculative);
    }

    #[test]
    fn refinement_in_wrong_state_ignored() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        // Still in S0
        let transitioned = fsm.process_event(TranscriptEvent::RefinementAvailable {
            whisper_text: "hello".to_string(),
            generation: 0,
        });
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S0Idle);
    }

    #[test]
    fn undo_refine_in_wrong_state_ignored() {
        let mut fsm = TranscriptAuthorityFsm::new(0);
        fsm.process_event(TranscriptEvent::FirstPartial {
            text: "hello".to_string(),
            generation: 0,
        });
        fsm.process_event(TranscriptEvent::UtteranceCommitted {
            text: "hello".to_string(),
            generation: 0,
        });

        // In S3, not S4
        let transitioned = fsm.process_event(TranscriptEvent::UndoRefine);
        assert!(!transitioned);
        assert_eq!(fsm.state(), TranscriptState::S3Committed);
    }

    #[test]
    fn transcript_state_as_str() {
        assert_eq!(TranscriptState::S0Idle.as_str(), "s0_idle");
        assert_eq!(TranscriptState::S1Speculative.as_str(), "s1_speculative");
        assert_eq!(TranscriptState::S2Stabilizing.as_str(), "s2_stabilizing");
        assert_eq!(TranscriptState::S3Committed.as_str(), "s3_committed");
        assert_eq!(TranscriptState::S4RefinedFinal.as_str(), "s4_refined_final");
    }
}
