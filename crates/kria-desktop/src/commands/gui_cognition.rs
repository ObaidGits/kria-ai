use super::*;
use kria_core::agent::atspi_engine::{AtSpiEngine, AtSpiSnapshot, AtSpiSnapshotRequest};
use kria_core::agent::browser_cognition::{BrowserCognitionEngine, BrowserResult};
use kria_core::agent::gui_cognition::executor::{
    sanitized_execution_evidence, select_gui_action_backend, GuiActionBackendStatus,
    GuiActionExecution, GuiActionExecutor, GuiActionKind, GuiActionRequest, GuiBackendProbeInput,
    GuiExecutionMode,
};
use kria_core::agent::gui_cognition::llm_planner::{
    FixtureGuiLlmPlanner, GuiLlmPlanner, GuiLlmPlannerFixture, LlmBackendGuiPlanner,
};
use kria_core::agent::gui_cognition::perception::{
    sanitize_gui_text, GuiBounds, GuiObservationCacheSummary, GuiObservationSnapshot,
    GuiPerceptionProvider, GuiProbeResult,
};
use kria_core::agent::gui_cognition::planner::{
    classify_gui_cognition_prompt, GuiCognitionIntentKind,
};
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnRequest};
use kria_core::tools::vision_automation::{OmniParserClient, ScreenshotCapture};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc as StdArc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OnceCell};

fn unix_now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn short_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
        .chars()
        .take(16)
        .collect()
}

fn gui_cognition_event_payload(
    session_id: &str,
    turn_id: &str,
    workflow_id: &str,
    sequence: u64,
    event: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "session_id": session_id,
        "turn_id": turn_id,
        "workflow_id": workflow_id,
        "sequence": sequence,
        "timestamp_ms": unix_now_ms(),
        "event": event,
    })
}

fn probe_from_tool_result(result: ToolResult) -> GuiProbeResult {
    GuiProbeResult {
        success: result.success,
        data: result.data,
        error: result.error,
    }
}

fn execution_from_tool_result(tool: &str, result: ToolResult) -> GuiActionExecution {
    GuiActionExecution {
        success: result.success,
        tool: tool.into(),
        error: result.error,
        evidence: result
            .data
            .get("evidence")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

struct DesktopGuiPerceptionProvider<'a> {
    app_state: &'a AppState,
    screenshot_bytes: OnceCell<Result<StdArc<Vec<u8>>, String>>,
    atspi_snapshot: OnceCell<Result<StdArc<AtSpiSnapshot>, String>>,
    cache_policy: GuiObservationCachePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiObservationCachePolicy {
    Disabled,
    ObservePlanShort,
}

impl GuiObservationCachePolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::ObservePlanShort => "observe_plan_ttl_750ms",
        }
    }

    fn enabled(self) -> bool {
        matches!(self, Self::ObservePlanShort)
    }
}

fn gui_observation_cache_policy_for_prompt(message: &str) -> GuiObservationCachePolicy {
    match classify_gui_cognition_prompt(message).kind {
        GuiCognitionIntentKind::Observe
        | GuiCognitionIntentKind::AnalyzePlan
        | GuiCognitionIntentKind::BrowserSearchPlan
        | GuiCognitionIntentKind::FillFormPlan
        | GuiCognitionIntentKind::AmbiguityCheck
        | GuiCognitionIntentKind::TargetAvailabilityCheck
        | GuiCognitionIntentKind::FocusRecovery => GuiObservationCachePolicy::ObservePlanShort,
        GuiCognitionIntentKind::FocusInput
        | GuiCognitionIntentKind::TypeText
        | GuiCognitionIntentKind::ClickControl
        | GuiCognitionIntentKind::SafeAction
        | GuiCognitionIntentKind::RiskApproval => GuiObservationCachePolicy::Disabled,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopGuiObservationCacheEntry {
    pub observation: GuiObservationSnapshot,
    pub stored_at: Instant,
}

const GUI_OBSERVATION_CACHE_TTL: Duration = Duration::from_millis(750);
const GUI_OCR_CACHE_TTL: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone)]
struct GuiOcrCacheEntry {
    data: serde_json::Value,
    stored_at: Instant,
}

static GUI_OCR_CACHE: LazyLock<Mutex<HashMap<String, GuiOcrCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Step 11: in-memory checkpoint store keyed by session_id. v1 is process-local
/// (survives pause/HITL/resume within a running app, not a full app restart).
static GUI_WORKFLOW_CHECKPOINTS: LazyLock<std::sync::Mutex<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn store_session_checkpoint(session_id: &str, checkpoint: serde_json::Value) {
    if let Ok(mut store) = GUI_WORKFLOW_CHECKPOINTS.lock() {
        store.insert(session_id.to_string(), checkpoint);
    }
}

fn load_session_checkpoint(
    session_id: &str,
) -> Option<kria_core::agent::gui_cognition::checkpoint::GuiWorkflowCheckpoint> {
    let value = GUI_WORKFLOW_CHECKPOINTS
        .lock()
        .ok()?
        .get(session_id)
        .cloned()?;
    serde_json::from_value(value).ok()
}

#[derive(Debug, Clone)]
struct FocusAuthorityCandidate {
    source: String,
    status: String,
    focused_window: Option<String>,
    focused_app: Option<String>,
    focused_control_id: Option<String>,
    focused_control_label: Option<String>,
    focused_control_role: Option<String>,
    focused_control_bounds: Option<GuiBounds>,
    keyboard_focus_known: bool,
    text_cursor_known: bool,
    editable_target_known: bool,
    terminal_like: bool,
    confidence: f64,
    reliability: String,
    adapter_status: String,
    latency_ms: u64,
    reason: Option<String>,
}

impl FocusAuthorityCandidate {
    fn unavailable(source: impl Into<String>, reason: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            source: source.into(),
            status: "unavailable".into(),
            focused_window: None,
            focused_app: None,
            focused_control_id: None,
            focused_control_label: None,
            focused_control_role: None,
            focused_control_bounds: None,
            keyboard_focus_known: false,
            text_cursor_known: false,
            editable_target_known: false,
            terminal_like: false,
            confidence: 0.0,
            reliability: "unavailable".into(),
            adapter_status: "unavailable".into(),
            latency_ms,
            reason: Some(sanitize_gui_text(&reason.into(), 220).text),
        }
    }

    fn matched(&self) -> bool {
        self.keyboard_focus_known && self.confidence > 0.0
    }

    fn failure_chain_entry(&self) -> serde_json::Value {
        serde_json::json!({
            "source": self.source,
            "status": self.status,
            "reliability": self.reliability,
            "confidence": self.confidence,
            "adapter_status": self.adapter_status,
            "latency_ms": self.latency_ms,
            "reason": self.reason,
        })
    }
}

struct FixtureGuiPerceptionProvider {
    fixture: GuiPerceptionFixture,
    observation_seq: StdArc<std::sync::atomic::AtomicU64>,
    cursor_focus_seq: StdArc<std::sync::atomic::AtomicU64>,
}

impl FixtureGuiPerceptionProvider {
    fn new(fixture: GuiPerceptionFixture) -> Self {
        Self {
            fixture,
            observation_seq: StdArc::new(std::sync::atomic::AtomicU64::new(0)),
            cursor_focus_seq: StdArc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Returns the number of observations already captured before this one. The
    /// first observation (pre-action) is 0; the post-action re-observe is >= 1.
    fn next_observation_index(&self) -> u64 {
        self.observation_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Independent counter advanced only by `get_cursor_focus_state`, so a Step 9
    /// fixture can model focus returning to the target on the post-recovery
    /// observation without racing the screenshot counter.
    fn next_cursor_focus_index(&self) -> u64 {
        self.cursor_focus_seq
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

enum GuiPerceptionProviderAdapter<'a> {
    Live(DesktopGuiPerceptionProvider<'a>),
    Fixture(FixtureGuiPerceptionProvider),
}

impl<'a> DesktopGuiPerceptionProvider<'a> {
    async fn execute_probe(&self, tool_name: &str, params: serde_json::Value) -> ToolResult {
        let Some(handler) = self.app_state.tool_registry.get_handler(tool_name) else {
            return ToolResult::err(format!("{tool_name} is not available in this KRIA runtime"));
        };
        handler.execute(params).await
    }

    async fn capture_screenshot_bytes(&self) -> Result<StdArc<Vec<u8>>, String> {
        self.screenshot_bytes
            .get_or_init(|| async {
                ScreenshotCapture::capture_full()
                    .await
                    .map(StdArc::new)
                    .map_err(|err| format!("screenshot capture unavailable: {err}"))
            })
            .await
            .clone()
    }

    async fn capture_atspi_snapshot(&self) -> Result<StdArc<AtSpiSnapshot>, String> {
        self.atspi_snapshot
            .get_or_init(|| async {
                Ok(StdArc::new(
                    AtSpiEngine::new()
                        .capture_snapshot(AtSpiSnapshotRequest::default())
                        .await,
                ))
            })
            .await
            .clone()
    }

    fn safe_focus_text(value: impl AsRef<str>, limit: usize) -> Option<String> {
        let text = sanitize_gui_text(value.as_ref(), limit).text;
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    fn role_is_editable(role: &str) -> bool {
        let role = role.to_ascii_lowercase();
        (role.contains("text")
            || role.contains("entry")
            || role.contains("editable")
            || role.contains("textbox")
            || role.contains("searchbox")
            || role.contains("editor")
            || role.contains("input"))
            && !role.contains("terminal")
    }

    fn value_as_i32(value: Option<&serde_json::Value>) -> Option<i32> {
        value
            .and_then(serde_json::Value::as_i64)
            .and_then(|number| i32::try_from(number).ok())
            .or_else(|| {
                value
                    .and_then(serde_json::Value::as_f64)
                    .map(|number| number.round() as i32)
            })
    }

    fn bounds_from_value(value: Option<&serde_json::Value>) -> Option<GuiBounds> {
        let value = value?;
        if let Some(array) = value.as_array() {
            if array.len() >= 4 {
                return Some(GuiBounds {
                    x: Self::value_as_i32(array.first())?,
                    y: Self::value_as_i32(array.get(1))?,
                    width: Self::value_as_i32(array.get(2))?,
                    height: Self::value_as_i32(array.get(3))?,
                });
            }
        }
        let object = value.as_object()?;
        Some(GuiBounds {
            x: Self::value_as_i32(object.get("x"))?,
            y: Self::value_as_i32(object.get("y"))?,
            width: Self::value_as_i32(object.get("width"))?,
            height: Self::value_as_i32(object.get("height"))?,
        })
    }

    fn bounds_from_atspi(bounds: Option<[i32; 4]>) -> Option<GuiBounds> {
        let [x, y, width, height] = bounds?;
        Some(GuiBounds {
            x,
            y,
            width,
            height,
        })
    }

    fn browser_focus_candidate(
        source: &str,
        app_name: &str,
        result: BrowserResult,
        latency_ms: u64,
        editable_confidence: f64,
        foreground_hint_matched: bool,
    ) -> FocusAuthorityCandidate {
        if !result.success {
            return FocusAuthorityCandidate::unavailable(source, result.evidence, latency_ms);
        }
        let Some(data) = result.data else {
            return FocusAuthorityCandidate::unavailable(
                source,
                "browser adapter returned no focus metadata",
                latency_ms,
            );
        };
        if !foreground_hint_matched
            && !data
                .get("document_has_focus")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        {
            return FocusAuthorityCandidate::unavailable(
                source,
                format!(
                    "{app_name} active element is from a page that does not have foreground focus"
                ),
                latency_ms,
            );
        }
        let tag = data
            .get("tag")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let input_type = data
            .get("input_type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let raw_role = data
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let inferred_role = if !raw_role.trim().is_empty() {
            raw_role.to_string()
        } else if input_type == "search" {
            "searchbox".into()
        } else if tag == "input" || tag == "textarea" {
            "textbox".into()
        } else if data
            .get("is_content_editable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            "editable_text".into()
        } else {
            tag.clone()
        };
        let disabled = data
            .get("disabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let readonly = data
            .get("readonly")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let editable = data
            .get("editable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_else(|| Self::role_is_editable(&inferred_role))
            && !disabled
            && !readonly;
        let bounds = Self::bounds_from_value(data.get("bounds"));
        let label = data
            .get("label")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Self::safe_focus_text(value, 120))
            .or_else(|| {
                if editable {
                    Some(format!("{app_name} focused input"))
                } else {
                    Some(format!("{app_name} focused element"))
                }
            });
        let focused_window = data
            .get("page_title")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Self::safe_focus_text(value, 140));
        let id_hash = data
            .get("id_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("noid");
        let class_hash = data
            .get("class_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("noclass");
        let confidence = if editable && bounds.is_some() {
            editable_confidence
        } else if editable {
            editable_confidence - 0.06
        } else {
            0.76
        };
        let reliability = if confidence >= 0.88 {
            "reliable"
        } else {
            "best_effort"
        };
        FocusAuthorityCandidate {
            source: source.into(),
            status: "matched".into(),
            focused_window,
            focused_app: Some(app_name.into()),
            focused_control_id: Some(format!(
                "{source}:{}:{}:{}",
                sanitize_gui_text(&inferred_role, 40).text,
                sanitize_gui_text(id_hash, 24).text,
                sanitize_gui_text(class_hash, 24).text
            )),
            focused_control_label: label,
            focused_control_role: Self::safe_focus_text(&inferred_role, 80),
            focused_control_bounds: bounds,
            keyboard_focus_known: true,
            text_cursor_known: editable,
            editable_target_known: editable,
            terminal_like: false,
            confidence,
            reliability: reliability.into(),
            adapter_status: "available".into(),
            latency_ms,
            reason: None,
        }
    }

    async fn chrome_focus_candidate(foreground_hint_matched: bool) -> FocusAuthorityCandidate {
        let started = Instant::now();
        match tokio::time::timeout(
            Duration::from_millis(180),
            BrowserCognitionEngine::new().get_chrome_focus_snapshot(),
        )
        .await
        {
            Ok(result) => Self::browser_focus_candidate(
                "chrome_cdp_active_element",
                "Chrome",
                result,
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                0.94,
                foreground_hint_matched,
            ),
            Err(_) => FocusAuthorityCandidate::unavailable(
                "chrome_cdp_active_element",
                "Chrome CDP focus probe timed out",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            ),
        }
    }

    async fn firefox_focus_candidate(foreground_hint_matched: bool) -> FocusAuthorityCandidate {
        let started = Instant::now();
        let port = std::env::var("KRIA_FIREFOX_BIDI_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(9223);
        match tokio::time::timeout(
            Duration::from_millis(220),
            BrowserCognitionEngine::get_firefox_bidi_focus_snapshot(port),
        )
        .await
        {
            Ok(result) => Self::browser_focus_candidate(
                "firefox_bidi_active_element",
                "Firefox",
                result,
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                0.90,
                foreground_hint_matched,
            ),
            Err(_) => FocusAuthorityCandidate::unavailable(
                "firefox_bidi_active_element",
                "Firefox WebDriver BiDi focus probe timed out",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            ),
        }
    }

    async fn vscode_focus_candidate() -> FocusAuthorityCandidate {
        let started = Instant::now();
        let endpoint = std::env::var("KRIA_VSCODE_FOCUS_ENDPOINT")
            .or_else(|_| {
                std::env::var("KRIA_VSCODE_FOCUS_PORT")
                    .map(|port| format!("http://127.0.0.1:{port}/focus"))
            })
            .unwrap_or_else(|_| "http://127.0.0.1:47323/focus".into());
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(120))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                return FocusAuthorityCandidate::unavailable(
                    "vscode_extension",
                    format!("VS Code focus adapter client unavailable: {error}"),
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                );
            }
        };
        let response =
            match tokio::time::timeout(Duration::from_millis(120), client.get(&endpoint).send())
                .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    return FocusAuthorityCandidate::unavailable(
                        "vscode_extension",
                        format!("VS Code focus adapter unavailable: {error}"),
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    );
                }
                Err(_) => {
                    return FocusAuthorityCandidate::unavailable(
                        "vscode_extension",
                        "VS Code focus adapter timed out",
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    );
                }
            };
        let data: serde_json::Value = match response.json().await {
            Ok(data) => data,
            Err(error) => {
                return FocusAuthorityCandidate::unavailable(
                    "vscode_extension",
                    format!("VS Code focus adapter returned invalid JSON: {error}"),
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                );
            }
        };
        if data.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
            return FocusAuthorityCandidate::unavailable(
                "vscode_extension",
                data.get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("VS Code extension did not report fresh focus"),
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            );
        }
        let observed_at = data
            .get("observed_at_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();
        if observed_at <= 0 || unix_now_ms().saturating_sub(observed_at) > 1_000 {
            return FocusAuthorityCandidate::unavailable(
                "vscode_extension",
                "VS Code focus adapter response is stale",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            );
        }
        if data
            .get("window_focused")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            return FocusAuthorityCandidate::unavailable(
                "vscode_extension",
                "VS Code focus adapter reported that the VS Code window is not foreground focused",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            );
        }
        let role = data
            .get("focused_control_role")
            .or_else(|| data.get("role"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let terminal_like = data
            .get("terminal_like")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_else(|| role.to_ascii_lowercase().contains("terminal"));
        let editable = data
            .get("editable_target_known")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or_else(|| Self::role_is_editable(role))
            && !terminal_like;
        let confidence = data
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(if editable || terminal_like {
                0.95
            } else {
                0.82
            });
        FocusAuthorityCandidate {
            source: "vscode_extension".into(),
            status: "matched".into(),
            focused_window: data
                .get("focused_window")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Self::safe_focus_text(value, 140)),
            focused_app: Some("VS Code".into()),
            focused_control_id: data
                .get("focused_control_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Self::safe_focus_text(value, 100))
                .or_else(|| Some(format!("vscode:{role}"))),
            focused_control_label: data
                .get("focused_control_label")
                .or_else(|| data.get("label"))
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Self::safe_focus_text(value, 120))
                .or_else(|| {
                    if terminal_like {
                        Some("VS Code integrated terminal".into())
                    } else if editable {
                        Some("VS Code editor".into())
                    } else {
                        Some("VS Code focused control".into())
                    }
                }),
            focused_control_role: Self::safe_focus_text(role, 80),
            focused_control_bounds: Self::bounds_from_value(data.get("focused_control_bounds")),
            keyboard_focus_known: true,
            text_cursor_known: data
                .get("text_cursor_known")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(editable),
            editable_target_known: editable,
            terminal_like,
            confidence,
            reliability: if confidence >= 0.9 {
                "reliable".into()
            } else {
                "best_effort".into()
            },
            adapter_status: "available".into(),
            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            reason: None,
        }
    }

    fn terminal_focus_candidate(snapshot: &AtSpiSnapshot) -> FocusAuthorityCandidate {
        let started = Instant::now();
        let focused_element = snapshot.elements.iter().find(|element| element.focused);
        let candidates = [
            focused_element.map(|element| element.role.as_str()),
            focused_element.map(|element| element.name.as_str()),
            snapshot.focused_window.as_deref(),
            snapshot.focused_app_label.as_deref(),
            snapshot.focused_app.as_deref(),
        ];
        let terminal_like = candidates.into_iter().flatten().any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("terminal")
                || value.contains("gnome-terminal")
                || value.contains("vte")
                || value.contains("pty")
                || value.contains("shell")
        });
        if !terminal_like {
            return FocusAuthorityCandidate::unavailable(
                "gnome_terminal_adapter",
                "active/focused window is not terminal-like",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            );
        }
        FocusAuthorityCandidate {
            source: "gnome_terminal_adapter".into(),
            status: "matched".into(),
            focused_window: snapshot
                .focused_window
                .as_deref()
                .and_then(|value| Self::safe_focus_text(value, 140)),
            focused_app: snapshot
                .focused_app_label
                .as_deref()
                .or(snapshot.focused_app.as_deref())
                .and_then(|value| Self::safe_focus_text(value, 100))
                .or_else(|| Some("GNOME Terminal".into())),
            focused_control_id: focused_element.map(|element| {
                sanitize_gui_text(
                    &format!("{}:{}:terminal", element.bus_name, element.path),
                    120,
                )
                .text
            }),
            focused_control_label: Some("Terminal focus".into()),
            focused_control_role: Some("terminal".into()),
            focused_control_bounds: focused_element
                .and_then(|element| Self::bounds_from_atspi(element.bounds)),
            keyboard_focus_known: true,
            text_cursor_known: false,
            editable_target_known: false,
            terminal_like: true,
            confidence: 0.82,
            reliability: "reliable".into(),
            adapter_status: "available".into(),
            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            reason: None,
        }
    }

    fn gnome_bridge_focus_candidate(
        result: &GuiProbeResult,
        latency_ms: u64,
    ) -> FocusAuthorityCandidate {
        if !result.success {
            return FocusAuthorityCandidate::unavailable(
                "gnome_bridge_focus",
                result
                    .error
                    .as_deref()
                    .unwrap_or("GNOME bridge focus fields are not available"),
                latency_ms,
            );
        }

        let title = result
            .data
            .get("title")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Self::safe_focus_text(value, 140));
        let app = result
            .data
            .get("app_name")
            .or_else(|| result.data.get("app"))
            .or_else(|| result.data.get("app_id"))
            .or_else(|| result.data.get("class"))
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Self::safe_focus_text(value, 100));
        let hint = [
            title.as_deref().unwrap_or_default(),
            app.as_deref().unwrap_or_default(),
        ]
        .join(" ")
        .to_ascii_lowercase();
        let terminal_like = hint.contains("terminal")
            || hint.contains("gnome-terminal")
            || hint.contains("vte")
            || hint.contains("pty")
            || hint.contains("shell");
        let confidence = if terminal_like { 0.86 } else { 0.72 };
        FocusAuthorityCandidate {
            source: "gnome_bridge_focus".into(),
            status: "matched".into(),
            focused_window: title,
            focused_app: app,
            focused_control_id: Some(if terminal_like {
                "gnome_bridge:terminal_window".into()
            } else {
                "gnome_bridge:focused_window".into()
            }),
            focused_control_label: Some(if terminal_like {
                "Terminal focus".into()
            } else {
                "Focused window".into()
            }),
            focused_control_role: Some(if terminal_like {
                "terminal".into()
            } else {
                "window".into()
            }),
            focused_control_bounds: None,
            keyboard_focus_known: true,
            text_cursor_known: false,
            editable_target_known: false,
            terminal_like,
            confidence,
            reliability: if terminal_like {
                "reliable".into()
            } else {
                "best_effort".into()
            },
            adapter_status: result
                .data
                .get("gnome_bridge_status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("available")
                .into(),
            latency_ms,
            reason: None,
        }
    }

    fn atspi_focus_candidate(snapshot: &AtSpiSnapshot) -> FocusAuthorityCandidate {
        let started = Instant::now();
        let focused_window = snapshot
            .focused_window
            .as_deref()
            .and_then(|value| Self::safe_focus_text(value, 140));
        let focused_app = snapshot
            .focused_app_label
            .as_deref()
            .or(snapshot.focused_app.as_deref())
            .and_then(|value| Self::safe_focus_text(value, 100));
        let focused_element = snapshot.elements.iter().find(|element| element.focused);
        let focused_control_role = focused_element
            .map(|element| element.role.trim().to_string())
            .filter(|value| !value.is_empty());
        let terminal_like = focused_control_role.as_deref().is_some_and(|role| {
            let role = role.to_ascii_lowercase();
            role.contains("terminal") || role.contains("vte") || role.contains("pty")
        });
        let editable_target_known = focused_control_role
            .as_deref()
            .is_some_and(Self::role_is_editable)
            && !terminal_like;
        let keyboard_focus_known =
            focused_element.is_some() || focused_window.is_some() || focused_app.is_some();
        let focus_confidence = if focused_element.is_some() {
            if editable_target_known {
                0.88
            } else {
                0.82
            }
        } else if focused_window.is_some() {
            0.72
        } else if focused_app.is_some() {
            0.64
        } else {
            0.0
        };
        let focus_reliability = if focus_confidence >= 0.80 {
            "reliable"
        } else if focus_confidence > 0.0 {
            "best_effort"
        } else {
            "unavailable"
        };
        let source = if focused_element.is_some() {
            "atspi_focused_object"
        } else if focused_window.is_some() {
            "atspi_focused_window"
        } else if focused_app.is_some() {
            "atspi_focused_app"
        } else {
            "atspi_focused_object"
        };
        if !keyboard_focus_known {
            return FocusAuthorityCandidate::unavailable(
                source,
                "AT-SPI snapshot did not expose focused object, window, or app",
                started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            );
        }
        FocusAuthorityCandidate {
            source: source.into(),
            status: if focused_element.is_some() {
                "matched".into()
            } else {
                "fallback".into()
            },
            focused_window,
            focused_app,
            focused_control_id: focused_element.map(|element| {
                sanitize_gui_text(
                    &format!("{}:{}:{}", element.bus_name, element.path, element.role),
                    140,
                )
                .text
            }),
            focused_control_label: focused_element
                .map(|element| element.name.trim().to_string())
                .filter(|value| !value.is_empty())
                .and_then(|value| Self::safe_focus_text(value, 120)),
            focused_control_role: focused_control_role
                .and_then(|value| Self::safe_focus_text(value, 80)),
            focused_control_bounds: focused_element
                .and_then(|element| Self::bounds_from_atspi(element.bounds)),
            keyboard_focus_known,
            text_cursor_known: editable_target_known,
            editable_target_known,
            terminal_like,
            confidence: focus_confidence,
            reliability: focus_reliability.into(),
            adapter_status: snapshot.status.clone(),
            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            reason: None,
        }
    }

    fn snapshot_focus_hint(snapshot: &AtSpiSnapshot) -> String {
        let focused_element = snapshot.elements.iter().find(|element| element.focused);
        [
            snapshot.focused_window.as_deref(),
            snapshot.focused_app_label.as_deref(),
            snapshot.focused_app.as_deref(),
            focused_element.map(|element| element.name.as_str()),
            focused_element.map(|element| element.role.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
    }

    fn focus_hint_matches_any(hint: &str, needles: &[&str]) -> bool {
        needles.iter().any(|needle| hint.contains(needle))
    }

    async fn focus_authority_probe(&self, snapshot: &AtSpiSnapshot) -> GuiProbeResult {
        let started = Instant::now();
        let mut attempts = Vec::new();
        let bridge_started = Instant::now();
        let bridge = Self::kria_gnome_bridge_probe().await;
        let bridge_latency = bridge_started
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let bridge_focus = Self::gnome_bridge_focus_candidate(&bridge, bridge_latency);
        let bridge_hint = [
            bridge_focus.focused_window.as_deref().unwrap_or_default(),
            bridge_focus.focused_app.as_deref().unwrap_or_default(),
            bridge_focus
                .focused_control_role
                .as_deref()
                .unwrap_or_default(),
        ]
        .join(" ");
        let focus_hint = [Self::snapshot_focus_hint(snapshot), bridge_hint]
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let has_focus_hint = !focus_hint.trim().is_empty();
        attempts.push(bridge_focus.failure_chain_entry());
        if bridge_focus.terminal_like && bridge_focus.matched() {
            return Self::focus_authority_result(bridge_focus, attempts, started);
        }

        let chrome_hint_matched = Self::focus_hint_matches_any(
            &focus_hint,
            &["chrome", "chromium", "google-chrome", "google chrome"],
        );
        let chrome = if !has_focus_hint || chrome_hint_matched {
            Self::chrome_focus_candidate(chrome_hint_matched).await
        } else {
            FocusAuthorityCandidate::unavailable(
                "chrome_cdp_active_element",
                "Chrome CDP skipped because the current focused app/window is not Chrome-like",
                0,
            )
        };
        attempts.push(chrome.failure_chain_entry());
        if chrome.matched() {
            return Self::focus_authority_result(chrome, attempts, started);
        }

        let firefox_hint_matched =
            Self::focus_hint_matches_any(&focus_hint, &["firefox", "mozilla"]);
        let firefox = if !has_focus_hint || firefox_hint_matched {
            Self::firefox_focus_candidate(firefox_hint_matched).await
        } else {
            FocusAuthorityCandidate::unavailable(
                "firefox_bidi_active_element",
                "Firefox BiDi skipped because the current focused app/window is not Firefox-like",
                0,
            )
        };
        attempts.push(firefox.failure_chain_entry());
        if firefox.matched() {
            return Self::focus_authority_result(firefox, attempts, started);
        }

        let vscode = if !has_focus_hint
            || Self::focus_hint_matches_any(
                &focus_hint,
                &["visual studio code", "vscode", "vs code", " code"],
            ) {
            Self::vscode_focus_candidate().await
        } else {
            FocusAuthorityCandidate::unavailable(
                "vscode_extension",
                "VS Code extension skipped because the current focused app/window is not VS Code-like",
                0,
            )
        };
        attempts.push(vscode.failure_chain_entry());
        if vscode.matched() {
            return Self::focus_authority_result(vscode, attempts, started);
        }

        let terminal = Self::terminal_focus_candidate(snapshot);
        attempts.push(terminal.failure_chain_entry());
        if terminal.matched() {
            return Self::focus_authority_result(terminal, attempts, started);
        }

        let atspi = Self::atspi_focus_candidate(snapshot);
        attempts.push(atspi.failure_chain_entry());
        if atspi.matched() {
            return Self::focus_authority_result(atspi, attempts, started);
        }

        if bridge_focus.matched() {
            return Self::focus_authority_result(bridge_focus, attempts, started);
        }

        let selected = FocusAuthorityCandidate::unavailable(
            "unavailable",
            "No configured focus adapter exposed a fresh focused element, focused window, or focused app",
            started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        );
        Self::focus_authority_result(selected, attempts, started)
    }

    fn focus_authority_result(
        candidate: FocusAuthorityCandidate,
        attempts: Vec<serde_json::Value>,
        started: Instant,
    ) -> GuiProbeResult {
        let data = serde_json::json!({
            "focused_window": candidate.focused_window,
            "focused_app": candidate.focused_app,
            "focused_control_id": candidate.focused_control_id,
            "focused_control_label": candidate.focused_control_label,
            "focused_control_role": candidate.focused_control_role,
            "focused_control_bounds": candidate.focused_control_bounds,
            "keyboard_focus_known": candidate.keyboard_focus_known,
            "text_cursor_known": candidate.text_cursor_known,
            "editable_target_known": candidate.editable_target_known,
            "terminal_like": candidate.terminal_like,
            "focus_confidence": candidate.confidence,
            "focus_reliability": candidate.reliability,
            "focus_failure_chain": attempts,
            "source": candidate.source,
            "adapter_status": candidate.adapter_status,
            "latency_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        });
        if candidate.matched() {
            GuiProbeResult::ok(data)
        } else {
            GuiProbeResult::err_with_data(
                candidate
                    .reason
                    .unwrap_or_else(|| "focus authority unavailable".into()),
                data,
            )
        }
    }

    fn screenshot_hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn prepare_ocr_png(bytes: &[u8]) -> Result<(Vec<u8>, String), String> {
        const MAX_OCR_WIDTH: u32 = 1000;
        let image = image::load_from_memory(bytes)
            .map_err(|error| format!("OCR unavailable: screenshot decode failed: {error}"))?;
        let width = image.width();
        let height = image.height();
        let image = if width > MAX_OCR_WIDTH {
            let target_height = ((height as f64) * (MAX_OCR_WIDTH as f64 / width as f64))
                .round()
                .max(1.0) as u32;
            image.resize(
                MAX_OCR_WIDTH,
                target_height,
                image::imageops::FilterType::Triangle,
            )
        } else {
            image
        };
        let status = if width > MAX_OCR_WIDTH {
            format!(
                "downscaled_{width}x{height}_to_{}x{}",
                image.width(),
                image.height()
            )
        } else {
            format!("original_{width}x{height}")
        };
        let mut buffer = Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .map_err(|error| format!("OCR unavailable: screenshot preprocess failed: {error}"))?;
        Ok((buffer.into_inner(), status))
    }

    fn normalize_ocr_error(error: impl AsRef<str>) -> String {
        error
            .as_ref()
            .replace("timed out", "did not become ready")
            .replace("timeout", "budget exceeded")
    }

    async fn cached_ocr_result(
        screen_hash: &str,
        wait_for_screenshot_ms: u64,
        started: Instant,
    ) -> Option<GuiProbeResult> {
        let mut cache = GUI_OCR_CACHE.lock().await;
        cache.retain(|_, entry| entry.stored_at.elapsed() <= GUI_OCR_CACHE_TTL);
        let entry = cache.get(screen_hash)?.clone();
        let age_ms = entry
            .stored_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut data = entry.data;
        if let Some(object) = data.as_object_mut() {
            object.insert("ocr_cache_hit".into(), serde_json::json!(true));
            object.insert(
                "ocr_fast_path".into(),
                serde_json::json!("screen_hash_cache"),
            );
            object.insert(
                "ocr_wait_for_screenshot_ms".into(),
                serde_json::json!(wait_for_screenshot_ms),
            );
            object.insert(
                "ocr_total_ms".into(),
                serde_json::json!(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            );
            object.insert("ocr_warm_start_ms".into(), serde_json::json!(age_ms));
            object.insert("screen_hash".into(), serde_json::json!(screen_hash));
        }
        Some(GuiProbeResult::ok(data))
    }

    async fn store_ocr_result(screen_hash: &str, data: &serde_json::Value) {
        let mut cache = GUI_OCR_CACHE.lock().await;
        cache.retain(|_, entry| entry.stored_at.elapsed() <= GUI_OCR_CACHE_TTL);
        cache.insert(
            screen_hash.to_string(),
            GuiOcrCacheEntry {
                data: data.clone(),
                stored_at: Instant::now(),
            },
        );
    }

    fn parse_gnome_shell_eval_title(output: &str) -> Option<String> {
        let trimmed = output.trim();
        if !trimmed.starts_with("(true,") {
            return None;
        }
        let start = trimmed.find('\'')?;
        let end = trimmed.rfind('\'')?;
        if end <= start {
            return None;
        }
        let title = trimmed[start + 1..end].trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    fn parse_gdbus_json_string(output: &str) -> Option<serde_json::Value> {
        let start = output.find('{')?;
        let end = output.rfind('}')?;
        if end <= start {
            return None;
        }
        let raw = &output[start..=end];
        serde_json::from_str(raw)
            .ok()
            .or_else(|| serde_json::from_str(&raw.replace("\\\"", "\"")).ok())
    }

    async fn kria_gnome_bridge_probe() -> GuiProbeResult {
        let output = match Self::command_stdout(
            "gdbus",
            &[
                "call",
                "--session",
                "--dest",
                "ai.kria.ActiveWindow",
                "--object-path",
                "/ai/kria/ActiveWindow",
                "--method",
                "ai.kria.ActiveWindow.GetActiveWindow",
            ],
            250,
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                return GuiProbeResult::err_with_data(
                    format!("gnome_bridge_unavailable: {error}"),
                    serde_json::json!({
                        "source": "kria_gnome_shell_bridge",
                        "gnome_bridge_status": "missing",
                    }),
                );
            }
        };

        let Some(value) = Self::parse_gdbus_json_string(&output) else {
            return GuiProbeResult::err_with_data(
                "gnome_bridge_unavailable: bridge returned an unparseable response",
                serde_json::json!({
                    "source": "kria_gnome_shell_bridge",
                    "gnome_bridge_status": "errored",
                }),
            );
        };

        let title = value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if title.is_none() {
            return GuiProbeResult::err_with_data(
                "gnome_bridge_unavailable: bridge did not expose a focused window title",
                serde_json::json!({
                    "source": "kria_gnome_shell_bridge",
                    "gnome_bridge_status": "available",
                }),
            );
        }

        let mut data = serde_json::Map::new();
        data.insert(
            "source".into(),
            serde_json::json!("kria_gnome_shell_bridge"),
        );
        data.insert("gnome_bridge_status".into(), serde_json::json!("available"));
        data.insert("title".into(), serde_json::json!(title.unwrap()));
        data.insert("confidence".into(), serde_json::json!(0.98));
        data.insert("reliability".into(), serde_json::json!("reliable"));
        data.insert("observed_at_ms".into(), serde_json::json!(unix_now_ms()));
        for key in [
            "app",
            "app_name",
            "app_id",
            "class",
            "wm_class",
            "pid",
            "workspace",
            "workspace_index",
            "monitor",
            "monitor_index",
            "fullscreen",
            "minimized",
        ] {
            if let Some(field) = value.get(key) {
                data.insert(key.into(), field.clone());
            }
        }
        if data.get("pid").is_none() {
            data.insert("confidence".into(), serde_json::json!(0.94));
        }
        GuiProbeResult::ok(serde_json::Value::Object(data))
    }

    fn focused_sway_node(value: &serde_json::Value) -> Option<&serde_json::Value> {
        if value
            .get("focused")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Some(value);
        }
        value
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                value
                    .get("floating_nodes")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .find_map(Self::focused_sway_node)
    }

    async fn command_stdout(
        program: &str,
        args: &[&str],
        budget_ms: u64,
    ) -> Result<String, String> {
        let mut command = tokio::process::Command::new(program);
        command.args(args).kill_on_drop(true);
        match tokio::time::timeout(Duration::from_millis(budget_ms), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            Ok(Ok(output)) => Err(String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(180)
                .collect::<String>()),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err("command budget exceeded".into()),
        }
    }

    async fn wayland_active_window_probe() -> GuiProbeResult {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_ascii_lowercase();

        if desktop.contains("gnome") {
            let bridge = Self::kria_gnome_bridge_probe().await;
            if bridge.success {
                return bridge;
            }
            let bridge_error = bridge.error.clone();
            let bridge_status = bridge
                .data
                .get("gnome_bridge_status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("missing")
                .to_string();

            if let Ok(stdout) = Self::command_stdout(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--dest",
                    "org.gnome.Shell",
                    "--object-path",
                    "/org/gnome/Shell",
                    "--method",
                    "org.gnome.Shell.Eval",
                    "global.display.focus_window ? global.display.focus_window.get_title() : \"\"",
                ],
                300,
            )
            .await
            {
                if let Some(title) = Self::parse_gnome_shell_eval_title(&stdout) {
                    return GuiProbeResult::ok(serde_json::json!({
                        "title": title,
                        "source": "gnome_shell_focus_window",
                        "wayland_compositor": "gnome",
                        "gnome_bridge_status": bridge_status,
                        "confidence": 0.94,
                        "reliability": "reliable",
                        "observed_at_ms": unix_now_ms(),
                    }));
                }
            }
            return GuiProbeResult::err_with_data(
                "Active window unavailable: GNOME Wayland did not expose a focused window through the KRIA GNOME bridge or compositor probe; AT-SPI focused-window fallback will be used if available",
                serde_json::json!({
                    "source": "gnome_shell_focus_window",
                    "gnome_bridge_status": bridge_status,
                    "gnome_bridge_error": bridge_error,
                }),
            );
        }

        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            if let Ok(stdout) = Self::command_stdout("hyprctl", &["activewindow", "-j"], 300).await
            {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(title) = value
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|title| !title.is_empty())
                    {
                        return GuiProbeResult::ok(serde_json::json!({
                            "title": title,
                            "app": value.get("class").and_then(serde_json::Value::as_str),
                            "source": "hyprctl_activewindow",
                            "wayland_compositor": "hyprland",
                            "confidence": 0.95,
                            "reliability": "reliable",
                            "observed_at_ms": unix_now_ms(),
                        }));
                    }
                }
            }
        }

        if std::env::var_os("SWAYSOCK").is_some() {
            if let Ok(stdout) = Self::command_stdout("swaymsg", &["-t", "get_tree"], 350).await {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    if let Some(node) = Self::focused_sway_node(&value) {
                        if let Some(title) = node
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|title| !title.is_empty())
                        {
                            return GuiProbeResult::ok(serde_json::json!({
                                "title": title,
                                "app": node
                                    .get("app_id")
                                    .or_else(|| node.get("window_properties").and_then(|props| props.get("class")))
                                    .and_then(serde_json::Value::as_str),
                                "source": "swaymsg_focused_node",
                                "wayland_compositor": "sway",
                                "confidence": 0.95,
                                "reliability": "reliable",
                                "observed_at_ms": unix_now_ms(),
                            }));
                        }
                    }
                }
            }
        }

        GuiProbeResult::err(
            "Active window unavailable: Wayland compositor does not expose a supported focused-window probe; AT-SPI focused-window fallback will be used if available",
        )
    }

    async fn run_local_tesseract(path: &std::path::Path, budget_ms: u64) -> GuiProbeResult {
        let path_str = path.to_string_lossy().to_string();
        let mut command = tokio::process::Command::new("tesseract");
        command
            .args([path_str.as_str(), "stdout", "--psm", "11"])
            .kill_on_drop(true);

        match tokio::time::timeout(Duration::from_millis(budget_ms), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if text.is_empty() {
                    GuiProbeResult::ok(serde_json::json!({
                        "blocks": [],
                        "source": "tesseract_cli",
                        "ocr_engine": "tesseract",
                    }))
                } else {
                    GuiProbeResult::ok(serde_json::json!({
                        "text": text,
                        "source": "tesseract_cli",
                        "ocr_engine": "tesseract",
                    }))
                }
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                GuiProbeResult::err(format!(
                    "OCR unavailable: tesseract failed: {}",
                    stderr.trim().chars().take(180).collect::<String>()
                ))
            }
            Ok(Err(error)) => GuiProbeResult::err(format!(
                "OCR unavailable: tesseract executable failed: {error}"
            )),
            Err(_) => GuiProbeResult::err(
                "OCR unavailable: local OCR budget exceeded; screenshot and accessibility summaries remain available",
            ),
        }
    }

    fn snapshot_operational(snapshot: &AtSpiSnapshot) -> bool {
        matches!(snapshot.status.as_str(), "healthy" | "degraded")
    }

    fn snapshot_stale_node_count(snapshot: &AtSpiSnapshot) -> usize {
        snapshot
            .elements
            .iter()
            .filter(|element| element.path.is_empty() || element.path.contains("/dead/"))
            .count()
    }

    fn snapshot_timeout_count(snapshot: &AtSpiSnapshot) -> usize {
        snapshot
            .source_blockers
            .iter()
            .filter(|blocker| blocker.to_ascii_lowercase().contains("timeout"))
            .count()
            + snapshot.skipped_apps.len()
    }

    fn snapshot_accessibility_confidence(snapshot: &AtSpiSnapshot) -> f64 {
        if !Self::snapshot_operational(snapshot) {
            return 0.0;
        }
        let mut score: f64 = if snapshot.status == "healthy" {
            0.88
        } else {
            0.72
        };
        if !snapshot.elements.is_empty() {
            score += 0.06;
        }
        if snapshot
            .elements
            .iter()
            .any(|element| element.visible && element.enabled && !element.name.trim().is_empty())
        {
            score += 0.06;
        }
        score -= (snapshot.skipped_apps.len() as f64 * 0.06).min(0.24);
        score -= (Self::snapshot_stale_node_count(snapshot) as f64 * 0.04).min(0.20);
        if snapshot.omitted_node_count > 0 {
            score -= 0.06;
        }
        score.clamp(0.0, 0.98)
    }

    fn snapshot_accessibility_status(snapshot: &AtSpiSnapshot, confidence: f64) -> &'static str {
        if !Self::snapshot_operational(snapshot) {
            "unavailable"
        } else if confidence >= 0.82 && snapshot.skipped_apps.is_empty() {
            "healthy"
        } else {
            "degraded"
        }
    }

    fn snapshot_source_status(snapshot: &AtSpiSnapshot) -> serde_json::Value {
        let confidence = Self::snapshot_accessibility_confidence(snapshot);
        let health_status = Self::snapshot_accessibility_status(snapshot, confidence);
        let stale_node_count = Self::snapshot_stale_node_count(snapshot);
        let timeout_count = Self::snapshot_timeout_count(snapshot);
        let app_scores = if snapshot.application_labels.is_empty() {
            snapshot
                .applications
                .iter()
                .map(|app| {
                    serde_json::json!({
                        "app_label": "accessible app",
                        "bus_name": app,
                        "node_count": snapshot.node_count,
                        "control_count": snapshot.elements.len(),
                        "timeout_count": timeout_count,
                        "stale_node_count": stale_node_count,
                        "score": confidence,
                        "status": health_status,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            snapshot
                .application_labels
                .iter()
                .map(|label| {
                    serde_json::json!({
                        "app_label": label,
                        "bus_name": short_hash(label),
                        "node_count": snapshot.node_count,
                        "control_count": snapshot.elements.len(),
                        "timeout_count": timeout_count,
                        "stale_node_count": stale_node_count,
                        "score": confidence,
                        "status": health_status,
                    })
                })
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "accessibility_source_status": snapshot.status,
            "accessibility_health_status": health_status,
            "accessibility_overall_status": health_status,
            "accessibility_overall_confidence": confidence,
            "accessibility_app_scores": app_scores,
            "atspi_stale_node_count": stale_node_count,
            "atspi_timeout_count": timeout_count,
            "atspi_cache_hit_count": 0,
            "atspi_stale_cache_rejected_count": 0,
            "atspi_snapshot_total_ms": snapshot.timing.total_ms,
            "atspi_connection_ms": snapshot.timing.connection_ms,
            "atspi_apps_ms": snapshot.timing.apps_ms,
            "atspi_focused_app_ms": snapshot.timing.focused_app_ms,
            "atspi_scan_ms": snapshot.timing.scan_ms,
            "atspi_skipped_app_count": snapshot.skipped_apps.len(),
            "atspi_omitted_node_count": snapshot.omitted_node_count,
            "atspi_timeout_reason": snapshot.source_blockers.first().cloned(),
            "source_blockers": snapshot.source_blockers,
            "accessibility_remediation": snapshot.remediation,
        })
    }

    fn element_matches_role(element_role: &str, role: &str) -> bool {
        element_role
            .to_ascii_lowercase()
            .contains(&role.to_ascii_lowercase())
    }

    fn snapshot_element_json(
        element: &kria_core::agent::atspi_engine::AccessibleElement,
    ) -> serde_json::Value {
        let has_label = !element.name.trim().is_empty();
        let has_bounds = element.bounds.is_some();
        let identity_confidence = if has_label { 0.86 } else { 0.35 };
        let bounds_confidence = if has_bounds { 0.86 } else { 0.0 };
        let state_confidence = 0.86;
        serde_json::json!({
            "role": element.role,
            "name": element.name,
            "label": element.name,
            "path": element.path,
            "control_id": format!("{}:{}:{}", element.bus_name, element.path, element.role),
            "focused": element.focused,
            "enabled": element.enabled,
            "visible": element.visible,
            "in_active_window": element.in_active_window,
            "bounds": element.bounds,
            "depth": element.depth,
            "score": element.score,
            "source": "accessibility",
            "label_source": if has_label { "accessible_name" } else { "missing" },
            "state_source": "accessibility_state",
            "identity_confidence": identity_confidence,
            "bounds_confidence": bounds_confidence,
            "state_confidence": state_confidence,
            "sources": ["accessibility"],
        })
    }
}

#[async_trait]
impl GuiPerceptionProvider for DesktopGuiPerceptionProvider<'_> {
    async fn get_active_window(&self) -> GuiProbeResult {
        if std::env::var("XDG_SESSION_TYPE")
            .unwrap_or_default()
            .eq_ignore_ascii_case("wayland")
        {
            return Self::wayland_active_window_probe().await;
        }
        probe_from_tool_result(
            self.execute_probe("get_active_window", serde_json::json!({}))
                .await,
        )
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        let snapshot = match self.capture_atspi_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return GuiProbeResult::err(error),
        };
        let mut data = Self::snapshot_source_status(&snapshot);
        if let Some(object) = data.as_object_mut() {
            object.insert("source".into(), serde_json::json!("atspi_snapshot"));
            object.insert(
                "applications".into(),
                serde_json::json!(if snapshot.application_labels.is_empty() {
                    snapshot.applications.clone()
                } else {
                    snapshot.application_labels.clone()
                }),
            );
            object.insert(
                "application_buses".into(),
                serde_json::json!(snapshot.applications.clone()),
            );
            object.insert(
                "dialog_visible".into(),
                serde_json::json!(snapshot.dialog_visible),
            );
            object.insert(
                "focused_window".into(),
                serde_json::json!(snapshot.focused_window),
            );
            object.insert(
                "focused_app".into(),
                serde_json::json!(snapshot
                    .focused_app_label
                    .clone()
                    .or_else(|| snapshot.focused_app.clone())),
            );
            object.insert(
                "focused_app_bus".into(),
                serde_json::json!(snapshot.focused_app.clone()),
            );
            object.insert(
                "element_count".into(),
                serde_json::json!(snapshot.node_count),
            );
            object.insert(
                "accessibility_operational".into(),
                serde_json::json!(Self::snapshot_operational(&snapshot)),
            );
        }
        GuiProbeResult::ok(data)
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        let snapshot = match self.capture_atspi_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return GuiProbeResult::err(error),
        };
        let operational = Self::snapshot_operational(&snapshot);
        let mut data = Self::snapshot_source_status(&snapshot);
        if let Some(object) = data.as_object_mut() {
            object.insert("source".into(), serde_json::json!("atspi_snapshot"));
            object.insert(
                "toolkit_accessibility_enabled".into(),
                serde_json::json!(operational),
            );
            object.insert("atspi_bus_available".into(), serde_json::json!(operational));
            object.insert(
                "accessible_apps_detected".into(),
                serde_json::json!(!snapshot.applications.is_empty()),
            );
            object.insert(
                "accessibility_operational".into(),
                serde_json::json!(operational),
            );
            object.insert("toolkits".into(), serde_json::json!([]));
            object.insert(
                "remediation".into(),
                serde_json::json!(snapshot.remediation),
            );
        }
        GuiProbeResult::ok(data)
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let snapshot = match self.capture_atspi_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return GuiProbeResult::err(error),
        };
        let elements = snapshot
            .elements
            .iter()
            .filter(|element| Self::element_matches_role(&element.role, role))
            .map(Self::snapshot_element_json)
            .collect::<Vec<_>>();
        let mut data = Self::snapshot_source_status(&snapshot);
        if let Some(object) = data.as_object_mut() {
            object.insert("source".into(), serde_json::json!("atspi_snapshot"));
            object.insert("role".into(), serde_json::json!(role));
            object.insert("count".into(), serde_json::json!(elements.len()));
            object.insert("elements".into(), serde_json::json!(elements));
        }
        GuiProbeResult::ok(data)
    }

    async fn focused_window_title(&self) -> Option<String> {
        self.capture_atspi_snapshot()
            .await
            .ok()
            .and_then(|snapshot| {
                snapshot
                    .focused_window
                    .clone()
                    .or_else(|| snapshot.focused_app_label.clone())
            })
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        match self.capture_screenshot_bytes().await {
            Ok(bytes) => GuiProbeResult::ok(serde_json::json!({
                "screen_hash": Self::screenshot_hash(bytes.as_ref()),
                "byte_count": bytes.len(),
                "source": "xcap",
            })),
            Err(error) => GuiProbeResult::err(error),
        }
    }

    async fn run_ocr(&self) -> GuiProbeResult {
        let started = Instant::now();
        let wait_for_screenshot_ms: u64;
        let bytes = match tokio::time::timeout(
            Duration::from_millis(1_850),
            self.capture_screenshot_bytes(),
        )
        .await
        {
            Ok(Ok(bytes)) => {
                wait_for_screenshot_ms =
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                bytes
            }
            Ok(Err(error)) => {
                let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                return GuiProbeResult::err_with_data(
                    error,
                    serde_json::json!({
                        "ocr_engine": "none",
                        "ocr_engine_status": "screenshot_blocked",
                        "ocr_wait_for_screenshot_ms": total_ms,
                        "ocr_total_ms": total_ms,
                    }),
                );
            }
            Err(_) => {
                let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                return GuiProbeResult::err_with_data(
                    "OCR unavailable: screenshot capture was not ready within the OCR budget; screenshot hash and accessibility summaries remain available",
                    serde_json::json!({
                        "ocr_engine": "none",
                        "ocr_engine_status": "screenshot_wait_budget_exceeded",
                        "ocr_wait_for_screenshot_ms": total_ms,
                        "ocr_total_ms": total_ms,
                    }),
                );
            }
        };
        let screen_hash = Self::screenshot_hash(bytes.as_ref());
        if let Some(cached) =
            Self::cached_ocr_result(&screen_hash, wait_for_screenshot_ms, started).await
        {
            return cached;
        }
        let (ocr_bytes, ocr_image_status) = match Self::prepare_ocr_png(bytes.as_ref()) {
            Ok(result) => result,
            Err(error) => {
                let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                return GuiProbeResult::err_with_data(
                    error,
                    serde_json::json!({
                        "ocr_engine": "none",
                        "ocr_engine_status": "screenshot_preprocess_failed",
                        "ocr_wait_for_screenshot_ms": wait_for_screenshot_ms,
                        "ocr_total_ms": total_ms,
                        "screen_hash": screen_hash,
                        "ocr_cache_hit": false,
                        "ocr_fast_path": "full_screen",
                        "ocr_roi_count": 0,
                        "ocr_changed_region_count": 0,
                    }),
                );
            }
        };
        let path = std::env::temp_dir().join(format!("kria-gui-ocr-{}.png", Uuid::new_v4()));
        if let Err(error) = tokio::fs::write(&path, &ocr_bytes).await {
            let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            return GuiProbeResult::err_with_data(
                format!("failed to create temporary OCR screenshot: {error}"),
                serde_json::json!({
                    "ocr_engine": "none",
                    "ocr_engine_status": "temporary_file_failed",
                    "ocr_image_status": ocr_image_status,
                    "ocr_wait_for_screenshot_ms": wait_for_screenshot_ms,
                    "ocr_total_ms": total_ms,
                }),
            );
        }

        let path_str = path.to_string_lossy().to_string();
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let sidecar_budget_ms = 3_200_u64
            .saturating_sub(elapsed_ms)
            .saturating_sub(50)
            .clamp(150, 350);
        let result = match tokio::time::timeout(
            Duration::from_millis(sidecar_budget_ms),
            self.execute_probe("ocr_image", serde_json::json!({ "path": path_str })),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => ToolResult::err("sidecar OCR budget exceeded"),
        };

        if result.success {
            let _ = tokio::fs::remove_file(&path).await;
            let data = serde_json::json!({
                "text": result
                    .data
                    .get("text")
                    .or_else(|| result.data.get("ocr_text"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                "source": result
                    .data
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("ocr_image"),
                "ocr_engine_selected": "ocr_image",
                "ocr_engine_status": "completed",
                "ocr_image_status": ocr_image_status,
                "ocr_wait_for_screenshot_ms": wait_for_screenshot_ms,
                "ocr_total_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                "screen_hash": screen_hash,
                "ocr_cache_hit": false,
                "ocr_fast_path": "full_screen_sidecar",
                "ocr_roi_count": 0,
                "ocr_changed_region_count": 0,
                "ocr_cold_start_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                "ocr_benchmark_summary": "rapidocr/paddleocr optional adapters not configured; sidecar OCR used",
            });
            Self::store_ocr_result(&screen_hash, &data).await;
            return GuiProbeResult::ok(data);
        }

        let sidecar_error =
            Self::normalize_ocr_error(result.error.clone().unwrap_or_else(|| "unknown".into()));
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let local_budget_ms = 3_800_u64
            .saturating_sub(elapsed_ms)
            .saturating_sub(50)
            .clamp(800, 2_200);
        let mut local_result = Self::run_local_tesseract(&path, local_budget_ms).await;
        if local_result.success {
            if let Some(object) = local_result.data.as_object_mut() {
                object.insert(
                    "screen_hash".into(),
                    serde_json::json!(Self::screenshot_hash(bytes.as_ref())),
                );
                object.insert(
                    "ocr_wait_for_screenshot_ms".into(),
                    serde_json::json!(wait_for_screenshot_ms),
                );
                object.insert(
                    "ocr_total_ms".into(),
                    serde_json::json!(
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
                    ),
                );
                object.insert(
                    "ocr_engine_selected".into(),
                    serde_json::json!("tesseract_cli"),
                );
                object.insert("ocr_engine_status".into(), serde_json::json!("completed"));
                object.insert(
                    "ocr_image_status".into(),
                    serde_json::json!(ocr_image_status),
                );
                object.insert("ocr_cache_hit".into(), serde_json::json!(false));
                object.insert(
                    "ocr_fast_path".into(),
                    serde_json::json!("full_screen_tesseract"),
                );
                object.insert("ocr_roi_count".into(), serde_json::json!(0));
                object.insert("ocr_changed_region_count".into(), serde_json::json!(0));
                object.insert(
                    "ocr_cold_start_ms".into(),
                    serde_json::json!(
                        started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
                    ),
                );
                object.insert(
                    "ocr_benchmark_summary".into(),
                    serde_json::json!(
                        "rapidocr/paddleocr optional adapters not configured; tesseract fallback used"
                    ),
                );
            }
            Self::store_ocr_result(&screen_hash, &local_result.data).await;
            let _ = tokio::fs::remove_file(&path).await;
            return local_result;
        }

        let local_error = local_result
            .error
            .clone()
            .unwrap_or_else(|| "local OCR unavailable".into());
        let _ = tokio::fs::remove_file(&path).await;
        let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        GuiProbeResult::err_with_data(
            format!(
                "OCR unavailable: sidecar OCR unavailable: {sidecar_error}; {}",
                Self::normalize_ocr_error(local_error)
            ),
            serde_json::json!({
                "ocr_engine": "ocr_image,tesseract_cli",
                "ocr_engine_selected": "ocr_image,tesseract_cli",
                "ocr_engine_status": if total_ms >= 3_700 { "ocr_budget_exceeded" } else { "engines_unavailable" },
                "ocr_image_status": ocr_image_status,
                "ocr_wait_for_screenshot_ms": wait_for_screenshot_ms,
                "ocr_total_ms": total_ms,
                "screen_hash": screen_hash,
                "ocr_cache_hit": false,
                "ocr_fast_path": "unavailable",
                "ocr_roi_count": 0,
                "ocr_changed_region_count": 0,
                "ocr_benchmark_summary": "rapidocr/paddleocr optional adapters not configured",
            }),
        )
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        let snapshot = match self.capture_atspi_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return GuiProbeResult::err(error),
        };
        let mut result = self.focus_authority_probe(&snapshot).await;
        if let Some(object) = result.data.as_object_mut() {
            object.insert(
                "accessibility_source_status".into(),
                serde_json::json!(snapshot.status),
            );
        }
        result
    }

    async fn get_accessibility_tree_summary(&self) -> GuiProbeResult {
        let snapshot = match self.capture_atspi_snapshot().await {
            Ok(snapshot) => snapshot,
            Err(error) => return GuiProbeResult::err(error),
        };
        let mut data = Self::snapshot_source_status(&snapshot);
        if let Some(object) = data.as_object_mut() {
            object.insert("source".into(), serde_json::json!("atspi_snapshot"));
            object.insert("node_count".into(), serde_json::json!(snapshot.node_count));
            object.insert(
                "omitted_node_count".into(),
                serde_json::json!(snapshot.omitted_node_count),
            );
            object.insert(
                "applications".into(),
                serde_json::json!(snapshot.applications.len()),
            );
            object.insert(
                "control_count".into(),
                serde_json::json!(snapshot.elements.len()),
            );
        }
        GuiProbeResult::ok(data)
    }

    async fn detect_visual_controls(&self) -> GuiProbeResult {
        let started = Instant::now();
        let bytes = match tokio::time::timeout(
            Duration::from_millis(1_850),
            self.capture_screenshot_bytes(),
        )
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                return GuiProbeResult::err_with_data(
                    error,
                    serde_json::json!({
                        "source": "vision_sidecar",
                        "visual_detector_status": "screenshot_blocked",
                    }),
                );
            }
            Err(_) => {
                return GuiProbeResult::err_with_data(
                    "visual control detection unavailable: screenshot was not ready within the visual detector budget",
                    serde_json::json!({
                        "source": "vision_sidecar",
                        "visual_detector_status": "screenshot_wait_budget_exceeded",
                    }),
                );
            }
        };
        let endpoint = std::env::var("KRIA_OMNIPARSER_ENDPOINT")
            .or_else(|_| std::env::var("KRIA_VISION_ENDPOINT"))
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into());
        let client = OmniParserClient::new(endpoint.clone());
        let output = match tokio::time::timeout(
            Duration::from_millis(950),
            client.parse_screenshot(bytes.as_ref()),
        )
        .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                return GuiProbeResult::err_with_data(
                    format!("visual control detection unavailable: {error}"),
                    serde_json::json!({
                        "source": "vision_sidecar",
                        "visual_detector_status": "unavailable",
                        "endpoint": endpoint,
                    }),
                );
            }
            Err(_) => {
                return GuiProbeResult::err_with_data(
                    "visual control detection unavailable: sidecar parse budget exceeded",
                    serde_json::json!({
                        "source": "vision_sidecar",
                        "visual_detector_status": "timeout",
                        "endpoint": endpoint,
                    }),
                );
            }
        };
        let elements = output
            .elements
            .into_iter()
            .take(80)
            .map(|element| {
                serde_json::json!({
                    "id": element.id,
                    "control_type": element.element_type,
                    "label": element.label,
                    "bbox": element.bbox,
                    "confidence": element.confidence,
                    "source": "vision_sidecar",
                    "visual_hash": element.visual_hash,
                })
            })
            .collect::<Vec<_>>();
        GuiProbeResult::ok(serde_json::json!({
            "source": "vision_sidecar",
            "visual_detector_status": "completed",
            "visual_detector_total_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            "screen_hash": Self::screenshot_hash(bytes.as_ref()),
            "elements": elements,
        }))
    }

    fn observation_cache_policy(&self) -> &'static str {
        self.cache_policy.as_str()
    }

    async fn cached_observation(
        &self,
        observation_id: &str,
        context_id: &str,
    ) -> Option<GuiObservationSnapshot> {
        if !self.cache_policy.enabled() {
            return None;
        }
        let cache = self.app_state.gui_cognition_observation_cache.lock().await;
        let entry = cache.as_ref()?;
        let age = entry.stored_at.elapsed();
        if age > GUI_OBSERVATION_CACHE_TTL {
            return None;
        }
        let mut observation = entry.observation.clone();
        observation.observation_id = observation_id.into();
        observation.context_id = context_id.into();
        observation.cache = GuiObservationCacheSummary {
            cache_hit: true,
            cache_age_ms: Some(age.as_millis().min(u128::from(u64::MAX)) as u64),
            cache_policy: self.cache_policy.as_str().into(),
            freshness: "cached_recent".into(),
        };
        Some(observation)
    }

    async fn store_observation_cache(&self, observation: &GuiObservationSnapshot) {
        if !self.cache_policy.enabled()
            || observation.cache.cache_hit
            || !observation.has_useful_signal()
        {
            return;
        }
        let mut cached = observation.clone();
        cached.cache.cache_hit = false;
        cached.cache.cache_age_ms = None;
        cached.cache.cache_policy = self.cache_policy.as_str().into();
        cached.cache.freshness = "fresh".into();
        let mut cache = self.app_state.gui_cognition_observation_cache.lock().await;
        *cache = Some(DesktopGuiObservationCacheEntry {
            observation: cached,
            stored_at: Instant::now(),
        });
    }
}

impl FixtureGuiPerceptionProvider {
    fn active_title(&self) -> &'static str {
        match self.fixture {
            GuiPerceptionFixture::GnomeBridgeReliable => "Visual Studio Code - KRIA",
            GuiPerceptionFixture::GnomeEvalFallback => "Firefox - KRIA",
            GuiPerceptionFixture::SecretActiveWindow => "Project [REDACTED] token=secret-value",
            GuiPerceptionFixture::ChromeCdpFocusSearchBox => "Google Search - Chrome",
            GuiPerceptionFixture::FirefoxBidiFocusSearchBox => "Google Search - Firefox",
            GuiPerceptionFixture::VscodeExtensionEditorFocus => "main.rs - Visual Studio Code",
            GuiPerceptionFixture::VscodeExtensionTerminalFocus => "Terminal - Visual Studio Code",
            GuiPerceptionFixture::GnomeTerminalFocus => "Terminal - KRIA",
            GuiPerceptionFixture::AllFocusAdaptersUnavailable => "KRIA Perception Test",
            _ => "KRIA Perception Test",
        }
    }

    fn focused_title(&self) -> Option<String> {
        match self.fixture {
            GuiPerceptionFixture::BridgeMissingFallback | GuiPerceptionFixture::AtspiFallback => {
                Some("Terminal - KRIA".into())
            }
            GuiPerceptionFixture::SingleWindowBestEffort
            | GuiPerceptionFixture::FailureChainPrecise
            | GuiPerceptionFixture::AllFocusAdaptersUnavailable => None,
            _ => Some(self.active_title().into()),
        }
    }

    fn desktop_applications(&self) -> Vec<&'static str> {
        match self.fixture {
            GuiPerceptionFixture::SingleWindowBestEffort => vec!["Only Visible App"],
            GuiPerceptionFixture::FailureChainPrecise => vec!["Firefox", "Terminal", "VS Code"],
            GuiPerceptionFixture::BridgeMissingFallback | GuiPerceptionFixture::AtspiFallback => {
                vec!["Terminal - KRIA", "Firefox"]
            }
            GuiPerceptionFixture::GnomeEvalFallback => vec!["Firefox - KRIA", "Terminal"],
            GuiPerceptionFixture::ChromeCdpFocusSearchBox => vec!["Google Search - Chrome"],
            GuiPerceptionFixture::FirefoxBidiFocusSearchBox => vec!["Google Search - Firefox"],
            GuiPerceptionFixture::VscodeExtensionEditorFocus
            | GuiPerceptionFixture::VscodeExtensionTerminalFocus => vec!["Visual Studio Code"],
            GuiPerceptionFixture::GnomeTerminalFocus => vec!["Terminal - KRIA"],
            GuiPerceptionFixture::AllFocusAdaptersUnavailable => {
                vec!["Firefox", "VS Code", "Terminal"]
            }
            _ => vec![self.active_title()],
        }
    }

    fn elements(&self, role: &str) -> GuiProbeResult {
        let elements = match (self.fixture, role) {
            (GuiPerceptionFixture::Step9FocusRecovers, "text") => vec![serde_json::json!({
                "role": "text",
                "name": "Search",
                "label": "Search",
                "path": "/fixture/text/Step9Search",
                "control_id": "fixture-step9-search-field",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
            })],
            (GuiPerceptionFixture::FocusedSearchField, "text") => vec![serde_json::json!({
                "role": "text",
                "name": "Search",
                "label": "Search",
                "path": "/fixture/text/Search",
                "control_id": "fixture-focused-search-field",
                "enabled": true,
                "visible": true,
                "focused": true,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
            })],
            (GuiPerceptionFixture::HiddenDisabledControls, "push button") => vec![
                serde_json::json!({
                    "role": "push button",
                    "name": "Search",
                    "label": "Search",
                    "path": "/fixture/button/SearchDisabled",
                    "control_id": "fixture-search-disabled",
                    "enabled": false,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 384, "y": 20, "width": 90, "height": 32 },
                }),
                serde_json::json!({
                    "role": "push button",
                    "name": "Hidden Search",
                    "label": "Hidden Search",
                    "path": "/fixture/button/SearchHidden",
                    "control_id": "fixture-search-hidden",
                    "enabled": true,
                    "visible": false,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 480, "y": 20, "width": 90, "height": 32 },
                }),
            ],
            (GuiPerceptionFixture::DuplicateSearchButtons, "push button") => vec![
                serde_json::json!({
                    "role": "push button",
                    "name": "Search",
                    "label": "Search",
                    "path": "/fixture/button/SearchA",
                    "control_id": "fixture-search-button-a",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 384, "y": 20, "width": 90, "height": 32 },
                }),
                serde_json::json!({
                    "role": "push button",
                    "name": "Search",
                    "label": "Search",
                    "path": "/fixture/button/SearchB",
                    "control_id": "fixture-search-button-b",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 486, "y": 20, "width": 90, "height": 32 },
                }),
            ],
            (GuiPerceptionFixture::OcrOnlyControl, "push button") => vec![serde_json::json!({
                "role": "push button",
                "name": "Search",
                "label": "Search",
                "path": "/fixture/ocr/Search",
                "control_id": "fixture-ocr-search",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "source": "ocr_label_evidence",
                "sources": ["ocr_label_evidence"],
                "bounds": { "x": 384, "y": 20, "width": 90, "height": 32 },
            })],
            (GuiPerceptionFixture::VisualOnlyButton, "push button") => vec![serde_json::json!({
                "role": "push button",
                "name": "Search",
                "label": "Search",
                "path": "/fixture/visual/Search",
                "control_id": "fixture-visual-search",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "source": "visual_detector",
                "sources": ["visual_detector"],
                "bounds": { "x": 384, "y": 20, "width": 90, "height": 32 },
            })],
            (_, "text") => vec![serde_json::json!({
                "role": "text",
                "name": "Search KRIA",
                "label": "Search KRIA",
                "path": "/fixture/text/Search KRIA",
                "control_id": "fixture-search-field",
                "enabled": true,
                "visible": true,
                "focused": true,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
            })],
            (_, "push button") => vec![
                serde_json::json!({
                    "role": "push button",
                    "name": "Submit Test",
                    "label": "Submit Test",
                    "path": "/fixture/button/Submit Test",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 260, "y": 20, "width": 112, "height": 32 },
                }),
                serde_json::json!({
                    "role": "push button",
                    "name": "Search",
                    "label": "Search",
                    "path": "/fixture/button/Search",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 384, "y": 20, "width": 90, "height": 32 },
                }),
                serde_json::json!({
                    "role": "push button",
                    "name": "Save",
                    "label": "Save",
                    "path": "/fixture/button/Save",
                    "enabled": true,
                    "visible": true,
                    "focused": false,
                    "in_active_window": true,
                    "bounds": { "x": 486, "y": 20, "width": 80, "height": 32 },
                }),
            ],
            (_, "check box") => vec![serde_json::json!({
                "role": "check box",
                "name": "Enable option",
                "label": "Enable option",
                "path": "/fixture/checkbox/Enable option",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 64, "width": 160, "height": 28 },
            })],
            (_, "link") => vec![serde_json::json!({
                "role": "link",
                "name": "Learn more",
                "label": "Learn more",
                "path": "/fixture/link/Learn more",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 104, "width": 120, "height": 24 },
            })],
            (_, "page tab") => vec![serde_json::json!({
                "role": "page tab",
                "name": "Overview",
                "label": "Overview",
                "path": "/fixture/tab/Overview",
                "enabled": true,
                "visible": true,
                "focused": false,
                "in_active_window": true,
                "bounds": { "x": 10, "y": 140, "width": 96, "height": 28 },
            })],
            (_, "dialog") => Vec::new(),
            _ => Vec::new(),
        };
        GuiProbeResult::ok(serde_json::json!({ "elements": elements }))
    }
}

#[async_trait]
impl GuiPerceptionProvider for FixtureGuiPerceptionProvider {
    async fn get_active_window(&self) -> GuiProbeResult {
        match self.fixture {
            GuiPerceptionFixture::GnomeBridgeReliable => GuiProbeResult::ok(serde_json::json!({
                "title": "Visual Studio Code - KRIA",
                "app_name": "Code",
                "app_id": "code.desktop",
                "pid": 4242,
                "workspace": 1,
                "monitor": 0,
                "fullscreen": false,
                "minimized": false,
                "source": "kria_gnome_shell_bridge",
                "confidence": 0.98,
                "reliability": "reliable",
                "gnome_bridge_status": "available",
                "observed_at_ms": unix_now_ms(),
            })),
            GuiPerceptionFixture::GnomeEvalFallback => GuiProbeResult::ok(serde_json::json!({
                "title": "Firefox - KRIA",
                "app_name": "Firefox",
                "app_id": "firefox.desktop",
                "source": "gnome_shell_focus_window",
                "confidence": 0.94,
                "reliability": "reliable",
                "gnome_bridge_status": "missing",
                "observed_at_ms": unix_now_ms(),
            })),
            GuiPerceptionFixture::BridgeMissingFallback | GuiPerceptionFixture::AtspiFallback => {
                GuiProbeResult::err_with_data(
                    "gnome_bridge_unavailable: fixture bridge missing",
                    serde_json::json!({
                        "source": "kria_gnome_shell_bridge",
                        "gnome_bridge_status": "missing",
                    }),
                )
            }
            GuiPerceptionFixture::SingleWindowBestEffort => GuiProbeResult::err_with_data(
                "gnome_bridge_unavailable: fixture bridge missing",
                serde_json::json!({
                    "source": "kria_gnome_shell_bridge",
                    "gnome_bridge_status": "missing",
                }),
            ),
            GuiPerceptionFixture::FailureChainPrecise => GuiProbeResult::err_with_data(
                "Active window unavailable: fixture source chain exhausted",
                serde_json::json!({
                    "source": "kria_gnome_shell_bridge",
                    "gnome_bridge_status": "missing",
                }),
            ),
            GuiPerceptionFixture::SecretActiveWindow => GuiProbeResult::ok(serde_json::json!({
                "title": "Project password=secret-value",
                "app_name": "Secrets App",
                "source": "kria_gnome_shell_bridge",
                "confidence": 0.94,
                "reliability": "reliable",
                "gnome_bridge_status": "available",
                "observed_at_ms": unix_now_ms(),
            })),
            GuiPerceptionFixture::TextFieldAndButton
            | GuiPerceptionFixture::DuplicateSearchButtons
            | GuiPerceptionFixture::HiddenDisabledControls
            | GuiPerceptionFixture::OcrOnlyControl
            | GuiPerceptionFixture::FocusedSearchField
            | GuiPerceptionFixture::VisualOnlyButton
            | GuiPerceptionFixture::Step8ClickResultChanges
            | GuiPerceptionFixture::Step8ClickNoChange
            | GuiPerceptionFixture::Step8TypedTextVisible
            | GuiPerceptionFixture::Step8SecretTypeStateChanges
            | GuiPerceptionFixture::Step9FocusRecovers => {
                GuiProbeResult::ok(serde_json::json!({
                    "title": "KRIA Perception Test",
                    "app_name": "KRIA Perception Test",
                    "source": "gui_cognition_test_fixture",
                }))
            }
            GuiPerceptionFixture::ChromeCdpFocusSearchBox => {
                GuiProbeResult::ok(serde_json::json!({
                    "title": "Google Search - Chrome",
                    "app_name": "Chrome",
                    "app_id": "google-chrome.desktop",
                    "source": "gui_cognition_test_fixture",
                    "confidence": 0.98,
                    "reliability": "reliable",
                }))
            }
            GuiPerceptionFixture::FirefoxBidiFocusSearchBox => {
                GuiProbeResult::ok(serde_json::json!({
                    "title": "Google Search - Firefox",
                    "app_name": "Firefox",
                    "app_id": "firefox.desktop",
                    "source": "gui_cognition_test_fixture",
                    "confidence": 0.98,
                    "reliability": "reliable",
                }))
            }
            GuiPerceptionFixture::VscodeExtensionEditorFocus
            | GuiPerceptionFixture::VscodeExtensionTerminalFocus => {
                GuiProbeResult::ok(serde_json::json!({
                    "title": self.active_title(),
                    "app_name": "VS Code",
                    "app_id": "code.desktop",
                    "source": "gui_cognition_test_fixture",
                    "confidence": 0.98,
                    "reliability": "reliable",
                }))
            }
            GuiPerceptionFixture::GnomeTerminalFocus => GuiProbeResult::ok(serde_json::json!({
                "title": "Terminal - KRIA",
                "app_name": "GNOME Terminal",
                "app_id": "org.gnome.Terminal.desktop",
                "source": "gui_cognition_test_fixture",
                "confidence": 0.98,
                "reliability": "reliable",
            })),
            GuiPerceptionFixture::AllFocusAdaptersUnavailable => {
                GuiProbeResult::ok(serde_json::json!({
                    "title": "KRIA Perception Test",
                    "app_name": "KRIA Perception Test",
                    "source": "gui_cognition_test_fixture",
                    "confidence": 0.72,
                    "reliability": "best_effort",
                }))
            }
        }
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        let focused_title = match self.fixture {
            GuiPerceptionFixture::AtspiFallback => String::new(),
            _ => self.focused_title().unwrap_or_default(),
        };
        let focused_app = match self.fixture {
            GuiPerceptionFixture::AtspiFallback | GuiPerceptionFixture::BridgeMissingFallback => {
                "Terminal"
            }
            GuiPerceptionFixture::SingleWindowBestEffort => "",
            GuiPerceptionFixture::FailureChainPrecise => "",
            GuiPerceptionFixture::GnomeBridgeReliable => "Code",
            GuiPerceptionFixture::GnomeEvalFallback => "Firefox",
            GuiPerceptionFixture::SecretActiveWindow => "Secrets App",
            GuiPerceptionFixture::TextFieldAndButton
            | GuiPerceptionFixture::DuplicateSearchButtons
            | GuiPerceptionFixture::HiddenDisabledControls
            | GuiPerceptionFixture::OcrOnlyControl
            | GuiPerceptionFixture::FocusedSearchField
            | GuiPerceptionFixture::VisualOnlyButton
            | GuiPerceptionFixture::Step8ClickResultChanges
            | GuiPerceptionFixture::Step8ClickNoChange
            | GuiPerceptionFixture::Step8TypedTextVisible
            | GuiPerceptionFixture::Step8SecretTypeStateChanges
            | GuiPerceptionFixture::Step9FocusRecovers => "KRIA Perception Test",
            GuiPerceptionFixture::ChromeCdpFocusSearchBox => "Chrome",
            GuiPerceptionFixture::FirefoxBidiFocusSearchBox => "Firefox",
            GuiPerceptionFixture::VscodeExtensionEditorFocus
            | GuiPerceptionFixture::VscodeExtensionTerminalFocus => "VS Code",
            GuiPerceptionFixture::GnomeTerminalFocus => "GNOME Terminal",
            GuiPerceptionFixture::AllFocusAdaptersUnavailable => "",
        };
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": focused_title,
            "focused_app": focused_app,
            "accessibility_operational": true,
            "applications": self.desktop_applications(),
            "element_count": 87,
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "atspi_bus_available": true }))
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        let mut result = self.elements(role);
        if matches!(
            self.fixture,
            GuiPerceptionFixture::AllFocusAdaptersUnavailable
        ) {
            if let Some(elements) = result
                .data
                .get_mut("elements")
                .and_then(serde_json::Value::as_array_mut)
            {
                for element in elements {
                    if let Some(object) = element.as_object_mut() {
                        object.insert("focused".into(), serde_json::json!(false));
                    }
                }
            }
        }
        result
    }

    async fn focused_window_title(&self) -> Option<String> {
        self.focused_title()
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        let index = self.next_observation_index();
        // Some Step 8 fixtures model a real screen change after the action so
        // post-action verification can observe a genuine state transition. The
        // pre-action observation (index 0) and post-action re-observe (index >= 1)
        // therefore return different screen hashes.
        let screen_changes_after_action = matches!(
            self.fixture,
            GuiPerceptionFixture::Step8ClickResultChanges
                | GuiPerceptionFixture::Step8SecretTypeStateChanges
        );
        let screen_hash = if screen_changes_after_action && index >= 1 {
            "fixture-screen-hash-post-action"
        } else {
            "fixture-screen-hash"
        };
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": screen_hash,
            "byte_count": 16,
            "source": "fixture",
        }))
    }

    async fn run_ocr(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "blocks": [
                {
                    "text": "KRIA_VISIBLE_TEXT_123",
                    "bounds": [10, 180, 260, 204],
                    "confidence": 0.94
                }
            ],
            "source": "fixture",
            "ocr_engine_selected": "fixture_ocr",
            "ocr_engine_status": "completed",
            "ocr_image_status": "fixture_original",
            "ocr_wait_for_screenshot_ms": 0,
            "ocr_total_ms": 1,
            "ocr_cache_hit": false,
            "ocr_fast_path": "fixture_roi",
            "ocr_roi_count": 1,
            "ocr_changed_region_count": 0,
            "ocr_cold_start_ms": 1,
            "ocr_benchmark_summary": "fixture OCR known text path",
            "screen_hash": "fixture-screen-hash",
        }))
    }

    async fn get_monitor_layout(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "monitors": [
                {
                    "id": "fixture-primary",
                    "name": "Fixture Primary",
                    "x": 0,
                    "y": 0,
                    "width": 1280,
                    "height": 720,
                    "scale_factor": 1.0,
                    "primary": true
                }
            ]
        }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        if matches!(self.fixture, GuiPerceptionFixture::Step9FocusRecovers) {
            // Focus is on the wrong control for the pre-action (0) and
            // post-action (1) observations, then returns to the Search field on
            // the post-recovery observation (>= 2), so RefocusSameTarget can
            // verify recovery.
            let seq = self.next_cursor_focus_index();
            let on_target = seq >= 2;
            return GuiProbeResult::ok(serde_json::json!({
                "focused_window": "KRIA Perception Test",
                "focused_app": "KRIA Perception Test",
                "focused_control_id": "fixture-step9-other-control",
                "focused_control_label": if on_target { "Search" } else { "Other Field" },
                "focused_control_role": "text",
                "keyboard_focus_known": true,
                "text_cursor_known": true,
                "editable_target_known": true,
                "terminal_like": false,
                "focus_confidence": 0.9,
            }));
        }
        match self.fixture {
            GuiPerceptionFixture::ChromeCdpFocusSearchBox => {
                return GuiProbeResult::ok(serde_json::json!({
                    "focused_window": "Google Search - Chrome",
                    "focused_app": "Chrome",
                    "focused_control_id": "chrome_cdp_active_element:searchbox:fixture",
                    "focused_control_label": "Search Google",
                    "focused_control_role": "searchbox",
                    "focused_control_bounds": { "x": 256, "y": 148, "width": 520, "height": 44 },
                    "keyboard_focus_known": true,
                    "text_cursor_known": true,
                    "editable_target_known": true,
                    "terminal_like": false,
                    "focus_confidence": 0.94,
                    "focus_reliability": "reliable",
                    "adapter_status": "available",
                    "latency_ms": 18,
                    "focus_failure_chain": [
                        {
                            "source": "gnome_bridge_focus",
                            "status": "unavailable",
                            "reliability": "unavailable",
                            "reason": "GNOME bridge focus fields are not exposed by the installed bridge schema"
                        },
                        {
                            "source": "chrome_cdp_active_element",
                            "status": "matched",
                            "reliability": "reliable",
                            "confidence": 0.94,
                            "adapter_status": "available",
                            "latency_ms": 18,
                            "reason": null
                        }
                    ],
                    "source": "chrome_cdp_active_element",
                }));
            }
            GuiPerceptionFixture::FirefoxBidiFocusSearchBox => {
                return GuiProbeResult::ok(serde_json::json!({
                    "focused_window": "Google Search - Firefox",
                    "focused_app": "Firefox",
                    "focused_control_id": "firefox_bidi_active_element:searchbox:fixture",
                    "focused_control_label": "Search Google",
                    "focused_control_role": "searchbox",
                    "focused_control_bounds": { "x": 240, "y": 150, "width": 530, "height": 44 },
                    "keyboard_focus_known": true,
                    "text_cursor_known": true,
                    "editable_target_known": true,
                    "terminal_like": false,
                    "focus_confidence": 0.90,
                    "focus_reliability": "reliable",
                    "adapter_status": "available",
                    "latency_ms": 22,
                    "focus_failure_chain": [
                        {
                            "source": "chrome_cdp_active_element",
                            "status": "unavailable",
                            "reliability": "unavailable",
                            "reason": "Chrome CDP adapter unavailable in Firefox fixture"
                        },
                        {
                            "source": "firefox_bidi_active_element",
                            "status": "matched",
                            "reliability": "reliable",
                            "confidence": 0.90,
                            "adapter_status": "available",
                            "latency_ms": 22,
                            "reason": null
                        }
                    ],
                    "source": "firefox_bidi_active_element",
                }));
            }
            GuiPerceptionFixture::VscodeExtensionEditorFocus => {
                return GuiProbeResult::ok(serde_json::json!({
                    "focused_window": "main.rs - Visual Studio Code",
                    "focused_app": "VS Code",
                    "focused_control_id": "vscode:editor",
                    "focused_control_label": "VS Code editor",
                    "focused_control_role": "editor",
                    "keyboard_focus_known": true,
                    "text_cursor_known": true,
                    "editable_target_known": true,
                    "terminal_like": false,
                    "focus_confidence": 0.95,
                    "focus_reliability": "reliable",
                    "adapter_status": "available",
                    "latency_ms": 12,
                    "focus_failure_chain": [
                        {
                            "source": "vscode_extension",
                            "status": "matched",
                            "reliability": "reliable",
                            "confidence": 0.95,
                            "adapter_status": "available",
                            "latency_ms": 12,
                            "reason": null
                        }
                    ],
                    "source": "vscode_extension",
                }));
            }
            GuiPerceptionFixture::VscodeExtensionTerminalFocus => {
                return GuiProbeResult::ok(serde_json::json!({
                    "focused_window": "Terminal - Visual Studio Code",
                    "focused_app": "VS Code",
                    "focused_control_id": "vscode:terminal",
                    "focused_control_label": "VS Code integrated terminal",
                    "focused_control_role": "terminal",
                    "keyboard_focus_known": true,
                    "text_cursor_known": false,
                    "editable_target_known": false,
                    "terminal_like": true,
                    "focus_confidence": 0.95,
                    "focus_reliability": "reliable",
                    "adapter_status": "available",
                    "latency_ms": 12,
                    "focus_failure_chain": [
                        {
                            "source": "vscode_extension",
                            "status": "matched",
                            "reliability": "reliable",
                            "confidence": 0.95,
                            "adapter_status": "available",
                            "latency_ms": 12,
                            "reason": null
                        }
                    ],
                    "source": "vscode_extension",
                }));
            }
            GuiPerceptionFixture::GnomeTerminalFocus => {
                return GuiProbeResult::ok(serde_json::json!({
                    "focused_window": "Terminal - KRIA",
                    "focused_app": "GNOME Terminal",
                    "focused_control_id": "gnome-terminal:terminal",
                    "focused_control_label": "Terminal focus",
                    "focused_control_role": "terminal",
                    "keyboard_focus_known": true,
                    "text_cursor_known": false,
                    "editable_target_known": false,
                    "terminal_like": true,
                    "focus_confidence": 0.82,
                    "focus_reliability": "reliable",
                    "adapter_status": "available",
                    "latency_ms": 8,
                    "focus_failure_chain": [
                        {
                            "source": "gnome_terminal_adapter",
                            "status": "matched",
                            "reliability": "reliable",
                            "confidence": 0.82,
                            "adapter_status": "available",
                            "latency_ms": 8,
                            "reason": null
                        }
                    ],
                    "source": "gnome_terminal_adapter",
                }));
            }
            GuiPerceptionFixture::AllFocusAdaptersUnavailable => {
                return GuiProbeResult::err_with_data(
                    "No configured focus adapter exposed a fresh focused element, focused window, or focused app",
                    serde_json::json!({
                        "focused_window": null,
                        "focused_app": null,
                        "focused_control_id": null,
                        "focused_control_label": null,
                        "focused_control_role": null,
                        "keyboard_focus_known": false,
                        "text_cursor_known": false,
                        "editable_target_known": false,
                        "terminal_like": false,
                        "focus_confidence": 0.0,
                        "focus_reliability": "unavailable",
                        "adapter_status": "unavailable",
                        "latency_ms": 5,
                        "focus_failure_chain": [
                            {
                                "source": "chrome_cdp_active_element",
                                "status": "unavailable",
                                "reliability": "unavailable",
                                "reason": "Chrome CDP adapter unavailable"
                            },
                            {
                                "source": "firefox_bidi_active_element",
                                "status": "unavailable",
                                "reliability": "unavailable",
                                "reason": "Firefox BiDi adapter unavailable"
                            },
                            {
                                "source": "vscode_extension",
                                "status": "unavailable",
                                "reliability": "unavailable",
                                "reason": "VS Code focus adapter unavailable"
                            },
                            {
                                "source": "atspi_focused_object",
                                "status": "unavailable",
                                "reliability": "unavailable",
                                "reason": "AT-SPI focused object unavailable"
                            }
                        ],
                        "source": "unavailable",
                    }),
                );
            }
            _ => {}
        }
        let focused_window = self.focused_title().unwrap_or_default();
        let focus_known = !focused_window.is_empty();
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": focused_window,
            "focused_app": if focus_known { "KRIA Perception Test" } else { "" },
            "focused_control_id": "fixture-search-field",
            "focused_control_label": "Search KRIA",
            "focused_control_role": "text",
            "focused_control_bounds": { "x": 10, "y": 20, "width": 240, "height": 32 },
            "keyboard_focus_known": focus_known,
            "text_cursor_known": focus_known,
            "editable_target_known": focus_known,
            "terminal_like": false,
            "focus_confidence": if focus_known { 0.91 } else { 0.0 },
            "focus_reliability": if focus_known { "reliable" } else { "unavailable" },
            "adapter_status": "fixture",
            "latency_ms": 1,
            "focus_failure_chain": [
                {
                    "source": "fixture_focus",
                    "status": if focus_known { "matched" } else { "missing" },
                    "reliability": if focus_known { "reliable" } else { "unavailable" },
                    "reason": null
                }
            ],
            "source": "fixture",
        }))
    }

    async fn get_accessibility_tree_summary(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "source": "fixture",
            "node_count": 87,
            "omitted_node_count": 0,
            "applications": 1,
            "control_count": 7,
            "accessibility_source_status": "healthy",
            "accessibility_health_status": "healthy",
            "accessibility_overall_status": "healthy",
            "accessibility_overall_confidence": 0.94,
            "accessibility_app_scores": [
                {
                    "app_label": "KRIA Perception Test",
                    "bus_name": "fixture",
                    "node_count": 87,
                    "control_count": 7,
                    "timeout_count": 0,
                    "stale_node_count": 0,
                    "score": 0.94,
                    "status": "healthy"
                }
            ],
            "atspi_stale_node_count": 0,
            "atspi_timeout_count": 0,
            "atspi_cache_hit_count": 1,
            "atspi_stale_cache_rejected_count": 0,
            "atspi_snapshot_total_ms": 25,
        }))
    }

    async fn detect_visual_controls(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "source": "fixture_visual_detector",
            "visual_detector_status": "completed",
            "visual_detector_total_ms": 2,
            "elements": [
                {
                    "id": "visual-submit",
                    "control_type": "button",
                    "label": "Submit Test",
                    "bbox": [260, 20, 372, 52],
                    "confidence": 0.94,
                    "source": "fixture_visual_detector"
                },
                {
                    "id": "visual-search",
                    "control_type": "input",
                    "label": "Search KRIA",
                    "bbox": [10, 20, 250, 52],
                    "confidence": 0.92,
                    "source": "fixture_visual_detector"
                },
                {
                    "id": "visual-overview",
                    "control_type": "tab",
                    "label": "Overview",
                    "bbox": [10, 140, 106, 168],
                    "confidence": 0.9,
                    "source": "fixture_visual_detector"
                }
            ]
        }))
    }
}

#[async_trait]
impl GuiPerceptionProvider for GuiPerceptionProviderAdapter<'_> {
    async fn get_active_window(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_active_window().await,
            Self::Fixture(provider) => provider.get_active_window().await,
        }
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_desktop_state().await,
            Self::Fixture(provider) => provider.get_desktop_state().await,
        }
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_accessibility_capabilities().await,
            Self::Fixture(provider) => provider.get_accessibility_capabilities().await,
        }
    }

    async fn find_ui_elements(&self, role: &str) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.find_ui_elements(role).await,
            Self::Fixture(provider) => provider.find_ui_elements(role).await,
        }
    }

    async fn focused_window_title(&self) -> Option<String> {
        match self {
            Self::Live(provider) => provider.focused_window_title().await,
            Self::Fixture(provider) => provider.focused_window_title().await,
        }
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.capture_screenshot().await,
            Self::Fixture(provider) => provider.capture_screenshot().await,
        }
    }

    async fn run_ocr(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.run_ocr().await,
            Self::Fixture(provider) => provider.run_ocr().await,
        }
    }

    async fn get_monitor_layout(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_monitor_layout().await,
            Self::Fixture(provider) => provider.get_monitor_layout().await,
        }
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_cursor_focus_state().await,
            Self::Fixture(provider) => provider.get_cursor_focus_state().await,
        }
    }

    async fn get_accessibility_tree_summary(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.get_accessibility_tree_summary().await,
            Self::Fixture(provider) => provider.get_accessibility_tree_summary().await,
        }
    }

    async fn detect_visual_controls(&self) -> GuiProbeResult {
        match self {
            Self::Live(provider) => provider.detect_visual_controls().await,
            Self::Fixture(provider) => provider.detect_visual_controls().await,
        }
    }

    fn observation_cache_policy(&self) -> &'static str {
        match self {
            Self::Live(provider) => provider.observation_cache_policy(),
            Self::Fixture(provider) => provider.observation_cache_policy(),
        }
    }

    async fn cached_observation(
        &self,
        observation_id: &str,
        context_id: &str,
    ) -> Option<GuiObservationSnapshot> {
        match self {
            Self::Live(provider) => {
                provider
                    .cached_observation(observation_id, context_id)
                    .await
            }
            Self::Fixture(provider) => {
                provider
                    .cached_observation(observation_id, context_id)
                    .await
            }
        }
    }

    async fn store_observation_cache(&self, observation: &GuiObservationSnapshot) {
        match self {
            Self::Live(provider) => provider.store_observation_cache(observation).await,
            Self::Fixture(provider) => provider.store_observation_cache(observation).await,
        }
    }
}

struct DesktopGuiActionExecutor<'a> {
    app_state: &'a AppState,
    backend_status: GuiActionBackendStatus,
    execution_mode: GuiExecutionMode,
}

impl<'a> DesktopGuiActionExecutor<'a> {
    async fn execute_tool(&self, tool_name: &str, params: serde_json::Value) -> ToolResult {
        let Some(handler) = self.app_state.tool_registry.get_handler(tool_name) else {
            return ToolResult::err(format!("{tool_name} is not available in this KRIA runtime"));
        };
        handler.execute(params).await
    }

    fn with_backend_receipt(
        &self,
        request: &GuiActionRequest,
        execution: GuiActionExecution,
        started_at_ms: i64,
        completed_at_ms: i64,
    ) -> GuiActionExecution {
        let backend_used = self.backend_status.selected_backend.clone();
        let action_hash = short_hash(&format!(
            "{}:{}:{}:{:?}",
            backend_used,
            request.kind.as_str(),
            request.target_name,
            request.value
        ));
        let target_hash = short_hash(&format!(
            "{}:{}:{}",
            request.role, request.target_name, self.backend_status.session_type
        ));
        let tool_evidence = execution.evidence.clone();
        GuiActionExecution {
            evidence: serde_json::json!({
                "backend_used": backend_used,
                "action_kind": request.kind.as_str(),
                "target_id": request.target_name,
                "target_hash": target_hash,
                "action_hash": action_hash,
                "screen_hash": "post_action_observation_required",
                "started_at_ms": started_at_ms,
                "completed_at_ms": completed_at_ms,
                "verification_required": true,
                "tool": execution.tool,
                "tool_evidence": tool_evidence,
            }),
            ..execution
        }
    }
}

#[async_trait]
impl GuiActionExecutor for DesktopGuiActionExecutor<'_> {
    async fn action_backend_status(&self) -> GuiActionBackendStatus {
        self.backend_status.clone()
    }

    async fn execute(&self, request: GuiActionRequest) -> GuiActionExecution {
        if matches!(self.execution_mode, GuiExecutionMode::ExecuteFixture) {
            let started_at_ms = unix_now_ms();
            let execution = GuiActionExecution::ok(
                "fixture_executor",
                serde_json::json!({
                    "fixture_executed": true,
                    "action_kind": request.kind.as_str(),
                    "target": request.target_name,
                    "payload_hash_only": request.value.as_ref().map(|value| short_hash(value)),
                }),
            );
            let completed_at_ms = unix_now_ms();
            return self.with_backend_receipt(&request, execution, started_at_ms, completed_at_ms);
        }
        if !self.backend_status.supports_action(&request.kind) {
            return GuiActionExecution::err(
                self.backend_status.selected_backend.clone(),
                self.backend_status.primary_blocker(&request.kind),
            );
        }
        if !matches!(
            self.backend_status.selected_backend.as_str(),
            "uinput_accessibility" | "ydotool_accessibility" | "xdotool_accessibility"
        ) {
            return GuiActionExecution::err(
                self.backend_status.selected_backend.clone(),
                format!(
                    "Selected GUI backend {} is not an executable action backend",
                    self.backend_status.selected_backend
                ),
            );
        }

        let started_at_ms = unix_now_ms();
        let execution = match request.execution_hint.as_str() {
            "open_application" => {
                let result = self
                    .execute_tool(
                        "open_application",
                        serde_json::json!({
                            "name": request.target_name,
                            "args": [],
                        }),
                    )
                    .await;
                execution_from_tool_result("open_application", result)
            }
            "focus_window" => {
                let result = self
                    .execute_tool(
                        "focus_window",
                        serde_json::json!({
                            "title": request.target_name,
                        }),
                    )
                    .await;
                execution_from_tool_result("focus_window", result)
            }
            "fill_form_field" => {
                let result = self
                    .execute_tool(
                        "fill_form_field",
                        serde_json::json!({
                            "label": request.target_name,
                            "value": request.value.clone().unwrap_or_default(),
                        }),
                    )
                    .await;
                execution_from_tool_result("fill_form_field", result)
            }
            "atspi_type_into_focused" => {
                let text = request.value.clone().unwrap_or_default();
                let result = kria_core::agent::atspi_engine::AtSpiEngine::new()
                    .type_into_focused(&text)
                    .await;
                if result.success {
                    GuiActionExecution::ok(
                        "atspi_type_into_focused",
                        serde_json::json!(sanitized_execution_evidence(&result.evidence)),
                    )
                } else {
                    GuiActionExecution::err(
                        "atspi_type_into_focused",
                        sanitized_execution_evidence(&result.evidence),
                    )
                }
            }
            "press_shortcut" => {
                let keys = match request.kind {
                    GuiActionKind::Copy => vec!["ctrl", "c"],
                    GuiActionKind::Paste => vec!["ctrl", "v"],
                    _ => request
                        .value
                        .as_deref()
                        .map(|value| vec![value])
                        .unwrap_or_else(|| vec!["enter"]),
                };
                let result = self
                    .execute_tool(
                        "press_shortcut",
                        serde_json::json!({
                            "keys": keys,
                        }),
                    )
                    .await;
                execution_from_tool_result("press_shortcut", result)
            }
            "scroll" => GuiActionExecution::err(
                self.backend_status.selected_backend.clone(),
                "Scroll execution is not supported by the selected Step 7 backend yet.",
            ),
            _ => {
                let role = request.role.clone();
                let result = self
                    .execute_tool(
                        "click_ui_element",
                        serde_json::json!({ "role": role, "name": request.target_name }),
                    )
                    .await;
                execution_from_tool_result("click_ui_element", result)
            }
        };
        let completed_at_ms = unix_now_ms();
        self.with_backend_receipt(&request, execution, started_at_ms, completed_at_ms)
    }
}

fn service_liveness_label(value: kria_core::orchestrator::ServiceLiveness) -> String {
    use kria_core::orchestrator::ServiceLiveness::*;
    match value {
        Stopped => "stopped",
        Starting => "starting",
        Running => "running",
        Failed => "failed",
    }
    .to_string()
}

fn executable_available(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join(name))
                .find(|candidate| candidate.is_file())
        })
        .is_some()
}

async fn xdotool_display_usable(session_type: &str, xdotool_available: bool) -> bool {
    if session_type != "x11" || !xdotool_available || std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    let Ok(mut child) = tokio::process::Command::new("xdotool")
        .arg("getactivewindow")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    match tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await {
        Ok(Ok(status)) => status.success(),
        _ => {
            let _ = child.kill().await;
            false
        }
    }
}

async fn uinput_socket_accessible(path: &std::path::Path) -> bool {
    match tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::UnixStream::connect(path),
    )
    .await
    {
        Ok(Ok(_stream)) => true,
        _ => false,
    }
}

async fn ydotool_permission_probe(ydotool_available: bool) -> bool {
    if !ydotool_available {
        return false;
    }
    if std::env::var("KRIA_ENABLE_YDOTOOL_GUI_BACKEND").as_deref() != Ok("1") {
        return false;
    }
    let Ok(mut child) = tokio::process::Command::new("ydotool")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    match tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await {
        Ok(Ok(status)) => status.success(),
        _ => {
            let _ = child.kill().await;
            false
        }
    }
}

fn current_session_type() -> String {
    let explicit = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !explicit.is_empty() {
        return explicit;
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return "wayland".into();
    }
    if std::env::var_os("DISPLAY").is_some() {
        return "x11".into();
    }
    "unknown".into()
}

fn halt_kind_for_backend(
    global_halt_engaged: bool,
    automation_enabled: bool,
    orchestrator_available: bool,
    vision_sidecar: &str,
    uinput_daemon: &str,
    selected_backend: &str,
    halt_reason: Option<&str>,
) -> String {
    if !orchestrator_available {
        return "orchestrator_unavailable".into();
    }
    if !automation_enabled || halt_reason.is_some_and(|reason| reason.contains("user disabled")) {
        return "user_disabled".into();
    }
    if !global_halt_engaged && selected_backend != "unavailable" {
        return "none".into();
    }
    if global_halt_engaged
        && (vision_sidecar == "starting"
            || uinput_daemon == "starting"
            || halt_reason.is_some_and(|reason| {
                reason.contains("warming")
                    || reason.contains("startup")
                    || reason.contains("re-spawning")
            }))
    {
        return "startup_warming".into();
    }
    if global_halt_engaged
        || vision_sidecar == "failed"
        || uinput_daemon == "failed"
        || vision_sidecar == "stopped"
        || uinput_daemon == "stopped"
        || selected_backend == "unavailable"
    {
        return "service_not_ready".into();
    }
    "emergency".into()
}

fn release_conditions_for_backend(
    halt_kind: &str,
    vision_sidecar: &str,
    uinput_daemon: &str,
    session_type: &str,
) -> Vec<String> {
    match halt_kind {
        "none" => Vec::new(),
        "startup_warming" => vec![
            "Wait for vision sidecar and uinput daemon to report running.".into(),
            "Retry the GUI action after startup completes.".into(),
        ],
        "user_disabled" => vec!["Enable GUI automation in Settings.".into()],
        "orchestrator_unavailable" => {
            vec!["Restart KRIA with the GUI service orchestrator available.".into()]
        }
        "service_not_ready" => {
            let mut conditions = Vec::new();
            if vision_sidecar != "running" {
                conditions.push("Start or repair the vision sidecar.".into());
            }
            if uinput_daemon != "running" {
                conditions.push(
                    "Start or repair the uinput daemon and sudoers/socket permissions.".into(),
                );
            }
            if session_type == "wayland" {
                conditions
                    .push("On Wayland, use a running uinput daemon or install ydotool.".into());
            }
            if conditions.is_empty() {
                conditions.push("Resolve the GUI backend blocker, then retry.".into());
            }
            conditions
        }
        _ => vec!["Check GUI automation logs and restart GUI automation services.".into()],
    }
}

fn backend_status_from_probe(input: GuiBackendProbeInput) -> GuiActionBackendStatus {
    let selection = select_gui_action_backend(&input);
    let halt_kind = halt_kind_for_backend(
        input.global_halt_engaged,
        input.automation_enabled,
        input.orchestrator_available,
        &input.vision_sidecar,
        &input.uinput_daemon,
        &selection.selected_backend,
        input.halt_reason.as_deref(),
    );
    let release_conditions = release_conditions_for_backend(
        &halt_kind,
        &input.vision_sidecar,
        &input.uinput_daemon,
        &input.session_type,
    );

    GuiActionBackendStatus {
        global_halt_engaged: input.global_halt_engaged,
        halt_kind,
        halt_reason: input.halt_reason,
        release_conditions,
        startup_elapsed_ms: None,
        can_observe: true,
        can_plan: true,
        automation_enabled: input.automation_enabled,
        vision_sidecar: input.vision_sidecar,
        uinput_daemon: input.uinput_daemon,
        orchestrator_available: input.orchestrator_available,
        session_type: input.session_type,
        xdotool_available: input.xdotool_available,
        ydotool_available: input.ydotool_available,
        uinput_available: input.uinput_available,
        selected_backend: selection.selected_backend,
        backend_selection_reason: selection.backend_selection_reason,
        backend_probe_status: selection.backend_probe_status,
        backend_probe_errors: selection.backend_probe_errors,
        input_backend_kind: selection.input_backend_kind,
        focus_supported: selection.focus_supported,
        typing_supported: selection.typing_supported,
        click_supported: selection.click_supported,
        verification_supported: selection.verification_supported,
        xdotool_usable_for_actions: selection.xdotool_usable_for_actions,
        ydotool_usable_for_actions: selection.ydotool_usable_for_actions,
        uinput_socket_path: input.uinput_socket_path,
        uinput_socket_accessible: input.uinput_socket_accessible,
        can_execute_actions: selection.can_execute_actions,
        blockers: selection.blockers,
        capabilities: selection.capabilities,
    }
}

fn fixture_probe(
    session_type: &str,
    vision_sidecar: &str,
    uinput_daemon: &str,
) -> GuiBackendProbeInput {
    GuiBackendProbeInput {
        global_halt_engaged: false,
        halt_reason: None,
        automation_enabled: true,
        orchestrator_available: true,
        session_type: session_type.into(),
        vision_sidecar: vision_sidecar.into(),
        uinput_daemon: uinput_daemon.into(),
        xdotool_available: false,
        xdotool_display_usable: false,
        ydotool_available: false,
        ydotool_permission_ok: false,
        uinput_available: uinput_daemon == "running",
        uinput_socket_path: Some("/run/user/1000/kria-uinput.sock".into()),
        uinput_socket_accessible: uinput_daemon == "running",
    }
}

fn fixture_backend_status(fixture: &GuiActionBackendFixture) -> GuiActionBackendStatus {
    let input = match fixture {
        GuiActionBackendFixture::GlobalHalt => {
            let mut input = fixture_probe("wayland", "stopped", "stopped");
            input.global_halt_engaged = true;
            input.halt_reason = Some("test fixture global safety halt".into());
            input.automation_enabled = false;
            input.xdotool_available = true;
            input
        }
        GuiActionBackendFixture::StartupWarming => {
            let mut input = fixture_probe("wayland", "starting", "starting");
            input.global_halt_engaged = true;
            input.halt_reason =
                Some("service warming up (vision=starting, uinput=starting)".into());
            input.xdotool_available = true;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::VisionFailed => {
            let mut input = fixture_probe("wayland", "failed", "running");
            input.global_halt_engaged = true;
            input.halt_reason = Some("service not ready (vision=FAILED, uinput=ok)".into());
            input.xdotool_available = true;
            input
        }
        GuiActionBackendFixture::UinputFailed => {
            let mut input = fixture_probe("wayland", "running", "failed");
            input.global_halt_engaged = true;
            input.halt_reason = Some("service not ready (vision=ok, uinput=FAILED)".into());
            input.xdotool_available = true;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::UserDisabled => {
            let mut input = fixture_probe("wayland", "running", "running");
            input.automation_enabled = false;
            input.halt_reason = Some("user disabled automation via UI".into());
            input.xdotool_available = true;
            input
        }
        GuiActionBackendFixture::ServicesHealthy | GuiActionBackendFixture::WaylandUinput => {
            let mut input = fixture_probe("wayland", "running", "running");
            input.xdotool_available = true;
            input
        }
        GuiActionBackendFixture::WaylandNoBackend | GuiActionBackendFixture::WaylandXdotoolOnly => {
            let mut input = fixture_probe("wayland", "running", "stopped");
            input.xdotool_available = true;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::WaylandUinputSocketDenied => {
            let mut input = fixture_probe("wayland", "running", "running");
            input.xdotool_available = true;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::WaylandYdotoolPermissionDenied => {
            let mut input = fixture_probe("wayland", "running", "stopped");
            input.xdotool_available = true;
            input.ydotool_available = true;
            input.ydotool_permission_ok = false;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::WaylandYdotoolReady => {
            let mut input = fixture_probe("wayland", "running", "stopped");
            input.xdotool_available = true;
            input.ydotool_available = true;
            input.ydotool_permission_ok = true;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::X11Xdotool => {
            let mut input = fixture_probe("x11", "running", "stopped");
            input.xdotool_available = true;
            input.xdotool_display_usable = true;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::X11XdotoolDisplayFailed => {
            let mut input = fixture_probe("x11", "running", "stopped");
            input.xdotool_available = true;
            input.xdotool_display_usable = false;
            input.uinput_available = false;
            input.uinput_socket_accessible = false;
            input
        }
        GuiActionBackendFixture::UnknownSession => {
            let mut input = fixture_probe("unknown", "running", "running");
            input.uinput_socket_accessible = false;
            input
        }
    };
    let startup_elapsed =
        matches!(fixture, GuiActionBackendFixture::StartupWarming).then_some(1_250);
    let mut status = backend_status_from_probe(input);
    status.startup_elapsed_ms = startup_elapsed;
    status
}

pub(super) async fn build_gui_action_backend_status(
    app_state: &AppState,
    fixture: Option<&GuiActionBackendFixture>,
) -> GuiActionBackendStatus {
    if let Some(fixture) = fixture {
        return fixture_backend_status(fixture);
    }

    let session_type = current_session_type();
    let xdotool_available = executable_available("xdotool");
    let ydotool_available = executable_available("ydotool");
    let global_halt_engaged = kria_core::safety::is_halted();
    let halt_reason = kria_core::safety::halt_reason();

    let (vision_sidecar, uinput_daemon, automation_enabled, orchestrator_available) =
        match app_state.gui_orchestrator.as_ref() {
            Some(orch) => {
                let status = orch.status().await;
                (
                    service_liveness_label(status.vision_sidecar),
                    service_liveness_label(status.uinput_daemon),
                    status.automation_enabled,
                    true,
                )
            }
            None => ("stopped".into(), "stopped".into(), false, false),
        };
    let uinput_available = uinput_daemon == "running";
    let uinput_socket_path = kria_core::agent::gui_services::default_uinput_socket_path();
    let uinput_socket_accessible =
        uinput_available && uinput_socket_accessible(&uinput_socket_path).await;
    let xdotool_display_usable = xdotool_display_usable(&session_type, xdotool_available).await;
    let ydotool_permission_ok = ydotool_permission_probe(ydotool_available).await;

    backend_status_from_probe(GuiBackendProbeInput {
        global_halt_engaged,
        halt_reason,
        automation_enabled,
        orchestrator_available,
        vision_sidecar,
        uinput_daemon,
        session_type,
        xdotool_available,
        xdotool_display_usable,
        ydotool_available,
        ydotool_permission_ok,
        uinput_available,
        uinput_socket_path: Some(uinput_socket_path.display().to_string()),
        uinput_socket_accessible,
    })
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuiActionBackendFixture {
    GlobalHalt,
    StartupWarming,
    VisionFailed,
    UinputFailed,
    UserDisabled,
    ServicesHealthy,
    WaylandNoBackend,
    WaylandXdotoolOnly,
    WaylandUinput,
    WaylandUinputSocketDenied,
    WaylandYdotoolPermissionDenied,
    WaylandYdotoolReady,
    X11Xdotool,
    X11XdotoolDisplayFailed,
    UnknownSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GuiPerceptionFixture {
    TextFieldAndButton,
    GnomeBridgeReliable,
    GnomeEvalFallback,
    BridgeMissingFallback,
    AtspiFallback,
    SingleWindowBestEffort,
    FailureChainPrecise,
    SecretActiveWindow,
    ChromeCdpFocusSearchBox,
    FirefoxBidiFocusSearchBox,
    VscodeExtensionEditorFocus,
    VscodeExtensionTerminalFocus,
    GnomeTerminalFocus,
    AllFocusAdaptersUnavailable,
    DuplicateSearchButtons,
    HiddenDisabledControls,
    OcrOnlyControl,
    FocusedSearchField,
    VisualOnlyButton,
    Step8ClickResultChanges,
    Step8ClickNoChange,
    Step8TypedTextVisible,
    Step8SecretTypeStateChanges,
    Step9FocusRecovers,
}

#[derive(Debug, Clone, Default)]
pub(super) struct GuiCognitionCommandOptions {
    pub llm_planner_fixture: Option<GuiLlmPlannerFixture>,
    pub disable_live_llm_planner: bool,
    pub action_backend_fixture: Option<GuiActionBackendFixture>,
    pub perception_fixture: Option<GuiPerceptionFixture>,
    pub hitl_decision_fixture: Option<GuiHitlDecisionFixture>,
    pub execution_mode: GuiExecutionMode,
    pub workflow_enabled: bool,
    pub workflow_resume: bool,
    pub resume_reason: Option<String>,
}

pub(super) async fn desktop_gui_cognition_command_capture(
    message: String,
    app_state: &AppState,
    session_id_override: Option<String>,
    event_scope_prefix: &str,
    options: Option<GuiCognitionCommandOptions>,
) -> Result<super::chat::DesktopChatCommandCapture, String> {
    let session_id = match session_id_override.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => app_state.current_session_id.read().await.clone(),
    };
    let turn_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4().to_string();

    let options = options.unwrap_or_default();
    let perception = match options.perception_fixture {
        Some(fixture) => {
            GuiPerceptionProviderAdapter::Fixture(FixtureGuiPerceptionProvider::new(fixture))
        }
        None => GuiPerceptionProviderAdapter::Live(DesktopGuiPerceptionProvider {
            app_state,
            screenshot_bytes: OnceCell::new(),
            atspi_snapshot: OnceCell::new(),
            cache_policy: gui_observation_cache_policy_for_prompt(&message),
        }),
    };
    let backend_status =
        build_gui_action_backend_status(app_state, options.action_backend_fixture.as_ref()).await;
    let executor = DesktopGuiActionExecutor {
        app_state,
        backend_status,
        execution_mode: options.execution_mode,
    };
    let fixture_planner = options
        .llm_planner_fixture
        .clone()
        .map(FixtureGuiLlmPlanner::new);
    let live_planner = if fixture_planner.is_none() && !options.disable_live_llm_planner {
        app_state
            .model_router
            .route("gui_cognition_planner")
            .await
            .map(LlmBackendGuiPlanner::new)
    } else {
        None
    };
    let planner_ref: Option<&dyn GuiLlmPlanner> = match (&fixture_planner, &live_planner) {
        (Some(planner), _) => Some(planner),
        (None, Some(planner)) => Some(planner),
        (None, None) => None,
    };
    let runtime = GuiCognitionRuntime::new(&perception, &executor).with_llm_planner(planner_ref);
    // Step 11: load a previously saved checkpoint for this session when resuming.
    let resume_checkpoint = if options.workflow_resume {
        load_session_checkpoint(&session_id)
    } else {
        None
    };
    let mut outcome = runtime
        .run_turn(GuiTurnRequest {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            workflow_id: workflow_id.clone(),
            message: message.clone(),
            route_path: "send_manual_tool_message".into(),
            llm_tool_loop: false,
            hitl_decision_fixture: options.hitl_decision_fixture.clone(),
            execution_mode: options.execution_mode,
            workflow_enabled: options.workflow_enabled,
            resume_checkpoint,
            resume_reason: options.resume_reason.clone(),
        })
        .await;

    // Step 11: persist the latest checkpoint for this session (in-memory store).
    if let Some(checkpoint) = outcome
        .response
        .pointer("/gui_cognition/workflow_checkpoint")
        .filter(|value| !value.is_null())
        .cloned()
    {
        store_session_checkpoint(&session_id, checkpoint);
    }

    if let Some(proposal_value) = outcome
        .response
        .pointer("/gui_cognition/safety_gate/proposal")
        .cloned()
    {
        if outcome
            .response
            .pointer("/gui_cognition/safety_gate/safety_status")
            .and_then(serde_json::Value::as_str)
            == Some("approval_required")
        {
            if let Ok(proposal) = serde_json::from_value::<
                kria_core::agent::gui_cognition::safety_hitl::GuiActionProposal,
            >(proposal_value)
            {
                let mut store = app_state.gui_cognition_hitl_proposals.write().await;
                for decision in store.insert_pending(proposal) {
                    outcome.events.push(decision.invalidated_event_payload());
                }
            }
        }
    }

    let mut events = vec![super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:thinking"),
        serde_json::json!({"status": "processing", "mode": "gui_cognition"}),
    )];
    for (index, event) in outcome.events.into_iter().enumerate() {
        if event.get("type").and_then(serde_json::Value::as_str) == Some("HitlRequired") {
            if let Some(approval_request) = event.get("approval_request").cloned() {
                events.push(super::chat::desktop_chat_event(
                    format!("{event_scope_prefix}:approval_required"),
                    approval_request,
                ));
            }
        }
        events.push(super::chat::desktop_chat_event(
            "gui_cognition:event",
            gui_cognition_event_payload(
                &session_id,
                &turn_id,
                &workflow_id,
                (index + 1) as u64,
                event,
            ),
        ));
    }

    let memory_writer: Arc<dyn MemoryManager> = app_state.memory_store.clone();
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id.clone(),
        message,
        String::new(),
        None,
        None,
        None,
    ));
    let _ = memory_writer.store_turn(&memory_turn_write(
        session_id,
        String::new(),
        outcome.reply.clone(),
        Some("gui_cognition".into()),
        Some(outcome.response["gui_cognition"].to_string()),
        None,
    ));

    events.push(super::chat::desktop_chat_stage_event(
        "gui_cognition_mode_handled",
        "GUI Cognition prompt handled by dedicated selected-mode route",
        Some(serde_json::json!({
            "path": "send_manual_tool_message",
            "llm_tool_loop": false,
            "workflow_id": workflow_id,
        })),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:token"),
        serde_json::json!({ "text": outcome.reply.clone() }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:tool_result"),
        serde_json::json!({
            "tool": "gui_cognition",
            "result": outcome.response.clone(),
        }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:done"),
        serde_json::json!({}),
    ));

    Ok(super::chat::DesktopChatCommandCapture {
        status_code: 200,
        status: "processing".into(),
        reply: outcome.reply,
        response: outcome.response,
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_cognition_event_payload_contains_required_envelope_fields() {
        let payload = gui_cognition_event_payload(
            "session-1",
            "turn-1",
            "workflow-1",
            7,
            serde_json::json!({ "type": "TurnStarted" }),
        );

        assert_eq!(payload["version"], 1);
        assert_eq!(payload["session_id"], "session-1");
        assert_eq!(payload["turn_id"], "turn-1");
        assert_eq!(payload["workflow_id"], "workflow-1");
        assert_eq!(payload["sequence"], 7);
        assert_eq!(payload["event"]["type"], "TurnStarted");
        assert!(payload["timestamp_ms"].as_i64().unwrap_or_default() > 0);
    }

    #[test]
    fn gui_cognition_event_payload_sequences_can_be_monotonic() {
        let first = gui_cognition_event_payload(
            "session-1",
            "turn-1",
            "workflow-1",
            1,
            serde_json::json!({ "type": "TurnStarted" }),
        );
        let second = gui_cognition_event_payload(
            "session-1",
            "turn-1",
            "workflow-1",
            2,
            serde_json::json!({ "type": "RouteConfirmed" }),
        );

        assert!(second["sequence"].as_u64().unwrap() > first["sequence"].as_u64().unwrap());
    }
}
