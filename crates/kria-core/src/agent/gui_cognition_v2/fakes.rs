//! GUI Cognition V2 — deterministic fake implementations.
//!
//! These let the loop, Hands, and integration points be tested without a real
//! screen, model, or input substrate. They are test/dev scaffolding (Phase 0),
//! NOT a production fallback.

use async_trait::async_trait;

use super::traits::{GuiBrain, GuiHands, Sight};
use super::types::{Action, ActionResult, Bbox, Decision, Observation, TurnStep, UiElement};

/// A Sight that returns a fixed observation.
pub struct FakeSight {
    pub observation: Observation,
}

impl FakeSight {
    /// A minimal one-button observation useful as a default.
    pub fn one_button(label: &str) -> Self {
        Self {
            observation: Observation {
                observation_id: "fake-obs".into(),
                screenshot_path: String::new(),
                screen_w: 1920,
                screen_h: 1080,
                active_window: Some("Fake Window".into()),
                elements: vec![UiElement {
                    id: 1,
                    bbox: Bbox { x: 10, y: 20, width: 100, height: 40 },
                    monitor_index: 0,
                    kind: "button".into(),
                    label: label.into(),
                    interactable: true,
                    confidence: 0.9,
                }],
                som_image_path: None,
                source: "fake".into(),
            },
        }
    }
}

#[async_trait]
impl Sight for FakeSight {
    async fn observe(&self, _want_som: bool) -> anyhow::Result<Observation> {
        Ok(self.observation.clone())
    }
}

/// A Brain that returns a fixed, pre-scripted sequence of decisions, then `Done`.
pub struct FakeBrain {
    script: std::sync::Mutex<std::collections::VecDeque<Decision>>,
}

impl FakeBrain {
    pub fn new(decisions: Vec<Decision>) -> Self {
        Self {
            script: std::sync::Mutex::new(decisions.into()),
        }
    }

    /// Convenience: click element 1 once, then declare done.
    pub fn click_then_done() -> Self {
        Self::new(vec![
            Decision {
                action: Action::Click { element_id: 1 },
                reason: "fake click".into(),
                risk_hint: None,
            },
            Decision {
                action: Action::Done { summary: "fake done".into() },
                reason: "fake done".into(),
                risk_hint: None,
            },
        ])
    }
}

#[async_trait]
impl GuiBrain for FakeBrain {
    async fn decide(
        &self,
        _task: &str,
        _observation: &Observation,
        _history: &[TurnStep],
    ) -> anyhow::Result<Decision> {
        let mut script = self.script.lock().unwrap();
        Ok(script.pop_front().unwrap_or(Decision {
            action: Action::Done { summary: "script exhausted".into() },
            reason: "fake".into(),
            risk_hint: None,
        }))
    }

    fn label(&self) -> &str {
        "fake"
    }
}

/// A Hands that records executed decisions and always succeeds (no real input).
#[derive(Default)]
pub struct FakeHands {
    pub executed: std::sync::Mutex<Vec<Decision>>,
}

#[async_trait]
impl GuiHands for FakeHands {
    async fn execute(
        &self,
        decision: &Decision,
        observation: &Observation,
    ) -> anyhow::Result<ActionResult> {
        // Honor the "no invented target" contract even in the fake so loop tests
        // exercise the real failure path (Property 2 / Requirement 4.6).
        if let Action::Click { element_id } = &decision.action {
            if observation.element(*element_id).is_none() {
                return Ok(ActionResult::failed(
                    "fake",
                    format!("element id {element_id} not present in observation"),
                ));
            }
        }
        self.executed.lock().unwrap().push(decision.clone());
        let mut result = ActionResult::ok("fake");
        result.screen_changed = Some(true);
        Ok(result)
    }
}
