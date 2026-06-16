//! GUI Cognition V2 — the bounded observe → decide → act → verify loop.
//!
//! Phase 0 establishes the minimal, real control flow wiring the three injected
//! layers with a hard step cap and clean termination on `Done`/`Ask`. Later
//! phases add: the safety/HITL gate before execution (Task 9), per-step
//! verification + re-observe + no-progress/cancel/watchdog guards (Task 9), and
//! incremental event streaming (Task 9). Those hooks are called out with TODO
//! markers so the skeleton stays honest about what is and isn't wired yet.

use super::traits::{GuiBrain, GuiHands, Sight};
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
    /// A layer returned an unrecoverable error.
    StoppedError,
}

impl TurnStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnStatus::Completed => "completed",
            TurnStatus::NeedsClarification => "needs_clarification",
            TurnStatus::StoppedStepCap => "stopped_step_cap",
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
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_steps: 12,
            want_som: false,
        }
    }
}

/// Run one GUI Cognition V2 turn over the three injected layers.
///
/// Phase 0 control flow (real, minimal):
/// observe → decide → (execute if executable) → record → repeat, bounded by
/// `max_steps`, terminating on `Done`/`Ask`.
pub async fn run_turn_v2(
    sight: &dyn Sight,
    brain: &dyn GuiBrain,
    hands: &dyn GuiHands,
    task: &str,
    config: LoopConfig,
) -> TurnOutcomeV2 {
    let mut steps: Vec<TurnStep> = Vec::new();

    for step_index in 0..config.max_steps {
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

        // TODO(Task 9): safety/HITL gate here — risky decision must be approved
        // before execution. Phase 0 skeleton executes directly.

        // 3. EXECUTE.
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
        steps.push(TurnStep {
            step_index,
            decision,
            result,
            target_label,
        });

        // TODO(Task 9): per-step verification (screenshot-diff/re-observe),
        // no-progress detection, cancel-token check, watchdog. Phase 0 relies on
        // the step cap + Brain `Done`/`Ask` for termination.
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
    use super::super::types::{Action, Decision};
    use super::*;

    #[tokio::test]
    async fn loop_executes_then_completes_on_done() {
        let sight = FakeSight::one_button("New Tab");
        let brain = FakeBrain::click_then_done();
        let hands = FakeHands::default();

        let outcome = run_turn_v2(&sight, &brain, &hands, "click it", LoopConfig::default()).await;

        assert_eq!(outcome.status, TurnStatus::Completed);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].target_label.as_deref(), Some("New Tab"));
        assert_eq!(hands.executed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn loop_stops_at_step_cap() {
        // Brain always clicks, never done → must stop at the cap.
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![]); // empty → "script exhausted" => Done
        let hands = FakeHands::default();

        // Use a brain that never returns Done: build one that always clicks.
        struct AlwaysClick;
        #[async_trait::async_trait]
        impl GuiBrain for AlwaysClick {
            async fn decide(
                &self,
                _t: &str,
                _o: &super::super::types::Observation,
                _h: &[TurnStep],
            ) -> anyhow::Result<Decision> {
                Ok(Decision {
                    action: Action::Click { element_id: 1 },
                    reason: "loop".into(),
                    risk_hint: None,
                })
            }
            fn label(&self) -> &str {
                "always_click"
            }
        }
        let _ = brain;
        let outcome = run_turn_v2(
            &sight,
            &AlwaysClick,
            &hands,
            "loop forever",
            LoopConfig { max_steps: 3, want_som: false },
        )
        .await;
        assert_eq!(outcome.status, TurnStatus::StoppedStepCap);
        assert_eq!(outcome.steps.len(), 3);
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
        let outcome = run_turn_v2(&sight, &brain, &hands, "do x", LoopConfig::default()).await;
        assert_eq!(outcome.status, TurnStatus::NeedsClarification);
        assert_eq!(outcome.reply, "which one?");
        assert!(outcome.steps.is_empty());
    }

    #[tokio::test]
    async fn hands_rejects_missing_element_id() {
        // Observation has element id 1; brain clicks id 99 → fake Hands fails.
        let sight = FakeSight::one_button("Btn");
        let brain = FakeBrain::new(vec![Decision {
            action: Action::Click { element_id: 99 },
            reason: "bad id".into(),
            risk_hint: None,
        }]);
        let hands = FakeHands::default();
        let outcome = run_turn_v2(&sight, &brain, &hands, "do x", LoopConfig::default()).await;
        // Step recorded but result is a failure (no fallback click).
        assert_eq!(outcome.steps.len(), 1);
        assert!(!outcome.steps[0].result.ok);
        assert_eq!(hands.executed.lock().unwrap().len(), 0);
    }
}
