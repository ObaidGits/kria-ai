//! AT-SPI GUI Interaction Tools
//!
//! Exposes the AT-SPI engine as KRIA tools for:
//! - Clicking buttons and UI elements by semantic name
//! - Filling form fields
//! - Detecting and dismissing dialogs/popups
//! - Checking desktop state
//! - Verifying app responsiveness
//! - Accessibility capability detection
//! - Accessibility doctor diagnostics

use crate::agent::atspi_engine::{detect_capabilities, AtSpiEngine, DesktopState};
use crate::infra::ToolResult;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

// ─── ClickElement ─────────────────────────────────────────────────────────────

struct ClickElement;

#[async_trait]
impl ToolHandler for ClickElement {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let role = params["role"].as_str().unwrap_or("push button");
        let name = params["name"].as_str().unwrap_or("");
        if name.is_empty() {
            return ToolResult::err("name is required");
        }

        let engine = AtSpiEngine::new();
        let result = engine.click_element(role, name).await;

        if result.success {
            ToolResult::ok(serde_json::json!({
                "clicked": true,
                "role": role,
                "name": name,
                "evidence": result.evidence,
                "failure_reason": result.failure_reason.as_ref().map(|r| r.to_string()),
            }))
        } else {
            // Return structured failure with remediation hints
            let failure_str = result
                .failure_reason
                .as_ref()
                .map(|r| r.to_string())
                .unwrap_or_else(|| result.evidence.clone());
            ToolResult::err(failure_str)
        }
    }
}

// ─── FillField ────────────────────────────────────────────────────────────────

struct FillField;

#[async_trait]
impl ToolHandler for FillField {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let label = params["label"].as_str().unwrap_or("");
        let value = params["value"].as_str().unwrap_or("");
        if label.is_empty() || value.is_empty() {
            return ToolResult::err("label and value are required");
        }

        let engine = AtSpiEngine::new();
        let result = engine.fill_field(label, value).await;

        if result.success {
            ToolResult::ok(serde_json::json!({
                "filled": true,
                "label": label,
                "evidence": result.evidence,
            }))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

// ─── DetectDialog ─────────────────────────────────────────────────────────────

struct DetectDialog;

#[async_trait]
impl ToolHandler for DetectDialog {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let engine = AtSpiEngine::new();
        let dialog = engine.detect_dialog().await;

        match dialog {
            Some(el) => ToolResult::ok(serde_json::json!({
                "dialog_found": true,
                "role": el.role,
                "name": el.name,
                "path": el.path,
            })),
            None => ToolResult::ok(serde_json::json!({
                "dialog_found": false,
            })),
        }
    }
}

// ─── DismissDialog ────────────────────────────────────────────────────────────

struct DismissDialog;

#[async_trait]
impl ToolHandler for DismissDialog {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let engine = AtSpiEngine::new();
        let result = engine.dismiss_dialog().await;

        if result.success {
            ToolResult::ok(serde_json::json!({
                "dismissed": true,
                "evidence": result.evidence,
            }))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

// ─── GetDesktopState ──────────────────────────────────────────────────────────

struct GetDesktopState;

#[async_trait]
impl ToolHandler for GetDesktopState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let state = DesktopState::capture().await;
        ToolResult::ok(serde_json::to_value(state).unwrap_or(serde_json::Value::Null))
    }
}

// ─── CheckAppResponding ───────────────────────────────────────────────────────

struct CheckAppResponding;

#[async_trait]
impl ToolHandler for CheckAppResponding {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let app_name = params["app_name"].as_str().unwrap_or("");
        if app_name.is_empty() {
            return ToolResult::err("app_name is required");
        }

        let engine = AtSpiEngine::new();
        let responding = engine.is_app_responding(app_name).await;

        ToolResult::ok(serde_json::json!({
            "app_name": app_name,
            "responding": responding,
        }))
    }
}

// ─── FindElements ─────────────────────────────────────────────────────────────

struct FindElements;

#[async_trait]
impl ToolHandler for FindElements {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let role = params["role"].as_str().unwrap_or("push button");
        let name_contains = params["name_contains"].as_str();

        let engine = AtSpiEngine::new();
        let elements = engine.find_elements(role, name_contains).await;

        ToolResult::ok(serde_json::json!({
            "count": elements.len(),
            "elements": elements.iter().map(|e| serde_json::json!({
                "role": e.role,
                "name": e.name,
                "path": e.path,
            })).collect::<Vec<_>>(),
        }))
    }
}

// ─── AccessibilityCapabilities ───────────────────────────────────────────────

struct GetAccessibilityCapabilities;

#[async_trait]
impl ToolHandler for GetAccessibilityCapabilities {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let caps = detect_capabilities().await;
        ToolResult::ok(serde_json::to_value(&caps).unwrap_or(serde_json::Value::Null))
    }
}

// ─── AccessibilityDoctor ──────────────────────────────────────────────────────

struct AccessibilityDoctor;

#[async_trait]
impl ToolHandler for AccessibilityDoctor {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let diag = AtSpiEngine::accessibility_doctor().await;
        let summary = diag.summary();
        ToolResult::ok(serde_json::json!({
            "overall_pass": diag.overall_pass,
            "summary": summary,
            "checks": diag.checks.iter().map(|c| serde_json::json!({
                "name": c.name,
                "passed": c.passed,
                "detail": c.detail,
            })).collect::<Vec<_>>(),
            "recommendations": diag.recommendations,
        }))
    }
}

// ─── Registration ─────────────────────────────────────────────────────────────

pub fn register(reg: &ToolRegistry) {
    reg.register(
        ToolDef {
            name: "click_ui_element".into(),
            description: "Click a UI element by its accessible role and name. Works on both X11 and Wayland via AT-SPI. Use for buttons, menu items, checkboxes, etc.".into(),
            category: "gui_interaction".into(),
            parameters: vec![
                param("role", "string", "AT-SPI role: 'push button', 'menu item', 'check box', 'radio button', 'toggle button', 'combo box', 'list item'", false),
                param("name", "string", "Name or label of the element to click (partial match)", true),
            ],
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
        },
        std::sync::Arc::new(ClickElement),
    );

    reg.register(
        ToolDef {
            name: "fill_form_field".into(),
            description: "Fill a form text field by its label. Works on both X11 and Wayland via AT-SPI. Use for text inputs, search boxes, etc.".into(),
            category: "gui_interaction".into(),
            parameters: vec![
                param("label", "string", "Label or accessible name of the text field", true),
                param("value", "string", "Text to enter into the field", true),
            ],
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
        },
        std::sync::Arc::new(FillField),
    );

    reg.register(
        ToolDef {
            name: "detect_dialog".into(),
            description: "Detect if a dialog, popup, or alert is currently visible on screen. Returns dialog role and name if found.".into(),
            category: "gui_interaction".into(),
            parameters: vec![],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(DetectDialog),
    );

    reg.register(
        ToolDef {
            name: "dismiss_dialog".into(),
            description: "Dismiss the currently visible dialog by clicking Cancel, Close, or No. Falls back to OK/Yes if no dismiss button found.".into(),
            category: "gui_interaction".into(),
            parameters: vec![],
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
        },
        std::sync::Arc::new(DismissDialog),
    );

    reg.register(
        ToolDef {
            name: "get_desktop_state".into(),
            description: "Get the current desktop state: running applications, focused window, visible dialogs. Uses AT-SPI accessibility tree.".into(),
            category: "gui_interaction".into(),
            parameters: vec![],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(GetDesktopState),
    );

    reg.register(
        ToolDef {
            name: "check_app_responding".into(),
            description: "Check if an application is responding (not frozen). Returns true if the app is accessible via AT-SPI within 3 seconds.".into(),
            category: "gui_interaction".into(),
            parameters: vec![
                param("app_name", "string", "Name of the application to check", true),
            ],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(CheckAppResponding),
    );

    reg.register(
        ToolDef {
            name: "find_ui_elements".into(),
            description: "Find UI elements by role and optional name. Returns a list of matching elements with their roles, names, and paths.".into(),
            category: "gui_interaction".into(),
            parameters: vec![
                param("role", "string", "AT-SPI role to search for: 'push button', 'text', 'menu item', 'dialog', etc.", false),
                param("name_contains", "string", "Optional: filter by name containing this string", false),
            ],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(FindElements),
    );

    reg.register(
        ToolDef {
            name: "get_accessibility_capabilities".into(),
            description: "Detect whether AT-SPI accessibility is enabled and operational. Returns structured capability state with remediation commands if disabled.".into(),
            category: "gui_interaction".into(),
            parameters: vec![],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(GetAccessibilityCapabilities),
    );

    reg.register(
        ToolDef {
            name: "accessibility_doctor".into(),
            description: "Run the accessibility doctor — validates gsettings, AT-SPI bus, registry, app exposure, GTK_MODULES, Qt accessibility. Returns pass/fail for each check with remediation commands.".into(),
            category: "gui_interaction".into(),
            parameters: vec![],
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
        },
        std::sync::Arc::new(AccessibilityDoctor),
    );

    tracing::info!(target: "atspi_tools", "Registered 9 AT-SPI GUI interaction tools (including capability detection and doctor)");
}
