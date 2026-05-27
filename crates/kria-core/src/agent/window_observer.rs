//! Window observation runtime.
//!
//! This module deliberately observes desktop/window state without depending on
//! the input-injection daemon. Input and observation are separate authorities:
//! observation can degrade or fail without implying keyboard/mouse injection is
//! unavailable, and input can be disabled without making window truth unknowable.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;

use crate::agent::environment_grounder::{
    DisplayServerType, EnvironmentGrounder, GroundingCapabilities, LiveEnvironmentGrounder,
};
use crate::agent::execution_verifier::{
    VerificationEvidence, VerificationEvidenceSource, VerificationReliability,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOperationSupport {
    Supported,
    Degraded,
    Forbidden,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DisplayCapabilityMatrix {
    pub display_server: DisplayServerType,
    pub active_window_query: WindowOperationSupport,
    pub window_visibility_query: WindowOperationSupport,
    pub arbitrary_focus_activation: WindowOperationSupport,
    pub keyboard_target_confirmation: WindowOperationSupport,
}

impl DisplayCapabilityMatrix {
    pub fn from_grounding(capabilities: GroundingCapabilities) -> Self {
        match capabilities.display_server {
            DisplayServerType::X11 => Self {
                display_server: capabilities.display_server,
                active_window_query: if capabilities.has_window_query {
                    WindowOperationSupport::Supported
                } else {
                    WindowOperationSupport::Degraded
                },
                window_visibility_query: if capabilities.has_window_list {
                    WindowOperationSupport::Supported
                } else {
                    WindowOperationSupport::Degraded
                },
                arbitrary_focus_activation: WindowOperationSupport::Supported,
                keyboard_target_confirmation: if capabilities.has_window_query {
                    WindowOperationSupport::Supported
                } else {
                    WindowOperationSupport::Degraded
                },
            },
            DisplayServerType::XWayland => Self {
                display_server: capabilities.display_server,
                active_window_query: WindowOperationSupport::Degraded,
                window_visibility_query: if capabilities.has_window_list {
                    WindowOperationSupport::Degraded
                } else {
                    WindowOperationSupport::Unknown
                },
                arbitrary_focus_activation: WindowOperationSupport::Degraded,
                keyboard_target_confirmation: WindowOperationSupport::Degraded,
            },
            DisplayServerType::Wayland => Self {
                display_server: capabilities.display_server,
                active_window_query: WindowOperationSupport::Degraded,
                window_visibility_query: WindowOperationSupport::Degraded,
                arbitrary_focus_activation: WindowOperationSupport::Forbidden,
                keyboard_target_confirmation: WindowOperationSupport::Degraded,
            },
            DisplayServerType::Unknown => Self {
                display_server: capabilities.display_server,
                active_window_query: WindowOperationSupport::Unknown,
                window_visibility_query: WindowOperationSupport::Unknown,
                arbitrary_focus_activation: WindowOperationSupport::Unknown,
                keyboard_target_confirmation: WindowOperationSupport::Unknown,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowObservation {
    pub active_title: Option<String>,
    pub active_class: Option<String>,
    pub active_pid: Option<u32>,
    pub visible_match: bool,
    pub active_match: bool,
    pub keyboard_target_confirmed: bool,
    pub capability_matrix: DisplayCapabilityMatrix,
    pub evidence: VerificationEvidence,
}

#[async_trait]
pub trait WindowObserver: Send + Sync {
    async fn observe(
        &self,
        title_contains: Option<&str>,
        class: Option<&str>,
        pid: Option<u32>,
    ) -> WindowObservation;

    fn capabilities(&self) -> DisplayCapabilityMatrix;
}

pub struct LiveWindowObserver {
    grounder: Arc<dyn EnvironmentGrounder>,
    capabilities: DisplayCapabilityMatrix,
}

impl LiveWindowObserver {
    pub fn new() -> Self {
        let grounder = Arc::new(LiveEnvironmentGrounder::new());
        let capabilities = DisplayCapabilityMatrix::from_grounding(GroundingCapabilities::probe());
        Self {
            grounder,
            capabilities,
        }
    }

    pub fn with_grounder(grounder: Arc<dyn EnvironmentGrounder>) -> Self {
        let capabilities = DisplayCapabilityMatrix::from_grounding(GroundingCapabilities::probe());
        Self {
            grounder,
            capabilities,
        }
    }
}

impl Default for LiveWindowObserver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WindowObserver for LiveWindowObserver {
    async fn observe(
        &self,
        title_contains: Option<&str>,
        class: Option<&str>,
        pid: Option<u32>,
    ) -> WindowObservation {
        let started = std::time::Instant::now();
        let facts = self.grounder.ground(&[]).await;

        let active = facts.focused_window.as_ref();
        let title_match = match (active, title_contains) {
            (Some(w), Some(t)) => w.title.to_lowercase().contains(&t.to_lowercase()),
            (Some(_), None) => true,
            _ => false,
        };
        let class_match = match (active, class) {
            (Some(w), Some(c)) => w.class.to_lowercase().contains(&c.to_lowercase()),
            (Some(_), None) => true,
            _ => false,
        };
        let pid_match = match (active, pid) {
            (Some(w), Some(p)) => w.pid == p,
            (Some(_), None) => true,
            _ => false,
        };
        let active_match = title_match && class_match && pid_match;

        let visible_match = facts.visible_windows.iter().any(|w| {
            let title_ok = title_contains
                .map(|t| w.title.to_lowercase().contains(&t.to_lowercase()))
                .unwrap_or(true);
            let class_ok = class
                .map(|c| w.class.to_lowercase().contains(&c.to_lowercase()))
                .unwrap_or(true);
            let pid_ok = pid.map(|p| w.pid == p).unwrap_or(true);
            title_ok && class_ok && pid_ok
        }) || active_match;

        let keyboard_target_confirmed = active_match
            && !matches!(
                self.capabilities.keyboard_target_confirmation,
                WindowOperationSupport::Forbidden | WindowOperationSupport::Unknown
            );

        let source = if facts.capabilities.has_window_query || facts.capabilities.has_window_list {
            VerificationEvidenceSource::WindowManager
        } else {
            VerificationEvidenceSource::Heuristic
        };
        let reliability = match self.capabilities.display_server {
            DisplayServerType::X11 if keyboard_target_confirmed => VerificationReliability::Strong,
            DisplayServerType::X11 => VerificationReliability::Partial,
            DisplayServerType::XWayland | DisplayServerType::Wayland => {
                if active_match {
                    VerificationReliability::Partial
                } else {
                    VerificationReliability::Weak
                }
            }
            DisplayServerType::Unknown => VerificationReliability::Unobservable,
        };
        let confidence = if keyboard_target_confirmed {
            0.82
        } else if active_match {
            0.68
        } else if visible_match {
            0.52
        } else {
            0.10
        };

        WindowObservation {
            active_title: active.map(|w| w.title.clone()),
            active_class: active.map(|w| w.class.clone()),
            active_pid: active.map(|w| w.pid),
            visible_match,
            active_match,
            keyboard_target_confirmed,
            capability_matrix: self.capabilities.clone(),
            evidence: VerificationEvidence {
                source,
                reliability,
                confidence,
                semantic_meaning: if keyboard_target_confirmed {
                    "keyboard_target_confirmed".into()
                } else if active_match {
                    "active_window_matches".into()
                } else if visible_match {
                    "window_visible_not_confirmed_active".into()
                } else {
                    "window_not_observed".into()
                },
                observed_at: SystemTime::now(),
                freshness_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
                ambiguous: !keyboard_target_confirmed,
                details: format!(
                    "display={:?}, active_title={:?}, active_class={:?}, active_pid={:?}",
                    self.capabilities.display_server,
                    active.map(|w| w.title.as_str()),
                    active.map(|w| w.class.as_str()),
                    active.map(|w| w.pid),
                ),
            },
        }
    }

    fn capabilities(&self) -> DisplayCapabilityMatrix {
        self.capabilities.clone()
    }
}

pub fn observation_timeout() -> Duration {
    Duration::from_millis(1_500)
}
