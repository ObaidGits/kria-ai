//! Cognition Tools — OCR, Browser, IDE, and Session tools
//!
//! Registers tools for:
//! - OCR screen reading (tesseract-based)
//! - Browser cognition (CDP-based DOM interaction)
//! - IDE cognition (LSP-based diagnostics)
//! - Session management (workflow checkpointing)

use crate::agent::browser_cognition::BrowserCognitionEngine;
use crate::agent::ide_cognition::IdeCognitionEngine;
use crate::agent::ocr_engine::OcrEngine;
use crate::agent::psdg::PsdgHandle;
use crate::agent::workflow_continuation::WorkflowContinuationRuntime;
use crate::agent::workflow_session::SessionManager;
use crate::infra::ToolResult;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

// ─── OCR Tools ────────────────────────────────────────────────────────────────

struct ReadScreen;
#[async_trait]
impl ToolHandler for ReadScreen {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let engine = OcrEngine::new();
        let result = engine.read_screen().await;
        if result.success {
            ToolResult::ok(serde_json::json!({
                "text": result.text,
                "chars": result.text.len(),
                "evidence": result.evidence,
            }))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

struct CheckTextOnScreen;
#[async_trait]
impl ToolHandler for CheckTextOnScreen {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = params["text"].as_str().unwrap_or("");
        if text.is_empty() {
            return ToolResult::err("text is required");
        }
        let engine = OcrEngine::new();
        let visible = engine.text_visible_on_screen(text).await;
        ToolResult::ok(serde_json::json!({
            "text": text,
            "visible": visible,
        }))
    }
}

// ─── Browser Cognition Tools ──────────────────────────────────────────────────

/// Shared PSDG-aware browser engine factory.
///
/// Every browser tool uses this to ensure state is always persisted to the
/// WorldModelStore after each operation, regardless of which tool fires.
fn make_browser_engine(psdg: &Option<Arc<PsdgHandle>>) -> BrowserCognitionEngine {
    let mut engine = BrowserCognitionEngine::new();
    if let Some(ref h) = psdg {
        engine = engine.with_world_model((**h).clone());
    }
    engine
}

struct GetBrowserState {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for GetBrowserState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let engine = make_browser_engine(&self.psdg);
        let state = engine.get_state().await;
        ToolResult::ok(serde_json::to_value(state).unwrap_or(serde_json::Value::Null))
    }
}

struct LaunchBrowserWithDebugging;
#[async_trait]
impl ToolHandler for LaunchBrowserWithDebugging {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match BrowserCognitionEngine::launch_with_debugging().await {
            Ok(pid) => ToolResult::ok(serde_json::json!({
                "launched": true,
                "pid": pid,
                "cdp_port": 9222,
                "evidence": format!("Chrome launched with CDP on port 9222 (pid={})", pid),
            })),
            Err(e) => ToolResult::err(format!(
                "Failed to launch Chrome with debugging: {}. \
                 Install Chrome/Chromium or start it manually with: \
                 google-chrome --remote-debugging-port=9222",
                e
            )),
        }
    }
}

struct BrowserNavigateCdp {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for BrowserNavigateCdp {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let url = params["url"].as_str().unwrap_or("");
        if url.is_empty() {
            return ToolResult::err("url is required");
        }
        let engine = make_browser_engine(&self.psdg);
        let result = engine.navigate(url).await;
        if result.success {
            ToolResult::ok(
                serde_json::json!({"navigated": true, "url": url, "evidence": result.evidence}),
            )
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

struct GetPageText {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for GetPageText {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let engine = make_browser_engine(&self.psdg);
        let result = engine.get_page_text().await;
        if result.success {
            ToolResult::ok(result.data.unwrap_or(serde_json::Value::Null))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

struct BrowserClickElement {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for BrowserClickElement {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let selector = params["selector"].as_str().unwrap_or("");
        if selector.is_empty() {
            return ToolResult::err("selector is required");
        }
        let engine = make_browser_engine(&self.psdg);
        let result = engine.click_element(selector).await;
        if result.success {
            ToolResult::ok(serde_json::json!({"clicked": true, "evidence": result.evidence}))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

struct BrowserFillField {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for BrowserFillField {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let label = params["label"].as_str().unwrap_or("");
        let value = params["value"].as_str().unwrap_or("");
        if label.is_empty() || value.is_empty() {
            return ToolResult::err("label and value are required");
        }
        let engine = make_browser_engine(&self.psdg);
        let result = engine.fill_field(label, value).await;
        if result.success {
            ToolResult::ok(serde_json::json!({"filled": true, "evidence": result.evidence}))
        } else {
            ToolResult::err(result.evidence)
        }
    }
}

// ─── IDE Cognition Tools ──────────────────────────────────────────────────────

struct GetIdeState {
    psdg: Option<Arc<PsdgHandle>>,
}
#[async_trait]
impl ToolHandler for GetIdeState {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let mut engine = IdeCognitionEngine::new();
        if let Some(ref h) = self.psdg {
            engine = engine.with_world_model((**h).clone());
        }
        let state = engine.get_state().await;
        ToolResult::ok(serde_json::to_value(state).unwrap_or(serde_json::Value::Null))
    }
}

struct CheckFileDiagnostics;
#[async_trait]
impl ToolHandler for CheckFileDiagnostics {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let file_path = params["file_path"].as_str().unwrap_or("");
        if file_path.is_empty() {
            return ToolResult::err("file_path is required");
        }
        // SECURITY: Validate that the path exists and is a regular file.
        // This prevents path traversal and injection attacks.
        let path = std::path::Path::new(file_path);
        if !path.exists() {
            return ToolResult::err(format!("File does not exist: {}", file_path));
        }
        if !path.is_file() {
            return ToolResult::err(format!("Path is not a file: {}", file_path));
        }
        // Only allow files in safe locations (generated files or temp)
        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Cannot resolve path: {}", e)),
        };
        let canonical_str = canonical.to_string_lossy();
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
        let allowed_prefixes = [
            format!("{}/.kria/generated/", home),
            "/tmp/kria_".to_string(),
            "/tmp/kria-".to_string(),
        ];
        if !allowed_prefixes
            .iter()
            .any(|p| canonical_str.starts_with(p.as_str()))
        {
            return ToolResult::err(format!(
                "File path '{}' is outside allowed directories. \
                 Only files in ~/.kria/generated/ or /tmp/kria_* are allowed.",
                file_path
            ));
        }
        let engine = IdeCognitionEngine::new();
        let diagnostics = engine.check_file(file_path).await;
        let error_count = diagnostics.iter().filter(|d| d.severity == "error").count();
        let warning_count = diagnostics
            .iter()
            .filter(|d| d.severity == "warning")
            .count();
        ToolResult::ok(serde_json::json!({
            "file": file_path,
            "error_count": error_count,
            "warning_count": warning_count,
            "diagnostics": serde_json::to_value(&diagnostics).unwrap_or(serde_json::Value::Array(vec![])),
        }))
    }
}

// ─── Session Management Tools ─────────────────────────────────────────────────

struct ListSessions;
#[async_trait]
impl ToolHandler for ListSessions {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let manager = SessionManager::new();
        let sessions = manager.list_sessions();
        let summaries: Vec<serde_json::Value> = sessions
            .iter()
            .take(10)
            .map(|s| {
                serde_json::json!({
                    "session_id": s.session_id,
                    "intent": s.user_intent,
                    "complete": s.complete,
                    "steps": s.completed_steps.len(),
                    "artifacts": s.artifacts.len(),
                    "summary": s.summary(),
                })
            })
            .collect();
        ToolResult::ok(serde_json::json!({
            "count": sessions.len(),
            "sessions": summaries,
        }))
    }
}

struct GetSession;
#[async_trait]
impl ToolHandler for GetSession {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let session_id = params["session_id"].as_str().unwrap_or("");
        if session_id.is_empty() {
            return ToolResult::err("session_id is required");
        }
        let manager = SessionManager::new();
        match manager.load(session_id) {
            Some(session) => {
                ToolResult::ok(serde_json::to_value(session).unwrap_or(serde_json::Value::Null))
            }
            None => ToolResult::err(format!("Session '{}' not found", session_id)),
        }
    }
}

struct FindContinuableSessions;
#[async_trait]
impl ToolHandler for FindContinuableSessions {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let manager = SessionManager::new();
        let sessions = manager.find_continuable();
        let summaries: Vec<serde_json::Value> = sessions
            .iter()
            .map(|s| {
                serde_json::json!({
                    "session_id": s.session_id,
                    "intent": s.user_intent,
                    "steps_completed": s.completed_steps.len(),
                    "continuation_hint": s.continuation_hint,
                    "error": s.error,
                })
            })
            .collect();
        ToolResult::ok(serde_json::json!({
            "count": sessions.len(),
            "sessions": summaries,
        }))
    }
}

// ─── Resume Workflow Tool ─────────────────────────────────────────────────────

/// Tool handler: resume a previously paused workflow checkpoint by session ID.
///
/// Calls `WorkflowContinuationRuntime::resume_workflow()` and returns the
/// `ResumeResult` as structured JSON. The LLM can invoke this tool whenever
/// the user asks to continue a previously interrupted task.
struct ResumeWorkflow {
    continuation_runtime: Arc<WorkflowContinuationRuntime>,
}

#[async_trait]
impl ToolHandler for ResumeWorkflow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let session_id = params["session_id"].as_str().unwrap_or("");
        if session_id.is_empty() {
            return ToolResult::err("session_id is required");
        }
        let result = self.continuation_runtime.resume_workflow(session_id);
        ToolResult::ok(serde_json::json!({
            "success": result.success,
            "summary": result.summary,
            "next_action": format!("{:?}", result.next_action),
            "continuation_hint": result.session
                .as_ref()
                .and_then(|s| s.continuation_hint.clone()),
            "steps_completed": result.session
                .as_ref()
                .map(|s| s.completed_steps.len())
                .unwrap_or(0),
        }))
    }
}

// ─── Registration ─────────────────────────────────────────────────────────────

/// Register all cognition tools.
///
/// Pass `psdg` to enable persistent browser/IDE state writing to WorldModelStore.
/// Pass `continuation_runtime` to enable the `resume_workflow` tool.
/// All browser and IDE operations will automatically persist their state after
/// each call, enabling cross-turn semantic continuity.
pub fn register(
    reg: &ToolRegistry,
    psdg: Option<PsdgHandle>,
    continuation_runtime: Option<Arc<WorkflowContinuationRuntime>>,
) {
    // OCR tools
    reg.register(
        ToolDef {
            name: "read_screen_text".into(),
            description: "Read all visible text from the screen using OCR (tesseract). Returns the extracted text. Works on X11 and XWayland.".into(),
            category: "gui_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(ReadScreen),
    );

    reg.register(
        ToolDef {
            name: "check_text_on_screen".into(),
            description:
                "Check if specific text is visible on screen using OCR. Returns true/false.".into(),
            category: "gui_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![param(
                "text",
                "string",
                "Text to search for on screen",
                true,
            )],
        },
        std::sync::Arc::new(CheckTextOnScreen),
    );

    // Wrap psdg in Arc so we can share it across all browser/IDE handlers.
    let psdg = psdg.map(Arc::new);

    // Browser cognition tools
    reg.register(
        ToolDef {
            name: "get_browser_state".into(),
            description: "Get the current browser state: URL, title, tab count. Requires Chrome running with --remote-debugging-port=9222.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(GetBrowserState { psdg: psdg.clone() }),
    );

    reg.register(
        ToolDef {
            name: "launch_browser_with_debugging".into(),
            description: "Launch Chrome/Chromium with remote debugging enabled (port 9222). Required before using browser_navigate_cdp, get_page_text, browser_click_element, or browser_fill_field.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
            parameters: vec![],
        },
        std::sync::Arc::new(LaunchBrowserWithDebugging),
    );

    reg.register(
        ToolDef {
            name: "browser_navigate_cdp".into(),
            description: "Navigate the browser to a URL using Chrome DevTools Protocol. More reliable than xdg-open for complex navigation.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
            parameters: vec![
                param("url", "string", "URL to navigate to", true),
            ],
        },
        std::sync::Arc::new(BrowserNavigateCdp { psdg: psdg.clone() }),
    );

    reg.register(
        ToolDef {
            name: "get_page_text".into(),
            description: "Get the text content of the current browser page. Requires Chrome with CDP.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(GetPageText { psdg: psdg.clone() }),
    );

    reg.register(
        ToolDef {
            name: "browser_click_element".into(),
            description: "Click an element in the browser by CSS selector or text content. Requires Chrome with CDP.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
            parameters: vec![
                param("selector", "string", "CSS selector or text content of element to click", true),
            ],
        },
        std::sync::Arc::new(BrowserClickElement { psdg: psdg.clone() }),
    );

    reg.register(
        ToolDef {
            name: "browser_fill_field".into(),
            description: "Fill a form field in the browser by label or placeholder text. Requires Chrome with CDP.".into(),
            category: "browser_cognition".into(),
            default_tier: crate::safety::RiskLevel::Yellow,
            min_tier: "standard",
            parameters: vec![
                param("label", "string", "Label or placeholder of the field to fill", true),
                param("value", "string", "Value to enter", true),
            ],
        },
        std::sync::Arc::new(BrowserFillField { psdg: psdg.clone() }),
    );

    // IDE cognition tools
    reg.register(
        ToolDef {
            name: "get_ide_state".into(),
            description: "Get the current IDE state: active file, workspace, diagnostics. Works with VS Code.".into(),
            category: "ide_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(GetIdeState { psdg: psdg.clone() }),
    );

    reg.register(
        ToolDef {
            name: "check_file_diagnostics".into(),
            description: "Check a file for syntax errors and warnings using the appropriate language tool (py_compile, rustc, node --check).".into(),
            category: "ide_cognition".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![
                param("file_path", "string", "Absolute path to the file to check", true),
            ],
        },
        std::sync::Arc::new(CheckFileDiagnostics),
    );

    // Session management tools
    reg.register(
        ToolDef {
            name: "list_workflow_sessions".into(),
            description: "List recent workflow sessions with their status and summaries.".into(),
            category: "session_management".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(ListSessions),
    );

    reg.register(
        ToolDef {
            name: "get_workflow_session".into(),
            description: "Get details of a specific workflow session by ID.".into(),
            category: "session_management".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![param(
                "session_id",
                "string",
                "Session ID to retrieve",
                true,
            )],
        },
        std::sync::Arc::new(GetSession),
    );

    reg.register(
        ToolDef {
            name: "find_continuable_sessions".into(),
            description:
                "Find workflow sessions that can be continued (failed with a continuation hint)."
                    .into(),
            category: "session_management".into(),
            default_tier: crate::safety::RiskLevel::Green,
            min_tier: "lite",
            parameters: vec![],
        },
        std::sync::Arc::new(FindContinuableSessions),
    );

    // Batch 2 Step 3: resume_workflow tool — requires WorkflowContinuationRuntime.
    if let Some(rt) = continuation_runtime {
        reg.register(
            ToolDef {
                name: "resume_workflow".into(),
                description: "Resume a previously paused or interrupted workflow checkpoint by \
                               session ID. Use find_continuable_sessions first to list available \
                               sessions, then call this with the session_id to resume."
                    .into(),
                category: "session_management".into(),
                default_tier: crate::safety::RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![param(
                    "session_id",
                    "string",
                    "Session ID of the paused workflow to resume",
                    true,
                )],
            },
            std::sync::Arc::new(ResumeWorkflow {
                continuation_runtime: rt,
            }),
        );
        tracing::info!(target: "cognition_tools", "Registered resume_workflow tool");
    }

    let psdg_status = if psdg.is_some() {
        "PSDG-persistent"
    } else {
        "ephemeral"
    };
    tracing::info!(target: "cognition_tools", mode = psdg_status, "Registered 13+ cognition tools (OCR, browser, IDE, session, resume)");
}
