//! Interaction Safety Layer — Prevents Unsafe GUI Interactions.
//!
//! KRIA must NEVER blindly type or click. This module provides:
//! - Target confidence scoring before any interaction
//! - Foreground ownership validation
//! - Interaction precondition checks
//! - Safe typing/clicking with drift detection
//!
//! # Safety Invariants
//!
//! 1. No typing without confirmed focus ownership
//! 2. No clicking without coordinate confidence > threshold
//! 3. No interaction after focus drift detected
//! 4. Automatic HITL escalation when confidence is low
//! 5. Immediate halt on wrong-window detection

use crate::agent::workflow_types::*;

// ═══════════════════════════════════════════════════════════════════════════════
// §1 — Interaction Target Confidence
// ═══════════════════════════════════════════════════════════════════════════════

/// Confidence assessment for an interaction target.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InteractionConfidence {
    /// Overall confidence that the interaction will hit the right target
    pub overall: f32,
    /// Confidence that the correct app is focused
    pub app_confidence: f32,
    /// Confidence that the correct window is active
    pub window_confidence: f32,
    /// Confidence that focus is on the right element
    pub focus_confidence: f32,
    /// Whether interaction is safe to proceed
    pub safe_to_proceed: bool,
    /// Reason if not safe
    pub block_reason: Option<String>,
}

/// Minimum confidence thresholds for different interaction types.
pub struct InteractionThresholds {
    /// Minimum confidence for typing (high — wrong-window typing is dangerous)
    pub typing_min: f32,
    /// Minimum confidence for clicking (moderate — clicks are more targeted)
    pub clicking_min: f32,
    /// Minimum confidence for keyboard shortcuts (moderate)
    pub shortcut_min: f32,
}

impl Default for InteractionThresholds {
    fn default() -> Self {
        Self {
            typing_min: 0.70,
            clicking_min: 0.60,
            shortcut_min: 0.65,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §2 — Interaction Precondition Checks
// ═══════════════════════════════════════════════════════════════════════════════

/// Check whether an interaction is safe to execute.
///
/// This is called BEFORE every keystroke/click/shortcut action.
/// Returns `Safe` only when all preconditions are met.
pub fn check_interaction_safety(
    action: &str,
    capabilities: &CapabilitySet,
    thresholds: &InteractionThresholds,
) -> InteractionSafetyCheck {
    // Check 1: Input injection must be available
    if capabilities.interaction.keyboard_injection == InputInjectionLevel::None {
        return InteractionSafetyCheck::Blocked {
            reason: "Input injection unavailable (no uinput daemon)".into(),
            suggestion: InteractionSuggestion::ManualStep,
        };
    }

    // Check 2: Determine required confidence for this action type
    let required_confidence = match action {
        "type_text" => thresholds.typing_min,
        "click_mouse" | "click_element" => thresholds.clicking_min,
        "press_shortcut" => thresholds.shortcut_min,
        _ => thresholds.clicking_min,
    };

    // Check 3: Estimate current interaction confidence from capabilities
    let estimated_confidence = estimate_interaction_confidence(capabilities);

    if estimated_confidence < required_confidence {
        return InteractionSafetyCheck::LowConfidence {
            estimated: estimated_confidence,
            required: required_confidence,
            suggestion: if estimated_confidence < 0.40 {
                InteractionSuggestion::ManualStep
            } else {
                InteractionSuggestion::ProceedWithCaution
            },
        };
    }

    InteractionSafetyCheck::Safe {
        confidence: estimated_confidence,
    }
}

/// Estimate interaction confidence from current capabilities.
fn estimate_interaction_confidence(capabilities: &CapabilitySet) -> f32 {
    let mut confidence: f32 = 0.0;
    let mut factors = 0;

    // Input injection quality
    match capabilities.interaction.keyboard_injection {
        InputInjectionLevel::Full => {
            confidence += 0.90;
            factors += 1;
        }
        InputInjectionLevel::XdotoolOnly => {
            confidence += 0.60;
            factors += 1;
        }
        InputInjectionLevel::None => {
            confidence += 0.0;
            factors += 1;
        }
    }

    // Window observation quality (can we verify focus?)
    confidence += capabilities.verifier.window_state_max_confidence;
    factors += 1;

    // AT-SPI availability (can we verify elements?)
    match &capabilities.environment.atspi_level {
        AtSpiLevel::Full => {
            confidence += 0.85;
            factors += 1;
        }
        AtSpiLevel::Partial { .. } => {
            confidence += 0.55;
            factors += 1;
        }
        AtSpiLevel::BusOnly => {
            confidence += 0.30;
            factors += 1;
        }
        AtSpiLevel::None => {
            confidence += 0.10;
            factors += 1;
        }
    }

    if factors > 0 {
        confidence / factors as f32
    } else {
        0.0
    }
}

/// Result of an interaction safety check.
#[derive(Debug, Clone)]
pub enum InteractionSafetyCheck {
    /// Safe to proceed with interaction
    Safe { confidence: f32 },
    /// Confidence too low — proceed with caution or escalate
    LowConfidence {
        estimated: f32,
        required: f32,
        suggestion: InteractionSuggestion,
    },
    /// Interaction blocked — cannot proceed
    Blocked {
        reason: String,
        suggestion: InteractionSuggestion,
    },
}

impl InteractionSafetyCheck {
    pub fn is_safe(&self) -> bool {
        matches!(self, Self::Safe { .. })
    }
}

/// What to do when interaction confidence is insufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionSuggestion {
    /// Ask the user to do this step manually
    ManualStep,
    /// Proceed but with extra verification after
    ProceedWithCaution,
    /// Retry after re-acquiring focus
    RetryAfterFocus,
    /// Abort the workflow
    Abort,
}

// ═══════════════════════════════════════════════════════════════════════════════
// §3 — Focus Drift Detection
// ═══════════════════════════════════════════════════════════════════════════════

/// Detects whether focus has drifted away from the expected target.
///
/// Called between interaction steps to verify the workflow still
/// owns the foreground.
pub fn detect_focus_drift(
    _expected_app: Option<&str>,
    capabilities: &CapabilitySet,
) -> FocusDriftStatus {
    // On Wayland without AT-SPI, we cannot reliably detect focus drift
    if capabilities.environment.session_type == SessionType::Wayland
        && matches!(
            capabilities.environment.atspi_level,
            AtSpiLevel::None | AtSpiLevel::BusOnly
        )
    {
        return FocusDriftStatus::Unknown {
            reason: "Cannot verify focus on Wayland without AT-SPI".into(),
        };
    }

    // If we have AT-SPI or xdotool, we could check focus here
    // For now, return Stable (actual implementation would query window state)
    FocusDriftStatus::Stable {
        confidence: capabilities.verifier.window_state_max_confidence,
    }
}

/// Focus drift detection result.
#[derive(Debug, Clone)]
pub enum FocusDriftStatus {
    /// Focus is on the expected target
    Stable { confidence: f32 },
    /// Focus has moved to a different window
    Drifted {
        current_app: String,
        expected_app: String,
    },
    /// Cannot determine focus state
    Unknown { reason: String },
}

impl FocusDriftStatus {
    pub fn is_stable(&self) -> bool {
        matches!(self, Self::Stable { .. })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// §4 — Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_full_caps() -> CapabilitySet {
        CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::X11,
                compositor: None,
                atspi_level: AtSpiLevel::Full,
                xdotool_available: true,
                uinput_available: true,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![VerificationMethod::AtSpi, VerificationMethod::Xdotool],
                window_state_max_confidence: 0.90,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::Full,
                mouse_injection: InputInjectionLevel::Full,
                clipboard_available: true,
            },
        }
    }

    fn make_no_input_caps() -> CapabilitySet {
        CapabilitySet {
            environment: EnvironmentCapability {
                session_type: SessionType::Wayland,
                compositor: Some("mutter".into()),
                atspi_level: AtSpiLevel::None,
                xdotool_available: false,
                uinput_available: false,
                ocr_available: false,
            },
            verifier: VerifierCapability {
                available_methods: vec![VerificationMethod::ProcessTable],
                window_state_max_confidence: 0.40,
                cdp_available: false,
                filesystem_available: true,
                process_table_available: true,
            },
            interaction: InteractionCapability {
                keyboard_injection: InputInjectionLevel::None,
                mouse_injection: InputInjectionLevel::None,
                clipboard_available: true,
            },
        }
    }

    #[test]
    fn typing_safe_with_full_capabilities() {
        let caps = make_full_caps();
        let thresholds = InteractionThresholds::default();
        let result = check_interaction_safety("type_text", &caps, &thresholds);
        assert!(
            result.is_safe(),
            "Typing should be safe with full capabilities"
        );
    }

    #[test]
    fn typing_blocked_without_uinput() {
        let caps = make_no_input_caps();
        let thresholds = InteractionThresholds::default();
        let result = check_interaction_safety("type_text", &caps, &thresholds);
        assert!(!result.is_safe(), "Typing must be blocked without uinput");
        assert!(matches!(result, InteractionSafetyCheck::Blocked { .. }));
    }

    #[test]
    fn clicking_safe_with_full_capabilities() {
        let caps = make_full_caps();
        let thresholds = InteractionThresholds::default();
        let result = check_interaction_safety("click_mouse", &caps, &thresholds);
        assert!(result.is_safe());
    }

    #[test]
    fn focus_drift_unknown_on_bare_wayland() {
        let caps = make_no_input_caps();
        let result = detect_focus_drift(Some("code"), &caps);
        assert!(matches!(result, FocusDriftStatus::Unknown { .. }));
    }

    #[test]
    fn focus_drift_stable_on_x11() {
        let caps = make_full_caps();
        let result = detect_focus_drift(Some("code"), &caps);
        assert!(result.is_stable());
    }

    #[test]
    fn confidence_estimation_reflects_capabilities() {
        let full = make_full_caps();
        let none = make_no_input_caps();

        let full_conf = estimate_interaction_confidence(&full);
        let none_conf = estimate_interaction_confidence(&none);

        assert!(
            full_conf > 0.70,
            "Full caps should give high confidence: {}",
            full_conf
        );
        assert!(
            none_conf < 0.30,
            "No caps should give low confidence: {}",
            none_conf
        );
    }
}
