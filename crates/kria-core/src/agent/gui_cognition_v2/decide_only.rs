//! GUI Cognition V2 — Phase 3: Sight + Brain decision-only integration.
//!
//! Pipes a real [`Sight`] observation into a [`GuiBrain`] and returns the
//! [`Decision`] **without executing it**, plus a diagnostic that attributes a
//! wrong outcome to the responsible layer:
//!
//! - **Sight** — the expected target was never surfaced (not in the
//!   observation, or perception degraded).
//! - **Brain** — the target WAS present, but the Brain referenced an absent id
//!   or picked the wrong present element.
//!
//! This is the integration seam used for safe, no-side-effect diagnosis (and by
//! the eval harness): it never touches Hands, so it can run anywhere.

use serde::Serialize;

use super::traits::{GuiBrain, Sight};
use super::types::{Action, Decision, Observation};

/// Which layer a (mis)decision is attributable to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "attribution", rename_all = "snake_case")]
pub enum OutcomeAttribution {
    /// Perception degraded (sidecar down / no elements) — Sight unavailable.
    DegradedSight,
    /// Brain returned `Done`/`Ask` — no execution intended.
    Terminal,
    /// A direct action (Key/Type/Scroll/ClickPoint) needing no element grounding.
    DirectAction,
    /// The expected target was NOT present in the observation → Sight layer.
    TargetMissingFromSight,
    /// The Brain referenced an element id absent from the observation → Brain.
    BrainPickedAbsentElement,
    /// The Brain clicked a present element that matches the expected target.
    BrainPickedExpected { label: String },
    /// The expected target WAS present, but the Brain clicked a different one → Brain.
    BrainPickedWrongElement { label: String },
    /// The Brain clicked a present element (no expectation supplied to judge against).
    BrainPickedPresent { label: String },
}

impl OutcomeAttribution {
    /// Which layer to blame for a wrong outcome: "sight" | "brain" | "none".
    pub fn blame_layer(&self) -> &'static str {
        match self {
            OutcomeAttribution::DegradedSight | OutcomeAttribution::TargetMissingFromSight => {
                "sight"
            }
            OutcomeAttribution::BrainPickedAbsentElement
            | OutcomeAttribution::BrainPickedWrongElement { .. } => "brain",
            _ => "none",
        }
    }

    /// One-line layman explanation.
    pub fn human(&self) -> String {
        match self {
            OutcomeAttribution::DegradedSight => "Sight couldn't see the screen (perception degraded).".into(),
            OutcomeAttribution::Terminal => "The model chose to finish or ask a question (no action).".into(),
            OutcomeAttribution::DirectAction => "The model chose a keyboard/scroll action (no on-screen target needed).".into(),
            OutcomeAttribution::TargetMissingFromSight => "Sight never showed the requested control, so it couldn't be acted on.".into(),
            OutcomeAttribution::BrainPickedAbsentElement => "The model referenced a control that isn't on screen (model mistake).".into(),
            OutcomeAttribution::BrainPickedExpected { label } => format!("The model correctly chose '{label}'."),
            OutcomeAttribution::BrainPickedWrongElement { label } => format!("The control was visible, but the model chose '{label}' instead (model mistake)."),
            OutcomeAttribution::BrainPickedPresent { label } => format!("The model chose the on-screen control '{label}'."),
        }
    }
}

/// The result of a decision-only turn: the decision + layer attribution.
#[derive(Debug, Clone, Serialize)]
pub struct DecisionDiagnostic {
    pub observation_id: String,
    pub source: String,
    pub degraded: bool,
    pub element_count: usize,
    pub decision: Decision,
    /// Label of the clicked element, when the decision was a `Click` on a
    /// present element (semantic, never a stale id).
    pub matched_label: Option<String>,
    pub attribution: OutcomeAttribution,
}

/// Case-insensitive "does any element label contain `needle`".
fn observation_contains_label(obs: &Observation, needle: &str) -> bool {
    let n = needle.trim().to_ascii_lowercase();
    if n.is_empty() {
        return false;
    }
    obs.elements
        .iter()
        .any(|e| e.label.to_ascii_lowercase().contains(&n))
}

/// Pure attribution of a decision against an observation (no I/O), optionally
/// judged against an `expected_label` the caller knows the task wanted.
pub fn attribute(
    observation: &Observation,
    decision: &Decision,
    expected_label: Option<&str>,
) -> OutcomeAttribution {
    if observation.is_degraded() {
        return OutcomeAttribution::DegradedSight;
    }
    match &decision.action {
        Action::Done { .. } | Action::Ask { .. } => OutcomeAttribution::Terminal,
        Action::OpenApp { .. }
        | Action::ClickPoint { .. }
        | Action::Type { .. }
        | Action::TypeAndSubmit { .. }
        | Action::Navigate { .. }
        | Action::Key { .. }
        | Action::Scroll { .. } => OutcomeAttribution::DirectAction,
        Action::Click { element_id } => {
            let expected_present =
                expected_label.is_some_and(|exp| observation_contains_label(observation, exp));
            match observation.element(*element_id) {
                None => {
                    if expected_label.is_some() && !expected_present {
                        OutcomeAttribution::TargetMissingFromSight
                    } else {
                        OutcomeAttribution::BrainPickedAbsentElement
                    }
                }
                Some(element) => match expected_label {
                    None => OutcomeAttribution::BrainPickedPresent {
                        label: element.label.clone(),
                    },
                    Some(exp) => {
                        let exp_l = exp.trim().to_ascii_lowercase();
                        if element.label.to_ascii_lowercase().contains(&exp_l) {
                            OutcomeAttribution::BrainPickedExpected {
                                label: element.label.clone(),
                            }
                        } else if expected_present {
                            OutcomeAttribution::BrainPickedWrongElement {
                                label: element.label.clone(),
                            }
                        } else {
                            OutcomeAttribution::BrainPickedPresent {
                                label: element.label.clone(),
                            }
                        }
                    }
                },
            }
        }
    }
}

/// Observe with the real Sight, decide with the real Brain, and return the
/// decision + attribution — WITHOUT executing anything (Phase 3 / Task 6).
pub async fn decide_only(
    sight: &dyn Sight,
    brain: &dyn GuiBrain,
    task: &str,
    want_som: bool,
    expected_label: Option<&str>,
) -> anyhow::Result<DecisionDiagnostic> {
    let observation = sight.observe(want_som).await?;
    let decision = brain.decide(task, &observation, &[]).await?;
    let matched_label = match &decision.action {
        Action::Click { element_id } => observation.element(*element_id).map(|e| e.label.clone()),
        _ => None,
    };
    let attribution = attribute(&observation, &decision, expected_label);
    Ok(DecisionDiagnostic {
        observation_id: observation.observation_id.clone(),
        source: observation.source.clone(),
        degraded: observation.is_degraded(),
        element_count: observation.elements.len(),
        decision,
        matched_label,
        attribution,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fakes::{FakeBrain, FakeSight};
    use super::super::types::{Action, Bbox, Decision, Observation, UiElement};
    use super::*;

    fn obs_with(elements: Vec<(u32, &str)>) -> Observation {
        Observation {
            observation_id: "obs".into(),
            screenshot_path: String::new(),
            screen_w: 1920,
            screen_h: 1080,
            active_window: Some("Win".into()),
            elements: elements
                .into_iter()
                .map(|(id, label)| UiElement {
                    id,
                    bbox: Bbox { x: 0, y: 0, width: 10, height: 10 },
                    monitor_index: 0,
                    kind: "button".into(),
                    label: label.into(),
                    interactable: true,
                    confidence: 0.9,
                })
                .collect(),
            som_image_path: None,
            source: "omniparser".into(),
        }
    }

    fn click(id: u32) -> Decision {
        Decision { action: Action::Click { element_id: id }, reason: String::new(), risk_hint: None }
    }

    #[test]
    fn brain_correct_pick_is_attributed_to_brain_expected() {
        let obs = obs_with(vec![(1, "New Tab"), (2, "Close")]);
        let a = attribute(&obs, &click(1), Some("new tab"));
        assert_eq!(a, OutcomeAttribution::BrainPickedExpected { label: "New Tab".into() });
        assert_eq!(a.blame_layer(), "none");
    }

    #[test]
    fn target_absent_from_observation_blames_sight() {
        // Expected "New Tab" is NOT present → Sight never surfaced it.
        let obs = obs_with(vec![(1, "Close"), (2, "Reload")]);
        let a = attribute(&obs, &click(9), Some("new tab"));
        assert_eq!(a, OutcomeAttribution::TargetMissingFromSight);
        assert_eq!(a.blame_layer(), "sight");
    }

    #[test]
    fn target_present_but_wrong_pick_blames_brain() {
        // "New Tab" IS present (id 1) but the Brain clicked "Close" (id 2).
        let obs = obs_with(vec![(1, "New Tab"), (2, "Close")]);
        let a = attribute(&obs, &click(2), Some("new tab"));
        assert_eq!(a, OutcomeAttribution::BrainPickedWrongElement { label: "Close".into() });
        assert_eq!(a.blame_layer(), "brain");
    }

    #[test]
    fn absent_id_with_target_present_blames_brain() {
        let obs = obs_with(vec![(1, "New Tab")]);
        let a = attribute(&obs, &click(99), Some("new tab"));
        assert_eq!(a, OutcomeAttribution::BrainPickedAbsentElement);
        assert_eq!(a.blame_layer(), "brain");
    }

    #[test]
    fn degraded_sight_is_attributed_to_sight() {
        let mut obs = obs_with(vec![]);
        obs.source = "degraded:sidecar_unavailable".into();
        let a = attribute(&obs, &click(1), Some("new tab"));
        assert_eq!(a, OutcomeAttribution::DegradedSight);
        assert_eq!(a.blame_layer(), "sight");
    }

    #[test]
    fn direct_and_terminal_actions_blame_no_layer() {
        let obs = obs_with(vec![(1, "x")]);
        assert_eq!(
            attribute(&obs, &Decision { action: Action::Key { combo: "ctrl+t".into() }, reason: String::new(), risk_hint: None }, None),
            OutcomeAttribution::DirectAction
        );
        assert_eq!(
            attribute(&obs, &Decision { action: Action::Done { summary: "ok".into() }, reason: String::new(), risk_hint: None }, None),
            OutcomeAttribution::Terminal
        );
    }

    #[tokio::test]
    async fn decide_only_runs_sight_then_brain_without_execution() {
        let sight = FakeSight::one_button("New Tab");
        let brain = FakeBrain::new(vec![click(1)]);
        let diag = decide_only(&sight, &brain, "open a new tab", false, Some("new tab"))
            .await
            .unwrap();
        assert_eq!(diag.element_count, 1);
        assert_eq!(diag.matched_label.as_deref(), Some("New Tab"));
        assert_eq!(diag.attribution, OutcomeAttribution::BrainPickedExpected { label: "New Tab".into() });
        assert!(!diag.degraded);
    }
}
