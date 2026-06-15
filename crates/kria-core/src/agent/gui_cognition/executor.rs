use async_trait::async_trait;
use std::collections::HashMap;

use super::perception::{sanitize_gui_text, stable_hash, GuiBounds, GuiMonitorSummary};
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
    // Task 6.1 (Requirement 5): explicit typed primitives so each visible single
    // action routes through the correct executor mapping/backend instead of the
    // legacy `ClickControl` catch-all. These are produced ONLY when the
    // `gui_cog_primitives` flag is ON (via
    // [`GuiPrimitivesConfig::resolve_action_kind`]); the legacy
    // [`GuiActionKind::from_action_type`] never emits them, so the flag-OFF path
    // is byte-for-byte unchanged.
    /// Clear the focused field's contents (select-all + delete).
    ClearField,
    /// Select all content in the focused field/view (Ctrl+A).
    SelectAll,
    /// Set/toggle a labeled checkbox to a target state.
    SetCheckbox,
    /// Close / dismiss the active dialog (Escape or its close control).
    CloseDialog,
    /// In-app search: focus the app's search box and submit a query.
    InAppSearch,
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
            Self::ClearField => "ClearField",
            Self::SelectAll => "SelectAll",
            Self::SetCheckbox => "SetCheckbox",
            Self::CloseDialog => "CloseDialog",
            Self::InAppSearch => "InAppSearch",
        }
    }

    /// Legacy action-type mapping — preserved byte-for-byte. Unrecognized
    /// action types fall back to `ClickControl`. This is the mapping used while
    /// the `gui_cog_primitives` flag is OFF; it NEVER emits the Task 6.1 typed
    /// primitives ([`ClearField`](Self::ClearField) etc.).
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

    /// Task 6.1 (Requirement 5): the richer action-type mapping used when the
    /// `gui_cog_primitives` flag is ON. Every recognized legacy action type maps
    /// to the SAME kind as [`from_action_type`](Self::from_action_type), and the
    /// previously-defaulting primitive verbs (clear/select-all/checkbox/
    /// dialog-close/in-app-search) map to their correct typed primitive instead
    /// of the legacy `ClickControl` catch-all. Unknown verbs still fall back to
    /// `ClickControl` so behavior is never less defined than the legacy mapping.
    pub fn from_primitive_action_type(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "clearfield" | "clear_field" | "clear" | "cleartext" | "clear_text" => Self::ClearField,
            "selectall" | "select_all" | "select-all" | "select" => Self::SelectAll,
            "setcheckbox" | "set_checkbox" | "checkbox" | "check" | "uncheck" | "toggle_checkbox" => {
                Self::SetCheckbox
            }
            "closedialog" | "close_dialog" | "dialog_close" | "dismiss_dialog" | "dismiss" => {
                Self::CloseDialog
            }
            "inappsearch" | "in_app_search" | "in-app-search" | "app_search" | "search" => {
                Self::InAppSearch
            }
            // Everything else keeps the legacy mapping so recognized verbs are
            // identical to the flag-OFF path.
            other => Self::from_action_type(other),
        }
    }

    /// Task 6.3 (Requirements 5, 15): the GREEN/YELLOW primitive [tier] for this
    /// executor action kind. Every `GuiActionKind` is a concrete GREEN/YELLOW
    /// primitive (the destructive / approval-gated RED/BLACK band never
    /// materializes as a `GuiActionKind` — it stays governed by the safety/HITL
    /// gate), so this classification is total. GREEN = read-only /
    /// non-state-changing (focus/scroll/select-all/in-app-search/switch-window);
    /// YELLOW = a visible LOCAL state change that needs care but is not
    /// external/destructive. A key-combo ([`Hotkey`](Self::Hotkey)) is
    /// conservatively YELLOW because it can mutate visible state.
    ///
    /// [tier]: GuiPrimitiveTier
    pub fn primitive_tier(&self) -> GuiPrimitiveTier {
        match self {
            Self::SwitchWindow
            | Self::FocusField
            | Self::Scroll
            | Self::SelectAll
            | Self::InAppSearch => GuiPrimitiveTier::Green,
            Self::OpenApp
            | Self::FillField
            | Self::TypeText
            | Self::ClickControl
            | Self::PressKey
            | Self::Hotkey
            | Self::Copy
            | Self::Paste
            | Self::ClearField
            | Self::SetCheckbox
            | Self::CloseDialog => GuiPrimitiveTier::Yellow,
        }
    }
}

/// Task 6.3 (Requirements 5, 15): the coarse safety TIER of a GUI primitive,
/// surfaced for events/telemetry when the `gui_cog_primitives` flag is ON.
///
/// This classifies ONLY the GREEN/YELLOW primitive band. RED/BLACK actions
/// (destructive / approval-gated / external-effect) are NOT primitives and stay
/// governed by the existing safety/HITL gate — [`primitive_tier`] returns `None`
/// for them so they can never be down-classified into this band.
///
/// * `Green` — safe, read-only or non-state-changing: focus / observe / scroll /
///   select-all / in-app-search / summarize / wait / verify / ask-clarification /
///   switch-window. Every GREEN primitive is ALSO idempotent (re-running it
///   converges to the same state with no extra side effect), so the consistency
///   invariant is `GREEN ⇒ idempotent`.
/// * `Yellow` — a visible LOCAL state change that needs care but is not
///   external/destructive: type / clear / paste / click / checkbox / key-press /
///   copy / close-dialog / open-app / browser-navigate. A YELLOW primitive MAY
///   be idempotent (e.g. `ClearField` converges to an empty field) — tier and
///   idempotency are independent axes; what unifies YELLOW is that it mutates
///   visible state. Conversely, every NON-idempotent primitive is YELLOW (a
///   non-idempotent action can never be GREEN).
///
/// Serializes to the product risk-model token (`"GREEN"` / `"YELLOW"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GuiPrimitiveTier {
    #[serde(rename = "GREEN")]
    Green,
    #[serde(rename = "YELLOW")]
    Yellow,
}

impl GuiPrimitiveTier {
    /// Stable telemetry/event token (`"GREEN"` / `"YELLOW"`), matching the
    /// product risk-model naming.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Green => "GREEN",
            Self::Yellow => "YELLOW",
        }
    }
}

/// Task 6.3 (Requirements 5, 15): classify a typed plan `step_type` into its
/// GREEN/YELLOW primitive [tier]. Returns `None` for any step type that is NOT a
/// GREEN/YELLOW primitive — the destructive / approval-gated band
/// (`RequireApproval`, `Save`, `Download`) and any unknown step type — so those
/// are never reclassified into the primitive band and stay governed by the
/// existing safety/HITL gate (GREEN/YELLOW only here).
///
/// The classification is kept consistent with
/// [`default_idempotent_for`](super::llm_planner::default_idempotent_for): every
/// GREEN step type is idempotent, and every non-idempotent step type is YELLOW.
///
/// [tier]: GuiPrimitiveTier
pub fn primitive_tier(step_type: &str) -> Option<GuiPrimitiveTier> {
    match step_type {
        // GREEN — read-only / non-state-changing. Each is ALSO idempotent in
        // `default_idempotent_for`, satisfying the `GREEN ⇒ idempotent` invariant.
        "Observe" | "FocusField" | "Scroll" | "SelectAll" | "InAppSearch"
        | "SummarizeVisibleContent" | "WaitForState" | "VerifyState" | "AskClarification"
        | "SwitchWindow" => Some(GuiPrimitiveTier::Green),
        // YELLOW — visible LOCAL state change, care required, not
        // external/destructive. `ClearField` is YELLOW yet idempotent (clearing
        // converges to an empty field); the rest are non-idempotent.
        "TypeText" | "ClearField" | "Paste" | "ClickControl" | "SetCheckbox" | "PressKey"
        | "Copy" | "CloseDialog" | "OpenApp" | "BrowserNavigate" => Some(GuiPrimitiveTier::Yellow),
        // RED/BLACK / approval-gated (RequireApproval/Save/Download) / unknown →
        // NOT a GREEN/YELLOW primitive; governed by the safety/HITL gate.
        _ => None,
    }
}

/// Environment variable that enables the `gui_cog_primitives` flag (Task 6).
///
/// Truthy (`1`/`true`/`yes`/`on`) turns the richer primitive-coverage executor
/// mapping + DPI/multi-monitor-aware bounds transform ON. Default (unset or any
/// other value) keeps it OFF, preserving the existing Step 1–12 executor path
/// byte-for-byte. The wave gate (Task 6.5) flips the live/desktop path to
/// default ON.
pub const PRIMITIVES_ENV_FLAG: &str = "KRIA_GUI_COG_PRIMITIVES";

/// Parse a `gui_cog_primitives` env value as truthy (`1`/`true`/`yes`/`on`).
fn primitives_flag_truthy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Parse a `gui_cog_primitives` env value as an explicit falsy opt-out
/// (`0`/`false`/`no`/`off`/empty) — the documented rollback switch. An absent
/// value (`None`) is NOT falsy: the default stays ON for the default-on path.
fn primitives_flag_falsy(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off") | Some("")
    )
}

/// The `gui_cog_primitives` feature-flag bundle (default OFF) — Task 6.1.
///
/// When enabled, the runtime maps each visible single-action primitive
/// (focus/type/clear/select-all/copy/paste/key-press/scroll/click/checkbox/
/// dialog-close/in-app-search) to its correct typed executor action kind via
/// [`GuiActionKind::from_primitive_action_type`] (instead of the legacy
/// `ClickControl` catch-all), and annotates control actions with
/// DPI/multi-monitor-aware physical bounds via [`physical_bounds_for_target`].
/// When disabled (the default), neither path runs and the produced action kinds
/// + events are preserved exactly — the prior Step 1–12 behavior. The wave gate
/// (Task 6.5) flips this flag ON for the live/desktop path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GuiPrimitivesConfig {
    /// Whether the richer primitive mapping + bounds transform is active.
    pub enabled: bool,
}

impl Default for GuiPrimitivesConfig {
    fn default() -> Self {
        // Task 6: flag default OFF until the wave gate (Task 6.5) flips it.
        Self { enabled: false }
    }
}

impl GuiPrimitivesConfig {
    /// Construct an explicitly-enabled primitives config.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Construct an explicitly-disabled primitives config.
    pub fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Derive the config from the process environment. The flag is OFF unless
    /// [`PRIMITIVES_ENV_FLAG`] is truthy.
    pub fn from_env() -> Self {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env`](Self::from_env) with an injectable lookup.
    pub fn from_env_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            enabled: primitives_flag_truthy(lookup(PRIMITIVES_ENV_FLAG).as_deref()),
        }
    }

    /// Derive the config from the process environment with the flag defaulting
    /// **ON** (wave gate flip, Task 6.5). The primitive-coverage path is active
    /// unless [`PRIMITIVES_ENV_FLAG`] is explicitly falsy
    /// (`0`/`false`/`no`/`off`/empty), which is the documented rollback switch.
    /// An absent env value keeps the flag ON.
    pub fn from_env_default_on() -> Self {
        Self::from_env_lookup_default_on(|key| std::env::var(key).ok())
    }

    /// Testable core of [`from_env_default_on`](Self::from_env_default_on) with
    /// an injectable lookup.
    pub fn from_env_lookup_default_on<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        // Default ON: enabled unless the operator explicitly opts out via a
        // falsy env value (the rollback switch). Absent (None) is NOT falsy.
        Self {
            enabled: !primitives_flag_falsy(lookup(PRIMITIVES_ENV_FLAG).as_deref()),
        }
    }

    /// Whether the richer primitive mapping + bounds transform should run.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Resolve an action-type string to its executor [`GuiActionKind`]. When the
    /// flag is ON this uses the richer primitive mapping
    /// ([`GuiActionKind::from_primitive_action_type`]); when OFF it delegates to
    /// the legacy [`GuiActionKind::from_action_type`] so the result is identical
    /// to the pre-Task-6.1 path for every input.
    pub fn resolve_action_kind(&self, action_type: &str) -> GuiActionKind {
        if self.enabled {
            GuiActionKind::from_primitive_action_type(action_type)
        } else {
            GuiActionKind::from_action_type(action_type)
        }
    }
}

/// Redacted placeholder surfaced in events/logs/summaries in place of a
/// password / secure-entry field's value (Task 6.2, Requirement 5/15). Chosen to
/// contain the `[redacted]` token so it is rejected by
/// [`GuiPayloadVault::insert`] (the value is never echoed AND the secret flag is
/// forced downstream), and so it reads as a legible secret marker.
pub const GUI_SECRET_FIELD_PLACEHOLDER: &str = "[secret] [redacted]";

/// Detect whether a resolved/proposed target is a password or secure-entry
/// field from its role/label descriptor (Task 6.2, Requirement 5/15).
///
/// AT-SPI exposes a secure text entry with the role `"password text"`; some
/// toolkits use `"password"` / a `secure`/`protected` qualifier in the role
/// string. A control whose descriptor indicates a secure entry MUST be treated
/// as secret: focusing it, and any typed payload destined for it, is routed
/// through the payload vault and never logged/echoed. The match is conservative
/// — it keys on the secure-entry role signal, not benign labels like
/// "Password Manager" — so a normal text field is never mis-flagged. Detection
/// reads ONLY the sanitized role/label descriptor; it never sources a raw
/// secret. `secure_state` is a control-model secure/protected flag when the
/// observation layer can supply one (currently always `false` until a state
/// flag is wired), and forces the secret treatment when set.
pub fn is_password_or_secure_field(role: &str, label: &str, secure_state: bool) -> bool {
    if secure_state {
        return true;
    }
    let role_l = role.trim().to_ascii_lowercase();
    if role_l.contains("password")
        || role_l.contains("passphrase")
        || role_l.contains("secure")
        || role_l.contains("protected")
    {
        return true;
    }
    // Some a11y bridges fold the secure-entry signal into the control label
    // rather than the role. Match only an explicit secure-entry label so benign
    // text such as "Confirm your account" is never mis-flagged.
    let label_l = label.trim().to_ascii_lowercase();
    label_l == "password"
        || label_l == "passphrase"
        || label_l.contains("password field")
        || label_l.contains("password input")
        || label_l.contains("secure entry")
}

/// DPI/multi-monitor-aware transform of a logical control rectangle into the
/// physical (scaled) coordinates the input backend needs (Task 6.1,
/// Requirement 5). The transform is derived ONLY from real observed data — the
/// resolved-target's logical bounds and the observed `monitor_layout` (each
/// monitor's logical bounds + `scale_factor`). It never invents coordinates.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiPhysicalBounds {
    /// The id of the monitor the bounds were resolved to.
    pub monitor_id: String,
    /// The monitor's DPI scale factor applied to the transform.
    pub scale_factor: f64,
    /// Physical bounds relative to the target monitor's top-left (logical offset
    /// within the monitor, multiplied by the monitor scale). This is the
    /// unambiguous, layout-independent form a per-monitor backend uses.
    pub monitor_local: GuiBounds,
    /// Physical bounds in the global physical coordinate space. The monitor's
    /// physical origin is the cumulative physical extent of monitors positioned
    /// strictly to its left (x) / above it (y) — a documented horizontal/vertical
    /// layout model that is exact for side-by-side and stacked arrangements.
    pub global_physical: GuiBounds,
}

/// Select the monitor a logical control belongs to. Prefers an explicit
/// `monitor_id`, then the monitor whose logical bounds contain the control's
/// center, then the primary monitor, then the first monitor.
pub fn select_target_monitor<'a>(
    monitors: &'a [GuiMonitorSummary],
    bounds: &GuiBounds,
    monitor_id: Option<&str>,
) -> Option<&'a GuiMonitorSummary> {
    if monitors.is_empty() {
        return None;
    }
    if let Some(id) = monitor_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(found) = monitors.iter().find(|monitor| monitor.id == id) {
            return Some(found);
        }
    }
    let center_x = bounds.x + bounds.width / 2;
    let center_y = bounds.y + bounds.height / 2;
    if let Some(found) = monitors.iter().find(|monitor| {
        let mb = &monitor.bounds;
        center_x >= mb.x
            && center_x < mb.x + mb.width
            && center_y >= mb.y
            && center_y < mb.y + mb.height
    }) {
        return Some(found);
    }
    monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.first())
}

/// Convert logical control `bounds` to DPI/multi-monitor-aware physical bounds
/// for the monitor they belong to. Returns `None` when no monitor layout is
/// available (degraded observation) — the caller then leaves bounds untouched
/// rather than inventing a transform.
pub fn physical_bounds_for_target(
    monitors: &[GuiMonitorSummary],
    bounds: &GuiBounds,
    monitor_id: Option<&str>,
) -> Option<GuiPhysicalBounds> {
    let monitor = select_target_monitor(monitors, bounds, monitor_id)?;
    // A non-positive/absent scale is treated as 1.0 so we never zero-out or
    // invert real bounds.
    let scale = if monitor.scale_factor.is_finite() && monitor.scale_factor > 0.0 {
        monitor.scale_factor
    } else {
        1.0
    };
    let scale_i = |v: i32| (v as f64 * scale).round() as i32;

    let local_x = bounds.x - monitor.bounds.x;
    let local_y = bounds.y - monitor.bounds.y;
    let monitor_local = GuiBounds {
        x: scale_i(local_x),
        y: scale_i(local_y),
        width: scale_i(bounds.width),
        height: scale_i(bounds.height),
    };

    // Physical origin of this monitor: cumulative physical extent of monitors
    // strictly to its left (x-axis) / strictly above it (y-axis).
    let mut origin_x = 0i32;
    let mut origin_y = 0i32;
    for other in monitors {
        if other.id == monitor.id {
            continue;
        }
        let other_scale = if other.scale_factor.is_finite() && other.scale_factor > 0.0 {
            other.scale_factor
        } else {
            1.0
        };
        if other.bounds.x < monitor.bounds.x {
            origin_x += (other.bounds.width as f64 * other_scale).round() as i32;
        }
        if other.bounds.y < monitor.bounds.y {
            origin_y += (other.bounds.height as f64 * other_scale).round() as i32;
        }
    }
    let global_physical = GuiBounds {
        x: origin_x + monitor_local.x,
        y: origin_y + monitor_local.y,
        width: monitor_local.width,
        height: monitor_local.height,
    };

    Some(GuiPhysicalBounds {
        monitor_id: monitor.id.clone(),
        scale_factor: scale,
        monitor_local,
        global_physical,
    })
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
            // Task 6.1 typed primitives. Field-state primitives need fill;
            // checkbox/dialog/in-app-search need click (or fill for the typed
            // query). All are supported when the backend can act on controls.
            GuiActionKind::ClearField | GuiActionKind::SelectAll => self.fill_field,
            GuiActionKind::SetCheckbox | GuiActionKind::CloseDialog => self.click_control,
            GuiActionKind::InAppSearch => self.click_control || self.fill_field,
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

    /// The primary blocker preventing this backend from executing actions,
    /// independent of a specific action kind (used by the preconditions
    /// health-gate, Task 1.4 / Requirement 25). Prefers the global-halt reason,
    /// then the first recorded blocker, then the backend selection reason.
    pub fn primary_backend_blocker(&self) -> String {
        if self.global_halt_engaged {
            return self
                .halt_reason
                .clone()
                .unwrap_or_else(|| "global safety halt is engaged".into());
        }
        if let Some(blocker) = self.blockers.first() {
            return blocker.clone();
        }
        if !self.backend_selection_reason.trim().is_empty() {
            return self.backend_selection_reason.clone();
        }
        "GUI action backend is unavailable".into()
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
    surface_scroll_primitives: bool,
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
            | GuiActionKind::ClearField
            | GuiActionKind::SelectAll
            | GuiActionKind::SetCheckbox
            | GuiActionKind::CloseDialog
            | GuiActionKind::InAppSearch
    );
    if control_action {
        if target_resolution.status != "resolved" {
            blockers.push(format!("target resolution is {}", target_resolution.status));
        }
        // Task 4 (Issue #5) / Task 5 (Issue #4): a `Scroll` or a `PressKey`/
        // `Hotkey` is a SURFACE action when the `gui_cog_primitives` flag is ON —
        // it acts on the active focused window/viewport and has NO named control,
        // so it carries no `resolved_target`, no trusted bounds, and no stable
        // target identity. For a surface action we keep the `status == "resolved"`
        // requirement above (an unobservable/unfocused surface still blocks) but
        // SKIP the control-identity sub-checks below. While the flag is OFF,
        // `surface_action` is false and these actions stay strict control actions
        // exactly as before (byte-for-byte).
        let surface_action = (surface_scroll_primitives
            && matches!(
                action_kind,
                GuiActionKind::Scroll | GuiActionKind::PressKey | GuiActionKind::Hotkey
            ))
            || (matches!(action_kind, GuiActionKind::TypeText | GuiActionKind::FillField)
                && proposal.target_label.as_deref()
                    == Some(super::llm_planner::BROWSER_ADDRESSBAR_HINT));
        if !surface_action {
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
            if target.bounds.is_none() && !matches!(action_kind, GuiActionKind::PressKey | GuiActionKind::Hotkey | GuiActionKind::ClearField | GuiActionKind::SelectAll) {
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

#[cfg(test)]
mod primitives_tests {
    //! Task 6.1 (Requirement 5) T1 unit tests: each primitive maps to the
    //! correct executor action kind, the DPI/multi-monitor bounds transform is
    //! correct for ≥2 monitors with different scales, and the `gui_cog_primitives`
    //! flag is OFF by default (flag OFF = unchanged mapping).
    use super::*;
    use crate::agent::gui_cognition::perception::{GuiBounds, GuiMonitorSummary};

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| {
            owned
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        }
    }

    fn monitor(id: &str, x: i32, y: i32, w: i32, h: i32, scale: f64, primary: bool) -> GuiMonitorSummary {
        GuiMonitorSummary {
            id: id.into(),
            name: Some(id.into()),
            bounds: GuiBounds {
                x,
                y,
                width: w,
                height: h,
            },
            work_area: None,
            scale_factor: scale,
            primary,
        }
    }

    // ── Flag plumbing (mirrors prior flags) ─────────────────────────────────

    #[test]
    fn primitives_flag_defaults_off() {
        assert!(!GuiPrimitivesConfig::default().is_enabled());
        assert!(GuiPrimitivesConfig::enabled().is_enabled());
        assert!(!GuiPrimitivesConfig::disabled().is_enabled());
    }

    #[test]
    fn primitives_flag_off_unless_truthy_env() {
        // Unset env → OFF.
        assert!(!GuiPrimitivesConfig::from_env_lookup(lookup_from(&[])).is_enabled());
        for raw in ["0", "false", "no", "off", "", "maybe"] {
            let cfg =
                GuiPrimitivesConfig::from_env_lookup(lookup_from(&[(PRIMITIVES_ENV_FLAG, raw)]));
            assert!(!cfg.is_enabled(), "value {raw:?} must keep primitives OFF");
        }
    }

    #[test]
    fn primitives_flag_on_when_truthy_env() {
        for raw in ["1", "true", "YES", "On", " on "] {
            let cfg =
                GuiPrimitivesConfig::from_env_lookup(lookup_from(&[(PRIMITIVES_ENV_FLAG, raw)]));
            assert!(cfg.is_enabled(), "value {raw:?} must enable primitives");
        }
    }

    #[test]
    fn primitives_default_on_path_and_rollback() {
        // Absent / truthy keep it ON.
        assert!(GuiPrimitivesConfig::from_env_lookup_default_on(lookup_from(&[])).is_enabled());
        for raw in ["1", "true", "YES", "On", "anything-else"] {
            let cfg = GuiPrimitivesConfig::from_env_lookup_default_on(lookup_from(&[(
                PRIMITIVES_ENV_FLAG,
                raw,
            )]));
            assert!(cfg.is_enabled(), "value {raw:?} must keep default-on ON");
        }
        // Explicit falsy = rollback.
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            let cfg = GuiPrimitivesConfig::from_env_lookup_default_on(lookup_from(&[(
                PRIMITIVES_ENV_FLAG,
                raw,
            )]));
            assert!(!cfg.is_enabled(), "value {raw:?} must roll back to OFF");
        }
    }

    #[test]
    fn primitives_flag_roundtrips_through_serde() {
        let cfg = GuiPrimitivesConfig::enabled();
        let json = serde_json::to_value(cfg).expect("serialize");
        assert_eq!(json["enabled"], serde_json::json!(true));
        let back: GuiPrimitivesConfig = serde_json::from_value(json).expect("deserialize");
        assert!(back.is_enabled());
        // Absent field → serde default OFF.
        let empty: GuiPrimitivesConfig =
            serde_json::from_value(serde_json::json!({})).expect("deserialize empty");
        assert!(!empty.is_enabled());
    }

    // ── Each primitive maps to the correct executor action kind ─────────────

    #[test]
    fn flag_off_mapping_is_byte_for_byte_legacy() {
        // While OFF the resolve mapping is IDENTICAL to the legacy
        // `from_action_type` for every input, including the previously
        // unsupported verbs (which legacy mapped to ClickControl).
        let off = GuiPrimitivesConfig::disabled();
        for verb in [
            "open_app",
            "switch_window",
            "focus_field",
            "type_text",
            "fill_field",
            "click_control",
            "press_key",
            "hotkey",
            "scroll",
            "copy",
            "paste",
            "clear",
            "select_all",
            "checkbox",
            "close_dialog",
            "in_app_search",
            "totally-unknown-verb",
        ] {
            assert_eq!(
                off.resolve_action_kind(verb),
                GuiActionKind::from_action_type(verb),
                "OFF mapping for {verb:?} must equal legacy from_action_type"
            );
        }
    }

    #[test]
    fn flag_on_maps_each_primitive_to_correct_kind() {
        let on = GuiPrimitivesConfig::enabled();
        let cases = [
            ("focus_field", GuiActionKind::FocusField),
            ("type_text", GuiActionKind::TypeText),
            ("clear", GuiActionKind::ClearField),
            ("clear_field", GuiActionKind::ClearField),
            ("select_all", GuiActionKind::SelectAll),
            ("select", GuiActionKind::SelectAll),
            ("copy", GuiActionKind::Copy),
            ("paste", GuiActionKind::Paste),
            ("press_key", GuiActionKind::PressKey),
            ("scroll", GuiActionKind::Scroll),
            ("click_control", GuiActionKind::ClickControl),
            ("checkbox", GuiActionKind::SetCheckbox),
            ("set_checkbox", GuiActionKind::SetCheckbox),
            ("close_dialog", GuiActionKind::CloseDialog),
            ("dialog_close", GuiActionKind::CloseDialog),
            ("in_app_search", GuiActionKind::InAppSearch),
            ("search", GuiActionKind::InAppSearch),
        ];
        for (verb, expected) in cases {
            assert_eq!(
                on.resolve_action_kind(verb),
                expected,
                "ON mapping for {verb:?} must be {expected:?}"
            );
        }
    }

    #[test]
    fn flag_on_keeps_legacy_verbs_identical() {
        // Verbs recognized by the legacy mapping resolve identically whether the
        // flag is ON or OFF — only the previously-defaulting primitives change.
        let on = GuiPrimitivesConfig::enabled();
        for verb in [
            "open_app",
            "switch_window",
            "focus_field",
            "type_text",
            "fill_field",
            "click_control",
            "press_key",
            "hotkey",
            "scroll",
            "copy",
            "paste",
        ] {
            assert_eq!(
                on.resolve_action_kind(verb),
                GuiActionKind::from_action_type(verb),
                "recognized legacy verb {verb:?} must be stable across the flag"
            );
        }
    }

    #[test]
    fn new_primitives_supported_by_capable_backend_and_route_via_input_backend() {
        // Each new primitive is supported by a fully-capable (uinput-backed)
        // backend, confirming it routes through the Wayland-capable input path
        // rather than being dropped as unsupported.
        let caps = GuiExecutorCapabilityMatrix::all_available();
        for kind in [
            GuiActionKind::ClearField,
            GuiActionKind::SelectAll,
            GuiActionKind::SetCheckbox,
            GuiActionKind::CloseDialog,
            GuiActionKind::InAppSearch,
        ] {
            assert!(caps.supports(&kind), "{kind:?} must be supported by a full backend");
        }
        // Observe-only backend cannot act on these primitives.
        let observe = GuiExecutorCapabilityMatrix::observe_only();
        for kind in [
            GuiActionKind::ClearField,
            GuiActionKind::SetCheckbox,
            GuiActionKind::CloseDialog,
        ] {
            assert!(!observe.supports(&kind), "observe-only must not support {kind:?}");
        }
    }

    #[test]
    fn wayland_backend_selection_supports_new_primitives() {
        // A healthy Wayland session selecting the uinput backend exposes the
        // full capability matrix, so each Task 6.1 primitive routes through it.
        let selection = select_gui_action_backend(&GuiBackendProbeInput {
            global_halt_engaged: false,
            halt_reason: None,
            automation_enabled: true,
            orchestrator_available: true,
            session_type: "wayland".into(),
            vision_sidecar: "ready".into(),
            uinput_daemon: "running".into(),
            xdotool_available: false,
            xdotool_display_usable: false,
            ydotool_available: false,
            ydotool_permission_ok: false,
            uinput_available: true,
            uinput_socket_path: Some("/run/kria/uinput.sock".into()),
            uinput_socket_accessible: true,
        });
        assert_eq!(selection.input_backend_kind, "uinput");
        assert!(selection.can_execute_actions);
        for kind in [
            GuiActionKind::ClearField,
            GuiActionKind::SelectAll,
            GuiActionKind::SetCheckbox,
            GuiActionKind::CloseDialog,
            GuiActionKind::InAppSearch,
        ] {
            assert!(
                selection.capabilities.supports(&kind),
                "uinput backend must support {kind:?}"
            );
        }
    }

    // ── DPI / multi-monitor bounds transform ────────────────────────────────

    #[test]
    fn bounds_transform_returns_none_without_monitor_layout() {
        let bounds = GuiBounds {
            x: 10,
            y: 10,
            width: 100,
            height: 20,
        };
        assert!(physical_bounds_for_target(&[], &bounds, None).is_none());
    }

    #[test]
    fn bounds_transform_scales_on_primary_hidpi_monitor() {
        // Single 2.0x monitor: logical → physical doubles, origin at 0.
        let monitors = [monitor("HiDPI", 0, 0, 1920, 1080, 2.0, true)];
        let bounds = GuiBounds {
            x: 100,
            y: 50,
            width: 200,
            height: 40,
        };
        let physical = physical_bounds_for_target(&monitors, &bounds, None).expect("transform");
        assert_eq!(physical.monitor_id, "HiDPI");
        assert_eq!(physical.scale_factor, 2.0);
        assert_eq!(
            physical.monitor_local,
            GuiBounds {
                x: 200,
                y: 100,
                width: 400,
                height: 80
            }
        );
        // Primary monitor at logical origin → global == local.
        assert_eq!(physical.global_physical, physical.monitor_local);
    }

    #[test]
    fn bounds_transform_correct_for_two_monitors_with_different_scales() {
        // Monitor A: 1920x1080 @ 1.0 (primary, left). Monitor B: 2560x1440 @
        // 2.0 (right of A, logical origin x=1920).
        let monitors = [
            monitor("A", 0, 0, 1920, 1080, 1.0, true),
            monitor("B", 1920, 0, 2560, 1440, 2.0, false),
        ];

        // A control on monitor A (left, scale 1.0): identity transform.
        let on_a = GuiBounds {
            x: 100,
            y: 100,
            width: 50,
            height: 30,
        };
        let pa = physical_bounds_for_target(&monitors, &on_a, None).expect("transform A");
        assert_eq!(pa.monitor_id, "A");
        assert_eq!(pa.scale_factor, 1.0);
        assert_eq!(
            pa.monitor_local,
            GuiBounds {
                x: 100,
                y: 100,
                width: 50,
                height: 30
            }
        );
        assert_eq!(pa.global_physical, pa.monitor_local);

        // A control on monitor B (logical x=2000): selected by center
        // containment; local offset (80,100) scaled by 2.0 → (160,200); global
        // origin x = A.width * A.scale = 1920.
        let on_b = GuiBounds {
            x: 2000,
            y: 100,
            width: 200,
            height: 50,
        };
        let pb = physical_bounds_for_target(&monitors, &on_b, None).expect("transform B");
        assert_eq!(pb.monitor_id, "B");
        assert_eq!(pb.scale_factor, 2.0);
        assert_eq!(
            pb.monitor_local,
            GuiBounds {
                x: 160,
                y: 200,
                width: 400,
                height: 100
            }
        );
        assert_eq!(
            pb.global_physical,
            GuiBounds {
                x: 1920 + 160,
                y: 200,
                width: 400,
                height: 100
            }
        );
    }

    #[test]
    fn bounds_transform_prefers_explicit_monitor_id() {
        let monitors = [
            monitor("A", 0, 0, 1920, 1080, 1.0, true),
            monitor("B", 1920, 0, 2560, 1440, 2.0, false),
        ];
        // Bounds geometrically over A, but explicit monitor_id forces B.
        let bounds = GuiBounds {
            x: 10,
            y: 10,
            width: 40,
            height: 40,
        };
        let physical =
            physical_bounds_for_target(&monitors, &bounds, Some("B")).expect("transform");
        assert_eq!(physical.monitor_id, "B");
        assert_eq!(physical.scale_factor, 2.0);
    }

    #[test]
    fn bounds_transform_non_positive_scale_falls_back_to_identity() {
        let monitors = [monitor("Bad", 0, 0, 1920, 1080, 0.0, true)];
        let bounds = GuiBounds {
            x: 30,
            y: 40,
            width: 10,
            height: 20,
        };
        let physical = physical_bounds_for_target(&monitors, &bounds, None).expect("transform");
        assert_eq!(physical.scale_factor, 1.0);
        assert_eq!(physical.monitor_local, bounds);
    }

    // ── Task 6.3: tier classification + tier↔idempotent consistency ─────────

    use crate::agent::gui_cognition::llm_planner::default_idempotent_for;

    /// Every supported GREEN primitive step type (read-only / non-state-changing).
    const GREEN_PRIMITIVES: &[&str] = &[
        "Observe",
        "FocusField",
        "Scroll",
        "SelectAll",
        "InAppSearch",
        "SummarizeVisibleContent",
        "WaitForState",
        "VerifyState",
        "AskClarification",
        "SwitchWindow",
    ];

    /// Every supported YELLOW primitive step type (visible local state change).
    const YELLOW_PRIMITIVES: &[&str] = &[
        "TypeText",
        "ClearField",
        "Paste",
        "ClickControl",
        "SetCheckbox",
        "PressKey",
        "Copy",
        "CloseDialog",
        "OpenApp",
        "BrowserNavigate",
    ];

    #[test]
    fn every_supported_primitive_has_a_tier() {
        for st in GREEN_PRIMITIVES.iter().chain(YELLOW_PRIMITIVES) {
            assert!(primitive_tier(st).is_some(), "{st} must have a tier");
        }
    }

    #[test]
    fn green_primitives_are_green_and_yellow_are_yellow() {
        for st in GREEN_PRIMITIVES {
            assert_eq!(
                primitive_tier(st),
                Some(GuiPrimitiveTier::Green),
                "{st} must be GREEN"
            );
        }
        for st in YELLOW_PRIMITIVES {
            assert_eq!(
                primitive_tier(st),
                Some(GuiPrimitiveTier::Yellow),
                "{st} must be YELLOW"
            );
        }
    }

    #[test]
    fn tier_and_idempotent_are_consistent_for_every_primitive() {
        // Invariant 1: every GREEN primitive is idempotent (read-only / converges
        // to the same state with no extra side effect).
        for st in GREEN_PRIMITIVES {
            assert!(
                default_idempotent_for(st),
                "GREEN primitive {st} must be idempotent"
            );
        }
        // Invariant 2: every NON-idempotent primitive is YELLOW (a
        // state-mutating, non-converging action can never be GREEN).
        for st in GREEN_PRIMITIVES.iter().chain(YELLOW_PRIMITIVES) {
            if !default_idempotent_for(st) {
                assert_eq!(
                    primitive_tier(st),
                    Some(GuiPrimitiveTier::Yellow),
                    "non-idempotent primitive {st} must be YELLOW"
                );
            }
        }
        // The documented YELLOW-but-idempotent case: clearing a field mutates
        // visible state (→ YELLOW) yet re-running converges (→ idempotent). This
        // proves tier and idempotency are independent axes.
        assert_eq!(primitive_tier("ClearField"), Some(GuiPrimitiveTier::Yellow));
        assert!(default_idempotent_for("ClearField"));
    }

    #[test]
    fn approval_gated_and_unknown_steps_are_not_primitive_tier() {
        // GREEN/YELLOW only here — destructive/approval-gated stay governed by
        // the safety/HITL gate and must NOT be classified into this band.
        for st in ["RequireApproval", "Save", "Download", "TotallyUnknownStep"] {
            assert!(
                primitive_tier(st).is_none(),
                "{st} must NOT be classified into the GREEN/YELLOW band"
            );
        }
    }

    #[test]
    fn tier_serializes_to_risk_model_token() {
        assert_eq!(GuiPrimitiveTier::Green.as_str(), "GREEN");
        assert_eq!(GuiPrimitiveTier::Yellow.as_str(), "YELLOW");
        assert_eq!(
            serde_json::to_value(GuiPrimitiveTier::Green).expect("serialize"),
            serde_json::json!("GREEN")
        );
        assert_eq!(
            serde_json::to_value(GuiPrimitiveTier::Yellow).expect("serialize"),
            serde_json::json!("YELLOW")
        );
        let back: GuiPrimitiveTier =
            serde_json::from_value(serde_json::json!("YELLOW")).expect("deserialize");
        assert_eq!(back, GuiPrimitiveTier::Yellow);
    }

    #[test]
    fn action_kind_tier_agrees_with_step_type_classifier() {
        // The executor action-kind tier agrees with the step-type classifier for
        // every concrete `GuiActionKind`. Hotkey/FillField have no 1:1 step type
        // and are asserted directly (YELLOW — they mutate visible state).
        let cases = [
            (GuiActionKind::SwitchWindow, GuiPrimitiveTier::Green),
            (GuiActionKind::FocusField, GuiPrimitiveTier::Green),
            (GuiActionKind::Scroll, GuiPrimitiveTier::Green),
            (GuiActionKind::SelectAll, GuiPrimitiveTier::Green),
            (GuiActionKind::InAppSearch, GuiPrimitiveTier::Green),
            (GuiActionKind::OpenApp, GuiPrimitiveTier::Yellow),
            (GuiActionKind::FillField, GuiPrimitiveTier::Yellow),
            (GuiActionKind::TypeText, GuiPrimitiveTier::Yellow),
            (GuiActionKind::ClickControl, GuiPrimitiveTier::Yellow),
            (GuiActionKind::PressKey, GuiPrimitiveTier::Yellow),
            (GuiActionKind::Hotkey, GuiPrimitiveTier::Yellow),
            (GuiActionKind::Copy, GuiPrimitiveTier::Yellow),
            (GuiActionKind::Paste, GuiPrimitiveTier::Yellow),
            (GuiActionKind::ClearField, GuiPrimitiveTier::Yellow),
            (GuiActionKind::SetCheckbox, GuiPrimitiveTier::Yellow),
            (GuiActionKind::CloseDialog, GuiPrimitiveTier::Yellow),
        ];
        for (kind, tier) in cases {
            assert_eq!(kind.primitive_tier(), tier, "{kind:?} tier mismatch");
            // Where the action kind has a 1:1 step-type name, the two classifiers
            // agree exactly.
            if let Some(st_tier) = primitive_tier(kind.as_str()) {
                assert_eq!(
                    st_tier,
                    kind.primitive_tier(),
                    "{kind:?}: step-type and action-kind classifiers disagree"
                );
            }
        }
    }
}
