use async_trait::async_trait;
use std::collections::HashMap;

use super::perception::{sanitize_gui_text, stable_hash, GuiBounds};
use super::resolver::GuiTargetResolutionSummary;
use super::safety_hitl::{GuiActionProposal, GuiHitlDecision};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiActionKind {
    OpenApp,
    SwitchWindow,
    FocusField,
    FillField,
    TypeText,
    ClickControl,
    PressKey,
    Hotkey,
    Scroll,
    Copy,
    Paste,
}

impl GuiActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenApp => "OpenApp",
            Self::SwitchWindow => "SwitchWindow",
            Self::FocusField => "FocusField",
            Self::FillField => "FillField",
            Self::TypeText => "TypeText",
            Self::ClickControl => "ClickControl",
            Self::PressKey => "PressKey",
            Self::Hotkey => "Hotkey",
            Self::Scroll => "Scroll",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
        }
    }

    pub fn from_action_type(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "openapp" | "open_app" => Self::OpenApp,
            "switchwindow" | "switch_window" => Self::SwitchWindow,
            "focusfield" | "focus_field" | "focusinput" | "focus_input" => Self::FocusField,
            "fillfield" | "fill_field" => Self::TypeText,
            "typetext" | "type_text" => Self::TypeText,
            "clickcontrol" | "click_control" => Self::ClickControl,
            "presskey" | "press_key" => Self::PressKey,
            "hotkey" => Self::Hotkey,
            "scroll" => Self::Scroll,
            "copy" | "copy_content" => Self::Copy,
            "paste" | "paste_content" => Self::Paste,
            _ => Self::ClickControl,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiExecutorCapabilityMatrix {
    pub observe: bool,
    pub focus_field: bool,
    pub fill_field: bool,
    pub click_control: bool,
    pub post_action_observe: bool,
    pub verify: bool,
    pub recovery_focus: bool,
    pub recovery_modal: bool,
}

impl GuiExecutorCapabilityMatrix {
    pub fn all_available() -> Self {
        Self {
            observe: true,
            focus_field: true,
            fill_field: true,
            click_control: true,
            post_action_observe: true,
            verify: true,
            recovery_focus: true,
            recovery_modal: true,
        }
    }

    pub fn observe_only() -> Self {
        Self {
            observe: true,
            focus_field: false,
            fill_field: false,
            click_control: false,
            post_action_observe: true,
            verify: true,
            recovery_focus: false,
            recovery_modal: true,
        }
    }

    pub fn supports(&self, kind: &GuiActionKind) -> bool {
        match kind {
            GuiActionKind::OpenApp | GuiActionKind::SwitchWindow => self.focus_field,
            GuiActionKind::FocusField => self.focus_field,
            GuiActionKind::FillField | GuiActionKind::TypeText => self.fill_field,
            GuiActionKind::ClickControl => self.click_control,
            GuiActionKind::PressKey
            | GuiActionKind::Hotkey
            | GuiActionKind::Scroll
            | GuiActionKind::Copy
            | GuiActionKind::Paste => self.click_control || self.fill_field,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiExecutionMode {
    SafetyOnly,
    ExecuteFixture,
    ExecuteLive,
}

impl Default for GuiExecutionMode {
    fn default() -> Self {
        Self::SafetyOnly
    }
}

impl GuiExecutionMode {
    pub fn allows_execution(self) -> bool {
        !matches!(self, Self::SafetyOnly)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SafetyOnly => "safety_only",
            Self::ExecuteFixture => "execute_fixture",
            Self::ExecuteLive => "execute_live",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiExecutionAuthorizationSource {
    SafeNoApprovalRequired,
    HitlApproved,
}

impl GuiExecutionAuthorizationSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SafeNoApprovalRequired => "safe_no_approval_required",
            Self::HitlApproved => "hitl_approved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiExecutionRequest {
    pub execution_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub action_type: String,
    pub target_hash: String,
    pub stable_target_identity_hash: Option<String>,
    pub target_control_id: Option<String>,
    pub target_bounds: Option<GuiBounds>,
    pub text_payload_hash: Option<String>,
    pub text_payload_handle: Option<String>,
    pub expected_precondition: String,
    pub expected_postcondition: String,
    pub authorization_source: GuiExecutionAuthorizationSource,
    pub approved_decision_id: Option<String>,
    pub context_id: String,
    pub observation_id: String,
    pub created_at_ms: i64,
    pub prompt_hash: String,
}

impl GuiExecutionRequest {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "action_type": self.action_type,
            "target_hash": self.target_hash,
            "stable_target_identity_hash": self.stable_target_identity_hash,
            "target_control_id": self.target_control_id,
            "target_bounds": self.target_bounds,
            "text_payload_hash": self.text_payload_hash,
            "text_payload_handle": self.text_payload_handle,
            "expected_precondition": self.expected_precondition,
            "expected_postcondition": self.expected_postcondition,
            "authorization_source": self.authorization_source.as_str(),
            "approved_decision_id": self.approved_decision_id,
            "context_id": self.context_id,
            "observation_id": self.observation_id,
            "created_at_ms": self.created_at_ms,
            "prompt_hash": self.prompt_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiExecutionPreconditionReport {
    pub status: String,
    pub can_start_action: bool,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub checked_at_ms: i64,
}

impl GuiExecutionPreconditionReport {
    pub fn allowed(now_ms: i64, warnings: Vec<String>) -> Self {
        Self {
            status: "valid".into(),
            can_start_action: true,
            blockers: Vec::new(),
            warnings,
            checked_at_ms: now_ms,
        }
    }

    pub fn blocked(now_ms: i64, blockers: Vec<String>, warnings: Vec<String>) -> Self {
        Self {
            status: "blocked".into(),
            can_start_action: false,
            blockers,
            warnings,
            checked_at_ms: now_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiExecutionResult {
    pub execution_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub action_type: String,
    pub status: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub backend_used: String,
    pub precondition_check: GuiExecutionPreconditionReport,
    pub postcondition_check: String,
    pub verification_result: String,
    pub error_code: Option<String>,
    pub safe_error_summary: Option<String>,
    pub can_retry: bool,
    pub recovery_hint: Option<String>,
    pub prompt_hash: String,
}

impl GuiExecutionResult {
    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "action_type": self.action_type,
            "status": self.status,
            "started_at_ms": self.started_at_ms,
            "completed_at_ms": self.completed_at_ms,
            "backend_used": self.backend_used,
            "precondition_check": self.precondition_check,
            "postcondition_check": self.postcondition_check,
            "verification_result": self.verification_result,
            "error_code": self.error_code,
            "safe_error_summary": self.safe_error_summary,
            "can_retry": self.can_retry,
            "recovery_hint": self.recovery_hint,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn blocked_event_payload(&self, backend: &GuiActionBackendStatus) -> serde_json::Value {
        serde_json::json!({
            "type": "ExecutionBlocked",
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "proposal_hash": self.proposal_hash,
            "action_kind": self.action_type,
            "reason": self.safe_error_summary,
            "status": self.status,
            "backend_used": self.backend_used,
            "selected_backend": backend.selected_backend,
            "session_type": backend.session_type,
            "global_halt_engaged": backend.global_halt_engaged,
            "halt_kind": backend.halt_kind,
            "halt_reason": backend.halt_reason,
            "release_conditions": backend.release_conditions,
            "blockers": self.precondition_check.blockers,
            "can_retry": self.can_retry,
            "recovery_hint": self.recovery_hint,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn verification_event_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "ExecutionVerificationCompleted",
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "status": self.status,
            "postcondition_check": self.postcondition_check,
            "verification_result": self.verification_result,
            "can_retry": self.can_retry,
            "recovery_hint": self.recovery_hint,
            "prompt_hash": self.prompt_hash,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GuiPayloadEntry {
    pub proposal_id: String,
    pub payload_hash: String,
    pub raw_payload: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Default, Clone)]
pub struct GuiPayloadVault {
    entries: HashMap<String, GuiPayloadEntry>,
}

impl GuiPayloadVault {
    pub fn insert(
        &mut self,
        proposal_id: &str,
        payload_hash: &str,
        raw_payload: &str,
        expires_at_ms: i64,
    ) -> Option<String> {
        let payload = raw_payload.trim();
        if payload.is_empty() || payload.contains("[redacted]") {
            return None;
        }
        let handle = format!(
            "payload-{}",
            stable_hash(&format!("{proposal_id}|{payload_hash}|{expires_at_ms}"))
        );
        self.entries.insert(
            handle.clone(),
            GuiPayloadEntry {
                proposal_id: proposal_id.to_string(),
                payload_hash: payload_hash.to_string(),
                raw_payload: payload.to_string(),
                expires_at_ms,
            },
        );
        Some(handle)
    }

    pub fn get(
        &self,
        handle: &str,
        proposal_id: &str,
        payload_hash: &str,
        now_ms: i64,
    ) -> Option<&str> {
        let entry = self.entries.get(handle)?;
        if entry.proposal_id != proposal_id
            || entry.payload_hash != payload_hash
            || now_ms > entry.expires_at_ms
        {
            return None;
        }
        Some(entry.raw_payload.as_str())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiActionBackendStatus {
    pub global_halt_engaged: bool,
    pub halt_kind: String,
    pub halt_reason: Option<String>,
    pub release_conditions: Vec<String>,
    pub startup_elapsed_ms: Option<u64>,
    pub can_observe: bool,
    pub can_plan: bool,
    pub automation_enabled: bool,
    pub vision_sidecar: String,
    pub uinput_daemon: String,
    pub orchestrator_available: bool,
    pub session_type: String,
    pub xdotool_available: bool,
    pub ydotool_available: bool,
    pub uinput_available: bool,
    pub selected_backend: String,
    pub backend_selection_reason: String,
    pub backend_probe_status: String,
    pub backend_probe_errors: Vec<String>,
    pub input_backend_kind: String,
    pub focus_supported: bool,
    pub typing_supported: bool,
    pub click_supported: bool,
    pub verification_supported: bool,
    pub xdotool_usable_for_actions: bool,
    pub ydotool_usable_for_actions: bool,
    pub uinput_socket_path: Option<String>,
    pub uinput_socket_accessible: bool,
    pub can_execute_actions: bool,
    pub blockers: Vec<String>,
    pub capabilities: GuiExecutorCapabilityMatrix,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiBackendProbeInput {
    pub global_halt_engaged: bool,
    pub halt_reason: Option<String>,
    pub automation_enabled: bool,
    pub orchestrator_available: bool,
    pub session_type: String,
    pub vision_sidecar: String,
    pub uinput_daemon: String,
    pub xdotool_available: bool,
    pub xdotool_display_usable: bool,
    pub ydotool_available: bool,
    pub ydotool_permission_ok: bool,
    pub uinput_available: bool,
    pub uinput_socket_path: Option<String>,
    pub uinput_socket_accessible: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiBackendSelection {
    pub selected_backend: String,
    pub backend_selection_reason: String,
    pub backend_probe_status: String,
    pub backend_probe_errors: Vec<String>,
    pub input_backend_kind: String,
    pub focus_supported: bool,
    pub typing_supported: bool,
    pub click_supported: bool,
    pub verification_supported: bool,
    pub xdotool_usable_for_actions: bool,
    pub ydotool_usable_for_actions: bool,
    pub can_execute_actions: bool,
    pub blockers: Vec<String>,
    pub capabilities: GuiExecutorCapabilityMatrix,
}

pub fn select_gui_action_backend(input: &GuiBackendProbeInput) -> GuiBackendSelection {
    let session = input.session_type.trim().to_lowercase();
    let xdotool_usable_for_actions =
        session == "x11" && input.xdotool_available && input.xdotool_display_usable;
    let ydotool_usable_for_actions =
        session == "wayland" && input.ydotool_available && input.ydotool_permission_ok;
    let uinput_usable_for_actions =
        session == "wayland" && input.uinput_available && input.uinput_socket_accessible;

    let mut backend_probe_errors = Vec::new();
    if session == "wayland" && input.xdotool_available {
        backend_probe_errors
            .push("xdotool detected but not usable for Wayland GUI actions".to_string());
    }
    if input.uinput_available && !input.uinput_socket_accessible {
        backend_probe_errors
            .push("uinput daemon reported running but socket is not accessible".into());
    }
    if input.ydotool_available && !input.ydotool_permission_ok {
        backend_probe_errors
            .push("ydotool detected but permission/usability probe did not pass".into());
    }
    if session == "x11" && input.xdotool_available && !input.xdotool_display_usable {
        backend_probe_errors.push("xdotool detected but DISPLAY/active-window probe failed".into());
    }

    let mut selection = GuiBackendSelection {
        selected_backend: "unavailable".into(),
        backend_selection_reason: "No deterministic GUI action backend is available.".into(),
        backend_probe_status: "unknown_session_blocked".into(),
        backend_probe_errors,
        input_backend_kind: "none".into(),
        focus_supported: false,
        typing_supported: false,
        click_supported: false,
        verification_supported: true,
        xdotool_usable_for_actions,
        ydotool_usable_for_actions,
        can_execute_actions: false,
        blockers: Vec::new(),
        capabilities: GuiExecutorCapabilityMatrix::observe_only(),
    };

    if input.global_halt_engaged {
        selection.selected_backend = "blocked_global_halt".into();
        selection.backend_probe_status = "global_halt_blocked".into();
        selection.backend_selection_reason = input
            .halt_reason
            .clone()
            .unwrap_or_else(|| "Global safety halt is engaged.".into());
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    if !input.orchestrator_available {
        selection.backend_probe_status = "orchestrator_unavailable".into();
        selection.backend_selection_reason = "GUI service orchestrator is unavailable.".into();
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    if !input.automation_enabled {
        selection.selected_backend = "automation_disabled".into();
        selection.backend_probe_status = "automation_disabled".into();
        selection.backend_selection_reason = "GUI automation is disabled by user setting.".into();
        selection
            .blockers
            .push(selection.backend_selection_reason.clone());
        return selection;
    }

    match session.as_str() {
        "wayland" if uinput_usable_for_actions => {
            selection.selected_backend = "uinput_accessibility".into();
            selection.backend_probe_status = "wayland_uinput_ready".into();
            selection.backend_selection_reason =
                "Wayland session selected uinput because the daemon and socket are healthy.".into();
            selection.input_backend_kind = "uinput".into();
        }
        "wayland" if ydotool_usable_for_actions => {
            selection.selected_backend = "ydotool_accessibility".into();
            selection.backend_probe_status = "wayland_ydotool_ready".into();
            selection.backend_selection_reason =
                "Wayland session selected ydotool because its usability probe passed.".into();
            selection.input_backend_kind = "ydotool".into();
        }
        "wayland" => {
            selection.backend_probe_status = "wayland_no_input_backend".into();
            selection.backend_selection_reason =
                "Wayland session has no usable uinput socket or validated ydotool backend.".into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
        "x11" if xdotool_usable_for_actions => {
            selection.selected_backend = "xdotool_accessibility".into();
            selection.backend_probe_status = "x11_xdotool_ready".into();
            selection.backend_selection_reason =
                "X11 session selected xdotool because DISPLAY and active-window probe passed."
                    .into();
            selection.input_backend_kind = "xdotool".into();
        }
        "x11" => {
            selection.backend_probe_status = "x11_no_xdotool".into();
            selection.backend_selection_reason =
                "X11 session has no usable xdotool action backend.".into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
        _ => {
            selection.backend_probe_status = "unknown_session_blocked".into();
            selection.backend_selection_reason =
                "GUI session type is unknown and no deterministic action backend is available."
                    .into();
            selection
                .blockers
                .push(selection.backend_selection_reason.clone());
            return selection;
        }
    }

    selection.can_execute_actions = true;
    selection.focus_supported = true;
    selection.typing_supported = true;
    selection.click_supported = true;
    selection.capabilities = GuiExecutorCapabilityMatrix::all_available();
    selection
}

impl GuiActionBackendStatus {
    pub fn available(selected_backend: impl Into<String>) -> Self {
        let selected_backend = selected_backend.into();
        Self {
            global_halt_engaged: false,
            halt_kind: "none".into(),
            halt_reason: None,
            release_conditions: Vec::new(),
            startup_elapsed_ms: None,
            can_observe: true,
            can_plan: true,
            automation_enabled: true,
            vision_sidecar: "unknown".into(),
            uinput_daemon: "unknown".into(),
            orchestrator_available: true,
            session_type: "test".into(),
            xdotool_available: true,
            ydotool_available: true,
            uinput_available: true,
            selected_backend: selected_backend.clone(),
            backend_selection_reason: format!("Test backend {selected_backend} is available."),
            backend_probe_status: "test_backend_ready".into(),
            backend_probe_errors: Vec::new(),
            input_backend_kind: "test".into(),
            focus_supported: true,
            typing_supported: true,
            click_supported: true,
            verification_supported: true,
            xdotool_usable_for_actions: true,
            ydotool_usable_for_actions: true,
            uinput_socket_path: None,
            uinput_socket_accessible: true,
            can_execute_actions: true,
            blockers: Vec::new(),
            capabilities: GuiExecutorCapabilityMatrix::all_available(),
        }
    }

    pub fn blocked(
        selected_backend: impl Into<String>,
        blocker: impl Into<String>,
        session_type: impl Into<String>,
    ) -> Self {
        let selected_backend = selected_backend.into();
        let blocker = blocker.into();
        Self {
            global_halt_engaged: false,
            halt_kind: "service_not_ready".into(),
            halt_reason: None,
            release_conditions: vec!["Resolve the GUI action backend blocker, then retry.".into()],
            startup_elapsed_ms: None,
            can_observe: true,
            can_plan: true,
            automation_enabled: false,
            vision_sidecar: "unknown".into(),
            uinput_daemon: "unknown".into(),
            orchestrator_available: false,
            session_type: session_type.into(),
            xdotool_available: false,
            ydotool_available: false,
            uinput_available: false,
            selected_backend: selected_backend.clone(),
            backend_selection_reason: blocker.clone(),
            backend_probe_status: "test_backend_blocked".into(),
            backend_probe_errors: vec![blocker.clone()],
            input_backend_kind: "none".into(),
            focus_supported: false,
            typing_supported: false,
            click_supported: false,
            verification_supported: true,
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_path: None,
            uinput_socket_accessible: false,
            can_execute_actions: false,
            blockers: vec![blocker],
            capabilities: GuiExecutorCapabilityMatrix::observe_only(),
        }
    }

    pub fn supports_action(&self, kind: &GuiActionKind) -> bool {
        self.can_execute_actions && self.capabilities.supports(kind)
    }

    pub fn primary_blocker(&self, kind: &GuiActionKind) -> String {
        if self.global_halt_engaged {
            return self
                .halt_reason
                .clone()
                .unwrap_or_else(|| "global safety halt is engaged".into());
        }
        if let Some(blocker) = self.blockers.first() {
            return blocker.clone();
        }
        if !self.capabilities.supports(kind) {
            return format!(
                "{} is not supported by selected GUI backend {}",
                kind.as_str(),
                self.selected_backend
            );
        }
        "GUI action backend is unavailable".into()
    }
}

pub fn stable_target_identity_hash(
    control_id: Option<&str>,
    role: Option<&str>,
    label: Option<&str>,
    bounds: Option<&GuiBounds>,
    app_hint: Option<&str>,
    window_hint: Option<&str>,
) -> String {
    let bounds_seed = bounds
        .map(|bounds| {
            format!(
                "{}:{}:{}:{}",
                bounds.x, bounds.y, bounds.width, bounds.height
            )
        })
        .unwrap_or_else(|| "no_bounds".into());
    stable_hash(&format!(
        "{}|{}|{}|{}|{}|{}",
        sanitize_gui_text(control_id.unwrap_or_default(), 120).text,
        sanitize_gui_text(role.unwrap_or_default(), 80).text,
        stable_hash(&sanitize_gui_text(label.unwrap_or_default(), 160).text),
        stable_hash(&bounds_seed),
        sanitize_gui_text(app_hint.unwrap_or_default(), 120).text,
        sanitize_gui_text(window_hint.unwrap_or_default(), 120).text,
    ))
}

pub fn build_execution_request_from_proposal(
    proposal: &GuiActionProposal,
    target_resolution: &GuiTargetResolutionSummary,
    authorization_source: GuiExecutionAuthorizationSource,
    approved_decision_id: Option<String>,
    payload_vault: &mut GuiPayloadVault,
    now_ms: i64,
) -> GuiExecutionRequest {
    let target = target_resolution.resolved_target.as_ref();
    let stable_target_identity_hash = target.map(|target| {
        stable_target_identity_hash(
            Some(&target.control_id),
            Some(&target.role),
            Some(&target.label),
            target.bounds.as_ref(),
            target.app_hint.as_deref(),
            target.window_hint.as_deref(),
        )
    });
    let text_payload_handle = match (
        proposal.text_payload_hash.as_deref(),
        proposal.text_payload_summary.as_deref(),
    ) {
        (Some(hash), Some(summary)) => {
            payload_vault.insert(&proposal.proposal_id, hash, summary, proposal.expires_at_ms)
        }
        _ => None,
    };
    GuiExecutionRequest {
        execution_id: format!(
            "execution-{}",
            stable_hash(&format!(
                "{}|{}|{}",
                proposal.proposal_id, proposal.proposal_hash, now_ms
            ))
        ),
        proposal_id: proposal.proposal_id.clone(),
        proposal_hash: proposal.proposal_hash.clone(),
        action_type: proposal.action_type.clone(),
        target_hash: proposal.target_hash.clone(),
        stable_target_identity_hash,
        target_control_id: proposal.target_control_id.clone(),
        target_bounds: proposal.target_bounds.clone(),
        text_payload_hash: proposal.text_payload_hash.clone(),
        text_payload_handle,
        expected_precondition: proposal.expected_precondition.clone(),
        expected_postcondition: proposal.expected_postcondition.clone(),
        authorization_source,
        approved_decision_id,
        context_id: proposal.context_id.clone(),
        observation_id: proposal.observation_id.clone(),
        created_at_ms: now_ms,
        prompt_hash: proposal.prompt_hash.clone(),
    }
}

pub fn validate_execution_preconditions(
    mode: GuiExecutionMode,
    request: &GuiExecutionRequest,
    proposal: &GuiActionProposal,
    target_resolution: &GuiTargetResolutionSummary,
    backend: &GuiActionBackendStatus,
    hitl_decision: Option<&GuiHitlDecision>,
    payload_vault: &GuiPayloadVault,
    now_ms: i64,
) -> GuiExecutionPreconditionReport {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    if !mode.allows_execution() {
        blockers.push("execution mode is safety_only".to_string());
    }
    if proposal.proposal_id != request.proposal_id {
        blockers.push("proposal_id mismatch".into());
    }
    if proposal.proposal_hash != request.proposal_hash {
        blockers.push("proposal_hash mismatch".into());
    }
    if proposal.action_type != request.action_type {
        blockers.push("action_type mismatch".into());
    }
    if now_ms > proposal.expires_at_ms {
        blockers.push("proposal expired".into());
    }
    if proposal.target_hash != request.target_hash {
        blockers.push("target_hash mismatch".into());
    }
    if !backend.can_execute_actions {
        blockers.push(backend.primary_blocker(&GuiActionKind::from_action_type(
            &request.action_type,
        )));
    }
    let action_kind = GuiActionKind::from_action_type(&request.action_type);
    if !backend.supports_action(&action_kind) {
        blockers.push(backend.primary_blocker(&action_kind));
    }

    if proposal.requires_user_approval || matches!(proposal.risk_level.as_str(), "high" | "critical") {
        match (request.authorization_source.clone(), hitl_decision) {
            (GuiExecutionAuthorizationSource::HitlApproved, Some(decision))
                if decision.decision == "approved"
                    && decision.can_authorize_step7
                    && decision.proposal_hash == proposal.proposal_hash
                    && decision.target_hash == proposal.target_hash
                    && request.approved_decision_id.as_deref() == Some(decision.decision_id.as_str()) => {}
            _ => blockers.push("fresh HITL approval is required".into()),
        }
    } else if !matches!(
        request.authorization_source,
        GuiExecutionAuthorizationSource::SafeNoApprovalRequired
    ) && hitl_decision.is_none()
    {
        warnings.push("low-risk action used non-standard authorization source".into());
    }

    let control_action = matches!(
        action_kind,
        GuiActionKind::FocusField
            | GuiActionKind::FillField
            | GuiActionKind::TypeText
            | GuiActionKind::ClickControl
            | GuiActionKind::PressKey
            | GuiActionKind::Hotkey
            | GuiActionKind::Scroll
            | GuiActionKind::Copy
            | GuiActionKind::Paste
    );
    if control_action {
        if target_resolution.status != "resolved" {
            blockers.push(format!("target resolution is {}", target_resolution.status));
        }
        let Some(target) = target_resolution.resolved_target.as_ref() else {
            blockers.push("resolved target missing".into());
            return GuiExecutionPreconditionReport::blocked(now_ms, sanitize_list(blockers), warnings);
        };
        if target.target_hash != proposal.target_hash {
            blockers.push("resolved target hash does not match proposal".into());
        }
        if !target.visible || !target.enabled {
            blockers.push("resolved target is hidden or disabled".into());
        }
        if target.source.contains("ocr") || target.source.contains("visual_only") {
            blockers.push("resolved target is not trusted for execution".into());
        }
        if target.bounds.is_none() && !matches!(action_kind, GuiActionKind::PressKey | GuiActionKind::Hotkey) {
            blockers.push("control target is missing trusted bounds".into());
        }
        let stable_hash = stable_target_identity_hash(
            Some(&target.control_id),
            Some(&target.role),
            Some(&target.label),
            target.bounds.as_ref(),
            target.app_hint.as_deref(),
            target.window_hint.as_deref(),
        );
        if request.stable_target_identity_hash.as_deref() != Some(stable_hash.as_str()) {
            blockers.push("stable target identity mismatch".into());
        }
    }

    if matches!(action_kind, GuiActionKind::TypeText | GuiActionKind::FillField | GuiActionKind::Paste) {
        let Some(payload_hash) = request.text_payload_hash.as_deref() else {
            blockers.push("missing text payload hash".into());
            return GuiExecutionPreconditionReport::blocked(now_ms, sanitize_list(blockers), warnings);
        };
        let Some(handle) = request.text_payload_handle.as_deref() else {
            blockers.push("missing backend payload handle".into());
            return GuiExecutionPreconditionReport::blocked(now_ms, sanitize_list(blockers), warnings);
        };
        if payload_vault
            .get(handle, &request.proposal_id, payload_hash, now_ms)
            .is_none()
        {
            blockers.push("payload handle is stale or mismatched".into());
        }
    }

    if blockers.is_empty() {
        GuiExecutionPreconditionReport::allowed(now_ms, warnings)
    } else {
        GuiExecutionPreconditionReport::blocked(now_ms, sanitize_list(blockers), warnings)
    }
}

fn sanitize_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| sanitize_gui_text(&value, 180).text)
        .filter(|value| !value.trim().is_empty())
        .collect()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiActionRequest {
    pub kind: GuiActionKind,
    pub role: String,
    pub target_name: String,
    pub value: Option<String>,
    pub execution_hint: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiActionExecution {
    pub success: bool,
    pub tool: String,
    pub error: Option<String>,
    pub evidence: serde_json::Value,
}

impl GuiActionExecution {
    pub fn ok(tool: impl Into<String>, evidence: serde_json::Value) -> Self {
        Self {
            success: true,
            tool: tool.into(),
            error: None,
            evidence,
        }
    }

    pub fn err(tool: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            success: false,
            tool: tool.into(),
            error: Some(error.into()),
            evidence: serde_json::Value::Null,
        }
    }
}

#[async_trait]
pub trait GuiActionExecutor: Send + Sync {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        GuiActionBackendStatus::available("default_test_backend")
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution;
}

pub fn sanitized_execution_evidence(evidence: &str) -> String {
    let mut value = evidence.trim().chars().take(240).collect::<String>();
    if let Ok(re) = regex::Regex::new(r#"(?i)(password|token|api[_-]?key|secret)\S*"#) {
        value = re.replace_all(&value, "[redacted]").to_string();
    }
    value
}
