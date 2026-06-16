//! GUI Cognition V2 — the bounded observe → decide → gate → act → verify loop.
//!
//! Phase 0 established the minimal wiring. Task 9 adds the production guards that
//! are deterministically testable here:
//! - **Safety gate** (Property 5): a decided executable action is sent to the
//!   injected [`SafetyGate`] before Hands; a `Deny` stops the turn and the action
//!   never executes.
//! - **Cancellation** (Requirement 5.4): a shared cancel flag is checked each
//!   iteration; the loop halts before the next action.
//! - **No-progress detection** (Requirement 5.3): if a state-changing action
//!   produces no observable screen change across re-observe for `no_progress_limit`
//!   consecutive steps, the loop stops (never an infinite loop).
//! - **Step cap** (Requirement 5.1): a hard iteration bound.
//!
//! The desktop integration (live) supplies the real [`SafetyGate`] (existing
//! HITL/policy) and the real Hands input substrate; incremental event streaming
//! and screenshot-diff verification beyond `screen_changed` are wired there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::traits::{GateDecision, GuiBrain, GuiHands, SafetyGate, Sight};
use super::types::{Action, Decision, TurnStep};

/// Terminal status of a V2 turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStatus {
    /// Brain returned `Done`.
    Completed,
    /// Brain returned `Ask` — a clarification is required.
    NeedsClarification,
    /// The bounded step cap was reached without completion.
    StoppedStepCap,
    /// A state-changing action produced no screen change repeatedly.
    StoppedNoProgress,
    /// The safety gate denied a decided action.
    StoppedSafety,
    /// A cancel was requested.
    Cancelled,
    /// A layer returned an unrecoverable error.
    StoppedError,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnStatus::Completed => "completed",
            TurnStatus::NeedsClarification => "needs_clarification",
            TurnStatus::StoppedStepCap => "stopped_step_cap",
            TurnStatus::StoppedNoProgress => "stopped_no_progress",
            TurnStatus::StoppedSafety => "stopped_safety",
            TurnStatus::Cancelled => "cancelled",
            TurnStatus::StoppedError => "stopped_error",
        }
    }
}

/// Outcome of a V2 turn.
#[derive(Debug, Clone)]
pub struct TurnOutcomeV2 {
    pub status: TurnStatus,
    /// Layman-friendly summary line.
    pub reply: String,
    /// The completed steps (executed actions + results), in order.
    pub steps: Vec<TurnStep>,
}

/// Bounded loop configuration.
#[derive(Debug, Clone, Copy)]
pub struct LoopConfig {
    /// Hard cap on the number of decide/act iterations.
    pub max_steps: u32,
    /// Whether to request a Set-of-Mark image from Sight each observe.
    pub want_som: bool,
    /// Consecutive no-change steps (after a state-changing action) that trigger
    /// a no-progress stop. 0 disables the check.
    pub no_progress_limit: u32,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_steps: 12,
            want_som: false,
            no_progress_limit: 2,
        }
    }
}

/// Optional, injected runtime guards. `default()` wires none (skeleton behavior).
#[derive(Default, Clone)]
pub struct LoopGuards {
    /// Safety gate consulted before every executable action.
    pub safety: Option<Arc<dyn SafetyGate>>,
    /// Cooperative cancel flag, checked each iteration.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl LoopGuards {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_safety(mut self, gate: Arc<dyn SafetyGate>) -> Self {
        self.safety = Some(gate);
        self
    }

    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .map(|f| f.load(Ordering::SeqCst))
            .unwrap_or(false)
    }
}

/// Run one GUI Cognition V2 turn over the three injected layers, with guards.
pub async fn run_turn_v2(
    sight: &dyn Sight,
    brain: &dyn GuiBrain,
    hands: &dyn GuiHands,
    task: &str,
    config: LoopConfig,
    guards: &LoopGuards,
) -> TurnOutcomeV2 {
    let mut steps: Vec<TurnStep> = Vec::new();
    // Signature of the observation that the most recent executed state-changing
    // action was based on; used to detect "no screen change after acting".
    let mut prev_executed_sig: Option<String> = None;
    let mut no_progress_count: u32 = 0;

    for step_index in 0..config.max_steps {
        // Cancellation — check before any work each iteration.
        if guards.is_cancelled() {
            return TurnOutcomeV2 {
                status: TurnStatus::Cancelled,
                reply: "Turn cancelled.".into(),
                steps,
            };
        }

        // 1. OBSERVE (fresh observation each step — Property 3).
        let observation = match sight.observe(config.want_som).await {
            Ok(obs) => obs,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Could not observe the screen: {e}"),
                    steps,
                };
            }
        };

        // No-progress detection: a prior state-changing action left the screen
        // unchanged (Requirement 5.3).
        if config.no_progress_limit > 0 {
            if let Some(prev) = &prev_executed_sig {
                if *prev == observation.signature() {
                    no_progress_count += 1;
                    if no_progress_count >= config.no_progress_limit {
                        return TurnOutcomeV2 {
                            status: TurnStatus::StoppedNoProgress,
                            reply: "The screen did not change after the last action; stopping to avoid looping.".into(),
                            steps,
                        };
                    }
                } else {
                    no_progress_count = 0;
                }
            }
        }

        // 2. DECIDE (one action).
        let decision: Decision = match brain.decide(task, &observation, &steps).await {
            Ok(d) => d,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Could not decide the next action: {e}"),
                    steps,
                };
            }
        };

        // Terminal decisions end the loop cleanly.
        match &decision.action {
            Action::Done { summary } => {
                return TurnOutcomeV2 {
                    status: TurnStatus::Completed,
                    reply: summary.clone(),
                    steps,
                };
            }
            Action::Ask { question } => {
                return TurnOutcomeV2 {
                    status: TurnStatus::NeedsClarification,
                    reply: question.clone(),
                    steps,
                };
            }
            _ => {}
        }

        // 3. SAFETY GATE — never execute an action the gate does not allow
        // (Property 5). Only executable actions are gated.
        if decision.action.is_executable() {
            if let Some(gate) = guards.safety.as_ref() {
                if let GateDecision::Deny(reason) = gate.evaluate(&decision, &observation).await {
                    return TurnOutcomeV2 {
                        status: TurnStatus::StoppedSafety,
                        reply: format!("Blocked for safety: {reason}"),
                        steps,
                    };
                }
            }
        }

        // 4. EXECUTE.
        let result = match hands.execute(&decision, &observation).await {
            Ok(r) => r,
            Err(e) => {
                return TurnOutcomeV2 {
                    status: TurnStatus::StoppedError,
                    reply: format!("Action failed: {e}"),
                    steps,
                };
            }
        };

        // Record the step (history references the target LABEL, never a stale id).
        let target_label = match &decision.action {
            Action::Click { element_id } => {
                observation.element(*element_id).map(|e| e.label.clone())
            }
            _ => None,
        };
        let executed_ok = result.ok;
        steps.push(TurnStep {
            step_index,
            decision,
            result,
            target_label,
        });

        // Arm no-progress tracking: remember the signature the action acted on,
        // so the next observe can detect whether the screen actually changed.
        if executed_ok && config.no_progress_limit > 0 {
            prev_executed_sig = Some(observation.signature());
        }

        // TODO(Phase 5 live): incremental event streaming + screenshot-diff
        // verification beyond the `screen_changed` signal, wired in the desktop
        // integration alongside the real SafetyGate (HITL) and uinput sink.
    }

    TurnOutcomeV2 {
        status: TurnStatus::StoppedStepCap,
        reply: format!(
            "Reached the step limit ({}) without completing the task.",
            config.max_steps
        ),
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{FakeBrain, FakeHands, FakeSight};
    use super::super::traits::{GateDecision, SafetyGate};
    use super::super::types::{Action, Decision, Observation};
    use super::*;

    fn click(id: u32) -> Decision {
        Decision {
            action: Action::Click { element_id: id },
            reason: "x".into(),
            risk_hint: None,
        }
    }

    #[tokio::test]
    async fn loop_executes_then_completes_on_done() {
        let sight = FakeSight::one_button("New Tab");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let outcome =
            run_turn_v2(&sight, &brain, &hands, "click it", LoopConfig::default(), &LoopGuards::none())
                .await;
        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].target_label.as_deref(), Some("New Tab"));
        assert_eq!(hands.executed.lock().unwrap().len(), 1);
    }

    struct AlwaysClick;
    #[async_trait::async_trait]
    impl GuiBrain for AlwaysClick {
        async fn decide(
            &self,
            _t: &str,
            _o: &Observation,
            _h: &[TurnStep],
        ) -> anyhow::Result<Decision> {
            Ok(click(1))
        }
        fn label(&self) -> &str {
            "always_click"
        }
    }

    #[tokio::test]
    async fn loop_stops_at_step_cap_when_no_progress_disabled() {
        let sight = FakeSight::one_button("Btn");
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &AlwaysClick,
            &hands,
            "loop",
            LoopConfig { max_steps: 3, want_som: false, no_progress_limit: 0 },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedStepCap);
        assert_eq!(outcome.steps.len(), 3);
    }

    #[tokio::test]
    async fn loop_stops_on_no_progress() {
        // FakeSight returns the SAME observation each time → after the first
        // executed click, the next observe shows no change → no-progress stop.
        let sight = FakeSight::one_button("Btn");
        let hands = FakeHands::default();
        let outcome = run_turn_v2(
            &sight,
            &AlwaysClick,
            &hands,
            "loop",
            LoopConfig { max_steps: 20, want_som: false, no_progress_limit: 2 },
            &LoopGuards::none(),
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedNoProgress);
        // Stopped well before the step cap.
        assert!(outcome.steps.len() < 20);
    }

    #[tokio::test]
    async fn loop_asks_on_ambiguity() {
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![Decision {
            action: Action::Ask { question: "which one?".into() },
            reason: "ambiguous".into(),
            risk_hint: None,
        }]);
        let hands = FakeHands::default();
        let outcome =
            run_turn_v2(&sight, &brain, &hands, "do x", LoopConfig::default(), &LoopGuards::none())
                .await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert_eq!(outcome.reply, "which one?");
        assert!(outcome.steps.is_empty());
    }

    #[tokio::test]
    async fn hands_rejects_missing_element_id() {
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![click(99)]);
        let hands = FakeHands::default();
        let outcome =
            run_turn_v2(&sight, &brain, &hands, "do x", LoopConfig::default(), &LoopGuards::none())
                .await;
        assert_eq!(outcome.steps.len(), 1);
        assert!(!outcome.steps[0].result.ok);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
    }

    struct DenyGate;
    #[async_trait::async_trait]
    impl SafetyGate for DenyGate {
        async fn evaluate(&self, _d: &Decision, _o: &Observation) -> GateDecision {
            GateDecision::Deny("risky action requires approval".into())
        }
    }

    #[tokio::test]
    async fn safety_gate_deny_stops_before_execution() {
        let sight = FakeSight::one_button("Delete");
        let brain = FakeBrain::new(vec![click(1)]);
        let hands = FakeHands::default();
        let guards = LoopGuards::none().with_safety(Arc::new(DenyGate));
        let outcome =
            run_turn_v2(&sight, &brain, &hands, "delete it", LoopConfig::default(), &guards).await;
        assert_eq!(outcome.status, TurnStatus::StoppedSafety);
        // Action never executed (Property 5).
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
        assert!(outcome.steps.is_empty());
    }

    #[tokio::test]
    async fn cancel_flag_stops_the_loop() {
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();
        let flag = Arc::new(AtomicBool::new(true)); // already cancelled
        let guards = LoopGuards::none().with_cancel(flag);
        let outcome =
            run_turn_v2(&sight, &brain, &hands, "x", LoopConfig::default(), &guards).await;
        assert_eq!(outcome.status, TurnStatus::Cancelled);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
    }
}
