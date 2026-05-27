//! Bounded X11/Wayland critical GUI matrix support.
//!
//! Phase 5 intentionally reuses existing high-signal cases. It does not create
//! a broad matrix runner or duplicate prompts across every environment axis.

use serde::{Deserialize, Serialize};

use super::governance::derive_governance_metadata;
use super::suites::{process_verification_suite, wayland_x11_compatibility_suite};
use super::types::{DisplayServerRequirement, GuiEvalCase};

const DISPLAY_CRITICAL_CASE_IDS: &[&str] = &[
    "compat-001-file-substrate-wayland",
    "compat-002-browser-search-wayland",
    "proc-001-app-open-uses-process-launched",
    "proc-002-file-write-then-open-wayland",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiEvalMatrixProfile {
    DisplayCritical,
    X11Critical,
    WaylandCritical,
}

impl GuiEvalMatrixProfile {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim() {
            "display-critical" | "display_critical" | "critical-display" | "critical_display" => {
                Some(Self::DisplayCritical)
            }
            "x11-critical" | "x11_critical" => Some(Self::X11Critical),
            "wayland-critical" | "wayland_critical" => Some(Self::WaylandCritical),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DisplayCritical => "display-critical",
            Self::X11Critical => "x11-critical",
            Self::WaylandCritical => "wayland-critical",
        }
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Self::DisplayCritical => "display-critical",
            Self::X11Critical => "x11-critical",
            Self::WaylandCritical => "wayland-critical",
        }
    }

    fn display_requirement(&self) -> DisplayServerRequirement {
        match self {
            Self::DisplayCritical => DisplayServerRequirement::X11OrWayland,
            Self::X11Critical => DisplayServerRequirement::X11Only,
            Self::WaylandCritical => DisplayServerRequirement::WaylandOnly,
        }
    }
}

pub fn supported_matrix_profiles() -> &'static [&'static str] {
    &["display-critical", "x11-critical", "wayland-critical"]
}

pub fn display_critical_matrix_suite(profile: GuiEvalMatrixProfile) -> Vec<GuiEvalCase> {
    let mut source = Vec::new();
    source.extend(wayland_x11_compatibility_suite());
    source.extend(process_verification_suite());

    DISPLAY_CRITICAL_CASE_IDS
        .iter()
        .filter_map(|id| source.iter().find(|case| case.id == *id).cloned())
        .map(|case| apply_matrix_profile(case, profile))
        .collect()
}

fn apply_matrix_profile(mut case: GuiEvalCase, profile: GuiEvalMatrixProfile) -> GuiEvalCase {
    case.display_server = profile.display_requirement();
    case.requires_desktop = true;
    push_tag(&mut case, "display-critical-matrix");
    push_tag(&mut case, profile.tag());
    case.governance = derive_governance_metadata(
        &case.id,
        &case.description,
        &case.prompt,
        &case.expected_behavior,
        case.display_server,
        case.requires_desktop,
        &case.tags,
    );
    case
}

fn push_tag(case: &mut GuiEvalCase, tag: &str) {
    if !case.tags.iter().any(|existing| existing == tag) {
        case.tags.push(tag.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui_eval::governance::EvalEnvironmentProfile;

    #[test]
    fn profile_parser_accepts_supported_names() {
        assert_eq!(
            GuiEvalMatrixProfile::from_str("display-critical"),
            Some(GuiEvalMatrixProfile::DisplayCritical)
        );
        assert_eq!(
            GuiEvalMatrixProfile::from_str("x11_critical"),
            Some(GuiEvalMatrixProfile::X11Critical)
        );
        assert_eq!(
            GuiEvalMatrixProfile::from_str("wayland-critical"),
            Some(GuiEvalMatrixProfile::WaylandCritical)
        );
        assert_eq!(GuiEvalMatrixProfile::from_str("everything"), None);
    }

    #[test]
    fn display_critical_matrix_is_small_and_curated() {
        let cases = display_critical_matrix_suite(GuiEvalMatrixProfile::DisplayCritical);
        assert_eq!(cases.len(), DISPLAY_CRITICAL_CASE_IDS.len());
        for case in cases {
            assert!(DISPLAY_CRITICAL_CASE_IDS.contains(&case.id.as_str()));
            assert!(case.tags.contains(&"display-critical-matrix".to_string()));
            assert_eq!(case.display_server, DisplayServerRequirement::X11OrWayland);
        }
    }

    #[test]
    fn x11_and_wayland_profiles_stamp_environment_metadata() {
        let x11_cases = display_critical_matrix_suite(GuiEvalMatrixProfile::X11Critical);
        let wayland_cases = display_critical_matrix_suite(GuiEvalMatrixProfile::WaylandCritical);

        assert!(!x11_cases.is_empty());
        assert!(!wayland_cases.is_empty());
        assert!(x11_cases.iter().all(|case| {
            case.display_server == DisplayServerRequirement::X11Only
                && case.governance.environment_profile == Some(EvalEnvironmentProfile::HostGuiX11)
        }));
        assert!(wayland_cases.iter().all(|case| {
            case.display_server == DisplayServerRequirement::WaylandOnly
                && case.governance.environment_profile
                    == Some(EvalEnvironmentProfile::HostGuiWayland)
        }));
    }
}
