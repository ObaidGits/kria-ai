//! GUI Cognition V2 — Sight / Brain / Hands.
//!
//! A rebuild of GUI automation around three cleanly separated, independently
//! testable layers connected by a bounded observe → decide → act → verify loop:
//!
//! - [`Sight`](traits::Sight): screenshot → [`Observation`](types::Observation)
//!   (OmniParser-backed; Phase 1).
//! - [`GuiBrain`](traits::GuiBrain): (task, observation, history) → one
//!   [`Decision`](types::Decision). Model-agnostic seam — `LlmPlannerBrain` (Phase 2),
//!   `VisionBrain` drop-in (Phase 5b).
//! - [`GuiHands`](traits::GuiHands): execute a `Decision` via uinput (Phase 4).
//!
//! This module IS the GUI-cognition pipeline. As of Task 13 the over-built V1
//! pipeline has been removed, so every GUI turn routes through the V2
//! Sight/Brain/Hands loop. [`v2_enabled`] remains as a vestigial capability flag
//! (default ON); there is no longer a V1 path to fall back to.

pub mod fakes;
pub mod bridge;
pub mod hands_uinput;
pub mod decide_only;
pub mod loop_engine;
pub mod llm_brain;
pub mod planner;
pub mod safety;
pub mod sight_omniparser;
pub mod traits;
pub mod types;
pub mod vision_brain;
pub mod verifier;

pub use hands_uinput::{InputSink, UinputHands};
pub use decide_only::{attribute, decide_only, DecisionDiagnostic, OutcomeAttribution};
pub use loop_engine::{run_turn_v2, decision_needs_grounding, LoopConfig, LoopEvent, LoopGuards, LoopObserver, TurnOutcomeV2, TurnStatus};
pub use llm_brain::LlmPlannerBrain;
pub use planner::{fallback_plan, LlmPlanner, Plan, PLANNER_SYSTEM_PROMPT};
pub use bridge::{BridgeOutcome, GuiBridge, WorkingContext};
pub use safety::assess_action_risk;
pub use sight_omniparser::{OmniParserSight, ScreenCapturer};
pub use traits::{GateDecision, GuiBrain, GuiHands, GuiPlanner, SafetyGate, Sight};
pub use types::{Action, ActionResult, Bbox, Decision, Observation, SubGoal, SubGoalKind, TurnStep, UiElement};
pub use vision_brain::{VisionBrain, VISION_LABEL};
pub use verifier::{
    verify_sub_goal, Signal, StandardVerifier, SubGoalVerifier, Verdict, VerificationProbe,
    VerifyOutcome, CONFIDENCE_FLOOR,
};

/// Environment flag that routes GUI turns through the V2 Sight/Brain/Hands loop.
///
/// Default **OFF**: V2 is opt-in until it is proven on the real-verify eval
/// harness (Requirement 10.1). The Phase 6 gate (Task 12) flips this to a
/// default-ON helper with a falsy rollback to V1. Truthy values
/// (`1`/`true`/`yes`/`on`) enable it; everything else (including absent) is OFF.
pub const V2_ENV_FLAG: &str = "KRIA_GUI_COG_V2";

/// Whether the V2 pipeline is enabled.
///
/// DEFAULT-ON. As of Task 13 the over-built V1 pipeline has been REMOVED, so the
/// desktop entry routes every GUI turn through V2 unconditionally — this flag no
/// longer toggles between two pipelines (there is no V1 to fall back to). It is
/// retained as a vestigial capability signal; a falsy value is a no-op for
/// routing.
pub fn v2_enabled() -> bool {
    v2_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`v2_enabled`] with an injectable lookup.
pub fn v2_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    // Default-ON with a falsy rollback (mirrors the desktop `from_env_default_on`
    // convention): only an explicit falsy value routes back to V1.
    !matches!(
        lookup(V2_ENV_FLAG)
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// Environment flag selecting which [`GuiBrain`] implementation V2 uses.
/// Default `text` (model-agnostic, reasons over the element list); `vision`
/// selects the coordinate-emitting vision brain. Model-NEUTRAL: no vendor names.
/// `KRIA_GUI_COG_V2_BRAIN` is accepted as a legacy alias, as are the legacy
/// values `qwen` (→ Text) and `ui_tars` (→ Vision).
pub const BRAIN_ENV_FLAG: &str = "KRIA_GUI_COG_BRAIN";
/// Legacy env flag name, still honored for backward compatibility.
pub const V2_BRAIN_ENV_FLAG: &str = "KRIA_GUI_COG_V2_BRAIN";

/// Which Brain implementation to construct for a V2 turn. Model-neutral.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainChoice {
    /// Text-first brain ([`LlmPlannerBrain`]) over the numbered element list — the
    /// default. Backed by whatever LLM is configured (no vendor coupling).
    Text,
    /// Coordinate-emitting vision brain ([`VisionBrain`]).
    Vision,
}

impl BrainChoice {
    /// Stable, model-neutral label.
    pub fn label(self) -> &'static str {
        match self {
            BrainChoice::Text => "text",
            BrainChoice::Vision => "vision",
        }
    }
}

/// Resolve the configured Brain from the environment (default `text`).
pub fn brain_choice() -> BrainChoice {
    brain_choice_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`brain_choice`] with an injectable lookup. The neutral
/// `vision` (and legacy `ui_tars`) selects the vision brain; everything else
/// (including absent, `text`, or the legacy `qwen`) is the default Text brain.
/// Reads [`BRAIN_ENV_FLAG`] first, then the legacy [`V2_BRAIN_ENV_FLAG`].
pub fn brain_choice_lookup<F>(lookup: F) -> BrainChoice
where
    F: Fn(&str) -> Option<String>,
{
    let raw = lookup(BRAIN_ENV_FLAG)
        .or_else(|| lookup(V2_BRAIN_ENV_FLAG))
        .map(|v| v.trim().to_ascii_lowercase());
    match raw.as_deref() {
        Some("vision") | Some("ui_tars") | Some("uitars") | Some("ui-tars") => BrainChoice::Vision,
        _ => BrainChoice::Text,
    }
}

#[cfg(test)]
mod brain_choice_tests {
    use super::*;

    #[test]
    fn brain_choice_defaults_text_and_selects_vision() {
        assert_eq!(brain_choice_lookup(|_| None), BrainChoice::Text);
        assert_eq!(brain_choice_lookup(|_| Some("text".into())), BrainChoice::Text);
        assert_eq!(brain_choice_lookup(|_| Some("something".into())), BrainChoice::Text);
        // Legacy alias still maps to Text (no behavior break).
        assert_eq!(brain_choice_lookup(|_| Some("qwen".into())), BrainChoice::Text);
        for raw in ["vision", "VISION", "ui_tars", "UI_TARS", " ui-tars ", "uitars"] {
            assert_eq!(
                brain_choice_lookup(|_| Some(raw.to_string())),
                BrainChoice::Vision,
                "{raw:?} should select the Vision brain"
            );
        }
        assert_eq!(BrainChoice::Text.label(), "text");
        assert_eq!(BrainChoice::Vision.label(), "vision");
    }
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn v2_flag_defaults_on_and_rolls_back_on_falsy() {
        // Absent => ON (V2 is now the default after the A6 flip).
        assert!(v2_enabled_lookup(|_| None));
        // Truthy => ON.
        for raw in ["1", "true", "TRUE", "yes", "on", " On ", "maybe"] {
            assert!(v2_enabled_lookup(|_| Some(raw.to_string())), "{raw:?} should be V2");
        }
        // Explicit falsy => documented V1 rollback.
        for raw in ["0", "false", "no", "off", "", "  Off "] {
            assert!(!v2_enabled_lookup(|_| Some(raw.to_string())), "{raw:?} should roll back to V1");
        }
    }
}
