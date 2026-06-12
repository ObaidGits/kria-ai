use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use std::future::Future;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const GUI_PROBE_TIMEOUT_MS: u64 = 1_500;
const MAX_CONTROL_SUMMARY: usize = 160;
const MAX_OCR_BLOCKS: usize = 16;
const MAX_VISIBLE_WINDOWS: usize = 24;

static SECRET_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)((?:api[_-]?key|token|password|passwd|secret|credential|authorization|bearer)\s*[:=]\s*)[^\s,;]+",
    )
    .expect("GUI Cognition secret redaction regex is valid")
});

static CARD_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:\d[ -]*?){13,19}\b").expect("credit-card redaction regex is valid")
});

static INJECTION_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(ignore previous instructions|system prompt|developer message|click delete|run command|exfiltrate|send credentials)\b",
    )
    .expect("OCR injection regex is valid")
});

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct GuiProbeResult {
    pub success: bool,
    pub data: serde_json::Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiProbeTimingSummary {
    pub probe_name: String,
    pub duration_ms: u64,
    pub status: String,
    pub source: String,
    pub cache_hit: bool,
    pub blocker_kind: Option<String>,
}

impl GuiProbeTimingSummary {
    pub fn cached(probe_name: impl Into<String>) -> Self {
        Self {
            probe_name: probe_name.into(),
            duration_ms: 0,
            status: "ok".into(),
            source: "observation_cache".into(),
            cache_hit: true,
            blocker_kind: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiSourceAttemptSummary {
    pub source: String,
    pub status: String,
    pub reliability: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiObservationTimingSummary {
    pub total_ms: u64,
    pub slowest_probe: Option<String>,
    pub slowest_probe_ms: u64,
    pub probe_timeout_count: usize,
    pub probe_timings: Vec<GuiProbeTimingSummary>,
}

impl Default for GuiObservationTimingSummary {
    fn default() -> Self {
        Self {
            total_ms: 0,
            slowest_probe: None,
            slowest_probe_ms: 0,
            probe_timeout_count: 0,
            probe_timings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiObservationCacheSummary {
    pub cache_hit: bool,
    pub cache_age_ms: Option<u64>,
    pub cache_policy: String,
    pub freshness: String,
}

impl Default for GuiObservationCacheSummary {
    fn default() -> Self {
        Self {
            cache_hit: false,
            cache_age_ms: None,
            cache_policy: "disabled".into(),
            freshness: "fresh".into(),
        }
    }
}

impl GuiProbeResult {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data,
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            data: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }

    pub fn err_with_data(error: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: false,
            data,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiControlSummary {
    pub control_id: String,
    pub role: String,
    pub name: String,
    pub path: String,
    pub bounds: Option<GuiBounds>,
    pub enabled: bool,
    pub focused: bool,
    pub visible: bool,
    pub in_active_window: bool,
    pub source: String,
    pub confidence: f64,
    pub evidence: String,
    #[serde(default = "default_control_quality")]
    pub quality: String,
    #[serde(default = "default_control_label_source")]
    pub label_source: String,
    #[serde(default = "default_control_state_source")]
    pub state_source: String,
    #[serde(default)]
    pub rejection_reason: Option<String>,
    #[serde(default = "default_control_identity_confidence")]
    pub identity_confidence: f64,
    #[serde(default)]
    pub bounds_confidence: f64,
    #[serde(default = "default_control_state_confidence")]
    pub state_confidence: f64,
    #[serde(default)]
    pub executable_confidence: f64,
    #[serde(default)]
    pub sources: Vec<String>,
}

impl GuiControlSummary {
    pub fn new(role: impl Into<String>, name: impl Into<String>, path: impl Into<String>) -> Self {
        let role = role.into();
        let name = sanitize_gui_text(&name.into(), MAX_CONTROL_SUMMARY).text;
        let path = path.into();
        let control_id = stable_hash(&format!("{role}|{name}|{path}|accessibility"));
        Self {
            control_id,
            role,
            name,
            path,
            bounds: None,
            enabled: true,
            focused: false,
            visible: true,
            in_active_window: false,
            source: "accessibility".into(),
            confidence: 0.72,
            evidence: "accessible control summary".into(),
            quality: "partial".into(),
            label_source: "accessible_name".into(),
            state_source: "accessibility_state".into(),
            rejection_reason: None,
            identity_confidence: 0.72,
            bounds_confidence: 0.0,
            state_confidence: 0.72,
            executable_confidence: 0.0,
            sources: vec!["accessibility".into()],
        }
    }

    pub fn is_executable_candidate(&self) -> bool {
        self.enabled
            && self.visible
            && self.executable_confidence >= 0.75
            && self.quality == "trusted"
    }
}

fn default_control_quality() -> String {
    "partial".into()
}

fn default_control_label_source() -> String {
    "unknown".into()
}

fn default_control_state_source() -> String {
    "unknown".into()
}

fn default_control_identity_confidence() -> f64 {
    0.72
}

fn default_control_state_confidence() -> f64 {
    0.72
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiActiveWindowSummary {
    pub label: String,
    pub app_name: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub workspace: Option<i64>,
    #[serde(default)]
    pub monitor: Option<i64>,
    #[serde(default)]
    pub fullscreen: Option<bool>,
    #[serde(default)]
    pub minimized: Option<bool>,
    #[serde(default)]
    pub observed_at_ms: Option<i64>,
    pub source: String,
    pub confidence: f64,
    pub fallback_used: bool,
    pub blocker: Option<String>,
    #[serde(default = "default_active_window_reliability")]
    pub reliability: String,
    #[serde(default = "default_active_window_authority_status")]
    pub authority_status: String,
    #[serde(default = "default_gnome_bridge_status")]
    pub gnome_bridge_status: String,
    #[serde(default)]
    pub fallback_chain: Vec<GuiSourceAttemptSummary>,
}

impl Default for GuiActiveWindowSummary {
    fn default() -> Self {
        Self {
            label: "unknown".into(),
            app_name: None,
            app_id: None,
            pid: None,
            workspace: None,
            monitor: None,
            fullscreen: None,
            minimized: None,
            observed_at_ms: None,
            source: "unavailable".into(),
            confidence: 0.0,
            fallback_used: true,
            blocker: Some("No active window source exposed a focused window title.".into()),
            reliability: "unavailable".into(),
            authority_status: "unavailable".into(),
            gnome_bridge_status: "unknown".into(),
            fallback_chain: Vec::new(),
        }
    }
}

fn default_active_window_reliability() -> String {
    "unavailable".into()
}

fn default_active_window_authority_status() -> String {
    "unavailable".into()
}

fn default_gnome_bridge_status() -> String {
    "unknown".into()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiWindowSummary {
    pub title: String,
    pub app_name: Option<String>,
    pub bounds: Option<GuiBounds>,
    pub focused: bool,
    pub visible: bool,
    pub monitor_id: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiMonitorSummary {
    pub id: String,
    pub name: Option<String>,
    pub bounds: GuiBounds,
    pub work_area: Option<GuiBounds>,
    pub scale_factor: f64,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiCursorFocusSummary {
    pub cursor_x: Option<i32>,
    pub cursor_y: Option<i32>,
    pub focused_control_id: Option<String>,
    pub focused_window_label: Option<String>,
    pub keyboard_focus_known: bool,
    pub source: String,
    #[serde(default)]
    pub focused_app: Option<String>,
    #[serde(default)]
    pub focused_control_label: Option<String>,
    #[serde(default)]
    pub focused_control_role: Option<String>,
    #[serde(default)]
    pub focused_control_bounds: Option<GuiBounds>,
    #[serde(default)]
    pub text_cursor_known: bool,
    #[serde(default)]
    pub editable_target_known: bool,
    #[serde(default)]
    pub terminal_like: bool,
    #[serde(default)]
    pub confidence: f64,
    #[serde(default = "default_focus_reliability")]
    pub reliability: String,
    #[serde(default)]
    pub adapter_status: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub failure_chain: Vec<GuiSourceAttemptSummary>,
}

impl Default for GuiCursorFocusSummary {
    fn default() -> Self {
        Self {
            cursor_x: None,
            cursor_y: None,
            focused_control_id: None,
            focused_window_label: None,
            keyboard_focus_known: false,
            source: "unavailable".into(),
            focused_app: None,
            focused_control_label: None,
            focused_control_role: None,
            focused_control_bounds: None,
            text_cursor_known: false,
            editable_target_known: false,
            terminal_like: false,
            confidence: 0.0,
            reliability: "unavailable".into(),
            adapter_status: None,
            latency_ms: None,
            failure_chain: Vec::new(),
        }
    }
}

fn default_focus_reliability() -> String {
    "unavailable".into()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiAccessibilityAppScore {
    pub app_label: String,
    pub bus_name_hash: String,
    pub node_count: usize,
    pub control_count: usize,
    pub timeout_count: usize,
    pub stale_node_count: usize,
    pub score: f64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiAccessibilitySummary {
    pub available: bool,
    pub node_count: usize,
    pub control_count: usize,
    pub omitted_node_count: usize,
    pub enabled_control_count: usize,
    pub disabled_control_count: usize,
    pub visible_control_count: usize,
    pub focused_control_count: usize,
    pub source: String,
    #[serde(default)]
    pub source_status: String,
    #[serde(default)]
    pub snapshot_total_ms: Option<u64>,
    #[serde(default)]
    pub skipped_app_count: usize,
    #[serde(default)]
    pub remediation: Vec<String>,
    #[serde(default = "default_accessibility_overall_status")]
    pub overall_status: String,
    #[serde(default)]
    pub overall_confidence: f64,
    #[serde(default)]
    pub app_scores: Vec<GuiAccessibilityAppScore>,
    #[serde(default)]
    pub stale_node_count: usize,
    #[serde(default)]
    pub timeout_count: usize,
    #[serde(default)]
    pub cache_hit_count: usize,
    #[serde(default)]
    pub stale_cache_rejected_count: usize,
}

impl Default for GuiAccessibilitySummary {
    fn default() -> Self {
        Self {
            available: false,
            node_count: 0,
            control_count: 0,
            omitted_node_count: 0,
            enabled_control_count: 0,
            disabled_control_count: 0,
            visible_control_count: 0,
            focused_control_count: 0,
            source: "unavailable".into(),
            source_status: "unavailable".into(),
            snapshot_total_ms: None,
            skipped_app_count: 0,
            remediation: Vec::new(),
            overall_status: "unavailable".into(),
            overall_confidence: 0.0,
            app_scores: Vec::new(),
            stale_node_count: 0,
            timeout_count: 0,
            cache_hit_count: 0,
            stale_cache_rejected_count: 0,
        }
    }
}

fn default_accessibility_overall_status() -> String {
    "unavailable".into()
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiOcrBlock {
    pub block_id: String,
    pub safe_text_preview: String,
    pub text_hash: String,
    pub bounds: Option<GuiBounds>,
    pub confidence: f64,
    pub untrusted: bool,
    pub injection_suspected: bool,
    pub redaction_applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct GuiOcrDiagnostics {
    pub wait_for_screenshot_ms: Option<u64>,
    pub engine_selected: Option<String>,
    pub engine_status: Option<String>,
    pub image_status: Option<String>,
    pub total_ms: Option<u64>,
    #[serde(default)]
    pub fast_path: Option<String>,
    #[serde(default)]
    pub cache_hit: bool,
    #[serde(default)]
    pub roi_count: usize,
    #[serde(default)]
    pub changed_region_count: usize,
    #[serde(default)]
    pub cold_start_ms: Option<u64>,
    #[serde(default)]
    pub warm_start_ms: Option<u64>,
    #[serde(default)]
    pub benchmark_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiVisualControlDetection {
    pub visual_id: String,
    pub control_type: String,
    pub label_preview_safe: String,
    pub bounds: Option<GuiBounds>,
    pub confidence: f64,
    pub source: String,
    pub matched_control_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiSourceStatus {
    pub available: bool,
    pub detail: String,
    pub blocker: Option<String>,
    pub remediation: Vec<String>,
}

impl GuiSourceStatus {
    pub fn available(detail: impl Into<String>) -> Self {
        Self {
            available: true,
            detail: detail.into(),
            blocker: None,
            remediation: Vec::new(),
        }
    }

    pub fn blocked(detail: impl Into<String>, blocker: impl Into<String>) -> Self {
        Self {
            available: false,
            detail: detail.into(),
            blocker: Some(sanitize_gui_text(&blocker.into(), 240).text),
            remediation: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuiPerceptionCapabilities {
    pub active_window: GuiSourceStatus,
    pub desktop_state: GuiSourceStatus,
    pub accessibility: GuiSourceStatus,
    pub screenshot: GuiSourceStatus,
    pub ocr: GuiSourceStatus,
    pub monitor: GuiSourceStatus,
    pub cursor_focus: GuiSourceStatus,
}

impl Default for GuiPerceptionCapabilities {
    fn default() -> Self {
        Self {
            active_window: GuiSourceStatus::blocked("active_window", "not probed"),
            desktop_state: GuiSourceStatus::blocked("desktop_state", "not probed"),
            accessibility: GuiSourceStatus::blocked("accessibility", "not probed"),
            screenshot: GuiSourceStatus::blocked("screenshot", "not probed"),
            ocr: GuiSourceStatus::blocked("ocr", "not probed"),
            monitor: GuiSourceStatus::blocked("monitor", "not probed"),
            cursor_focus: GuiSourceStatus::blocked("cursor_focus", "not probed"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedText {
    pub text: String,
    pub redaction_applied: bool,
    pub injection_suspected: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiObservationSnapshot {
    pub observation_id: String,
    pub context_id: String,
    pub timestamp_ms: i64,
    pub screen_hash: Option<String>,
    pub active_window_label: String,
    pub active_window: GuiActiveWindowSummary,
    pub visible_windows: Vec<GuiWindowSummary>,
    pub visible_app_count: usize,
    pub monitors: Vec<GuiMonitorSummary>,
    pub cursor_focus: GuiCursorFocusSummary,
    pub accessibility: GuiAccessibilitySummary,
    pub ocr_blocks: Vec<GuiOcrBlock>,
    #[serde(default)]
    pub ocr_diagnostics: GuiOcrDiagnostics,
    pub capabilities: GuiPerceptionCapabilities,
    pub accessibility_ok: bool,
    pub ocr_available: bool,
    pub screenshot_available: bool,
    pub active_window_probe_ok: bool,
    pub desktop_state_probe_ok: bool,
    pub capabilities_probe_ok: bool,
    pub text_fields: Vec<GuiControlSummary>,
    pub buttons: Vec<GuiControlSummary>,
    pub dialogs: Vec<GuiControlSummary>,
    #[serde(default)]
    pub other_controls: Vec<GuiControlSummary>,
    #[serde(default)]
    pub visual_controls: Vec<GuiVisualControlDetection>,
    #[serde(default)]
    pub timing: GuiObservationTimingSummary,
    #[serde(default)]
    pub cache: GuiObservationCacheSummary,
}

impl GuiObservationSnapshot {
    pub fn visible_control_count(&self) -> usize {
        self.all_controls().len()
    }

    pub fn visible_accessible_control_count(&self) -> usize {
        self.all_controls()
            .iter()
            .filter(|control| control.visible)
            .count()
    }

    pub fn disabled_control_count(&self) -> usize {
        self.all_controls()
            .iter()
            .filter(|control| !control.enabled)
            .count()
    }

    pub fn hidden_control_count(&self) -> usize {
        self.all_controls()
            .iter()
            .filter(|control| !control.visible)
            .count()
    }

    pub fn control_quality_count(&self, quality: &str) -> usize {
        self.all_controls()
            .iter()
            .filter(|control| control.quality == quality)
            .count()
    }

    pub fn all_controls(&self) -> Vec<GuiControlSummary> {
        self.text_fields
            .iter()
            .chain(self.buttons.iter())
            .chain(self.dialogs.iter())
            .chain(self.other_controls.iter())
            .cloned()
            .collect()
    }

    pub fn active_window_display(&self) -> String {
        if self.active_window_label == "unknown" {
            let reason = self
                .active_window
                .blocker
                .as_deref()
                .unwrap_or("the OS/accessibility stack did not expose a focused window title");
            format!("unknown ({reason})")
        } else {
            self.active_window_label.clone()
        }
    }

    pub fn active_window_confidence(&self) -> f64 {
        self.active_window.confidence
    }

    pub fn has_useful_signal(&self) -> bool {
        self.active_window_label != "unknown"
            || self.visible_app_count > 0
            || self.visible_control_count() > 0
            || self.screenshot_available
            || !self.monitors.is_empty()
    }
}

#[async_trait]
pub trait GuiPerceptionProvider: Send + Sync {
    async fn get_active_window(&self) -> GuiProbeResult;
    async fn get_desktop_state(&self) -> GuiProbeResult;
    async fn get_accessibility_capabilities(&self) -> GuiProbeResult;
    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult;
    async fn focused_window_title(&self) -> Option<String>;

    async fn capture_screenshot(&self) -> GuiProbeResult {
        GuiProbeResult::err("screenshot capture is not implemented for this provider")
    }

    async fn run_ocr(&self) -> GuiProbeResult {
        GuiProbeResult::err("OCR is not implemented for this provider")
    }

    async fn get_monitor_layout(&self) -> GuiProbeResult {
        capture_monitor_layout_probe().await
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        GuiProbeResult::err("cursor/focus state is not implemented for this provider")
    }

    async fn get_accessibility_tree_summary(&self) -> GuiProbeResult {
        GuiProbeResult::err("accessibility tree summary is not implemented for this provider")
    }

    async fn detect_visual_controls(&self) -> GuiProbeResult {
        GuiProbeResult::err("visual control detection is not implemented for this provider")
    }

    fn observation_cache_policy(&self) -> &'static str {
        "disabled"
    }

    async fn cached_observation(
        &self,
        _observation_id: &str,
        _context_id: &str,
    ) -> Option<GuiObservationSnapshot> {
        None
    }

    async fn store_observation_cache(&self, _observation: &GuiObservationSnapshot) {}
}

pub async fn collect_observation<P: GuiPerceptionProvider>(
    provider: &P,
    observation_id: String,
    context_id: String,
) -> GuiObservationSnapshot {
    let observation_started = Instant::now();
    if let Some(mut observation) = provider
        .cached_observation(&observation_id, &context_id)
        .await
    {
        observation.observation_id = observation_id;
        observation.context_id = context_id;
        observation.timestamp_ms = unix_now_ms();
        observation.timing.total_ms = elapsed_ms(observation_started);
        if observation.timing.probe_timings.is_empty() {
            observation.timing.probe_timings =
                vec![GuiProbeTimingSummary::cached("observation_cache")];
        }
        observation.cache.cache_hit = true;
        if observation.cache.cache_policy.trim().is_empty() {
            observation.cache.cache_policy = provider.observation_cache_policy().into();
        }
        return observation;
    }

    let (
        (active_window, active_window_timing),
        (desktop_state, desktop_state_timing),
        (capabilities, capabilities_timing),
        (accessibility_tree, accessibility_tree_timing),
        (screenshot, screenshot_timing),
        (ocr, ocr_timing),
        (monitor_layout, monitor_layout_timing),
        (cursor_focus, cursor_focus_timing),
        (text_fields, text_fields_timing),
        (buttons, buttons_timing),
        (dialogs, dialogs_timing),
        (check_boxes, check_boxes_timing),
        (links, links_timing),
        (tabs, tabs_timing),
        (visual_controls, visual_controls_timing),
        focused_title_timed,
    ) = tokio::join!(
        timed_bounded_probe("get_active_window", provider.get_active_window()),
        timed_bounded_probe("get_desktop_state", provider.get_desktop_state()),
        timed_bounded_probe(
            "get_accessibility_capabilities",
            provider.get_accessibility_capabilities(),
        ),
        timed_bounded_probe(
            "get_accessibility_tree_summary",
            provider.get_accessibility_tree_summary(),
        ),
        timed_bounded_probe("capture_screenshot", provider.capture_screenshot()),
        timed_bounded_probe("run_ocr", provider.run_ocr()),
        timed_bounded_probe("get_monitor_layout", provider.get_monitor_layout()),
        timed_bounded_probe("get_cursor_focus_state", provider.get_cursor_focus_state()),
        timed_bounded_probe("find_ui_elements:text", provider.find_ui_elements("text")),
        timed_bounded_probe(
            "find_ui_elements:push_button",
            provider.find_ui_elements("push button"),
        ),
        timed_bounded_probe(
            "find_ui_elements:dialog",
            provider.find_ui_elements("dialog")
        ),
        timed_bounded_probe(
            "find_ui_elements:check_box",
            provider.find_ui_elements("check box")
        ),
        timed_bounded_probe("find_ui_elements:link", provider.find_ui_elements("link")),
        timed_bounded_probe(
            "find_ui_elements:page_tab",
            provider.find_ui_elements("page tab")
        ),
        timed_bounded_probe("detect_visual_controls", provider.detect_visual_controls()),
        timed_optional_probe("focused_window_title", provider.focused_window_title()),
    );

    let mut probe_timings = vec![
        active_window_timing,
        desktop_state_timing,
        capabilities_timing,
        accessibility_tree_timing,
        screenshot_timing,
        ocr_timing,
        monitor_layout_timing,
        cursor_focus_timing,
        text_fields_timing,
        buttons_timing,
        dialogs_timing,
        check_boxes_timing,
        links_timing,
        tabs_timing,
        visual_controls_timing,
        focused_title_timed.1,
    ];

    let focused_title = focused_title_timed
        .0
        .map(|value| sanitize_gui_text(&value, 160).text)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            cursor_focus
                .data
                .get("focused_window")
                .or_else(|| cursor_focus.data.get("focused_window_label"))
                .and_then(serde_json::Value::as_str)
                .map(|value| sanitize_gui_text(value, 160).text)
                .filter(|value| !value.trim().is_empty())
        });
    let active_window = active_window_details(&active_window, &desktop_state, focused_title);
    let mut text_fields = controls_from_probe_result(&text_fields);
    let mut buttons = controls_from_probe_result(&buttons);
    let mut dialogs = controls_from_probe_result(&dialogs);
    let mut other_controls = Vec::new();
    other_controls.extend(controls_from_probe_result(&check_boxes));
    other_controls.extend(controls_from_probe_result(&links));
    other_controls.extend(controls_from_probe_result(&tabs));
    let ocr_blocks = ocr_blocks_from_probe_result(&ocr);
    let visual_controls = visual_controls_from_probe_result(&visual_controls);
    apply_control_fusion(
        &mut text_fields,
        &mut buttons,
        &mut dialogs,
        &mut other_controls,
        &visual_controls,
        &ocr_blocks,
    );
    let all_controls = text_fields
        .iter()
        .chain(buttons.iter())
        .chain(dialogs.iter())
        .chain(other_controls.iter())
        .cloned()
        .collect::<Vec<_>>();
    let ocr_diagnostics = ocr_diagnostics_from_probe_result(&ocr);
    let monitors = monitors_from_probe_result(&monitor_layout);
    let screen_hash = screenshot
        .data
        .get("screen_hash")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            Some(fallback_screen_hash(
                &active_window.label,
                &all_controls,
                &monitors,
            ))
        });

    let accessibility_ok = accessibility_available(&capabilities, &desktop_state);
    let accessibility = accessibility_summary(
        &accessibility_tree,
        accessibility_ok,
        &all_controls,
        desktop_app_count(&desktop_state),
    );

    let capabilities = GuiPerceptionCapabilities {
        active_window: status_from_probe(
            "active_window",
            &active_window.source,
            &active_window.blocker,
            active_window.label != "unknown",
        ),
        desktop_state: status_from_probe_result("desktop_state", &desktop_state),
        accessibility: accessibility_status(&capabilities, accessibility.available),
        screenshot: status_from_probe_result("screenshot", &screenshot),
        ocr: status_from_probe_result("ocr", &ocr),
        monitor: status_from_probe_result("monitor", &monitor_layout),
        cursor_focus: status_from_probe_result("cursor_focus", &cursor_focus),
    };

    let mut cursor_focus = cursor_focus_from_probe_result(&cursor_focus);
    if let Some(focused_control_id) = cursor_focus.focused_control_id.clone() {
        if let Some(focused_control) = all_controls
            .iter()
            .find(|control| control.control_id == focused_control_id)
        {
            if cursor_focus.focused_control_label.is_none() {
                cursor_focus.focused_control_label = Some(focused_control.name.clone());
            }
            if cursor_focus.focused_control_role.is_none() {
                cursor_focus.focused_control_role = Some(focused_control.role.clone());
            }
            if cursor_focus.focused_control_bounds.is_none() {
                cursor_focus.focused_control_bounds = focused_control.bounds.clone();
            }
            cursor_focus.editable_target_known |=
                focused_control.role.to_lowercase().contains("text");
            cursor_focus.text_cursor_known |= cursor_focus.editable_target_known;
            cursor_focus.confidence =
                cursor_focus
                    .confidence
                    .max(if cursor_focus.editable_target_known {
                        0.86
                    } else {
                        0.72
                    });
        }
    } else if let Some(focused_control) = all_controls.iter().find(|control| control.focused) {
        cursor_focus.focused_control_id = Some(focused_control.control_id.clone());
        cursor_focus.focused_control_label = Some(focused_control.name.clone());
        cursor_focus.focused_control_role = Some(focused_control.role.clone());
        cursor_focus.focused_control_bounds = focused_control.bounds.clone();
        cursor_focus.keyboard_focus_known = true;
        cursor_focus.editable_target_known = focused_control.role.to_lowercase().contains("text");
        cursor_focus.text_cursor_known = cursor_focus.editable_target_known;
        cursor_focus.confidence =
            cursor_focus
                .confidence
                .max(if cursor_focus.editable_target_known {
                    0.86
                } else {
                    0.72
                });
        if cursor_focus.reliability == "unavailable" {
            cursor_focus.reliability = if cursor_focus.confidence >= 0.8 {
                "reliable".into()
            } else {
                "best_effort".into()
            };
        }
        if cursor_focus.source == "unavailable" {
            cursor_focus.source = "accessibility_controls".into();
        }
    }

    let timing = observation_timing_summary(elapsed_ms(observation_started), &mut probe_timings);
    tracing::debug!(
        target: "gui_cognition_perception",
        total_ms = timing.total_ms,
        slowest_probe = timing.slowest_probe.as_deref().unwrap_or("none"),
        slowest_probe_ms = timing.slowest_probe_ms,
        timeout_count = timing.probe_timeout_count,
        "GUI Cognition observation completed"
    );

    let observation = GuiObservationSnapshot {
        observation_id,
        context_id,
        timestamp_ms: unix_now_ms(),
        screen_hash,
        active_window_label: active_window.label.clone(),
        active_window,
        visible_windows: visible_windows_from_desktop_state(&desktop_state, MAX_VISIBLE_WINDOWS),
        visible_app_count: desktop_app_count(&desktop_state),
        monitors,
        cursor_focus,
        accessibility,
        ocr_available: ocr.success,
        ocr_blocks,
        ocr_diagnostics,
        screenshot_available: screenshot.success,
        accessibility_ok,
        active_window_probe_ok: capabilities.active_window.available,
        desktop_state_probe_ok: desktop_state.success,
        capabilities_probe_ok: capabilities.accessibility.available,
        capabilities,
        text_fields,
        buttons,
        dialogs,
        other_controls,
        visual_controls,
        timing,
        cache: GuiObservationCacheSummary {
            cache_hit: false,
            cache_age_ms: None,
            cache_policy: provider.observation_cache_policy().into(),
            freshness: "fresh".into(),
        },
    };
    provider.store_observation_cache(&observation).await;
    observation
}

async fn timed_bounded_probe<F>(
    label: &'static str,
    future: F,
) -> (GuiProbeResult, GuiProbeTimingSummary)
where
    F: Future<Output = GuiProbeResult> + Send,
{
    let started = Instant::now();
    match tokio::time::timeout(Duration::from_millis(probe_timeout_ms(label)), future).await {
        Ok(result) => {
            let timing = timing_from_probe_result(label, elapsed_ms(started), &result);
            (result, timing)
        }
        Err(_) => {
            let result = GuiProbeResult::err(format!("{label} timed out"));
            let timing = GuiProbeTimingSummary {
                probe_name: label.into(),
                duration_ms: elapsed_ms(started),
                status: "timeout".into(),
                source: label.into(),
                cache_hit: false,
                blocker_kind: Some("timeout".into()),
            };
            (result, timing)
        }
    }
}

fn probe_timeout_ms(label: &str) -> u64 {
    match label {
        "run_ocr" => 4_000,
        "capture_screenshot" => 1_850,
        _ => GUI_PROBE_TIMEOUT_MS,
    }
}

async fn timed_optional_probe<F>(
    label: &'static str,
    future: F,
) -> (Option<String>, GuiProbeTimingSummary)
where
    F: Future<Output = Option<String>> + Send,
{
    let started = Instant::now();
    match tokio::time::timeout(Duration::from_millis(GUI_PROBE_TIMEOUT_MS), future).await {
        Ok(value) => {
            let status = if value.as_ref().is_some_and(|text| !text.trim().is_empty()) {
                "ok"
            } else {
                "blocked"
            };
            (
                value,
                GuiProbeTimingSummary {
                    probe_name: label.into(),
                    duration_ms: elapsed_ms(started),
                    status: status.into(),
                    source: label.into(),
                    cache_hit: false,
                    blocker_kind: if status == "ok" {
                        None
                    } else {
                        Some("unavailable".into())
                    },
                },
            )
        }
        Err(_) => (
            None,
            GuiProbeTimingSummary {
                probe_name: label.into(),
                duration_ms: elapsed_ms(started),
                status: "timeout".into(),
                source: label.into(),
                cache_hit: false,
                blocker_kind: Some("timeout".into()),
            },
        ),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn timing_from_probe_result(
    label: &str,
    duration_ms: u64,
    result: &GuiProbeResult,
) -> GuiProbeTimingSummary {
    let status = if result.success {
        "ok"
    } else if result
        .error
        .as_deref()
        .is_some_and(|error| error.to_ascii_lowercase().contains("timed out"))
    {
        "timeout"
    } else if result.error.as_deref().is_some_and(is_blocked_probe_error) {
        "blocked"
    } else {
        "error"
    };
    GuiProbeTimingSummary {
        probe_name: label.into(),
        duration_ms,
        status: status.into(),
        source: first_string(&result.data, &["source"]).unwrap_or_else(|| label.into()),
        cache_hit: false,
        blocker_kind: if status == "ok" {
            None
        } else {
            Some(status.into())
        },
    }
}

fn is_blocked_probe_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "unavailable",
        "not implemented",
        "denied",
        "blocked",
        "not probed",
    ]
    .iter()
    .any(|needle| error.contains(needle))
}

fn observation_timing_summary(
    total_ms: u64,
    probe_timings: &mut Vec<GuiProbeTimingSummary>,
) -> GuiObservationTimingSummary {
    probe_timings.sort_by(|a, b| a.probe_name.cmp(&b.probe_name));
    let slowest = probe_timings
        .iter()
        .max_by_key(|timing| timing.duration_ms)
        .cloned();
    GuiObservationTimingSummary {
        total_ms,
        slowest_probe: slowest.as_ref().map(|timing| timing.probe_name.clone()),
        slowest_probe_ms: slowest
            .as_ref()
            .map(|timing| timing.duration_ms)
            .unwrap_or(0),
        probe_timeout_count: probe_timings
            .iter()
            .filter(|timing| timing.status == "timeout")
            .count(),
        probe_timings: probe_timings.clone(),
    }
}

pub async fn capture_monitor_layout_probe() -> GuiProbeResult {
    let result = tokio::task::spawn_blocking(|| {
        let monitors = xcap::Monitor::all().map_err(|err| err.to_string())?;
        let monitors = monitors
            .iter()
            .map(|monitor| {
                serde_json::json!({
                    "id": monitor.id().to_string(),
                    "name": monitor.name(),
                    "x": monitor.x(),
                    "y": monitor.y(),
                    "width": monitor.width(),
                    "height": monitor.height(),
                    "scale_factor": monitor.scale_factor(),
                    "primary": monitor.is_primary(),
                })
            })
            .collect::<Vec<_>>();
        Ok::<serde_json::Value, String>(serde_json::json!({ "monitors": monitors }))
    })
    .await
    .map_err(|err| err.to_string())
    .and_then(|inner| inner);

    match result {
        Ok(data) => GuiProbeResult::ok(data),
        Err(error) => GuiProbeResult::err(format!("monitor layout unavailable: {error}")),
    }
}

pub fn active_window_summary(
    active_window: &GuiProbeResult,
    desktop_state: &GuiProbeResult,
) -> String {
    active_window_details(active_window, desktop_state, None).label
}

pub fn accessibility_available(
    capabilities: &GuiProbeResult,
    desktop_state: &GuiProbeResult,
) -> bool {
    capabilities
        .data
        .get("atspi_bus_available")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            capabilities
                .data
                .get("accessibility_operational")
                .and_then(serde_json::Value::as_bool)
        })
        .or_else(|| {
            desktop_state
                .data
                .get("accessibility_operational")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

pub fn desktop_app_count(desktop_state: &GuiProbeResult) -> usize {
    desktop_state
        .data
        .get("applications")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

pub fn controls_from_probe_result(result: &GuiProbeResult) -> Vec<GuiControlSummary> {
    result
        .data
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .map(|elements| {
            elements
                .iter()
                .take(200)
                .filter_map(control_from_element)
                .collect()
        })
        .unwrap_or_default()
}

pub fn control_sample(controls: &[GuiControlSummary], limit: usize) -> Vec<String> {
    controls
        .iter()
        .filter_map(|control| {
            if control.name.trim().is_empty() {
                None
            } else {
                Some(control.name.clone())
            }
        })
        .take(limit)
        .collect()
}

pub fn matching_controls(controls: &[GuiControlSummary], name: &str) -> Vec<GuiControlSummary> {
    let target = name.trim().to_lowercase();
    controls
        .iter()
        .filter(|control| {
            control.is_executable_candidate() && control.name.to_lowercase().contains(&target)
        })
        .cloned()
        .collect()
}

fn visual_controls_from_probe_result(result: &GuiProbeResult) -> Vec<GuiVisualControlDetection> {
    if !result.success {
        return Vec::new();
    }
    result
        .data
        .get("elements")
        .or_else(|| result.data.get("visual_controls"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(80)
                .filter_map(|item| {
                    let control_type = item
                        .get("control_type")
                        .or_else(|| item.get("element_type"))
                        .or_else(|| item.get("type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .trim()
                        .to_ascii_lowercase();
                    let raw_label = item
                        .get("label")
                        .or_else(|| item.get("label_preview"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let label = sanitize_gui_text(raw_label, 120).text;
                    let bounds =
                        parse_bounds(item.get("bounds").or_else(|| item.get("bbox")), true);
                    let confidence = item
                        .get("confidence")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.5)
                        .clamp(0.0, 1.0);
                    let source = item
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("vision_sidecar");
                    Some(GuiVisualControlDetection {
                        visual_id: item
                            .get("id")
                            .or_else(|| item.get("visual_id"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                short_hash(&format!("{control_type}|{label}|{:?}", bounds))
                            }),
                        control_type,
                        label_preview_safe: label,
                        bounds,
                        confidence,
                        source: sanitize_gui_text(source, 80).text,
                        matched_control_id: item
                            .get("matched_control_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn apply_control_fusion(
    text_fields: &mut Vec<GuiControlSummary>,
    buttons: &mut Vec<GuiControlSummary>,
    dialogs: &mut Vec<GuiControlSummary>,
    other_controls: &mut Vec<GuiControlSummary>,
    visual_controls: &[GuiVisualControlDetection],
    ocr_blocks: &[GuiOcrBlock],
) {
    let mut seen_visual_ids = std::collections::HashSet::new();
    for control in text_fields
        .iter_mut()
        .chain(buttons.iter_mut())
        .chain(dialogs.iter_mut())
        .chain(other_controls.iter_mut())
    {
        if let Some(visual) = visual_controls
            .iter()
            .find(|visual| visual_supports_control(visual, control))
        {
            seen_visual_ids.insert(visual.visual_id.clone());
            merge_visual_evidence(control, visual);
        }
        let ocr_support_count = ocr_blocks
            .iter()
            .filter(|block| ocr_supports_control(block, control))
            .count();
        if ocr_support_count > 0 {
            push_unique_source(&mut control.sources, "ocr_layout");
            control.evidence = format!(
                "{}; {ocr_support_count} OCR block(s) matched as untrusted supporting evidence",
                control.evidence
            );
            control.identity_confidence = (control.identity_confidence + 0.03).min(0.98);
        }
        refresh_control_quality(control);
    }

    for visual in visual_controls {
        if seen_visual_ids.contains(&visual.visual_id) {
            continue;
        }
        let control = visual_only_control(visual);
        match visual.control_type.as_str() {
            "button" => buttons.push(control),
            "input" | "text" | "text_field" => text_fields.push(control),
            "dialog" => dialogs.push(control),
            _ => other_controls.push(control),
        }
    }
}

fn visual_supports_control(
    visual: &GuiVisualControlDetection,
    control: &GuiControlSummary,
) -> bool {
    if let Some(matched) = visual.matched_control_id.as_deref() {
        if matched == control.control_id {
            return true;
        }
    }
    if !visual_type_compatible_with_role(&visual.control_type, &control.role) {
        return false;
    }
    if !visual.label_preview_safe.trim().is_empty()
        && !control.name.trim().is_empty()
        && (visual
            .label_preview_safe
            .to_lowercase()
            .contains(&control.name.to_lowercase())
            || control
                .name
                .to_lowercase()
                .contains(&visual.label_preview_safe.to_lowercase()))
    {
        return true;
    }
    match (&visual.bounds, &control.bounds) {
        (Some(a), Some(b)) => bounds_overlap(a, b),
        _ => false,
    }
}

fn visual_type_compatible_with_role(control_type: &str, role: &str) -> bool {
    let control_type = control_type.to_ascii_lowercase();
    let role = role.to_ascii_lowercase();
    match control_type.as_str() {
        "button" => role.contains("button"),
        "toggle" => role.contains("toggle") || role.contains("check"),
        "input" | "text" | "text_field" => {
            role.contains("text") || role.contains("entry") || role.contains("input")
        }
        "link" => role.contains("link"),
        "tab" => role.contains("tab"),
        "menu" => role.contains("menu"),
        "dialog" => role.contains("dialog"),
        _ => true,
    }
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

fn merge_visual_evidence(control: &mut GuiControlSummary, visual: &GuiVisualControlDetection) {
    push_unique_source(&mut control.sources, &visual.source);
    if control.bounds.is_none() {
        control.bounds = visual.bounds.clone();
        control.bounds_confidence = visual.confidence.min(0.78);
    } else if control.bounds_confidence < 0.75 {
        control.bounds_confidence = control.bounds_confidence.max(visual.confidence.min(0.80));
    }
    if control.name.trim().is_empty() && !visual.label_preview_safe.trim().is_empty() {
        control.name = visual.label_preview_safe.clone();
        control.label_source = "visual_label_untrusted".into();
        control.identity_confidence = control.identity_confidence.max(visual.confidence.min(0.72));
    }
    control.confidence = control.confidence.max(visual.confidence.min(0.90));
    control.evidence = format!(
        "{}; visual {} evidence matched with confidence {:.0}%",
        control.evidence,
        visual.control_type,
        visual.confidence * 100.0
    );
}

fn visual_only_control(visual: &GuiVisualControlDetection) -> GuiControlSummary {
    let role = match visual.control_type.as_str() {
        "button" => "push button",
        "toggle" => "toggle button",
        "input" | "text_field" => "text",
        "link" => "link",
        "tab" => "page tab",
        other => other,
    };
    let mut control = GuiControlSummary::new(
        role,
        visual.label_preview_safe.clone(),
        format!("/visual/{}", visual.visual_id),
    );
    control.control_id = stable_hash(&format!(
        "visual|{}|{}",
        visual.control_type, visual.visual_id
    ));
    control.bounds = visual.bounds.clone();
    control.enabled = true;
    control.visible = visual.bounds.is_some();
    control.source = visual.source.clone();
    control.confidence = visual.confidence;
    control.identity_confidence = if visual.label_preview_safe.trim().is_empty() {
        0.35
    } else {
        visual.confidence.min(0.74)
    };
    control.bounds_confidence = if visual.bounds.is_some() {
        visual.confidence.min(0.72)
    } else {
        0.0
    };
    control.state_confidence = 0.45;
    control.executable_confidence = 0.0;
    control.quality = "not_executable".into();
    control.label_source = "visual_label_untrusted".into();
    control.state_source = "visual_inferred".into();
    control.rejection_reason = Some("visual_only_supporting_evidence".into());
    control.sources = vec![visual.source.clone()];
    control.evidence =
        "visual detector supporting evidence; not executable without trusted state".into();
    control
}

fn refresh_control_quality(control: &mut GuiControlSummary) {
    control.executable_confidence = executable_confidence_for(
        &control.source,
        control.enabled,
        control.visible,
        control.identity_confidence,
        control.bounds_confidence,
        control.state_confidence,
    );
    control.quality = control_quality(
        &control.source,
        control.enabled,
        control.visible,
        control.identity_confidence,
        control.bounds_confidence,
        control.state_confidence,
        control.executable_confidence,
    );
    control.rejection_reason = control_rejection_reason(
        &control.source,
        control.enabled,
        control.visible,
        control.bounds.is_some(),
        !control.name.trim().is_empty(),
    );
}

fn push_unique_source(sources: &mut Vec<String>, source: &str) {
    if !sources.iter().any(|value| value == source) {
        sources.push(source.to_string());
    }
}

pub fn sanitize_gui_text(value: &str, limit: usize) -> SanitizedText {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let injection_suspected = INJECTION_PATTERN.is_match(&compact);
    let after_secret = SECRET_PATTERN.replace_all(&compact, "$1[redacted]");
    let after_card = CARD_PATTERN.replace_all(&after_secret, "[redacted-card]");
    let mut redaction_applied = after_card != compact;
    let mut text = if injection_suspected {
        redaction_applied = true;
        "[untrusted text redacted]".to_string()
    } else {
        after_card.to_string()
    };
    if text.chars().count() > limit {
        text = text
            .chars()
            .take(limit.saturating_sub(1))
            .collect::<String>();
        text.push('…');
    }
    SanitizedText {
        text,
        redaction_applied,
        injection_suspected,
    }
}

pub fn stable_hash(value: &str) -> String {
    blake3::hash(value.as_bytes()).to_hex().to_string()
}

pub fn short_hash(value: &str) -> String {
    stable_hash(value).chars().take(16).collect()
}

fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn active_window_details(
    active_window: &GuiProbeResult,
    desktop_state: &GuiProbeResult,
    focused_title: Option<String>,
) -> GuiActiveWindowSummary {
    let active_source = first_string(&active_window.data, &["source"])
        .unwrap_or_else(|| "get_active_window".into());
    let active_reliability = first_string(&active_window.data, &["reliability"])
        .unwrap_or_else(|| active_window_reliability_for_source(&active_source).into());
    let active_confidence = active_window
        .data
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| active_window_confidence_for_data(&active_source, &active_window.data))
        .clamp(0.0, 1.0);
    let bridge_stale_reason =
        active_window_bridge_stale_reason(&active_source, &active_window.data);

    let candidates = vec![
        (
            active_source.as_str(),
            active_confidence,
            false,
            active_reliability.as_str(),
            if bridge_stale_reason.is_some() {
                None
            } else {
                first_string(
                    &active_window.data,
                    &["title", "window_title", "active_window"],
                )
            },
            bridge_stale_reason
                .clone()
                .or_else(|| active_window.error.clone()),
        ),
        (
            "desktop_state.focused_window",
            0.82,
            true,
            "best_effort",
            first_string(&desktop_state.data, &["focused_window"]),
            desktop_state.error.clone(),
        ),
        (
            "atspi.focused_window_title",
            0.72,
            true,
            "reliable",
            focused_title.clone(),
            None,
        ),
        (
            "atspi.focused_app",
            0.64,
            true,
            "best_effort",
            first_string(
                &desktop_state.data,
                &["focused_app", "focused_app_label", "focused_app_name"],
            ),
            desktop_state.error.clone(),
        ),
        (
            "desktop_state.single_application",
            0.42,
            true,
            "best_effort",
            single_application(&desktop_state.data),
            desktop_state.error.clone(),
        ),
    ];

    let mut fallback_chain = Vec::new();
    for (source, confidence, fallback_used, reliability, candidate, source_error) in candidates {
        let sanitized_candidate = candidate
            .map(|value| sanitize_gui_text(&value, 160).text)
            .filter(|value| !value.trim().is_empty() && value != "unknown");
        if let Some(label) = sanitized_candidate {
            fallback_chain.push(GuiSourceAttemptSummary {
                source: source.into(),
                status: "matched".into(),
                reliability: reliability.into(),
                reason: None,
            });
            return GuiActiveWindowSummary {
                app_name: first_string(&active_window.data, &["app", "app_name", "class"])
                    .or_else(|| {
                        first_string(
                            &desktop_state.data,
                            &["focused_app", "focused_app_label", "focused_window"],
                        )
                    })
                    .map(|value| sanitize_gui_text(&value, 120).text)
                    .or_else(|| Some(label.clone())),
                app_id: first_string(&active_window.data, &["app_id", "class", "wm_class"])
                    .map(|value| sanitize_gui_text(&value, 120).text),
                pid: first_u64(&active_window.data, &["pid"])
                    .and_then(|value| u32::try_from(value).ok()),
                workspace: first_i64(&active_window.data, &["workspace", "workspace_index"]),
                monitor: first_i64(&active_window.data, &["monitor", "monitor_index"]),
                fullscreen: first_bool(&active_window.data, &["fullscreen", "is_fullscreen"]),
                minimized: first_bool(&active_window.data, &["minimized", "is_minimized"]),
                observed_at_ms: first_i64(&active_window.data, &["observed_at_ms"]),
                label,
                source: source.into(),
                confidence,
                fallback_used,
                blocker: None,
                reliability: reliability.into(),
                authority_status: "available".into(),
                gnome_bridge_status: gnome_bridge_status_from_probe(active_window, source),
                fallback_chain,
            };
        }
        fallback_chain.push(GuiSourceAttemptSummary {
            source: source.into(),
            status: "missing".into(),
            reliability: reliability.into(),
            reason: source_error
                .map(|value| sanitize_gui_text(&value, 220).text)
                .or_else(|| Some("source did not expose a focused window title".into())),
        });
    }

    let blocker = active_window
        .error
        .as_ref()
        .or(desktop_state.error.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            "GNOME/Wayland did not expose focused window through compositor, AT-SPI, or desktop-state fallback.".into()
        });
    GuiActiveWindowSummary {
        blocker: Some(sanitize_gui_text(&blocker, 240).text),
        authority_status: "unavailable".into(),
        gnome_bridge_status: gnome_bridge_status_from_probe(active_window, "unavailable"),
        fallback_chain,
        ..GuiActiveWindowSummary::default()
    }
}

fn active_window_confidence_for_source(source: &str) -> f64 {
    match source {
        "kria_gnome_shell_bridge" => 0.98,
        "gnome_shell_focus_window" => 0.94,
        "hyprctl_activewindow" | "swaymsg_focused_node" => 0.95,
        "get_active_window" => 0.95,
        "gui_cognition_test_fixture" => 0.95,
        "atspi.focused_window_title" => 0.72,
        "atspi.focused_app" => 0.64,
        "desktop_state.single_application" => 0.42,
        _ => 0.0,
    }
}

fn active_window_confidence_for_data(source: &str, value: &serde_json::Value) -> f64 {
    if source == "kria_gnome_shell_bridge" && first_u64(value, &["pid"]).is_none() {
        return 0.94;
    }
    active_window_confidence_for_source(source)
}

fn active_window_reliability_for_source(source: &str) -> &'static str {
    match source {
        "kria_gnome_shell_bridge"
        | "gnome_shell_focus_window"
        | "hyprctl_activewindow"
        | "swaymsg_focused_node"
        | "get_active_window"
        | "gui_cognition_test_fixture"
        | "atspi.focused_window_title" => "reliable",
        "atspi.focused_app" | "desktop_state.single_application" => "best_effort",
        _ => "unavailable",
    }
}

fn active_window_bridge_stale_reason(source: &str, value: &serde_json::Value) -> Option<String> {
    if source != "kria_gnome_shell_bridge" {
        return None;
    }
    const MAX_BRIDGE_AGE_MS: i64 = 5_000;
    let observed_at_ms = first_i64(value, &["observed_at_ms"])?;
    let age_ms = unix_now_ms().saturating_sub(observed_at_ms);
    (age_ms > MAX_BRIDGE_AGE_MS)
        .then(|| format!("GNOME bridge active-window observation is stale ({age_ms}ms old)"))
}

fn gnome_bridge_status_from_probe(active_window: &GuiProbeResult, matched_source: &str) -> String {
    if matched_source == "kria_gnome_shell_bridge" {
        return "available".into();
    }
    first_string(&active_window.data, &["gnome_bridge_status"]).unwrap_or_else(|| {
        if active_window
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("gnome_bridge_unavailable")
        {
            "missing".into()
        } else {
            "unknown".into()
        }
    })
}

fn control_from_element(element: &serde_json::Value) -> Option<GuiControlSummary> {
    let role = element.get("role")?.as_str()?.trim().to_string();
    let (raw_name, label_source) = control_label_candidate(element);
    let name = sanitize_gui_text(&raw_name, MAX_CONTROL_SUMMARY).text;
    let path = element
        .get("path")
        .or_else(|| element.get("selector"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let source = element
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("accessibility")
        .to_string();
    let control_id = element
        .get("control_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| stable_hash(&format!("{role}|{name}|{path}|{source}")));
    let enabled = element
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let visible = element
        .get("visible")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let bounds = parse_bounds(element.get("bounds").or_else(|| element.get("bbox")), false);
    let state_source = element
        .get("state_source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("accessibility_state")
        .to_string();
    let confidence = element
        .get("confidence")
        .or_else(|| element.get("score"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.72)
        .clamp(0.0, 1.0);
    let has_label = !name.is_empty();
    let identity_confidence = element
        .get("identity_confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| identity_confidence_for(&source, has_label, confidence))
        .clamp(0.0, 1.0);
    let bounds_confidence = element
        .get("bounds_confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| if bounds.is_some() { 0.86 } else { 0.0 })
        .clamp(0.0, 1.0);
    let state_confidence = element
        .get("state_confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| state_confidence_for(&source, enabled, visible))
        .clamp(0.0, 1.0);
    let executable_confidence = executable_confidence_for(
        &source,
        enabled,
        visible,
        identity_confidence,
        bounds_confidence,
        state_confidence,
    );
    let quality = control_quality(
        &source,
        enabled,
        visible,
        identity_confidence,
        bounds_confidence,
        state_confidence,
        executable_confidence,
    );
    let rejection_reason =
        control_rejection_reason(&source, enabled, visible, bounds.is_some(), has_label);
    let sources = element
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|value| sanitize_gui_text(value, 80).text)
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec![source.clone()]);
    Some(GuiControlSummary {
        control_id,
        role,
        name,
        path,
        bounds: bounds.clone(),
        enabled,
        focused: element
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        visible,
        in_active_window: element
            .get("in_active_window")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        quality,
        source,
        confidence,
        evidence: format!(
            "accessibility state summary; label_source={label_source}; state_source={state_source}"
        ),
        label_source,
        state_source,
        rejection_reason,
        identity_confidence,
        bounds_confidence,
        state_confidence,
        executable_confidence,
        sources,
    })
}

fn control_label_candidate(element: &serde_json::Value) -> (String, String) {
    for (field, source) in [
        ("name", "accessible_name"),
        ("label", "accessible_label"),
        ("labelled_by", "labelled_by"),
        ("placeholder", "placeholder"),
        ("value", "safe_value"),
        ("description", "description"),
    ] {
        if let Some(value) = element
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return (value.to_string(), source.into());
        }
    }
    ("".into(), "missing".into())
}

fn identity_confidence_for(source: &str, has_label: bool, base_confidence: f64) -> f64 {
    if !has_label {
        return 0.35;
    }
    if source.eq_ignore_ascii_case("accessibility") {
        base_confidence.max(0.82)
    } else if source.eq_ignore_ascii_case("browser_dom")
        || source.eq_ignore_ascii_case("vscode_adapter")
    {
        base_confidence.max(0.86)
    } else {
        base_confidence.min(0.74)
    }
}

fn state_confidence_for(source: &str, enabled: bool, visible: bool) -> f64 {
    if source.eq_ignore_ascii_case("accessibility")
        || source.eq_ignore_ascii_case("browser_dom")
        || source.eq_ignore_ascii_case("vscode_adapter")
    {
        if enabled && visible {
            0.86
        } else {
            0.9
        }
    } else {
        0.45
    }
}

fn executable_confidence_for(
    source: &str,
    enabled: bool,
    visible: bool,
    identity_confidence: f64,
    bounds_confidence: f64,
    state_confidence: f64,
) -> f64 {
    if !enabled || !visible {
        return 0.0;
    }
    if !(source.eq_ignore_ascii_case("accessibility")
        || source.eq_ignore_ascii_case("browser_dom")
        || source.eq_ignore_ascii_case("vscode_adapter"))
    {
        return 0.0;
    }
    identity_confidence
        .min(bounds_confidence)
        .min(state_confidence)
        .clamp(0.0, 1.0)
}

fn control_quality(
    source: &str,
    enabled: bool,
    visible: bool,
    identity_confidence: f64,
    bounds_confidence: f64,
    state_confidence: f64,
    executable_confidence: f64,
) -> String {
    if !enabled || !visible {
        return "not_executable".into();
    }
    if executable_confidence >= 0.75
        && identity_confidence >= 0.80
        && bounds_confidence >= 0.75
        && state_confidence >= 0.80
        && (source.eq_ignore_ascii_case("accessibility")
            || source.eq_ignore_ascii_case("browser_dom")
            || source.eq_ignore_ascii_case("vscode_adapter"))
    {
        "trusted".into()
    } else if visible && identity_confidence >= 0.55 {
        "partial".into()
    } else {
        "not_executable".into()
    }
}

fn control_rejection_reason(
    source: &str,
    enabled: bool,
    visible: bool,
    has_bounds: bool,
    has_label: bool,
) -> Option<String> {
    if !source.eq_ignore_ascii_case("accessibility") {
        return Some("source_not_executable".into());
    }
    if !visible {
        return Some("hidden".into());
    }
    if !enabled {
        return Some("disabled".into());
    }
    if !has_bounds {
        return Some("bounds_missing".into());
    }
    if !has_label {
        return Some("label_missing".into());
    }
    None
}

fn ocr_blocks_from_probe_result(result: &GuiProbeResult) -> Vec<GuiOcrBlock> {
    if !result.success {
        return Vec::new();
    }

    if let Some(blocks) = result
        .data
        .get("blocks")
        .and_then(serde_json::Value::as_array)
    {
        return blocks
            .iter()
            .take(MAX_OCR_BLOCKS)
            .filter_map(|block| {
                let text = block
                    .get("text")
                    .or_else(|| block.get("label"))
                    .and_then(serde_json::Value::as_str)?;
                Some(ocr_block_from_text(
                    text,
                    parse_bounds(block.get("bounds").or_else(|| block.get("bbox")), true),
                    block
                        .get("confidence")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.5),
                ))
            })
            .collect();
    }

    result
        .data
        .get("text")
        .or_else(|| result.data.get("ocr_text"))
        .and_then(serde_json::Value::as_str)
        .map(|text| vec![ocr_block_from_text(text, None, 0.5)])
        .unwrap_or_default()
}

fn ocr_diagnostics_from_probe_result(result: &GuiProbeResult) -> GuiOcrDiagnostics {
    GuiOcrDiagnostics {
        wait_for_screenshot_ms: json_u64(result.data.get("ocr_wait_for_screenshot_ms")),
        engine_selected: first_string(
            &result.data,
            &["ocr_engine_selected", "ocr_engine", "source"],
        )
        .map(|value| sanitize_gui_text(&value, 80).text),
        engine_status: first_string(&result.data, &["ocr_engine_status"])
            .map(|value| sanitize_gui_text(&value, 120).text)
            .or_else(|| {
                if result.success {
                    Some("completed".into())
                } else {
                    Some("unavailable".into())
                }
            }),
        image_status: first_string(&result.data, &["ocr_image_status"])
            .map(|value| sanitize_gui_text(&value, 120).text),
        total_ms: json_u64(result.data.get("ocr_total_ms")),
        fast_path: first_string(&result.data, &["ocr_fast_path", "fast_path"])
            .map(|value| sanitize_gui_text(&value, 80).text),
        cache_hit: result
            .data
            .get("ocr_cache_hit")
            .or_else(|| result.data.get("cache_hit"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        roi_count: result
            .data
            .get("ocr_roi_count")
            .or_else(|| result.data.get("roi_count"))
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0),
        changed_region_count: result
            .data
            .get("ocr_changed_region_count")
            .or_else(|| result.data.get("changed_region_count"))
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0),
        cold_start_ms: json_u64(result.data.get("ocr_cold_start_ms")),
        warm_start_ms: json_u64(result.data.get("ocr_warm_start_ms")),
        benchmark_summary: first_string(
            &result.data,
            &["ocr_benchmark_summary", "benchmark_summary"],
        )
        .map(|value| sanitize_gui_text(&value, 160).text),
    }
}

fn ocr_block_from_text(text: &str, bounds: Option<GuiBounds>, confidence: f64) -> GuiOcrBlock {
    let sanitized = sanitize_gui_text(text, 120);
    GuiOcrBlock {
        block_id: short_hash(text),
        safe_text_preview: sanitized.text,
        text_hash: stable_hash(text),
        bounds,
        confidence: confidence.clamp(0.0, 1.0),
        untrusted: true,
        injection_suspected: sanitized.injection_suspected,
        redaction_applied: sanitized.redaction_applied,
    }
}

fn monitors_from_probe_result(result: &GuiProbeResult) -> Vec<GuiMonitorSummary> {
    result
        .data
        .get("monitors")
        .and_then(serde_json::Value::as_array)
        .map(|monitors| {
            monitors
                .iter()
                .filter_map(|monitor| {
                    let x = json_i32(monitor.get("x"))?;
                    let y = json_i32(monitor.get("y"))?;
                    let width = json_i32(monitor.get("width"))?;
                    let height = json_i32(monitor.get("height"))?;
                    Some(GuiMonitorSummary {
                        id: monitor
                            .get("id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("{x}:{y}:{width}:{height}")),
                        name: monitor
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .map(|value| sanitize_gui_text(value, 80).text)
                            .filter(|value| !value.is_empty()),
                        bounds: GuiBounds {
                            x,
                            y,
                            width,
                            height,
                        },
                        work_area: parse_bounds(monitor.get("work_area"), false),
                        scale_factor: monitor
                            .get("scale_factor")
                            .or_else(|| monitor.get("dpi_scale"))
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(1.0),
                        primary: monitor
                            .get("primary")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cursor_focus_from_probe_result(result: &GuiProbeResult) -> GuiCursorFocusSummary {
    if !result.success && result.data.is_null() {
        let mut summary = GuiCursorFocusSummary::default();
        if let Some(error) = result.error.as_deref() {
            summary.failure_chain.push(GuiSourceAttemptSummary {
                source: "focus_authority".into(),
                status: "missing".into(),
                reliability: "unavailable".into(),
                reason: Some(sanitize_gui_text(error, 220).text),
            });
        }
        return summary;
    }

    let cursor = result
        .data
        .get("cursor")
        .unwrap_or(&serde_json::Value::Null);
    let source = result
        .data
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("desktop")
        .to_string();
    let focused_control_label = first_string(
        &result.data,
        &[
            "focused_control_label",
            "focused_label",
            "focused_element_label",
        ],
    )
    .map(|value| sanitize_gui_text(&value, 160).text);
    let focused_control_role = first_string(
        &result.data,
        &[
            "focused_control_role",
            "focused_role",
            "focused_element_role",
        ],
    )
    .map(|value| sanitize_gui_text(&value, 80).text);
    let keyboard_focus_known = result
        .data
        .get("keyboard_focus_known")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let editable_target_known = result
        .data
        .get("editable_target_known")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| {
            focused_control_role
                .as_deref()
                .is_some_and(|role| role.to_ascii_lowercase().contains("text"))
        });
    let confidence = result
        .data
        .get("focus_confidence")
        .or_else(|| result.data.get("confidence"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| {
            if editable_target_known {
                0.86
            } else if keyboard_focus_known {
                0.72
            } else {
                0.0
            }
        })
        .clamp(0.0, 1.0);
    let reliability = result
        .data
        .get("focus_reliability")
        .or_else(|| result.data.get("reliability"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if confidence >= 0.8 {
                "reliable".into()
            } else if confidence > 0.0 {
                "best_effort".into()
            } else {
                "unavailable".into()
            }
        });
    let failure_chain = result
        .data
        .get("focus_failure_chain")
        .or_else(|| result.data.get("failure_chain"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(GuiSourceAttemptSummary {
                        source: first_string(item, &["source"])?,
                        status: first_string(item, &["status"]).unwrap_or_else(|| "missing".into()),
                        reliability: first_string(item, &["reliability"])
                            .unwrap_or_else(|| "unavailable".into()),
                        reason: first_string(item, &["reason"])
                            .map(|value| sanitize_gui_text(&value, 220).text),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            let reason = if !keyboard_focus_known {
                result
                    .error
                    .as_deref()
                    .map(|error| sanitize_gui_text(error, 220).text)
                    .or_else(|| Some("focus source did not expose keyboard focus".into()))
            } else {
                None
            };
            vec![GuiSourceAttemptSummary {
                source: source.clone(),
                status: if keyboard_focus_known {
                    "matched"
                } else {
                    "missing"
                }
                .into(),
                reliability: reliability.clone(),
                reason,
            }]
        });
    GuiCursorFocusSummary {
        cursor_x: json_i32(cursor.get("x")),
        cursor_y: json_i32(cursor.get("y")),
        focused_control_id: result
            .data
            .get("focused_control_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        focused_window_label: result
            .data
            .get("focused_window")
            .and_then(serde_json::Value::as_str)
            .map(|value| sanitize_gui_text(value, 160).text),
        keyboard_focus_known,
        source,
        focused_app: first_string(&result.data, &["focused_app", "focused_application"])
            .map(|value| sanitize_gui_text(&value, 120).text),
        focused_control_label,
        focused_control_role,
        focused_control_bounds: parse_bounds(
            result
                .data
                .get("focused_control_bounds")
                .or_else(|| result.data.get("focused_bounds"))
                .or_else(|| result.data.get("bounds")),
            false,
        ),
        text_cursor_known: result
            .data
            .get("text_cursor_known")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(editable_target_known),
        editable_target_known,
        terminal_like: result
            .data
            .get("terminal_like")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        confidence,
        reliability,
        adapter_status: first_string(&result.data, &["adapter_status", "focus_adapter_status"])
            .map(|value| sanitize_gui_text(&value, 120).text),
        latency_ms: result
            .data
            .get("latency_ms")
            .or_else(|| result.data.get("focus_latency_ms"))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
            }),
        failure_chain,
    }
}

fn accessibility_summary(
    tree_result: &GuiProbeResult,
    available: bool,
    controls: &[GuiControlSummary],
    app_count: usize,
) -> GuiAccessibilitySummary {
    let control_count = controls.len();
    let skipped_app_count = tree_result
        .data
        .get("atspi_skipped_app_count")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);
    let omitted_node_count = tree_result
        .data
        .get("omitted_node_count")
        .or_else(|| tree_result.data.get("atspi_omitted_node_count"))
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);
    let timeout_count = tree_result
        .data
        .get("atspi_timeout_count")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or_else(|| {
            usize::from(
                tree_result
                    .data
                    .get("atspi_timeout_reason")
                    .is_some_and(|value| !value.is_null()),
            )
        });
    let stale_node_count = tree_result
        .data
        .get("atspi_stale_node_count")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or_else(|| {
            controls
                .iter()
                .filter(|control| control.path.is_empty())
                .count()
        });
    let executable_count = controls
        .iter()
        .filter(|control| control.is_executable_candidate())
        .count();
    let overall_confidence = tree_result
        .data
        .get("accessibility_overall_confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_else(|| {
            accessibility_confidence(
                available,
                control_count,
                executable_count,
                skipped_app_count,
                omitted_node_count,
                timeout_count,
                stale_node_count,
            )
        })
        .clamp(0.0, 1.0);
    let overall_status = tree_result
        .data
        .get("accessibility_health_status")
        .or_else(|| tree_result.data.get("accessibility_overall_status"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            accessibility_status_from_confidence(available, overall_confidence).into()
        });
    GuiAccessibilitySummary {
        available,
        node_count: tree_result
            .data
            .get("node_count")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(control_count.max(app_count)),
        control_count,
        omitted_node_count,
        enabled_control_count: controls.iter().filter(|control| control.enabled).count(),
        disabled_control_count: controls.iter().filter(|control| !control.enabled).count(),
        visible_control_count: controls.iter().filter(|control| control.visible).count(),
        focused_control_count: controls.iter().filter(|control| control.focused).count(),
        source: if tree_result.success {
            tree_result
                .data
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("accessibility_tree_summary")
                .to_string()
        } else if available {
            "accessibility_controls".into()
        } else {
            "unavailable".into()
        },
        source_status: tree_result
            .data
            .get("accessibility_source_status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(if available { "healthy" } else { "unavailable" })
            .to_string(),
        snapshot_total_ms: tree_result
            .data
            .get("atspi_snapshot_total_ms")
            .and_then(serde_json::Value::as_u64),
        skipped_app_count,
        remediation: tree_result
            .data
            .get("accessibility_remediation")
            .or_else(|| tree_result.data.get("remediation"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(|value| sanitize_gui_text(value, 180).text)
                    .filter(|value| !value.trim().is_empty())
                    .take(4)
                    .collect()
            })
            .unwrap_or_default(),
        overall_status,
        overall_confidence,
        app_scores: accessibility_app_scores_from(tree_result, controls, overall_confidence),
        stale_node_count,
        timeout_count,
        cache_hit_count: tree_result
            .data
            .get("atspi_cache_hit_count")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0),
        stale_cache_rejected_count: tree_result
            .data
            .get("atspi_stale_cache_rejected_count")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0),
    }
}

fn accessibility_confidence(
    available: bool,
    control_count: usize,
    executable_count: usize,
    skipped_app_count: usize,
    omitted_node_count: usize,
    timeout_count: usize,
    stale_node_count: usize,
) -> f64 {
    if !available {
        return 0.0;
    }
    let mut score: f64 = 0.72;
    if control_count > 0 {
        score += 0.12;
    }
    if executable_count > 0 {
        score += 0.10;
    }
    score -= (skipped_app_count as f64 * 0.08).min(0.24);
    score -= (timeout_count as f64 * 0.12).min(0.30);
    score -= (stale_node_count as f64 * 0.08).min(0.24);
    if omitted_node_count > 100 {
        score -= 0.08;
    }
    score.clamp(0.0, 0.98)
}

fn accessibility_status_from_confidence(available: bool, confidence: f64) -> &'static str {
    if !available {
        "unavailable"
    } else if confidence >= 0.82 {
        "healthy"
    } else {
        "degraded"
    }
}

fn accessibility_app_scores_from(
    tree_result: &GuiProbeResult,
    controls: &[GuiControlSummary],
    fallback_score: f64,
) -> Vec<GuiAccessibilityAppScore> {
    if let Some(items) = tree_result
        .data
        .get("accessibility_app_scores")
        .and_then(serde_json::Value::as_array)
    {
        return items
            .iter()
            .take(12)
            .filter_map(|item| {
                let app_label = first_string(item, &["app_label", "label", "app"])
                    .unwrap_or_else(|| "unknown app".into());
                let bus_name = first_string(item, &["bus_name", "bus"]).unwrap_or_default();
                let score = item
                    .get("score")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(fallback_score)
                    .clamp(0.0, 1.0);
                Some(GuiAccessibilityAppScore {
                    app_label: sanitize_gui_text(&app_label, 120).text,
                    bus_name_hash: if bus_name.is_empty() {
                        short_hash(&app_label)
                    } else {
                        short_hash(&bus_name)
                    },
                    node_count: item
                        .get("node_count")
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(0),
                    control_count: item
                        .get("control_count")
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(0),
                    timeout_count: item
                        .get("timeout_count")
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(0),
                    stale_node_count: item
                        .get("stale_node_count")
                        .and_then(serde_json::Value::as_u64)
                        .map(|value| value as usize)
                        .unwrap_or(0),
                    status: first_string(item, &["status"]).unwrap_or_else(|| {
                        accessibility_status_from_confidence(true, score).into()
                    }),
                    score,
                })
            })
            .collect();
    }

    if controls.is_empty() {
        return Vec::new();
    }
    vec![GuiAccessibilityAppScore {
        app_label: "focused accessible app".into(),
        bus_name_hash: short_hash("focused accessible app"),
        node_count: tree_result
            .data
            .get("node_count")
            .and_then(serde_json::Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(controls.len()),
        control_count: controls.len(),
        timeout_count: 0,
        stale_node_count: controls
            .iter()
            .filter(|control| control.path.is_empty())
            .count(),
        score: fallback_score,
        status: accessibility_status_from_confidence(true, fallback_score).into(),
    }]
}

fn visible_windows_from_desktop_state(
    desktop_state: &GuiProbeResult,
    limit: usize,
) -> Vec<GuiWindowSummary> {
    desktop_state
        .data
        .get("applications")
        .and_then(serde_json::Value::as_array)
        .map(|apps| {
            let focused = desktop_state
                .data
                .get("focused_window")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            apps.iter()
                .take(limit)
                .filter_map(|app| app.as_str())
                .map(|app| {
                    let title = sanitize_gui_text(app, 120).text;
                    GuiWindowSummary {
                        focused: !focused.is_empty() && title.contains(focused),
                        title: title.clone(),
                        app_name: Some(title),
                        bounds: None,
                        visible: true,
                        monitor_id: None,
                        source: "desktop_state.applications".into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn accessibility_status(capabilities: &GuiProbeResult, available: bool) -> GuiSourceStatus {
    if available {
        return GuiSourceStatus::available("accessibility operational");
    }
    let remediation = capabilities
        .data
        .get("remediation")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(|item| sanitize_gui_text(item, 180).text)
                .take(4)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    GuiSourceStatus {
        available: false,
        detail: "accessibility unavailable".into(),
        blocker: capabilities
            .error
            .clone()
            .or_else(|| Some("accessibility stack is not operational".into())),
        remediation,
    }
}

fn status_from_probe_result(detail: &str, result: &GuiProbeResult) -> GuiSourceStatus {
    if result.success {
        GuiSourceStatus::available(detail)
    } else {
        GuiSourceStatus::blocked(
            detail,
            result.error.as_deref().unwrap_or("source unavailable"),
        )
    }
}

fn status_from_probe(
    detail: &str,
    source: &str,
    blocker: &Option<String>,
    available: bool,
) -> GuiSourceStatus {
    if available {
        GuiSourceStatus::available(format!("{detail}:{source}"))
    } else {
        GuiSourceStatus::blocked(
            format!("{detail}:{source}"),
            blocker.as_deref().unwrap_or("source unavailable"),
        )
    }
}

fn fallback_screen_hash(
    active_window_label: &str,
    controls: &[GuiControlSummary],
    monitors: &[GuiMonitorSummary],
) -> String {
    let mut seed = active_window_label.to_string();
    for control in controls.iter().take(80) {
        seed.push('|');
        seed.push_str(&control.role);
        seed.push(':');
        seed.push_str(&control.name);
        seed.push(':');
        seed.push_str(&control.path);
    }
    for monitor in monitors {
        seed.push('|');
        seed.push_str(&monitor.id);
        seed.push(':');
        seed.push_str(&monitor.bounds.width.to_string());
        seed.push('x');
        seed.push_str(&monitor.bounds.height.to_string());
    }
    stable_hash(&seed)
}

fn parse_bounds(value: Option<&serde_json::Value>, bbox_xyxy: bool) -> Option<GuiBounds> {
    let value = value?;
    if let Some(object) = value.as_object() {
        let x = json_i32(object.get("x"))?;
        let y = json_i32(object.get("y"))?;
        let width = object
            .get("width")
            .and_then(|value| json_i32(Some(value)))
            .or_else(|| object.get("w").and_then(|value| json_i32(Some(value))))?;
        let height = object
            .get("height")
            .and_then(|value| json_i32(Some(value)))
            .or_else(|| object.get("h").and_then(|value| json_i32(Some(value))))?;
        return Some(GuiBounds {
            x,
            y,
            width,
            height,
        });
    }
    let values = value.as_array()?;
    if values.len() < 4 {
        return None;
    }
    let x = json_i32(values.first())?;
    let y = json_i32(values.get(1))?;
    let third = json_i32(values.get(2))?;
    let fourth = json_i32(values.get(3))?;
    let (width, height) = if bbox_xyxy {
        ((third - x).max(0), (fourth - y).max(0))
    } else {
        (third, fourth)
    };
    Some(GuiBounds {
        x,
        y,
        width,
        height,
    })
}

fn json_i32(value: Option<&serde_json::Value>) -> Option<i32> {
    value
        .and_then(serde_json::Value::as_i64)
        .map(|value| value as i32)
        .or_else(|| {
            value
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as i32)
        })
}

fn json_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    value.and_then(serde_json::Value::as_u64).or_else(|| {
        value
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| u64::try_from(value).ok())
    })
}

fn first_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_i64)
            .or_else(|| {
                value
                    .get(*key)
                    .and_then(serde_json::Value::as_f64)
                    .map(|value| value as i64)
            })
    })
}

fn first_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| json_u64(value.get(*key)))
}

fn first_bool(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_bool))
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
}

fn single_application(value: &serde_json::Value) -> Option<String> {
    let apps = value.get("applications")?.as_array()?;
    if apps.len() == 1 {
        apps.first()
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    } else {
        None
    }
}
