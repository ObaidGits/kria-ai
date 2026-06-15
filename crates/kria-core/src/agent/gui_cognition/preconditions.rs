//! GUI Cognition preconditions health-gate (Task 1.4 — Requirement 25).
//!
//! Before a GUI Cognition turn executes *live* actions (`execute_live`) the
//! runtime health-checks the environment preconditions and reports readiness
//! (Requirement 25.1). When a required precondition is missing it degrades the
//! turn to **observe/plan-only** with a clear, sanitized reason — never a silent
//! failure or a misleading "completed" (Requirement 25.2).
//!
//! The four preconditions mirror the design pipeline header
//! (`preconditions health-gate (R25) — uinput/AT-SPI/focus/DISPLAY`):
//!
//! | Precondition  | Signal reused                                   |
//! |---------------|-------------------------------------------------|
//! | `uinput`      | action/input backend availability — [`GuiActionBackendStatus::can_execute_actions`] (uinput on Wayland, xdotool on X11, …) |
//! | `atspi`       | AT-SPI accessibility bus — [`GuiObservationSnapshot::accessibility_ok`] (derived from `atspi_bus_available` / `accessibility_operational`) |
//! | `focus_backend` | window/control focus capability — [`GuiActionBackendStatus::focus_supported`] |
//! | `display`     | DISPLAY / session type — [`GuiActionBackendStatus::session_type`] known (not `unknown`/empty) |
//!
//! This module **reuses** the existing precondition probes (the perception
//! provider observation and the GUI action backend status). It does not
//! re-implement the [`crate::safety::global_halt`] master kill-switch — that is
//! enforced separately by the backend and by the pre-action guard (Task 1.2).
//!
//! Enforcement (the downgrade to observe/plan-only) is gated behind the
//! `gui_cog_runtime_guards` flag by the caller; when the flag is OFF, existing
//! Step 1–12 behavior is preserved and no degrade occurs. The readiness summary
//! itself is additive and harmless.

use serde::{Deserialize, Serialize};

use super::executor::GuiActionBackendStatus;
use super::perception::{sanitize_gui_text, GuiObservationSnapshot};

/// Max length of a sanitized precondition detail/reason surfaced in events.
const DETAIL_LIMIT: usize = 200;

/// Stable precondition names (used in the missing-list + events). Keep stable —
/// part of the event/response contract.
pub mod precondition_name {
    /// Input/action backend availability (uinput / ydotool / xdotool).
    pub const UINPUT: &str = "uinput";
    /// AT-SPI accessibility bus availability.
    pub const ATSPI: &str = "atspi";
    /// Window/control focus backend availability.
    pub const FOCUS_BACKEND: &str = "focus_backend";
    /// DISPLAY / session type known.
    pub const DISPLAY: &str = "display";
}

/// Readiness of a single precondition probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiPreconditionStatus {
    /// Whether this precondition is satisfied for live execution.
    pub ready: bool,
    /// Sanitized, human-readable detail (status when ready, blocker when not).
    pub detail: String,
}

impl GuiPreconditionStatus {
    fn ready(detail: impl Into<String>) -> Self {
        Self {
            ready: true,
            detail: sanitize_detail(detail),
        }
    }

    fn missing(detail: impl Into<String>) -> Self {
        Self {
            ready: false,
            detail: sanitize_detail(detail),
        }
    }
}

/// Aggregated preconditions readiness report evaluated before `execute_live`.
///
/// Produced by [`GuiPreconditionsReport::evaluate`] from the existing backend
/// status + perception observation; surfaced as a sanitized structured summary
/// in `response.gui_cognition.preconditions` and as `PreconditionsChecked` /
/// `PreconditionsDegraded` events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuiPreconditionsReport {
    /// Input/action backend availability (Requirement 25.1).
    pub uinput: GuiPreconditionStatus,
    /// AT-SPI accessibility bus availability.
    pub atspi: GuiPreconditionStatus,
    /// Window/control focus backend availability.
    pub focus_backend: GuiPreconditionStatus,
    /// DISPLAY / session-type readiness.
    pub display: GuiPreconditionStatus,
    /// Whether ALL required preconditions are satisfied for live execution.
    pub ready: bool,
    /// Names of the missing preconditions (stable, see [`precondition_name`]).
    pub missing: Vec<String>,
    /// Sanitized degraded reason when not ready (`None` when fully ready).
    pub degraded_reason: Option<String>,
}

impl GuiPreconditionsReport {
    /// Evaluate preconditions from the GUI action backend status and the
    /// current perception observation. Reuses existing probes only.
    pub fn evaluate(
        backend: &GuiActionBackendStatus,
        observation: &GuiObservationSnapshot,
    ) -> Self {
        // uinput / action-backend availability (covers uinput on Wayland,
        // ydotool, xdotool on X11). `can_execute_actions` is the authoritative
        // signal already computed by the backend probe.
        let uinput = if backend.can_execute_actions {
            GuiPreconditionStatus::ready(format!(
                "action backend ready ({})",
                backend.selected_backend
            ))
        } else {
            GuiPreconditionStatus::missing(backend.primary_backend_blocker())
        };

        // AT-SPI accessibility bus (atspi_bus_available / accessibility_operational).
        let atspi = if observation.accessibility_ok {
            GuiPreconditionStatus::ready("AT-SPI accessibility bus available")
        } else {
            let detail = observation
                .capabilities
                .accessibility
                .blocker
                .clone()
                .unwrap_or_else(|| "AT-SPI accessibility bus is not operational".into());
            GuiPreconditionStatus::missing(detail)
        };

        // Focus backend (window/control focus capability).
        let focus_backend = if backend.focus_supported {
            GuiPreconditionStatus::ready("focus backend available")
        } else if !backend.can_execute_actions {
            GuiPreconditionStatus::missing(format!(
                "focus backend unavailable: {}",
                backend.primary_backend_blocker()
            ))
        } else {
            GuiPreconditionStatus::missing(
                "selected backend does not support focusing windows/controls",
            )
        };

        // DISPLAY / session type known (x11 / wayland / test, but not empty/unknown).
        let session = backend.session_type.trim().to_ascii_lowercase();
        let display = if session.is_empty() || session == "unknown" {
            GuiPreconditionStatus::missing(
                "DISPLAY/session type is unknown; no graphical session detected",
            )
        } else {
            GuiPreconditionStatus::ready(format!("session type {}", backend.session_type))
        };

        let mut missing = Vec::new();
        if !uinput.ready {
            missing.push(precondition_name::UINPUT.to_string());
        }
        if !atspi.ready {
            missing.push(precondition_name::ATSPI.to_string());
        }
        if !focus_backend.ready {
            missing.push(precondition_name::FOCUS_BACKEND.to_string());
        }
        if !display.ready {
            missing.push(precondition_name::DISPLAY.to_string());
        }

        let ready = missing.is_empty();
        let degraded_reason = if ready {
            None
        } else {
            let parts: Vec<String> = [
                (precondition_name::UINPUT, &uinput),
                (precondition_name::ATSPI, &atspi),
                (precondition_name::FOCUS_BACKEND, &focus_backend),
                (precondition_name::DISPLAY, &display),
            ]
            .into_iter()
            .filter(|(_, status)| !status.ready)
            .map(|(name, status)| format!("{name}: {}", status.detail))
            .collect();
            Some(sanitize_detail(format!(
                "Degraded to observe/plan-only — live execution preconditions not met ({}).",
                parts.join("; ")
            )))
        };

        Self {
            uinput,
            atspi,
            focus_backend,
            display,
            ready,
            missing,
            degraded_reason,
        }
    }

    /// Sanitized JSON summary for `response.gui_cognition.preconditions`.
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "uinput": self.uinput.ready,
            "atspi": self.atspi.ready,
            "focus_backend": self.focus_backend.ready,
            "display": self.display.ready,
            "ready": self.ready,
            "missing": self.missing,
            "degraded_reason": self.degraded_reason,
            "details": {
                "uinput": self.uinput.detail,
                "atspi": self.atspi.detail,
                "focus_backend": self.focus_backend.detail,
                "display": self.display.detail,
            },
        })
    }

    /// `PreconditionsChecked` event reporting readiness (Requirement 25.1).
    pub fn checked_event(&self) -> serde_json::Value {
        let mut payload = self.summary_json();
        if let Some(object) = payload.as_object_mut() {
            object.insert("type".into(), serde_json::json!("PreconditionsChecked"));
        }
        payload
    }

    /// `PreconditionsDegraded` event with the missing preconditions list and the
    /// clear, sanitized reason for the degrade (Requirement 25.2).
    pub fn degraded_event(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "PreconditionsDegraded",
            "missing": self.missing,
            "degraded_mode": "observe_plan_only",
            "reason": self
                .degraded_reason
                .clone()
                .unwrap_or_else(|| "preconditions not ready for live execution".into()),
        })
    }
}

fn sanitize_detail(detail: impl Into<String>) -> String {
    let cleaned = sanitize_gui_text(&detail.into(), DETAIL_LIMIT).text;
    if cleaned.trim().is_empty() {
        "unavailable".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::gui_cognition::executor::GuiActionBackendStatus;
    use crate::agent::gui_cognition::perception::{collect_observation, GuiPerceptionProvider, GuiProbeResult};
    use async_trait::async_trait;

    struct StubPerception {
        accessibility: bool,
    }

    #[async_trait]
    impl GuiPerceptionProvider for StubPerception {
        async fn get_active_window(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "title": "App", "app_name": "App" }))
        }
        async fn get_desktop_state(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({
                "focused_window": "App",
                "accessibility_operational": self.accessibility,
                "applications": ["App"],
            }))
        }
        async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": self.accessibility }))
        }
        async fn find_ui_elements(&self, _role: &str) -> GuiProbeResult {
            GuiProbeResult::ok(serde_json::json!({ "elements": [] }))
        }
        async fn focused_window_title(&self) -> Option<String> {
            Some("App".into())
        }
    }

    async fn observe(accessibility: bool) -> GuiObservationSnapshot {
        collect_observation(
            &StubPerception { accessibility },
            "precond-obs".to_string(),
            "precond-ctx".to_string(),
        )
        .await
    }

    #[tokio::test]
    async fn ready_when_backend_and_accessibility_healthy() {
        let backend = GuiActionBackendStatus::available("uinput_accessibility");
        let observation = observe(true).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        assert!(report.ready, "report: {report:?}");
        assert!(report.missing.is_empty());
        assert!(report.degraded_reason.is_none());
        assert_eq!(report.summary_json()["ready"], true);
    }

    #[tokio::test]
    async fn missing_uinput_marks_not_ready_with_reason() {
        let backend = GuiActionBackendStatus::blocked(
            "unavailable",
            "no usable uinput socket or validated ydotool backend",
            "wayland",
        );
        let observation = observe(true).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        assert!(!report.ready);
        assert!(report.missing.contains(&precondition_name::UINPUT.to_string()));
        let reason = report.degraded_reason.clone().expect("reason");
        assert!(reason.contains("observe/plan-only"), "reason: {reason}");
        // The degraded event lists the missing precondition.
        let event = report.degraded_event();
        assert_eq!(event["type"], "PreconditionsDegraded");
        assert_eq!(event["degraded_mode"], "observe_plan_only");
    }

    #[tokio::test]
    async fn missing_atspi_marks_not_ready() {
        let backend = GuiActionBackendStatus::available("uinput_accessibility");
        let observation = observe(false).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        assert!(!report.ready);
        assert!(report.missing.contains(&precondition_name::ATSPI.to_string()));
        assert!(report.uinput.ready, "uinput stays ready");
    }

    #[tokio::test]
    async fn unknown_session_marks_display_not_ready() {
        let mut backend = GuiActionBackendStatus::available("uinput_accessibility");
        backend.session_type = "unknown".into();
        let observation = observe(true).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        assert!(!report.ready);
        assert!(report.missing.contains(&precondition_name::DISPLAY.to_string()));
    }

    #[tokio::test]
    async fn missing_focus_backend_marks_not_ready() {
        // Backend can execute actions but cannot focus windows/controls.
        let mut backend = GuiActionBackendStatus::available("uinput_no_focus");
        backend.focus_supported = false;
        let observation = observe(true).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        assert!(!report.ready);
        assert!(report
            .missing
            .contains(&precondition_name::FOCUS_BACKEND.to_string()));
        // uinput stays ready: the backend can still execute non-focus actions.
        assert!(report.uinput.ready, "uinput stays ready");
        assert!(!report.focus_backend.ready);
    }

    #[tokio::test]
    async fn summary_and_checked_event_are_sanitized_and_structured() {
        let backend = GuiActionBackendStatus::available("uinput_accessibility");
        let observation = observe(true).await;
        let report = GuiPreconditionsReport::evaluate(&backend, &observation);
        let event = report.checked_event();
        assert_eq!(event["type"], "PreconditionsChecked");
        assert_eq!(event["uinput"], true);
        assert_eq!(event["atspi"], true);
        assert_eq!(event["focus_backend"], true);
        assert_eq!(event["display"], true);
    }
}
