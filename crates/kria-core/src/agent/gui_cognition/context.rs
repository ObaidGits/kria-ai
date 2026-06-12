use std::time::{SystemTime, UNIX_EPOCH};

use super::perception::{
    sanitize_gui_text, GuiActiveWindowSummary, GuiBounds, GuiControlSummary, GuiCursorFocusSummary,
    GuiMonitorSummary, GuiObservationSnapshot, GuiOcrBlock,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiEvidenceTrustLevel {
    TrustedExecutable,
    TrustedState,
    SupportingVisual,
    UntrustedText,
}

impl GuiEvidenceTrustLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TrustedExecutable => "trusted_executable",
            Self::TrustedState => "trusted_state",
            Self::SupportingVisual => "supporting_visual",
            Self::UntrustedText => "untrusted_text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiContextFreshness {
    Fresh,
    Stale,
    Unknown,
}

impl GuiContextFreshness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiSafetyFacts {
    pub llm_native_tool_loop: bool,
    pub raw_ocr_trusted: bool,
    pub ocr_trust_level: GuiEvidenceTrustLevel,
    pub accessibility_is_executable_authority: bool,
    pub screenshot_is_freshness_evidence: bool,
}

impl Default for GuiSafetyFacts {
    fn default() -> Self {
        Self {
            llm_native_tool_loop: false,
            raw_ocr_trusted: false,
            ocr_trust_level: GuiEvidenceTrustLevel::UntrustedText,
            accessibility_is_executable_authority: true,
            screenshot_is_freshness_evidence: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiContextSourceConfidence {
    pub active_window: f64,
    pub accessibility: f64,
    pub screenshot: f64,
    pub ocr: f64,
    pub monitor: f64,
    pub focus: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiAccessibilityContextEvidence {
    pub trust_level: GuiEvidenceTrustLevel,
    pub available: bool,
    pub node_count: usize,
    pub trusted_control_count: usize,
    pub executable_control_count: usize,
    pub disabled_or_hidden_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiOcrContextEvidence {
    pub trust_level: GuiEvidenceTrustLevel,
    pub block_count: usize,
    pub injection_count: usize,
    pub redaction_count: usize,
    pub safe_previews: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiVisualContextEvidence {
    pub trust_level: GuiEvidenceTrustLevel,
    pub screenshot_available: bool,
    pub screen_hash_prefix: Option<String>,
    pub monitor_count: usize,
    pub dpi_available: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GuiObservationDelta {
    pub active_window_changed: bool,
    pub screen_hash_changed: bool,
    pub monitor_layout_changed: bool,
    pub control_count_changed: bool,
    pub focused_control_changed: bool,
    pub stale_action_risk: bool,
    pub changed_summary: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GuiContextPreviousState {
    pub previous_context_id: Option<String>,
    pub previous_observation_id: Option<String>,
    pub delta: GuiObservationDelta,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiRedactionReport {
    pub redaction_count: usize,
    pub ocr_injection_count: usize,
    pub ocr_untrusted: bool,
    pub redacted_sources: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiContextBuildReport {
    pub status: String,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct GuiContextBuildRequest {
    pub observation: GuiObservationSnapshot,
    pub previous_context: Option<GuiContext>,
}

impl GuiContextBuildRequest {
    pub fn new(observation: GuiObservationSnapshot) -> Self {
        Self {
            observation,
            previous_context: None,
        }
    }

    pub fn with_previous(
        observation: GuiObservationSnapshot,
        previous_context: GuiContext,
    ) -> Self {
        Self {
            observation,
            previous_context: Some(previous_context),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiContext {
    pub context_id: String,
    pub observation_id: String,
    pub built_at_ms: i64,
    pub observation: GuiObservationSnapshot,
    pub active_window: GuiActiveWindowSummary,
    pub monitor_layout: Vec<GuiMonitorSummary>,
    pub focus_state: GuiCursorFocusSummary,
    pub accessibility_evidence: GuiAccessibilityContextEvidence,
    pub ocr_evidence: GuiOcrContextEvidence,
    pub visual_evidence: GuiVisualContextEvidence,
    pub source_confidence: GuiContextSourceConfidence,
    pub fused_controls: Vec<GuiControlSummary>,
    pub executable_controls: Vec<GuiControlSummary>,
    pub safety: GuiSafetyFacts,
    pub freshness: GuiContextFreshness,
    pub previous: GuiContextPreviousState,
    pub redaction_report: GuiRedactionReport,
    pub build_report: GuiContextBuildReport,
}

pub struct GuiContextBuilder;

impl GuiContextBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build(&self, request: GuiContextBuildRequest) -> GuiContext {
        let observation = request.observation;
        let all_controls = observation.all_controls();
        let fused_controls = fuse_controls_with_ocr(&all_controls, &observation.ocr_blocks);
        let executable_controls = fused_controls
            .iter()
            .filter(|control| control.is_executable_candidate())
            .cloned()
            .collect::<Vec<_>>();
        let redaction_report = redaction_report(&observation);
        let previous = request
            .previous_context
            .as_ref()
            .map(|previous| previous_state(previous, &observation))
            .unwrap_or_default();
        let freshness = freshness_for(&observation, &previous.delta);
        let source_confidence = GuiContextSourceConfidence {
            active_window: observation.active_window.confidence,
            accessibility: observation.accessibility.overall_confidence,
            screenshot: if observation.screenshot_available {
                0.85
            } else {
                0.0
            },
            ocr: if observation.ocr_available { 0.45 } else { 0.0 },
            monitor: if observation.monitors.is_empty() {
                0.0
            } else {
                0.82
            },
            focus: if observation.cursor_focus.keyboard_focus_known {
                observation.cursor_focus.confidence.max(0.75)
            } else {
                0.0
            },
        };

        let blockers = source_blockers_from_observation(&observation);
        let warnings = build_warnings(&observation, &redaction_report, &previous.delta);
        let status = if !observation.has_useful_signal() {
            "blocked"
        } else if matches!(freshness, GuiContextFreshness::Stale) {
            "stale"
        } else {
            "ready"
        };

        GuiContext {
            context_id: observation.context_id.clone(),
            observation_id: observation.observation_id.clone(),
            built_at_ms: unix_now_ms(),
            active_window: observation.active_window.clone(),
            monitor_layout: observation.monitors.clone(),
            focus_state: observation.cursor_focus.clone(),
            accessibility_evidence: GuiAccessibilityContextEvidence {
                trust_level: GuiEvidenceTrustLevel::TrustedExecutable,
                available: observation.accessibility.available,
                node_count: observation.accessibility.node_count,
                trusted_control_count: fused_controls
                    .iter()
                    .filter(|control| control.quality == "trusted")
                    .count(),
                executable_control_count: executable_controls.len(),
                disabled_or_hidden_count: all_controls
                    .iter()
                    .filter(|control| !control.enabled || !control.visible)
                    .count(),
            },
            ocr_evidence: GuiOcrContextEvidence {
                trust_level: GuiEvidenceTrustLevel::UntrustedText,
                block_count: observation.ocr_blocks.len(),
                injection_count: redaction_report.ocr_injection_count,
                redaction_count: redaction_report.redaction_count,
                safe_previews: observation
                    .ocr_blocks
                    .iter()
                    .map(|block| block.safe_text_preview.clone())
                    .filter(|preview| !preview.is_empty())
                    .take(6)
                    .collect(),
            },
            visual_evidence: GuiVisualContextEvidence {
                trust_level: GuiEvidenceTrustLevel::SupportingVisual,
                screenshot_available: observation.screenshot_available,
                screen_hash_prefix: observation
                    .screen_hash
                    .as_ref()
                    .map(|hash| hash.chars().take(16).collect()),
                monitor_count: observation.monitors.len(),
                dpi_available: !observation.monitors.is_empty(),
            },
            source_confidence,
            fused_controls,
            executable_controls,
            safety: GuiSafetyFacts::default(),
            freshness,
            previous,
            redaction_report,
            build_report: GuiContextBuildReport {
                status: status.to_string(),
                warnings,
                blockers,
            },
            observation,
        }
    }
}

impl Default for GuiContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GuiContext {
    pub fn from_observation(observation: GuiObservationSnapshot) -> Self {
        GuiContextBuilder::new().build(GuiContextBuildRequest::new(observation))
    }

    pub fn text_field_count(&self) -> usize {
        self.observation.text_fields.len()
    }

    pub fn button_count(&self) -> usize {
        self.observation.buttons.len()
    }

    pub fn dialog_count(&self) -> usize {
        self.observation.dialogs.len()
    }

    pub fn executable_text_fields(&self) -> Vec<GuiControlSummary> {
        self.executable_controls
            .iter()
            .filter(|control| control.role.to_lowercase().contains("text"))
            .cloned()
            .collect()
    }

    pub fn executable_buttons(&self) -> Vec<GuiControlSummary> {
        self.executable_controls
            .iter()
            .filter(|control| control.role.to_lowercase().contains("button"))
            .cloned()
            .collect()
    }

    pub fn active_window_is_terminal_like(&self) -> bool {
        if self.focus_state.terminal_like {
            return true;
        }
        let lower = self.active_window.label.to_lowercase();
        ["terminal", "konsole", "gnome-terminal", "xterm", "shell"]
            .iter()
            .any(|needle| lower.contains(needle))
    }

    pub fn ocr_has_injection(&self) -> bool {
        self.ocr_evidence.injection_count > 0
    }

    pub fn source_blockers(&self) -> Vec<String> {
        self.build_report.blockers.clone()
    }

    pub fn trusted_control_count(&self) -> usize {
        self.accessibility_evidence.trusted_control_count
    }

    pub fn executable_control_count(&self) -> usize {
        self.accessibility_evidence.executable_control_count
    }

    pub fn context_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "context_id": self.context_id,
            "observation_id": self.observation_id,
            "trusted_control_count": self.accessibility_evidence.trusted_control_count,
            "executable_control_count": self.accessibility_evidence.executable_control_count,
            "disabled_or_hidden_count": self.accessibility_evidence.disabled_or_hidden_count,
            "ocr_untrusted": self.redaction_report.ocr_untrusted,
            "ocr_injection_count": self.redaction_report.ocr_injection_count,
            "redaction_count": self.redaction_report.redaction_count,
            "freshness": self.freshness.as_str(),
            "status": self.build_report.status,
            "previous_context_id": self.previous.previous_context_id,
            "previous_observation_id": self.previous.previous_observation_id,
            "delta": self.previous.delta,
            "source_confidence": self.source_confidence,
            "screen_hash_prefix": self.visual_evidence.screen_hash_prefix,
            "focus": {
                "source": self.focus_state.source,
                "confidence": self.focus_state.confidence,
                "reliability": self.focus_state.reliability,
                "keyboard_focus_known": self.focus_state.keyboard_focus_known,
                "editable_target_known": self.focus_state.editable_target_known,
                "terminal_like": self.focus_state.terminal_like,
                "adapter_status": self.focus_state.adapter_status,
                "latency_ms": self.focus_state.latency_ms,
                "focused_window": self.focus_state.focused_window_label,
                "focused_app": self.focus_state.focused_app,
                "focused_control_label": self.focus_state.focused_control_label,
                "focused_control_role": self.focus_state.focused_control_role,
                "focused_control_bounds": self.focus_state.focused_control_bounds,
            },
            "accessibility_health": {
                "status": self.observation.accessibility.overall_status,
                "confidence": self.observation.accessibility.overall_confidence,
                "app_scores": self.observation.accessibility.app_scores,
                "stale_node_count": self.observation.accessibility.stale_node_count,
                "timeout_count": self.observation.accessibility.timeout_count,
                "cache_hit_count": self.observation.accessibility.cache_hit_count,
                "stale_cache_rejected_count": self.observation.accessibility.stale_cache_rejected_count,
            },
            "visual_controls": {
                "detected": self.observation.visual_controls.len(),
                "supporting_only": true,
            },
            "ocr_performance": {
                "fast_path": self.observation.ocr_diagnostics.fast_path,
                "cache_hit": self.observation.ocr_diagnostics.cache_hit,
                "roi_count": self.observation.ocr_diagnostics.roi_count,
                "changed_region_count": self.observation.ocr_diagnostics.changed_region_count,
                "cold_start_ms": self.observation.ocr_diagnostics.cold_start_ms,
                "warm_start_ms": self.observation.ocr_diagnostics.warm_start_ms,
            },
            "source_blockers": self.build_report.blockers,
            "warnings": self.build_report.warnings,
        })
    }

    pub fn context_built_event(&self) -> serde_json::Value {
        let mut summary = self.context_summary();
        if let Some(object) = summary.as_object_mut() {
            object.insert("type".into(), serde_json::json!("ContextBuilt"));
            object.insert(
                "active_window".into(),
                serde_json::json!(self.active_window.label),
            );
            object.insert(
                "control_summary".into(),
                serde_json::json!({
                    "text_fields": self.text_field_count(),
                    "buttons": self.button_count(),
                    "dialogs": self.dialog_count(),
                }),
            );
            object.insert(
                "safety".into(),
                serde_json::json!({
                    "llm_native_tool_loop": self.safety.llm_native_tool_loop,
                    "raw_ocr_trusted": self.safety.raw_ocr_trusted,
                    "ocr_trust_level": self.safety.ocr_trust_level.as_str(),
                    "accessibility_is_executable_authority": self.safety.accessibility_is_executable_authority,
                    "screenshot_is_freshness_evidence": self.safety.screenshot_is_freshness_evidence,
                }),
            );
            object.insert(
                "source_trust".into(),
                serde_json::json!({
                    "accessibility": self.accessibility_evidence.trust_level.as_str(),
                    "ocr": self.ocr_evidence.trust_level.as_str(),
                    "visual": self.visual_evidence.trust_level.as_str(),
                }),
            );
        }
        summary
    }
}

fn fuse_controls_with_ocr(
    controls: &[GuiControlSummary],
    ocr_blocks: &[GuiOcrBlock],
) -> Vec<GuiControlSummary> {
    controls
        .iter()
        .cloned()
        .map(|mut control| {
            let support_count = ocr_blocks
                .iter()
                .filter(|block| ocr_supports_control(block, &control))
                .count();
            if support_count > 0 {
                control.evidence = format!(
                    "{}; {support_count} OCR block(s) matched as untrusted supporting evidence",
                    control.evidence
                );
                control.confidence = (control.confidence + 0.05).min(0.98);
            }
            let sanitized_name = sanitize_gui_text(&control.name, 160);
            control.name = sanitized_name.text;
            control
        })
        .collect()
}

fn ocr_supports_control(block: &GuiOcrBlock, control: &GuiControlSummary) -> bool {
    let name = control.name.trim().to_lowercase();
    if !name.is_empty() && block.safe_text_preview.to_lowercase().contains(&name) {
        return true;
    }
    match (&block.bounds, &control.bounds) {
        (Some(block_bounds), Some(control_bounds)) => bounds_overlap(block_bounds, control_bounds),
        _ => false,
    }
}

fn bounds_overlap(a: &GuiBounds, b: &GuiBounds) -> bool {
    let ax2 = a.x + a.width;
    let ay2 = a.y + a.height;
    let bx2 = b.x + b.width;
    let by2 = b.y + b.height;
    a.x < bx2 && ax2 > b.x && a.y < by2 && ay2 > b.y
}

fn redaction_report(observation: &GuiObservationSnapshot) -> GuiRedactionReport {
    let ocr_redactions = observation
        .ocr_blocks
        .iter()
        .filter(|block| block.redaction_applied)
        .count();
    let ocr_injections = observation
        .ocr_blocks
        .iter()
        .filter(|block| block.injection_suspected)
        .count();
    let control_redactions = observation
        .text_fields
        .iter()
        .chain(observation.buttons.iter())
        .chain(observation.dialogs.iter())
        .filter(|control| sanitize_gui_text(&control.name, 160).redaction_applied)
        .count();
    let mut redacted_sources = Vec::new();
    if ocr_redactions > 0 || ocr_injections > 0 {
        redacted_sources.push("ocr".into());
    }
    if control_redactions > 0 {
        redacted_sources.push("accessibility_controls".into());
    }
    GuiRedactionReport {
        redaction_count: ocr_redactions + control_redactions,
        ocr_injection_count: ocr_injections,
        ocr_untrusted: true,
        redacted_sources,
    }
}

fn previous_state(
    previous: &GuiContext,
    observation: &GuiObservationSnapshot,
) -> GuiContextPreviousState {
    let mut delta = GuiObservationDelta::default();
    delta.active_window_changed = previous.active_window.label != observation.active_window.label;
    delta.screen_hash_changed = previous.observation.screen_hash != observation.screen_hash;
    delta.monitor_layout_changed =
        monitor_signature(&previous.monitor_layout) != monitor_signature(&observation.monitors);
    delta.control_count_changed =
        previous.observation.visible_control_count() != observation.visible_control_count();
    delta.focused_control_changed =
        previous.focus_state.focused_control_id != observation.cursor_focus.focused_control_id;
    delta.stale_action_risk = delta.active_window_changed
        || delta.screen_hash_changed
        || delta.monitor_layout_changed
        || delta.focused_control_changed;

    if delta.active_window_changed {
        delta.changed_summary.push("active_window_changed".into());
    }
    if delta.screen_hash_changed {
        delta.changed_summary.push("screen_hash_changed".into());
    }
    if delta.monitor_layout_changed {
        delta.changed_summary.push("monitor_layout_changed".into());
    }
    if delta.control_count_changed {
        delta.changed_summary.push("control_count_changed".into());
    }
    if delta.focused_control_changed {
        delta.changed_summary.push("focused_control_changed".into());
    }

    GuiContextPreviousState {
        previous_context_id: Some(previous.context_id.clone()),
        previous_observation_id: Some(previous.observation_id.clone()),
        delta,
    }
}

fn freshness_for(
    observation: &GuiObservationSnapshot,
    delta: &GuiObservationDelta,
) -> GuiContextFreshness {
    if !observation.has_useful_signal() {
        GuiContextFreshness::Unknown
    } else if delta.stale_action_risk {
        GuiContextFreshness::Stale
    } else {
        GuiContextFreshness::Fresh
    }
}

fn build_warnings(
    observation: &GuiObservationSnapshot,
    redaction: &GuiRedactionReport,
    delta: &GuiObservationDelta,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if redaction.ocr_injection_count > 0 {
        warnings.push("OCR injection text was detected and treated as untrusted evidence.".into());
    }
    if redaction.redaction_count > 0 {
        warnings.push("Sensitive text was redacted before context use.".into());
    }
    if delta.stale_action_risk {
        warnings.push(
            "Current observation differs from previous context; approvals/actions must revalidate."
                .into(),
        );
    }
    if !observation.accessibility.available {
        warnings
            .push("Accessibility is unavailable; executable target authority is limited.".into());
    }
    warnings
}

fn source_blockers_from_observation(observation: &GuiObservationSnapshot) -> Vec<String> {
    [
        (
            "active_window",
            &observation.capabilities.active_window.blocker,
        ),
        (
            "desktop_state",
            &observation.capabilities.desktop_state.blocker,
        ),
        (
            "accessibility",
            &observation.capabilities.accessibility.blocker,
        ),
        ("screenshot", &observation.capabilities.screenshot.blocker),
        ("ocr", &observation.capabilities.ocr.blocker),
        ("monitor", &observation.capabilities.monitor.blocker),
        (
            "cursor_focus",
            &observation.capabilities.cursor_focus.blocker,
        ),
    ]
    .into_iter()
    .filter_map(|(source, blocker)| {
        blocker
            .as_ref()
            .map(|blocker| format!("{source}: {}", sanitize_gui_text(blocker, 200).text))
    })
    .collect()
}

fn monitor_signature(monitors: &[GuiMonitorSummary]) -> Vec<String> {
    monitors
        .iter()
        .map(|monitor| {
            format!(
                "{}:{}:{}:{}:{}:{}",
                monitor.id,
                monitor.bounds.x,
                monitor.bounds.y,
                monitor.bounds.width,
                monitor.bounds.height,
                monitor.scale_factor
            )
        })
        .collect()
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}
