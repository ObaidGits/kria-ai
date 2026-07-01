//! GUI Cognition V2 — risk assessment for a decided action (A3 safety parity).
//!
//! V2 actions are low-level (click an element, press a key, type text, open an
//! app), so risk is judged from the ACTION + its on-screen target/payload:
//! - `Black` — the typed text matches the hardcoded blacklist (e.g. `rm -rf /`,
//!   credential theft). Always blocked; no approval can authorize it.
//! - `Red`   — a destructive intent on the target label / typed text (delete,
//!   remove, send, pay, format, shutdown, ...). Requires approval before it runs.
//! - `Green` — everything else (open app, navigate, benign clicks/keys).
//!
//! The gate (desktop) maps these to Allow / Deny / approval. Pure + testable.

use crate::safety::{BlacklistChecker, RiskLevel};

use super::types::{Action, Decision, Observation};

/// Destructive intent verbs that escalate a GUI action to `Red` (needs approval).
/// App-agnostic; matched as whole words on the target label / typed text.
const DESTRUCTIVE_TERMS: &[&str] = &[
    "delete",
    "remove",
    "erase",
    "wipe",
    "format",
    "shutdown",
    "reboot",
    "restart",
    "uninstall",
    "discard",
    "send",
    "pay",
    "purchase",
    "buy",
    "confirm",
    "trash",
    "factory",
    "reset",
    "drop",
    "destroy",
    "overwrite",
    "unsubscribe",
    "deactivate",
];

/// Collect the human-meaningful text a risk judgement should look at for a
/// decision: typed text, the clicked element's label, or the key combo.
fn risk_texts(decision: &Decision, observation: &Observation) -> Vec<String> {
    match &decision.action {
        Action::Type { text } => vec![text.clone()],
        Action::TypeAndSubmit { text } => vec![text.clone()],
        Action::Navigate { url } => vec![url.clone()],
        Action::Key { combo } => vec![combo.clone()],
        Action::Click { element_id } => observation
            .element(*element_id)
            .map(|e| vec![e.label.clone()])
            .unwrap_or_default(),
        // OpenApp / ClickPoint / Scroll / Done / Ask carry no destructive payload.
        _ => Vec::new(),
    }
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok.eq_ignore_ascii_case(word))
}

/// Classify a decided action's risk from its target/payload (pure).
pub fn assess_action_risk(decision: &Decision, observation: &Observation) -> RiskLevel {
    let texts = risk_texts(decision, observation);
    if texts.is_empty() {
        return RiskLevel::Green;
    }
    let joined = texts.join(" ");
    // Black: hardcoded blacklist (shell destruction / credential theft) in payload.
    let checker = BlacklistChecker::new();
    if checker.is_blocked(&joined) {
        return RiskLevel::Black;
    }
    // Red: a destructive intent verb on the target/payload.
    let lower = joined.to_ascii_lowercase();
    if DESTRUCTIVE_TERMS.iter().any(|w| contains_word(&lower, w)) {
        return RiskLevel::Red;
    }
    RiskLevel::Green
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::gui_cognition_v2::types::{Bbox, UiElement};

    fn obs(label: &str) -> Observation {
        Observation {
            observation_id: "o".into(),
            screenshot_path: String::new(),
            screen_w: 1920,
            screen_h: 1080,
            active_window: None,
            elements: vec![UiElement {
                id: 1,
                bbox: Bbox {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                },
                monitor_index: 0,
                kind: "button".into(),
                label: label.into(),
                interactable: true,
                confidence: 0.9,
            }],
            som_image_path: None,
            source: "omniparser".into(),
        }
    }

    fn decide(action: Action) -> Decision {
        Decision {
            action,
            reason: String::new(),
            risk_hint: None,
        }
    }

    #[test]
    fn benign_actions_are_green() {
        let o = obs("New Tab");
        assert_eq!(
            assess_action_risk(
                &decide(Action::OpenApp {
                    app: "chrome".into()
                }),
                &o
            ),
            RiskLevel::Green
        );
        assert_eq!(
            assess_action_risk(
                &decide(Action::Key {
                    combo: "ctrl+t".into()
                }),
                &o
            ),
            RiskLevel::Green
        );
        assert_eq!(
            assess_action_risk(&decide(Action::Click { element_id: 1 }), &o),
            RiskLevel::Green
        );
        assert_eq!(
            assess_action_risk(
                &decide(Action::Type {
                    text: "hello world".into()
                }),
                &o
            ),
            RiskLevel::Green
        );
        assert_eq!(
            assess_action_risk(
                &decide(Action::Scroll {
                    direction: "down".into(),
                    amount: None
                }),
                &o
            ),
            RiskLevel::Green
        );
    }

    #[test]
    fn destructive_click_target_is_red() {
        let o = obs("Delete account");
        assert_eq!(
            assess_action_risk(&decide(Action::Click { element_id: 1 }), &o),
            RiskLevel::Red
        );
        let o2 = obs("Send");
        assert_eq!(
            assess_action_risk(&decide(Action::Click { element_id: 1 }), &o2),
            RiskLevel::Red
        );
    }

    #[test]
    fn destructive_typed_text_is_red() {
        let o = obs("x");
        assert_eq!(
            assess_action_risk(
                &decide(Action::Type {
                    text: "format the disk".into()
                }),
                &o
            ),
            RiskLevel::Red
        );
    }

    #[test]
    fn blacklisted_typed_text_is_black() {
        let o = obs("x");
        assert_eq!(
            assess_action_risk(
                &decide(Action::Type {
                    text: "rm -rf /".into()
                }),
                &o
            ),
            RiskLevel::Black
        );
    }

    #[test]
    fn whole_word_match_avoids_false_positives() {
        // "resetting" should not match the whole word "reset"; "predelete" not "delete".
        let o = obs("Preset filters");
        assert_eq!(
            assess_action_risk(&decide(Action::Click { element_id: 1 }), &o),
            RiskLevel::Green
        );
    }
}
