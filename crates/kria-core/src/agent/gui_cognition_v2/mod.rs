//! GUI Cognition V2 — Sight / Brain / Hands.
//!
//! A rebuild of GUI automation around three cleanly separated, independently
//! testable layers connected by a bounded observe → decide → act → verify loop:
//!
//! - [`Sight`](traits::Sight): screenshot → [`Observation`](types::Observation)
//!   (OmniParser-backed; Phase 1).
//! - [`GuiBrain`](traits::GuiBrain): (task, observation, history) → one
//!   [`Decision`](types::Decision). Model-agnostic seam — `QwenBrain` (Phase 2),
//!   `UiTarsBrain` drop-in (Phase 5b).
//! - [`GuiHands`](traits::GuiHands): execute a `Decision` via uinput (Phase 4).
//!
//! This module is added IN PARALLEL with the existing `gui_cognition` (V1)
//! pipeline and is reached only when [`v2_enabled`] is true (`KRIA_GUI_COG_V2`,
//! default OFF until V2 is proven on the eval harness — Requirement 10.1). When
//! the flag is OFF, GUI turns route through V1 unchanged (byte-for-byte).

pub mod fakes;
pub mod hands_uinput;
pub mod loop_engine;
pub mod qwen_brain;
pub mod sight_omniparser;
pub mod traits;
pub mod types;

pub use hands_uinput::{InputSink, UinputHands};
pub use loop_engine::{run_turn_v2, LoopConfig, LoopGuards, TurnOutcomeV2, TurnStatus};
pub use qwen_brain::QwenBrain;
pub use sight_omniparser::{OmniParserSight, ScreenCapturer};
pub use traits::{GateDecision, GuiBrain, GuiHands, SafetyGate, Sight};
pub use types::{Action, ActionResult, Bbox, Decision, Observation, TurnStep, UiElement};

/// Environment flag that routes GUI turns through the V2 Sight/Brain/Hands loop.
///
/// Default **OFF**: V2 is opt-in until it is proven on the real-verify eval
/// harness (Requirement 10.1). The Phase 6 gate (Task 12) flips this to a
/// default-ON helper with a falsy rollback to V1. Truthy values
/// (`1`/`true`/`yes`/`on`) enable it; everything else (including absent) is OFF.
pub const V2_ENV_FLAG: &str = "KRIA_GUI_COG_V2";

/// Whether the V2 pipeline is enabled (default OFF — see [`V2_ENV_FLAG`]).
pub fn v2_enabled() -> bool {
    v2_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`v2_enabled`] with an injectable lookup.
pub fn v2_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    matches!(
        lookup(V2_ENV_FLAG)
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

#[cfg(test)]
mod flag_tests {
    use super::*;

    #[test]
    fn v2_flag_defaults_off_and_enables_on_truthy() {
        // Absent => OFF (V1 default until proven).
        assert!(!v2_enabled_lookup(|_| None));
        for raw in ["1", "true", "TRUE", "yes", "on", " On "] {
            assert!(v2_enabled_lookup(|_| Some(raw.to_string())), "{raw:?} should enable");
        }
        for raw in ["0", "false", "no", "off", "", "maybe"] {
            assert!(!v2_enabled_lookup(|_| Some(raw.to_string())), "{raw:?} should stay off");
        }
    }
}
