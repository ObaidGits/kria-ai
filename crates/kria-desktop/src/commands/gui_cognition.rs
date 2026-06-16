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
use kria_core::agent::gui_cognition::goal_contract::{extract_gui_goal_contract, GuiActionType};
use kria_core::agent::gui_cognition::safety_hitl::GuiHitlDecisionFixture;
use kria_core::agent::gui_cognition::{GuiCognitionRuntime, GuiTurnRequest};
use kria_core::agent::gui_cognition::event_stream::{GuiEventStreamSink, GuiStreamUxConfig};
use tauri::{AppHandle, Emitter};
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
    // Per-observation capture cache: the screenshot is captured ONCE per
    // observation and shared by the screenshot/OCR/visual probes. It is CLEARED
    // by `begin_observation` at the start of each fresh observation so a
    // re-observe in the SAME turn (the pre/post pair of a `screen_changed`
    // verification, or a multi-step combo) captures a fresh frame instead of
    // reusing the turn's first capture. (`OnceCell` could not be reset, which
    // made `screen_changed` always false for single-turn scroll/key actions.)
    screenshot_bytes: Mutex<Option<Result<StdArc<Vec<u8>>, String>>>,
    atspi_snapshot: OnceCell<Result<StdArc<AtSpiSnapshot>, String>>,
    cache_policy: GuiObservationCachePolicy,
    // Task 3 (Issue #9): when set, the current observation MUST bypass the OCR
    // cache (`GUI_OCR_CACHE`) and the screenshot memo so a post-action
    // verification re-observe is a TRUE fresh capture, never a pre-action cached
    // frame. Set/reset by `set_force_fresh` from
    // `collect_observation_with_freshness`; only ever true when the
    // `gui_cog_cache_coherence` flag is ON, so flag-OFF behavior is unchanged.
    force_fresh: std::sync::atomic::AtomicBool,
    // Task 9 (Issue #7): the turn's OCR scope, derived once from the prompt at
    // construction. `run_ocr` skips OCR for an `ActionIntent` turn when the
    // `gui_cog_ocr_quality` flag is ON (the verdict never reads screen text);
    // flag-OFF ignores this and runs OCR on every observation as before.
    ocr_scope: GuiOcrScope,
    // Task 12 (Issue #6): the turn's observation profile, derived once from the
    // goal-contract action type. A `FastAction` turn (open/scroll/key/switch)
    // skips the slow OCR + vision probes when the `gui_cog_fast_observe` flag is
    // ON (the verdict is active-window/screen-change evidence, captured by the
    // cheap probes); flag-OFF runs every probe as before.
    observe_profile: GuiObserveProfile,
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
    // Execute-and-verify-by-screen-change primitives (Scroll / PressKey) MUST
    // observe FRESH both before AND after the action. They classify as `Observe`
    // (their `intent_kind` is "scroll"/"press_key", which the intent map folds to
    // Observe), so without this guard they would use `ObservePlanShort` caching —
    // and the post-action re-observe, falling inside the 750ms TTL, would be
    // served the STALE pre-action snapshot, making `screen_changed` always false
    // (a false `verification_failed`). Disable the cache for them. Flag-gated by
    // `KRIA_GUI_COG_PRIMITIVES` (default-ON); flag-OFF keeps the prior caching
    // byte-for-byte.
    if primitives_cache_bypass_enabled() {
        let action = extract_gui_goal_contract(message, None).contract.action_type;
        if matches!(action, GuiActionType::Scroll | GuiActionType::PressKey) {
            return GuiObservationCachePolicy::Disabled;
        }
    }
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

/// Whether the screen-change primitive cache-bypass (Scroll / PressKey fresh
/// pre/post observation) is enabled. Shares the `gui_cog_primitives` flag
/// (`KRIA_GUI_COG_PRIMITIVES`, default-ON); an explicit falsy value
/// (`0`/`false`/`no`/`off`/empty) restores the prior caching behavior.
/// Task 7/8 (Issue #4/#1): per-observation budget (ms) for the visual-control
/// detector (`detect_visual_controls`). Default 950 ms preserves the prior
/// latency behavior byte-for-byte (a real VL-7B grounding that does not finish
/// in time is honestly reported as `timeout`). Raise via
/// `KRIA_GUI_COG_VISION_BUDGET_MS` when the resident VL-7B is served and real
/// vision-resolved bounds are wanted (e.g. for the Task 7 abs-pointer click),
/// trading latency for grounded detections. Clamped to [100, 60000].
fn visual_detector_budget_ms() -> u64 {
    match std::env::var("KRIA_GUI_COG_VISION_BUDGET_MS") {
        Ok(v) => v.trim().parse::<u64>().unwrap_or(950).clamp(100, 60_000),
        Err(_) => 950,
    }
}

fn primitives_cache_bypass_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_PRIMITIVES") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

/// Task 9 (Issue #7): whether the OCR quality + scope improvements are enabled —
/// (a) crop OCR to the active-window region-of-interest at an ADEQUATE
/// resolution (no blind 1920→1000 over-downscale), and (b) intent-gate OCR so it
/// runs only on read/summarize turns instead of every observation. Default ON;
/// an explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_OCR_QUALITY` rolls back to the prior full-screen,
/// every-observation OCR path byte-for-byte. An absent env value keeps it ON.
fn ocr_quality_enabled() -> bool {
    ocr_quality_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`ocr_quality_enabled`] with an injectable env lookup.
fn ocr_quality_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_OCR_QUALITY") {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        None => true,
    }
}

/// Task 10 (Issue #8): whether the consolidated, honest AT-SPI health signal is
/// surfaced in the accessibility source-status payload (`atspi_health`,
/// `atspi_resolution_trustworthy`, `atspi_health_reason`). Default ON; an
/// explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_ATSPI_HEALTH` rolls back to the prior payload byte-for-byte
/// (no health fields). Additive-only: the underlying snapshot/confidence
/// behavior is unchanged either way — this only adds telemetry the resolver/UI
/// can consult to prefer the extension/vision path when AT-SPI is degraded.
fn atspi_health_enabled() -> bool {
    atspi_health_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`atspi_health_enabled`] with an injectable env lookup.
fn atspi_health_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_ATSPI_HEALTH") {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        None => true,
    }
}

/// Task 12 (Issue #6): whether intent-aware fast observation is enabled — skip
/// the two SLOW probes (OCR + visual-control detection) for primitive ACTION
/// turns whose verdict needs neither (open/scroll/key/switch are verified by
/// the active-window or screen-change evidence, never OCR text or vision
/// boxes). Default ON; an explicit falsy value (`0`/`false`/`no`/`off`/empty)
/// in `KRIA_GUI_COG_FAST_OBSERVE` rolls back to running every probe on every
/// observation byte-for-byte. An absent env value keeps it ON.
fn fast_observe_enabled() -> bool {
    fast_observe_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`fast_observe_enabled`] with an injectable env lookup.
fn fast_observe_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_FAST_OBSERVE") {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        None => true,
    }
}

/// Task 11 (Issue #2): whether the LOCAL grammar planner rung (Rung B) is wired.
/// Default ON; an explicit falsy value (`0`/`false`/`no`/`off`/empty) in
/// `KRIA_GUI_COG_LOCAL_PLANNER` rolls back to the prior cloud→deterministic
/// ladder byte-for-byte (no local fallback planner wired → Rung A → Rung C). An
/// absent env value keeps it ON. The rung itself ALSO requires the
/// `gui_cog_structured_planner` ladder + a distinct grammar-capable local
/// backend; this flag is the dedicated kill-switch for the local rung.
fn local_planner_enabled() -> bool {
    local_planner_enabled_lookup(|key| std::env::var(key).ok())
}

/// Testable core of [`local_planner_enabled`] with an injectable env lookup.
fn local_planner_enabled_lookup<F>(lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KRIA_GUI_COG_LOCAL_PLANNER") {
        Some(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        None => true,
    }
}

/// Task 8 (Issue #1): real visual-perception mode. `Vl7b` (default) consumes
/// real VL-7B grounding detections + honestly degrades when the sidecar reports
/// a stub/unavailable model (never presents fabricated boxes as authoritative).
/// `Light` is the OCR+heuristic fallback. `Off` restores the prior perception
/// path byte-for-byte (the sidecar result is passed through unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiRealVisionMode {
    Vl7b,
    Light,
    Off,
}

/// Parse the `gui_cog_real_vision` mode from an env value. Default (absent or
/// unrecognized truthy) is `Vl7b`; `off`/falsy → `Off`; `light` → `Light`.
fn gui_real_vision_mode_from(value: Option<&str>) -> GuiRealVisionMode {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        None => GuiRealVisionMode::Vl7b,
        Some(v) => match v.as_str() {
            "0" | "false" | "no" | "off" | "" => GuiRealVisionMode::Off,
            "light" => GuiRealVisionMode::Light,
            // "vl7b", "1", "true", "on", or any other truthy value -> VL-7B.
            _ => GuiRealVisionMode::Vl7b,
        },
    }
}

/// The active `gui_cog_real_vision` mode from `KRIA_GUI_COG_REAL_VISION`.
fn gui_real_vision_mode() -> GuiRealVisionMode {
    gui_real_vision_mode_from(std::env::var("KRIA_GUI_COG_REAL_VISION").ok().as_deref())
}

/// Task 12: the observation profile for a turn. `FastAction` turns skip the slow
/// OCR + vision probes (the verdict is the active-window / screen-change
/// evidence, which the cheap probes still capture); `Full` turns run every
/// probe. The verdict-critical probes (`capture_screenshot` for the screen hash,
/// `get_active_window`, accessibility) are NEVER skipped — only OCR + vision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiObserveProfile {
    /// Primitive action (open/scroll/key/switch) — skip OCR + vision.
    FastAction,
    /// Read / element / plan / in-app resolve turn — run every probe.
    Full,
}

/// Classify a turn into a [`GuiObserveProfile`] from the GOAL CONTRACT action
/// type (authoritative for the primitives). Only the screen-change / active-
/// window-verified primitives are fast-pathed; control-resolving intents
/// (click/type-into-field) and read/summarize turns stay `Full` so vision (for
/// resolution) and OCR (for reading) are available.
fn gui_observe_profile_for_prompt(message: &str) -> GuiObserveProfile {
    let action = extract_gui_goal_contract(message, None).contract.action_type;
    match action {
        GuiActionType::OpenApp
        | GuiActionType::Scroll
        | GuiActionType::PressKey
        | GuiActionType::SwitchWindow => GuiObserveProfile::FastAction,
        _ => GuiObserveProfile::Full,
    }
}

/// Task 9: the OCR scope for a turn. OCR is expensive and only meaningful for
/// read/summarize intents. A pure ACTION turn (focus/type/click/scroll/key/
/// approval) never reads screen TEXT for its verdict — verification uses the
/// screen-change / active-window / clipboard evidence sources, never OCR text
/// (the Task 4 evidence contract). So OCR is skipped for action turns when the
/// flag is ON (an honest, benign empty result), and runs for read turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuiOcrScope {
    /// Read / summarize / observe / plan turn — OCR runs (ROI + adequate res).
    ReadIntent,
    /// Pure action turn — OCR is skipped (honest empty result) when flag ON.
    ActionIntent,
}

/// Classify a turn prompt into an [`GuiOcrScope`]. The ACTION set mirrors the
/// observation-cache `Disabled` set exactly (focus/type/click/safe-action/
/// risk-approval) so the two intent gates stay consistent; everything else
/// (observe/analyze/browser-search/fill-form/checks/recovery) is read-scoped.
fn gui_ocr_scope_for_prompt(message: &str) -> GuiOcrScope {
    match classify_gui_cognition_prompt(message).kind {
        GuiCognitionIntentKind::FocusInput
        | GuiCognitionIntentKind::TypeText
        | GuiCognitionIntentKind::ClickControl
        | GuiCognitionIntentKind::SafeAction
        | GuiCognitionIntentKind::RiskApproval => GuiOcrScope::ActionIntent,
        _ => GuiOcrScope::ReadIntent,
    }
}

/// Task 9: a PHYSICAL-pixel region of interest (the active window's frame rect)
/// to crop a screenshot to before OCR, so text is OCR'd at an adequate
/// resolution instead of a blindly down-scaled full (possibly multi-monitor)
/// span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OcrRoi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl OcrRoi {
    /// Clamp the ROI to the captured image. Returns `None` (→ caller uses the
    /// full frame) when the region does not sanely fit — fully outside, or the
    /// in-bounds remainder is smaller than a minimum content region — so a bad
    /// or stale bounds value can never crop OCR down to a useless sliver.
    fn clamp_to(self, img_w: u32, img_h: u32) -> Option<OcrRoi> {
        const MIN_ROI_EDGE: u32 = 64;
        if img_w == 0 || img_h == 0 {
            return None;
        }
        let x = self.x.min(img_w.saturating_sub(1));
        let y = self.y.min(img_h.saturating_sub(1));
        let width = self.width.min(img_w - x);
        let height = self.height.min(img_h - y);
        if width < MIN_ROI_EDGE || height < MIN_ROI_EDGE {
            return None;
        }
        Some(OcrRoi {
            x,
            y,
            width,
            height,
        })
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
        // Return the per-observation cached capture if present; otherwise capture
        // once and memoize for the rest of THIS observation. `begin_observation`
        // clears the slot so the next observation re-captures.
        {
            let guard = self.screenshot_bytes.lock().await;
            if let Some(cached) = guard.as_ref() {
                return cached.clone();
            }
        }
        // On Wayland an external xcap/portal grab is blocked or blind to native
        // Wayland windows (it returns the desktop background only), so
        // screen-change / OCR / element verification cannot see app content.
        // Prefer the KRIA GNOME Shell extension's in-shell `Shell.Screenshot`
        // capture (full composited stage, all windows) and fall back to xcap when
        // the extension is unavailable.
        let wayland = std::env::var("XDG_SESSION_TYPE")
            .unwrap_or_default()
            .eq_ignore_ascii_case("wayland");
        let captured: Result<StdArc<Vec<u8>>, String> = if wayland
            && kria_ext::ext_capture_enabled()
        {
            if let Some(bytes) = kria_ext::ext_capture_screen().await {
                tracing::info!(target: "gui_capture", backend = "extension", bytes = bytes.len(), "screen capture via GNOME extension");
                Ok(StdArc::new(bytes))
            } else {
                tracing::warn!(target: "gui_capture", "extension capture unavailable; falling back to xcap (blind on Wayland)");
                ScreenshotCapture::capture_full()
                    .await
                    .map(StdArc::new)
                    .map_err(|err| format!("screenshot capture unavailable: {err}"))
            }
        } else {
            ScreenshotCapture::capture_full()
                .await
                .map(StdArc::new)
                .map_err(|err| format!("screenshot capture unavailable: {err}"))
        };
        let mut guard = self.screenshot_bytes.lock().await;
        *guard = Some(captured.clone());
        captured
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

    // Retained as the byte-for-byte legacy reference (flag-OFF path delegates to
    // the scoped variant; the parity test asserts they match). Not called on the
    // hot path, hence `allow(dead_code)`.
    #[allow(dead_code)]
    fn prepare_ocr_png(bytes: &[u8]) -> Result<(Vec<u8>, String), String> {
        // Legacy / flag-OFF path: full-frame, downscaled to 1000px. Delegated to
        // the scoped variant with `quality_on=false` so it stays byte-for-byte.
        Self::prepare_ocr_png_scoped(bytes, None, false)
    }

    /// Task 9 (Issue #7): scope- and quality-aware OCR preprocessing.
    ///
    /// - `quality_on=false` (flag OFF): EXACTLY the prior behavior — no crop,
    ///   downscale the full frame to 1000px (`LEGACY_MAX_OCR_WIDTH`). Byte-for-
    ///   byte identical to the legacy `prepare_ocr_png`.
    /// - `quality_on=true` (flag ON): crop to the active-window `roi` (clamped to
    ///   the image; ignored if it does not sanely fit) and downscale only when
    ///   wider than 1600px (`QUALITY_MAX_OCR_WIDTH`), so 1080p/1440p text is OCR'd
    ///   at an adequate resolution instead of being blindly over-downscaled.
    fn prepare_ocr_png_scoped(
        bytes: &[u8],
        roi: Option<OcrRoi>,
        quality_on: bool,
    ) -> Result<(Vec<u8>, String), String> {
        const LEGACY_MAX_OCR_WIDTH: u32 = 1000;
        const QUALITY_MAX_OCR_WIDTH: u32 = 1600;
        let mut image = image::load_from_memory(bytes)
            .map_err(|error| format!("OCR unavailable: screenshot decode failed: {error}"))?;
        let full_width = image.width();
        let full_height = image.height();

        // Flag-ON: crop to the active-window ROI (physical px) so OCR sees the
        // content region at full detail. A ROI that does not sanely fit the
        // captured image is ignored (the full frame is used).
        let mut roi_status = "full_frame".to_string();
        if quality_on {
            if let Some(region) = roi.and_then(|r| r.clamp_to(full_width, full_height)) {
                image = image.crop_imm(region.x, region.y, region.width, region.height);
                roi_status = format!(
                    "roi_{}x{}+{}+{}",
                    region.width, region.height, region.x, region.y
                );
            }
        }

        let work_width = image.width();
        let work_height = image.height();
        let max_width = if quality_on {
            QUALITY_MAX_OCR_WIDTH
        } else {
            LEGACY_MAX_OCR_WIDTH
        };
        let resized = work_width > max_width;
        let image = if resized {
            let target_height = ((work_height as f64) * (max_width as f64 / work_width as f64))
                .round()
                .max(1.0) as u32;
            image.resize(max_width, target_height, image::imageops::FilterType::Triangle)
        } else {
            image
        };

        let status = if quality_on {
            let scale_part = if resized {
                format!(
                    "downscaled_{work_width}x{work_height}_to_{}x{}",
                    image.width(),
                    image.height()
                )
            } else {
                format!("adequate_{work_width}x{work_height}")
            };
            format!("quality_{roi_status}_{scale_part}_from_{full_width}x{full_height}")
        } else if resized {
            // Legacy status string — byte-for-byte with the prior implementation.
            format!(
                "downscaled_{full_width}x{full_height}_to_{}x{}",
                image.width(),
                image.height()
            )
        } else {
            format!("original_{full_width}x{full_height}")
        };

        let mut buffer = Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .map_err(|error| format!("OCR unavailable: screenshot preprocess failed: {error}"))?;
        Ok((buffer.into_inner(), status))
    }

    /// Task 9: best-effort PHYSICAL-pixel ROI from the GNOME extension's focused
    /// window frame rect (`GetFocusedWindow`). Bounded + fail-open: any
    /// missing/invalid value returns `None` so OCR falls back to the full frame.
    /// The frame rect is logical px; the screenshot is physical px, so the rect
    /// is scaled by the monitor scale.
    async fn active_window_ocr_roi() -> Option<OcrRoi> {
        let token = kria_ext::read_ext_token()?;
        let value = kria_ext::ext_call("GetFocusedWindow", &[token.as_str()], 400).await?;
        let window = value.get("window")?;
        if window.is_null() {
            return None;
        }
        let x = window.get("x").and_then(serde_json::Value::as_i64)?;
        let y = window.get("y").and_then(serde_json::Value::as_i64)?;
        let w = window.get("w").and_then(serde_json::Value::as_i64)?;
        let h = window.get("h").and_then(serde_json::Value::as_i64)?;
        if w <= 0 || h <= 0 {
            return None;
        }
        let scale = window
            .get("scale")
            .and_then(serde_json::Value::as_f64)
            .filter(|s| s.is_finite() && *s >= 1.0)
            .unwrap_or(1.0);
        let to_phys = |v: i64| -> u32 { ((v.max(0) as f64) * scale).round().max(0.0) as u32 };
        Some(OcrRoi {
            x: to_phys(x),
            y: to_phys(y),
            width: to_phys(w),
            height: to_phys(h),
        })
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
        Self::snapshot_source_status_with_health(snapshot, atspi_health_enabled())
    }

    /// Task 10: build the accessibility source-status payload. When `health_on`
    /// (the `gui_cog_atspi_health` flag), the consolidated honest health signal
    /// (`atspi_health` / `atspi_resolution_trustworthy` / `atspi_health_reason`)
    /// is added so the resolver/UI can prefer the extension/vision path on a
    /// degraded/unavailable snapshot. `health_on=false` returns the prior
    /// payload byte-for-byte (no health fields).
    fn snapshot_source_status_with_health(
        snapshot: &AtSpiSnapshot,
        health_on: bool,
    ) -> serde_json::Value {
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
        let mut payload = serde_json::json!({
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
        });
        if health_on {
            // Task 10: consolidated honest health (additive). When degraded/
            // unavailable, `atspi_resolution_trustworthy=false` signals the
            // resolver/UI to PREFER the extension/vision path over low-trust
            // AT-SPI candidates.
            let health = snapshot.health();
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "atspi_health".into(),
                    serde_json::json!(health.level.as_str()),
                );
                object.insert(
                    "atspi_resolution_trustworthy".into(),
                    serde_json::json!(health.resolution_trustworthy),
                );
                object.insert(
                    "atspi_health_reason".into(),
                    serde_json::json!(health.reason),
                );
            }
        }
        payload
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
    async fn begin_observation(&self) {
        // Drop the prior observation's memoized screenshot so this observation
        // captures a FRESH frame. Without this, the provider (which lives for the
        // whole turn) reused the turn's first capture for both the pre- and
        // post-action observations, making `screen_changed` always false. Gated
        // by the extension-capture flag (`KRIA_GUI_COG_EXT_CAPTURE`, default-ON);
        // flag-OFF keeps the prior per-turn memoization byte-for-byte.
        if kria_ext::ext_capture_enabled()
            || self
                .force_fresh
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            let mut guard = self.screenshot_bytes.lock().await;
            *guard = None;
        }
    }

    fn set_force_fresh(&self, force_fresh: bool) {
        // Task 3 (Issue #9): toggle the per-turn cache bypass for the current
        // (post-action / verification) observation. `run_ocr` consults this to
        // skip the OCR cache, and `begin_observation` clears the screenshot memo
        // when set, so the post-action frame is a true fresh capture.
        self.force_fresh
            .store(force_fresh, std::sync::atomic::Ordering::SeqCst);
    }

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
        // Task 9 (Issue #7): intent-gated OCR. On a pure ACTION turn the verdict
        // never reads screen text (Task 4 evidence contract), so skip the
        // expensive OCR entirely and return an honest, benign empty result.
        // Flag-OFF keeps OCR running on every observation (byte-for-byte).
        let quality_on = ocr_quality_enabled();
        // Task 12 (Issue #6): a FastAction turn (open/scroll/key/switch) also
        // skips OCR — its verdict is active-window/screen-change evidence.
        let fast_skip =
            fast_observe_enabled() && matches!(self.observe_profile, GuiObserveProfile::FastAction);
        if fast_skip || (quality_on && matches!(self.ocr_scope, GuiOcrScope::ActionIntent)) {
            let total_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let reason = if fast_skip {
                "skipped_fast_observe"
            } else {
                "skipped_non_read_intent"
            };
            return GuiProbeResult::ok(serde_json::json!({
                "text": "",
                "source": "intent_gated_skip",
                "ocr_engine": "none",
                "ocr_engine_status": reason,
                "ocr_scope": if fast_skip { "fast_action" } else { "action_intent" },
                "ocr_fast_path": "intent_gated_skip",
                "ocr_cache_hit": false,
                "ocr_roi_count": 0,
                "ocr_changed_region_count": 0,
                "ocr_wait_for_screenshot_ms": 0,
                "ocr_total_ms": total_ms,
            }));
        }
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
        // Task 3 (Issue #9): a force-fresh (post-action verification) observation
        // bypasses the OCR cache so it can never reuse a pre-action OCR result.
        if !self.force_fresh.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(cached) =
                Self::cached_ocr_result(&screen_hash, wait_for_screenshot_ms, started).await
            {
                return cached;
            }
        }
        let (ocr_bytes, ocr_image_status) = match Self::prepare_ocr_png_scoped(
            bytes.as_ref(),
            if quality_on {
                Self::active_window_ocr_roi().await
            } else {
                None
            },
            quality_on,
        ) {
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
            // Task 10 (Issue #8): surface the honest consolidated health on the
            // common observe path too (additive; flag-OFF = prior payload).
            if atspi_health_enabled() {
                let health = snapshot.health();
                object.insert(
                    "atspi_health".into(),
                    serde_json::json!(health.level.as_str()),
                );
                object.insert(
                    "atspi_resolution_trustworthy".into(),
                    serde_json::json!(health.resolution_trustworthy),
                );
                object.insert(
                    "atspi_health_reason".into(),
                    serde_json::json!(health.reason),
                );
            }
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
        // Task 12 (Issue #6): a FastAction turn (open/scroll/key/switch) skips
        // vision — its verdict is active-window/screen-change evidence, never a
        // visual-control box. Honest benign empty result; flag-OFF runs vision
        // on every observation (byte-for-byte).
        if fast_observe_enabled() && matches!(self.observe_profile, GuiObserveProfile::FastAction) {
            return GuiProbeResult::ok(serde_json::json!({
                "source": "vision_sidecar",
                "visual_detector_status": "skipped_fast_observe",
                "controls": [],
                "control_count": 0,
                "visual_detector_total_ms":
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            }));
        }
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
            Duration::from_millis(visual_detector_budget_ms()),
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
        // Task 8 (Issue #1): honest real-vision gating. When the flag is NOT
        // `off`, a sidecar that reports a stub/degraded model (e.g. the dummy
        // parser, or a VL-7B OOM) must NOT have its detections presented as
        // authoritative — emit an honest `vision_degraded` with NO elements
        // rather than fabricated boxes. `off` keeps the prior pass-through
        // (byte-for-byte). The real VL-7B path (non-degraded) flows through.
        let mode = gui_real_vision_mode();
        if !matches!(mode, GuiRealVisionMode::Off) && output.degraded {
            return GuiProbeResult::ok(serde_json::json!({
                "source": "vision_sidecar",
                "visual_detector_status": "vision_degraded",
                "real_vision_mode": match mode {
                    GuiRealVisionMode::Vl7b => "vl7b",
                    GuiRealVisionMode::Light => "light",
                    GuiRealVisionMode::Off => "off",
                },
                "vision_model": output.model,
                "visual_detector_total_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                "screen_hash": Self::screenshot_hash(bytes.as_ref()),
                "elements": [],
                "notice": "vision model unavailable or stub; no authoritative visual detections (degraded honestly, not fabricated)",
            }));
        }
        let mut result = serde_json::json!({
            "source": "vision_sidecar",
            "visual_detector_status": "completed",
            "visual_detector_total_ms": started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            "screen_hash": Self::screenshot_hash(bytes.as_ref()),
            "elements": elements,
        });
        // Additive telemetry: surface the active mode + reporting model when the
        // flag is engaged (not `off`). `off` stays byte-for-byte.
        if !matches!(mode, GuiRealVisionMode::Off) {
            if let Some(object) = result.as_object_mut() {
                object.insert(
                    "real_vision_mode".into(),
                    serde_json::json!(match mode {
                        GuiRealVisionMode::Vl7b => "vl7b",
                        GuiRealVisionMode::Light => "light",
                        GuiRealVisionMode::Off => "off",
                    }),
                );
                object.insert("vision_model".into(), serde_json::json!(output.model));
            }
        }
        GuiProbeResult::ok(result)
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
    async fn begin_observation(&self) {
        match self {
            Self::Live(provider) => provider.begin_observation().await,
            Self::Fixture(provider) => provider.begin_observation().await,
        }
    }

    fn set_force_fresh(&self, force_fresh: bool) {
        match self {
            Self::Live(provider) => provider.set_force_fresh(force_fresh),
            Self::Fixture(provider) => provider.set_force_fresh(force_fresh),
        }
    }

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
                let execution = execution_from_tool_result("open_application", result);
                // Task 2 (Issue #3): open-then-act focus guarantee. After a
                // successful open on Wayland, ACTIVATE the just-opened target
                // window via the GNOME extension so it becomes the focused window
                // (a freshly launched app usually auto-focuses, but an
                // ALREADY-RUNNING app is NOT raised by gio-launch). This ensures
                // the NEXT in-app step (FocusField/TypeText/Click) resolves
                // against the right window instead of flapping on the prior one.
                // Best-effort: never changes the OpenApp verdict; bounded retry
                // because the window may still be appearing. Flag-gated; flag-OFF
                // = prior open-only behavior byte-for-byte.
                if execution.success
                    && open_then_act_focus_enabled()
                    && self.backend_status.session_type.trim().eq_ignore_ascii_case("wayland")
                    && !request.target_name.trim().is_empty()
                {
                    if let Some(token) = read_ext_token() {
                        if ext_available(&token).await {
                            let focused = ext_activate_target_with_retry(
                                &token,
                                &request.target_name,
                                5,
                                500,
                            )
                            .await;
                            tracing::info!(
                                target: "gui_open_focus",
                                app = %request.target_name,
                                focused = ?focused,
                                "open-then-act: activated opened app for focus"
                            );
                        }
                    }
                }
                execution
            }
            "focus_window" => {
                // Task 3 (Issue #1): on Wayland the X11-only `xdotool
                // windowactivate` path used by the `focus_window` tool cannot
                // raise a window. When the `gui_cog_wayland_focus` flag is active
                // (default ON; rollback via `KRIA_GUI_COG_WAYLAND_FOCUS=0`) and the
                // session is Wayland, ACTIVATE the target window via the KRIA GNOME
                // Shell extension's token-gated `ActivateWindow` (preferred — runs
                // inside gnome-shell and bypasses focus-stealing prevention), and
                // only fall back to the gio-launch (`open_application`) best-effort
                // path when the extension/token is unavailable or it did not
                // confirm focus. The result is reported honestly; the runtime's
                // re-observe verification decides verified vs. inconclusive/failed.
                // Never fabricate success. On X11 / when the flag is OFF, fall back
                // to the legacy `focus_window` (xdotool) tool byte-for-byte.
                if wayland_focus_activation_enabled()
                    && self.backend_status.session_type.trim().eq_ignore_ascii_case("wayland")
                {
                    // Issue #1 (extension wiring): PREFER the KRIA GNOME Shell
                    // extension's token-gated `ActivateWindow`, which raises and
                    // focuses the target window from *inside* gnome-shell and so
                    // bypasses Mutter's focus-stealing prevention (the gio-launch
                    // path below is only best-effort and is frequently ignored by
                    // Mutter for an already-running window). We only treat the
                    // extension path as a success when it CONFIRMS focus
                    // (`{"ok":true,"activated":true}` AND `focused_after == id`);
                    // the runtime's re-observe verification independently
                    // re-confirms. On any miss/unavailable/unconfirmed result we
                    // fall back to the unchanged gio-launch activation. Never
                    // fabricate success.
                    let mut ext_execution: Option<GuiActionExecution> = None;
                    if let Some(token) = read_ext_token() {
                        if ext_available(&token).await {
                            if let Some(true) =
                                ext_activate_target(&token, &request.target_name).await
                            {
                                ext_execution = Some(GuiActionExecution::ok(
                                    "gnome_extension_activate",
                                    serde_json::json!({ "activated": true }),
                                ));
                            }
                        }
                    }
                    match ext_execution {
                        Some(execution) => execution,
                        None => {
                            let result = self
                                .execute_tool(
                                    "open_application",
                                    serde_json::json!({
                                        "name": request.target_name,
                                        "args": [],
                                    }),
                                )
                                .await;
                            // Label the backend honestly: this is the
                            // GNOME-bridge-class gio-launch activation path, not
                            // the legacy xdotool focus.
                            execution_from_tool_result("gnome_bridge_activate", result)
                        }
                    }
                } else {
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
            "browser_addressbar_type" => {
                // Task 2 (Issue #3): atomic, vision-free browser address-bar entry.
                // The preceding OpenApp step activated the browser window; focus
                // the address bar with Ctrl+L then type the query via synthetic
                // uinput keystrokes (works without app a11y, unlike AT-SPI; proven
                // by the editor type path). Typing visibly changes the screen, so
                // the single step is reliably verifiable (screen_changed) — there
                // is NO separately-gated, unobservable focus step.
                let text = request.value.clone().unwrap_or_default();
                // Focus the address bar (best-effort; never the verdict — the type
                // below produces the observable change the verifier checks).
                let _ = self
                    .execute_tool(
                        "press_shortcut",
                        serde_json::json!({ "keys": ["ctrl", "l"] }),
                    )
                    .await;
                // Let the address bar take focus before typing.
                tokio::time::sleep(Duration::from_millis(150)).await;
                let result = self
                    .execute_tool("type_text", serde_json::json!({ "text": text }))
                    .await;
                // Submit the search (Enter) so navigation produces an observable
                // change (the address-bar text alone is a minimal pixel delta).
                tokio::time::sleep(Duration::from_millis(120)).await;
                let _ = self
                    .execute_tool("press_shortcut", serde_json::json!({ "keys": ["enter"] }))
                    .await;
                execution_from_tool_result("browser_addressbar_type", result)
            }
            "press_shortcut" => {
                let keys = match request.kind {
                    GuiActionKind::Copy => vec!["ctrl", "c"],
                    GuiActionKind::Paste => vec!["ctrl", "v"],
                    // Task 6.1: select-all is a well-known shortcut; clear-field
                    // selects-all first (the delete step is wired in Task 6.4).
                    GuiActionKind::SelectAll | GuiActionKind::ClearField => vec!["ctrl", "a"],
                    // Task 2 (Issue #3): a combined shortcut value like "ctrl+l"
                    // must be split into individual key tokens (["ctrl","l"]) —
                    // the press_shortcut tool's parse_key_string rejects combined
                    // tokens. A bare token (e.g. "enter") passes through as-is.
                    _ => request
                        .value
                        .as_deref()
                        .map(|value| {
                            if value.contains('+') {
                                value
                                    .split('+')
                                    .map(|k| k.trim())
                                    .filter(|k| !k.is_empty())
                                    .collect::<Vec<_>>()
                            } else {
                                vec![value]
                            }
                        })
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
            "scroll" => {
                // Task 4 (Issue #5): real DIRECTION-AWARE scroll on the focused
                // window/viewport via the Wayland-capable `press_shortcut` tool.
                // The direction marker (`scroll:<dir>`) is threaded from the goal
                // contract → typed step → proposal target_label → `target_name`
                // (with `value` as a fallback). Keys per direction:
                //   down            → [page_down]
                //   up              → [page_up]
                //   bottom / end    → [ctrl, end]
                //   top / beginning → [ctrl, home]
                //   default/unknown → [page_down]
                // The result is the tool's REAL ok/err (no fabricated success);
                // the screen_changed verifier remains authoritative downstream.
                let keys = scroll_keys_for_request(&request);
                let result = self
                    .execute_tool(
                        "press_shortcut",
                        serde_json::json!({
                            "keys": keys,
                        }),
                    )
                    .await;
                execution_from_tool_result("scroll", result)
            }
            _ => {
                // Task 7 (Issue #4): when the runtime resolved a TRUSTED
                // absolute-pointer target for this click, dispatch via the
                // uinput EV_ABS path (lands on native Wayland windows) instead
                // of the AT-SPI role/name path (which cannot click a11y-off
                // windows). `abs_click` is only ever Some when the abs-pointer
                // flag is ON and trusted physical bounds were available — never
                // an invented coordinate. On any abs-click failure we fall back
                // to the role/name path (never a silent no-op).
                if let Some(abs) = request.abs_click {
                    let result = self
                        .execute_tool(
                            "click_mouse",
                            serde_json::json!({
                                "x": abs.x,
                                "y": abs.y,
                                "button": "left",
                                "absolute": true,
                            }),
                        )
                        .await;
                    let execution = execution_from_tool_result("click_mouse_abs", result);
                    if execution.success {
                        execution
                    } else {
                        // Honest fallback to the role/name click path.
                        let role = request.role.clone();
                        let fallback = self
                            .execute_tool(
                                "click_ui_element",
                                serde_json::json!({ "role": role, "name": request.target_name }),
                            )
                            .await;
                        execution_from_tool_result("click_ui_element", fallback)
                    }
                } else {
                    let role = request.role.clone();
                    let result = self
                        .execute_tool(
                            "click_ui_element",
                            serde_json::json!({ "role": role, "name": request.target_name }),
                        )
                        .await;
                    execution_from_tool_result("click_ui_element", result)
                }
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

/// Task 4 (Issue #5): pick the paging/arrow keys for a surface scroll from the
/// DIRECTION threaded onto the request. The direction is carried as a
/// `scroll:<dir>` marker in `target_name` (proposal `target_label`), with
/// `value` as a fallback; a bare direction word in either field is also
/// honored. Delegates to [`scroll_keys_for_direction`] for the pure mapping.
fn scroll_keys_for_request(request: &GuiActionRequest) -> Vec<&'static str> {
    let direction = scroll_direction_from_field(&request.target_name)
        .or_else(|| {
            request
                .value
                .as_deref()
                .and_then(scroll_direction_from_field)
        })
        .unwrap_or("");
    scroll_keys_for_direction(direction)
}

/// Extract the scroll direction token from a single request field. Accepts the
/// threaded `scroll:<dir>` marker (preferred) or a bare direction word.
fn scroll_direction_from_field(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.strip_prefix("scroll:").unwrap_or(trimmed).trim())
}

/// Task 4 (Issue #5): pure DIRECTION → keys mapping for a surface scroll. Kept
/// free of any backend so it is unit-testable without a live executor.
///   down            → [page_down]
///   up              → [page_up]
///   bottom / end    → [ctrl, end]
///   top / beginning → [ctrl, home]
///   default/unknown → [page_down]
fn scroll_keys_for_direction(direction: &str) -> Vec<&'static str> {
    match direction.trim().to_ascii_lowercase().as_str() {
        "up" => vec!["page_up"],
        "bottom" | "end" => vec!["ctrl", "end"],
        "top" | "beginning" => vec!["ctrl", "home"],
        "down" => vec!["page_down"],
        _ => vec!["page_down"],
    }
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

/// Task 13 (Issue #11): assemble the window-focus/capture/activate backend
/// availability status. Bounded best-effort extension probe; portal probing is
/// not yet implemented (treated as unavailable, documented). Pure assessment in
/// `GuiBackendStatus::assess`.
pub(crate) async fn assess_gui_backend_status(
    uinput_available: bool,
    xdotool_available: bool,
    is_wayland: bool,
) -> kria_core::agent::gui_cognition::window_focus::GuiBackendStatus {
    let extension_available = match kria_ext::read_ext_token() {
        Some(token) => kria_ext::ext_available(&token).await,
        None => false,
    };
    // Portal capture/activate fallback is scoped (design) but not yet probed.
    let portal_available = false;
    kria_core::agent::gui_cognition::window_focus::GuiBackendStatus::assess(
        extension_available,
        uinput_available,
        portal_available,
        xdotool_available,
        is_wayland,
    )
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

/// Issue #1 (extension wiring): helpers that talk to the KRIA GNOME Shell
/// extension's NEW token-gated D-Bus API (`ai.kria.ActiveWindow`) over `gdbus`,
/// reusing the same `tokio::process::Command` pattern as the unauthenticated
/// `GetActiveWindow` perception probe (no new crate dependency). These power the
/// Wayland `focus_window` activation path: the extension raises/focuses the
/// window from inside gnome-shell, bypassing Mutter's focus-stealing prevention.
mod kria_ext {
    use std::time::Duration;

    /// Read the extension auth token from `~/.kria/gui_ext_token` (trimmed).
    /// Returns `None` when the file is missing/empty/unreadable.
    pub(super) fn read_ext_token() -> Option<String> {
        let home = std::env::var_os("HOME")?;
        let path = std::path::Path::new(&home)
            .join(".kria")
            .join("gui_ext_token");
        let raw = std::fs::read_to_string(path).ok()?;
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    /// Format a Rust string as a GVariant string literal for `gdbus call`
    /// (quoted + backslash/quote escaped) so the `s` parameters parse cleanly.
    fn gvariant_string(value: &str) -> String {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }

    /// Run `gdbus` with a bounded timeout, returning trimmed stdout on success.
    /// Mirrors `DesktopGuiPerceptionProvider::command_stdout`.
    async fn gdbus_stdout(args: &[&str], budget_ms: u64) -> Result<String, String> {
        let mut command = tokio::process::Command::new("gdbus");
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

    /// Unwrap the JSON payload from a `gdbus call` result. `gdbus` wraps a
    /// string return as a tuple with a type tag, e.g.
    /// `(s "{\"ok\":true}",)` / `s "{\"ok\":true,...}"`. We strip the
    /// surrounding tuple parens, the `s ` type tag, the surrounding double
    /// quotes, then unescape `\"` -> `"` (and `\\` -> `\`) before parsing into a
    /// `serde_json::Value`. A brace-extraction fallback keeps parsing robust
    /// against formatting differences. Returns `None` on any parse failure.
    pub(super) fn unwrap_gdbus_string(output: &str) -> Option<serde_json::Value> {
        let trimmed = output.trim();
        // Strip the outer GVariant tuple: `( ... ,)`.
        let inner = trimmed
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .map(|s| s.trim().trim_end_matches(',').trim())
            .unwrap_or(trimmed);
        // Strip a leading string type tag (`s `, emitted by some gdbus builds).
        let inner = inner.strip_prefix("s ").map(str::trim).unwrap_or(inner);
        // Strip surrounding double quotes around the escaped JSON string.
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(inner);

        let unescaped = inner.replace("\\\"", "\"").replace("\\\\", "\\");
        for candidate in [inner, unescaped.as_str()] {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) {
                return Some(value);
            }
        }
        // Last-resort: extract the outermost `{ ... }` and try raw + unescaped.
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        if end <= start {
            return None;
        }
        let raw = &trimmed[start..=end];
        serde_json::from_str(raw)
            .ok()
            .or_else(|| serde_json::from_str(&raw.replace("\\\"", "\"")).ok())
    }

    /// Invoke `ai.kria.ActiveWindow.<method>` via `gdbus call` over the session
    /// bus. `args` are raw string values (the methods used here take `s`
    /// params); each is GVariant-quoted. Returns the parsed JSON payload or
    /// `None` on any failure/timeout.
    pub(super) async fn ext_call(
        method: &str,
        args: &[&str],
        timeout_ms: u64,
    ) -> Option<serde_json::Value> {
        let full_method = format!("ai.kria.ActiveWindow.{method}");
        let quoted: Vec<String> = args.iter().map(|a| gvariant_string(a)).collect();
        let mut argv: Vec<&str> = vec![
            "call",
            "--session",
            "--dest",
            "ai.kria.ActiveWindow",
            "--object-path",
            "/ai/kria/ActiveWindow",
            "--method",
            full_method.as_str(),
        ];
        for q in &quoted {
            argv.push(q.as_str());
        }
        let stdout = gdbus_stdout(&argv, timeout_ms).await.ok()?;
        unwrap_gdbus_string(&stdout)
    }

    /// `Ping(token)` returns `{"ok":true,...}` when the NEW token-gated API is
    /// loaded and the token is accepted.
    pub(super) async fn ext_available(token: &str) -> bool {
        match ext_call("Ping", &[token], 1500).await {
            Some(value) => value
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            None => false,
        }
    }

    /// Build the ordered set of case-insensitive search terms for a SwitchWindow
    /// hint. Index 0 is always the raw (lowercased) hint (highest weight); the
    /// rest are tolerant aliases for common apps (browser/file-manager/terminal/
    /// editor/calculator).
    fn alias_terms(hint: &str) -> Vec<String> {
        let h = hint.trim().to_ascii_lowercase();
        let mut terms = vec![h.clone()];
        let mut add = |items: &[&str]| {
            for item in items {
                let s = item.to_string();
                if !terms.contains(&s) {
                    terms.push(s);
                }
            }
        };
        if h.contains("chrome") {
            add(&["google-chrome", "chromium", "chrome"]);
        }
        if h.contains("chromium") {
            add(&["chromium", "chrome"]);
        }
        if h.contains("firefox") {
            add(&["firefox", "mozilla firefox"]);
        }
        if h.contains("file manager") || h == "files" || h.contains("nautilus") {
            add(&["nautilus", "files", "org.gnome.nautilus"]);
        }
        if h.contains("terminal") || h.contains("console") {
            add(&["gnome-terminal", "kgx", "console", "org.gnome.terminal"]);
        }
        if h.contains("text editor") || h.contains("editor") {
            add(&["gnome-text-editor", "gedit", "org.gnome.texteditor"]);
        }
        if h.contains("calculator") {
            add(&["gnome-calculator", "org.gnome.calculator"]);
        }
        terms
    }

    /// Extract the activation id (`id`) of a window object as a string.
    fn window_id(window: &serde_json::Value) -> Option<String> {
        match window.get("id") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    /// Score a single window against the term set across `app_name`, `wm_class`,
    /// `app_id`, `title`. The raw hint (term index 0) outweighs aliases; exact >
    /// prefix > substring > field-contained-in-hint. 0 means no match.
    fn window_match_score(window: &serde_json::Value, terms: &[String]) -> u32 {
        const FIELDS: [&str; 4] = ["app_name", "wm_class", "app_id", "title"];
        let mut best = 0u32;
        for (idx, term) in terms.iter().enumerate() {
            if term.is_empty() {
                continue;
            }
            let weight = if idx == 0 { 100 } else { 50 };
            for field in FIELDS {
                let Some(raw) = window.get(field).and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let value = raw.to_ascii_lowercase();
                if value.is_empty() {
                    continue;
                }
                let score = if value == *term {
                    weight + 30
                } else if value.starts_with(term.as_str()) {
                    weight + 20
                } else if value.contains(term.as_str()) {
                    weight + 10
                } else if value.len() >= 3 && term.contains(value.as_str()) {
                    weight
                } else {
                    0
                };
                if score > best {
                    best = score;
                }
            }
        }
        best
    }

    /// Pick the BEST-matching window id for `hint` from a windows JSON array.
    /// Returns `None` when nothing matches.
    pub(super) fn pick_window_match(windows: &[serde_json::Value], hint: &str) -> Option<String> {
        let terms = alias_terms(hint);
        let mut best_id: Option<String> = None;
        let mut best_score = 0u32;
        for window in windows {
            let score = window_match_score(window, &terms);
            if score > best_score {
                if let Some(id) = window_id(window) {
                    best_score = score;
                    best_id = Some(id);
                }
            }
        }
        best_id
    }

    /// `ListWindows` -> best match -> `ActivateWindow`. Returns:
    ///   `Some(true)`  when activation CONFIRMED focus
    ///                 (`ok && activated && focused_after == id`),
    ///   `Some(false)` when activate ran but did NOT confirm focus,
    ///   `None`        when no window matched / the extension was unavailable.
    pub(super) async fn ext_activate_target(token: &str, target_name: &str) -> Option<bool> {
        let listing = ext_call("ListWindows", &[token], 1500).await?;
        let windows = listing.get("windows").and_then(serde_json::Value::as_array)?;
        let id = pick_window_match(windows, target_name)?;
        let result = ext_call("ActivateWindow", &[token, id.as_str()], 1500).await?;
        let ok = result
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let activated = result
            .get("activated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let focused_after = result
            .get("focused_after")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Some(ok && activated && focused_after == id)
    }

    /// Task 2 (Issue #3): activate a just-OPENED app's window, tolerating that
    /// the window may still be appearing right after launch. Polls `ListWindows`
    /// + `ActivateWindow` up to `attempts` times (with `delay_ms` between) until
    /// the target window is found AND focus is confirmed. Returns `Some(true)`
    /// when focus was confirmed, `Some(false)` when activate ran but did not
    /// confirm, `None` when the window never appeared / extension unavailable.
    /// Best-effort: the OpenApp verdict is unchanged; this only guarantees the
    /// opened app is FOCUSED so the next in-app step resolves against it.
    pub(super) async fn ext_activate_target_with_retry(
        token: &str,
        target_name: &str,
        attempts: u32,
        delay_ms: u64,
    ) -> Option<bool> {
        let mut last: Option<bool> = None;
        for i in 0..attempts.max(1) {
            match ext_activate_target(token, target_name).await {
                Some(true) => return Some(true),
                other => last = other.or(last),
            }
            if i + 1 < attempts {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
        last
    }

    /// Whether the GNOME-extension screen-capture path is enabled. Default-ON;
    /// rollback with `KRIA_GUI_COG_EXT_CAPTURE` set to a falsy value
    /// (`0`/`false`/`no`/`off`/empty) to force the legacy xcap capture.
    pub(super) fn ext_capture_enabled() -> bool {
        match std::env::var("KRIA_GUI_COG_EXT_CAPTURE") {
            Ok(v) => {
                let v = v.trim().to_ascii_lowercase();
                !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
            }
            Err(_) => true,
        }
    }

    /// Capture the whole composited stage via the extension's `CaptureScreen`
    /// (in-shell `Shell.Screenshot`, which — unlike an external xcap/portal grab
    /// — actually sees native Wayland windows). Writes a temp PNG, reads its
    /// bytes, deletes it. Returns `None` on any failure (caller falls back to
    /// xcap). Bounded timeout so a wedged shell never stalls perception.
    pub(super) async fn ext_capture_screen() -> Option<Vec<u8>> {
        let token = read_ext_token()?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("kria_ext_cap_{}_{nanos}.png", std::process::id()));
        let path_str = path.to_str()?.to_string();
        let result = ext_call("CaptureScreen", &[token.as_str(), path_str.as_str()], 4000).await;
        let ok = result
            .as_ref()
            .and_then(|v| v.get("ok"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !ok {
            let _ = tokio::fs::remove_file(&path).await;
            return None;
        }
        let bytes = tokio::fs::read(&path).await.ok();
        let _ = tokio::fs::remove_file(&path).await;
        match bytes {
            Some(b) if !b.is_empty() => Some(b),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unwrap_gdbus_string_strips_type_tag_and_unescapes() {
            let raw = r#"(s "{\"ok\":true,\"activated\":true,\"focused_after\":\"w12\"}",)"#;
            let value = unwrap_gdbus_string(raw).expect("parse");
            assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
            assert_eq!(value.get("activated").and_then(|v| v.as_bool()), Some(true));
            assert_eq!(
                value.get("focused_after").and_then(|v| v.as_str()),
                Some("w12")
            );
        }

        #[test]
        fn unwrap_gdbus_string_handles_bare_quoted_string() {
            let raw = r#"s "{\"ok\":true,\"version\":\"1.2\"}""#;
            let value = unwrap_gdbus_string(raw).expect("parse");
            assert_eq!(value.get("ok").and_then(|v| v.as_bool()), Some(true));
            assert_eq!(value.get("version").and_then(|v| v.as_str()), Some("1.2"));
        }

        #[test]
        fn unwrap_gdbus_string_rejects_garbage() {
            assert!(unwrap_gdbus_string("not a dbus reply").is_none());
            assert!(unwrap_gdbus_string("").is_none());
        }

        fn windows_fixture() -> Vec<serde_json::Value> {
            serde_json::json!([
                {
                    "id": "w1", "app_name": "Files", "wm_class": "org.gnome.Nautilus",
                    "app_id": "org.gnome.Nautilus.desktop", "title": "Home"
                },
                {
                    "id": "w2", "app_name": "Google Chrome", "wm_class": "google-chrome",
                    "app_id": "google-chrome.desktop", "title": "New Tab - Google Chrome"
                },
                {
                    "id": "w3", "app_name": "Terminal", "wm_class": "gnome-terminal-server",
                    "app_id": "org.gnome.Terminal.desktop", "title": "obaid@host: ~"
                }
            ])
            .as_array()
            .cloned()
            .unwrap()
        }

        #[test]
        fn pick_window_match_direct_title_substring() {
            let windows = windows_fixture();
            assert_eq!(
                pick_window_match(&windows, "New Tab").as_deref(),
                Some("w2")
            );
        }

        #[test]
        fn pick_window_match_browser_alias() {
            let windows = windows_fixture();
            // "chrome" -> google-chrome alias picks the Chrome window.
            assert_eq!(pick_window_match(&windows, "chrome").as_deref(), Some("w2"));
        }

        #[test]
        fn pick_window_match_file_manager_alias() {
            let windows = windows_fixture();
            assert_eq!(
                pick_window_match(&windows, "file manager").as_deref(),
                Some("w1")
            );
        }

        #[test]
        fn pick_window_match_terminal_alias() {
            let windows = windows_fixture();
            assert_eq!(
                pick_window_match(&windows, "terminal").as_deref(),
                Some("w3")
            );
        }

        #[test]
        fn pick_window_match_no_match_returns_none() {
            let windows = windows_fixture();
            assert!(pick_window_match(&windows, "spotify").is_none());
        }
    }
}

use kria_ext::{ext_activate_target, ext_activate_target_with_retry, ext_available, read_ext_token};

/// Task 3 (Issue #1): whether the Wayland-native window-activation path
/// (`gio launch <.desktop>`) should be used for `focus_window` execution. Mirrors
/// how the other `gui_cog_*` flags are read on the desktop runtime — the
/// `gui_cog_wayland_focus` flag defaults ON and is rolled back without a code
/// change via `KRIA_GUI_COG_WAYLAND_FOCUS=0` (or `false`/`no`/`off`). Reuses the
/// shared kria-core config parser so the gate is identical to the runtime wiring.
fn wayland_focus_activation_enabled() -> bool {
    kria_core::agent::gui_cognition::window_focus::GuiWaylandFocusConfig::from_env_default_on()
        .is_enabled()
}

/// Task 2 (Issue #3): whether the open-then-act focus guarantee is active —
/// after `OpenApp` on Wayland, the just-opened target window is ACTIVATED via the
/// GNOME extension so the next in-app step resolves against the right (focused)
/// window instead of whatever was focused before (which caused the "open Chrome
/// and search" flap). Default ON; rollback via `KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS`
/// set to a falsy value (`0`/`false`/`no`/`off`/empty), restoring the prior
/// open-only behavior byte-for-byte.
fn open_then_act_focus_enabled() -> bool {
    match std::env::var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !matches!(v.as_str(), "0" | "false" | "no" | "off" | "")
        }
        Err(_) => true,
    }
}

#[cfg(test)]
mod open_then_act_focus_tests {
    use super::open_then_act_focus_enabled;

    #[test]
    fn flag_defaults_on_with_falsy_rollback() {
        let prev = std::env::var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS").ok();
        std::env::remove_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS");
        assert!(open_then_act_focus_enabled(), "default must be ON");
        std::env::set_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS", "0");
        assert!(!open_then_act_focus_enabled(), "0 must roll back (OFF)");
        std::env::set_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS", "off");
        assert!(!open_then_act_focus_enabled(), "off must roll back (OFF)");
        std::env::set_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS", "1");
        assert!(open_then_act_focus_enabled(), "1 must be ON");
        match prev {
            Some(v) => std::env::set_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS", v),
            None => std::env::remove_var("KRIA_GUI_COG_OPEN_THEN_ACT_FOCUS"),
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
    desktop_gui_cognition_command_capture_streamed(
        message,
        app_state,
        session_id_override,
        event_scope_prefix,
        options,
        None,
    )
    .await
}

/// Task 10.1 (`gui_cog_stream_ux`, default OFF): the streaming-aware capture
/// entry point. When the `gui_cog_stream_ux` flag is ON **and** an
/// `event_emitter` is supplied, the runtime is given an mpsc streaming sink and
/// a background task drains the receiver, emitting each `gui_cognition:event`
/// envelope to the frontend via the EXISTING `gui_cognition:event` Tauri event
/// the moment it is produced DURING the turn (observe → plan → per-step) instead
/// of waiting for the end-of-turn batch (Requirement 16, 24). The event NAME is
/// unchanged (frontend/backend contract). When the flag is OFF (the default) or
/// no emitter is supplied, no sink is attached and the end-of-turn batch is
/// returned/emitted exactly as before — byte-for-byte unchanged behavior.
pub(super) async fn desktop_gui_cognition_command_capture_streamed(
    message: String,
    app_state: &AppState,
    session_id_override: Option<String>,
    event_scope_prefix: &str,
    options: Option<GuiCognitionCommandOptions>,
    event_emitter: Option<AppHandle>,
) -> Result<super::chat::DesktopChatCommandCapture, String> {
    let session_id = match session_id_override.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => app_state.current_session_id.read().await.clone(),
    };

    // GUI Cognition V2 routing (Part B). When `KRIA_GUI_COG_V2` is truthy the
    // turn runs through the new Sight/Brain/Hands loop (`run_gui_cognition_v2`)
    // instead of the V1 runtime below. Read live per turn from the server-side
    // environment so a client cannot toggle it. Default OFF → V1 byte-for-byte.
    if kria_core::agent::gui_cognition_v2::v2_enabled() {
        return run_gui_cognition_v2(
            message,
            app_state,
            session_id,
            event_scope_prefix,
            options,
            event_emitter,
        )
        .await;
    }

    let turn_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4().to_string();

    let options = options.unwrap_or_default();
    let perception = match options.perception_fixture {
        Some(fixture) => {
            GuiPerceptionProviderAdapter::Fixture(FixtureGuiPerceptionProvider::new(fixture))
        }
        None => GuiPerceptionProviderAdapter::Live(DesktopGuiPerceptionProvider {
            app_state,
            screenshot_bytes: Mutex::new(None),
            atspi_snapshot: OnceCell::new(),
            cache_policy: gui_observation_cache_policy_for_prompt(&message),
            force_fresh: std::sync::atomic::AtomicBool::new(false),
            ocr_scope: gui_ocr_scope_for_prompt(&message),
            observe_profile: gui_observe_profile_for_prompt(&message),
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
    // Task 0 (Requirement 0.6): the structured-output adapter config, gated by
    // `gui_cog_structured_planner` (server-side env, default ON).
    let structured_planner_cfg =
        kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig::from_env_default_on();
    // Route the configured planner backend ONCE so it can be wrapped as the live
    // planner AND inspected for grammar capability (Task 0.9 Rung B).
    let routed_planner_backend = if fixture_planner.is_none() && !options.disable_live_llm_planner {
        app_state.model_router.route("gui_cognition_planner").await
    } else {
        None
    };
    let live_planner = routed_planner_backend.as_ref().map(|backend| {
        // Task 0 (Requirement 0.6): adopt the shared structured-output adapter
        // for the live planner, gated by `gui_cog_structured_planner`. Flag-OFF
        // restores the prior `chat_with_grammar` path byte-for-byte.
        LlmBackendGuiPlanner::new(backend.clone()).with_structured_config(structured_planner_cfg)
    });
    // Task 0.9 (Requirement 0.9 Rung B): build an optional LOCAL grammar fallback
    // planner ONLY when the structured flag is ON, the configured planner backend
    // is NOT itself grammar-capable, and a DIFFERENT, grammar-capable local
    // backend exists. This is the ladder's middle rung: when the configured (e.g.
    // cloud) plan is strictly rejected the runtime retries the plan ONCE through
    // this local grammar planner (which posts a real grammar/json_schema
    // constraint → ~100% schema-valid). When the configured backend is itself
    // grammar-capable (or no distinct local backend exists), no fallback is wired
    // and the ladder collapses to Rung A → Rung C.
    let local_fallback_planner = {
        match (
            structured_planner_cfg.is_enabled() && local_planner_enabled(),
            routed_planner_backend.as_ref(),
            app_state.model_router.local_backend(),
        ) {
            (true, Some(configured), Some(local))
                if !kria_core::llm::model_router::is_grammar_capable(configured)
                    && kria_core::llm::model_router::is_grammar_capable(&local)
                    && local.model_label() != configured.model_label() =>
            {
                Some(
                    LlmBackendGuiPlanner::new(local)
                        .with_structured_config(structured_planner_cfg),
                )
            }
            _ => None,
        }
    };
    let planner_ref: Option<&dyn GuiLlmPlanner> = match (&fixture_planner, &live_planner) {
        (Some(planner), _) => Some(planner),
        (None, Some(planner)) => Some(planner),
        (None, None) => None,
    };
    let local_fallback_ref: Option<&dyn GuiLlmPlanner> = local_fallback_planner
        .as_ref()
        .map(|planner| planner as &dyn GuiLlmPlanner);
    let runtime = GuiCognitionRuntime::new(&perception, &executor)
        .with_llm_planner(planner_ref)
        .with_local_grammar_planner(local_fallback_ref);
    // Task 1.2 (Requirement 21): runtime guards are read from the server-side
    // environment so a client cannot toggle enforcement. A cooperative cancel
    // token is registered under the session_id so a Tauri cancel command can
    // halt the active turn before its next action.
    //
    // Task 1.6 (gate flip): the Task 1 gate has passed, so the live/desktop
    // path now enables the runaway-control guards (cancel/watchdog/abort/
    // preconditions, Requirements 19/21/25) by DEFAULT via
    // `from_env_default_on()`. Rollback without a code change: set
    // `KRIA_GUI_COG_RUNTIME_GUARDS=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior Step 1–12 behavior. The deterministic
    // T2 fixture tier is unaffected because those runtimes construct their
    // guard config explicitly (never through this env path).
    let runtime_guards =
        kria_core::agent::gui_cognition::turn_budget::GuiRuntimeGuardConfig::from_env_default_on();
    let cancel_token =
        kria_core::agent::gui_cognition::cancel::gui_cancel_registry().register(&session_id);
    // Task 2.1 (Requirement 1, 4): the `gui_cog_smart_planner` flag (strict
    // schema-validate + exactly ONE repair-retry feeding the validation error
    // back, then deterministic fallback) is read from the server-side
    // environment so a client cannot toggle it.
    //
    // Task 2.9 (gate flip): the Task 2 gate has passed (CI-safe deterministic
    // T1/T2 evidence shows every supported intent/primitive/combo reaches
    // `valid_for_resolution` — planner-blocked families no longer land on "Plan
    // validation blocked"), so the live/desktop path now enables the
    // smart-planner repair-retry path by DEFAULT via `from_env_default_on()`.
    // Rollback without a code change: set `KRIA_GUI_COG_SMART_PLANNER=0` (or
    // `false`/`no`/`off`) in the desktop environment to restore the prior
    // single-attempt Step 1–12 behavior. The deterministic T2 fixture tier is
    // unaffected because those runtimes construct their planner config
    // explicitly (never through this env path).
    let smart_planner =
        kria_core::agent::gui_cognition::llm_planner::GuiSmartPlannerConfig::from_env_default_on();
    // Task 0 (Requirement 0): the `gui_cog_structured_planner` flag adopts the
    // shared multi-backend structured-output adapter for the GUI-cognition
    // planner — every OpenAI-compatible provider (local grammar + cloud
    // json_schema/json_object/tool-calling) returns a schema-valid typed plan,
    // and the bounded re-ask budget is raised to AT MOST 2 (feeding the strict
    // validation error back). It is read from the server-side environment so a
    // client cannot toggle it. The live/desktop path enables it by DEFAULT via
    // `from_env_default_on()` — mirroring the prior `gui_cog_*` flags. Rollback
    // without a code change: set `KRIA_GUI_COG_STRUCTURED_PLANNER=0` (or
    // `false`/`no`/`off`) in the desktop environment to restore the prior
    // `chat_with_grammar` planner path + one-shot repair behavior byte-for-byte.
    // The deterministic T2 fixture tier is unaffected — those runtimes set their
    // structured-planner config explicitly (never through this env path).
    let structured_planner =
        kria_core::agent::gui_cognition::llm_planner::GuiStructuredPlannerConfig::from_env_default_on();
    // Task 3.1 (Requirement 2): the `gui_cog_reobserve` flag gates the explicit
    // per-step re-observe hook (fresh GuiContext between steps, bounded by the
    // Task 1 runaway caps). It is read from the server-side environment so a
    // client cannot toggle it.
    //
    // Task 3.6 (Wave 3 gate flip): the Task 3 gate has passed (CI-safe
    // deterministic T1/T2 evidence shows representative multi-step combos
    // re-observe between steps with each step resolved+verified against the
    // FRESH context, bounded by the Task 1 runaway caps), so the live/desktop
    // path now enables the per-step re-observe hook by DEFAULT via
    // `from_env_default_on()` — mirroring Task 1's `gui_cog_runtime_guards` and
    // Task 2's `gui_cog_smart_planner`. Rollback without a code change: set
    // `KRIA_GUI_COG_REOBSERVE=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior re-observe behavior. The deterministic
    // T2 fixture tier is unaffected because those runtimes construct their
    // re-observe config explicitly (never through this env path).
    let reobserve =
        kria_core::agent::gui_cognition::turn_budget::GuiReobserveConfig::from_env_default_on();
    // Task 4.2 (Requirement 3): the `gui_cog_wayland_focus` flag routes
    // SwitchWindow through the Wayland-safe window-focus abstraction
    // (activate-by-window-identity preferred; truthful `backend_used`). The Wave 3
    // gate (Task 4.5) verified — at the CI-safe level — that SwitchWindow routes
    // through the abstraction, executes, is verified by re-observe, and the legacy
    // "wmctrl required" path is replaced by a clear actionable error. The
    // live/desktop path now enables the Wayland-safe focus abstraction by DEFAULT
    // via `from_env_default_on()` — mirroring Task 1's `gui_cog_runtime_guards`,
    // Task 2's `gui_cog_smart_planner`, and Task 3's `gui_cog_reobserve`. It is
    // read from the server-side environment so a client cannot toggle it. Rollback
    // without a code change: set `KRIA_GUI_COG_WAYLAND_FOCUS=0` (or
    // `false`/`no`/`off`) in the desktop environment to restore the prior
    // SwitchWindow behavior. The deterministic T2 fixture tier is unaffected — those
    // runtimes set their Wayland-focus config explicitly (never through this env path).
    let wayland_focus =
        kria_core::agent::gui_cognition::window_focus::GuiWaylandFocusConfig::from_env_default_on();
    // Task 5.1 (Requirement 4.2): the `gui_cog_step_completeness` flag gates the
    // plan post-processing pass that fills a type-correct `verification_strategy`
    // for any step missing/empty/incompatible one (and sources sanitized payloads,
    // converting genuinely-missing payloads to AskClarification rather than an
    // invalid step). The Wave 4 gate (Task 5.4) is GREEN at the CI-safe level
    // (file-manager search / summarize-visible / copy no longer blocked by the
    // validator; T1/T2 step-completeness suites pass; Steps 1–12 green), so the
    // live/desktop path now enables the step-completeness post-processing by
    // DEFAULT via `from_env_default_on()` — mirroring Task 1's
    // `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
    // `gui_cog_reobserve`, and Task 4's `gui_cog_wayland_focus`. It is read from
    // the server-side environment so a client cannot toggle it. Rollback without a
    // code change: set `KRIA_GUI_COG_STEP_COMPLETENESS=0` (or `false`/`no`/`off`)
    // in the desktop environment to restore the prior plan-preserving behavior.
    let step_completeness =
        kria_core::agent::gui_cognition::llm_planner::GuiStepCompletenessConfig::from_env_default_on();
    // Task 6.1 (Requirement 5): the `gui_cog_primitives` flag gates the richer
    // primitive-coverage executor mapping (clear/select-all/checkbox/
    // dialog-close/in-app-search route to their correct typed action kind
    // instead of the legacy ClickControl catch-all) plus the DPI/multi-monitor
    // aware physical-bounds annotation. Wave 5 / Task 6.5 gate PASSED at the
    // CI-safe level (held-out set frozen; audit dry-run green on real_session +
    // test_substrate; per-primitive tier/coverage/privacy suites green; Steps
    // 1–12 green; flag-OFF preserves the legacy mapping byte-for-byte), so the
    // live/desktop path now enables the richer primitive-coverage mapping by
    // DEFAULT via `from_env_default_on()` — mirroring Task 1's
    // `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
    // `gui_cog_reobserve`, Task 4's `gui_cog_wayland_focus`, and Task 5's
    // `gui_cog_step_completeness`. It is read from the server-side environment so
    // a client cannot toggle it. Rollback without a code change: set
    // `KRIA_GUI_COG_PRIMITIVES=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior executor path byte-for-byte (the richer
    // primitive mapping does not run). The deterministic T2 fixture tier is
    // unaffected — those runtimes set their primitives config explicitly (never
    // through this env path).
    let primitives =
        kria_core::agent::gui_cognition::executor::GuiPrimitivesConfig::from_env_default_on();
    // Task 7.1 (Requirements 5, 9, 26): the `gui_cog_browser` flag gates browser
    // chrome-UI targeting (address/URL bar, tab strip / individual tabs,
    // back/forward, reload/stop, in-page Find bar become targetable via the
    // accessibility resolver when the active app is a recognized browser;
    // page-content targeting stays out of scope — that is Task 7.2). Read/
    // summarize uses OCR/page text as DATA ONLY and never influences the planner
    // or executor (injection defense, Requirement 9). It is read from the
    // server-side environment so a client cannot toggle it. Task 7.5 (Wave 6
    // live gate) flipped the live/desktop default to ON via
    // `from_env_default_on()` — mirroring Task 1's `gui_cog_runtime_guards`,
    // Task 2's `gui_cog_smart_planner`, Task 3's `gui_cog_reobserve`, Task 4's
    // `gui_cog_wayland_focus`, Task 5's `gui_cog_step_completeness`, and Task 6's
    // `gui_cog_primitives`. The browser chrome-UI targeting + data-only
    // summarize path is now ON unless `KRIA_GUI_COG_BROWSER` is an explicit
    // opt-out (`0`/`false`/`no`/`off`/empty); an absent value keeps it ON.
    // Rollback without a code change: set `KRIA_GUI_COG_BROWSER=0` (or
    // `false`/`no`/`off`) in the desktop environment to restore the prior
    // executor / resolver path byte-for-byte. The deterministic T2 fixture tier
    // is unaffected — those runtimes set their browser config explicitly (never
    // through this env path).
    let browser =
        kria_core::agent::gui_cognition::browser::GuiBrowserConfig::from_env_default_on();
    // Task 8.1 (Requirements 6, 7, 8): the `gui_cog_crossapp` flag gates cross-app
    // clipboard combos (copy in one app → switch → paste in another), the
    // clipboard-safe SAVE → USE → RESTORE helper (the user's clipboard is
    // captured before the operation and restored afterwards, never clobbered —
    // Requirement 8), and file-manager select (Tasks 8.2/8.3). It is read from
    // the server-side environment so a client cannot toggle it. Like the
    // already-gated Tasks 1–7, this flag now defaults ON for the live/desktop turn
    // builder via `from_env_default_on()`. Task 8.5 (Wave 6 live gate) flipped the
    // live/desktop default to ON — mirroring Task 1's
    // `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
    // `gui_cog_reobserve`, Task 4's `gui_cog_wayland_focus`, Task 5's
    // `gui_cog_step_completeness`, Task 6's `gui_cog_primitives`, and Task 7's
    // `gui_cog_browser`. The cross-app clipboard combo + clipboard-safe
    // SAVE → USE → RESTORE helper + file-manager select path is now ON unless
    // `KRIA_GUI_COG_CROSSAPP` is an explicit opt-out (`0`/`false`/`no`/`off`/
    // empty); an absent value keeps it ON. Rollback without a code change: set
    // `KRIA_GUI_COG_CROSSAPP=0` (or `false`/`no`/`off`) in the desktop environment
    // to restore the prior executor / runtime path byte-for-byte (the user's
    // clipboard is never borrowed and no cross-app/fm-select layer runs; the
    // Steps 1–12 path is preserved). The deterministic T2 fixture tier is
    // unaffected — those runtimes set their cross-app config explicitly (never
    // through this env path).
    let crossapp =
        kria_core::agent::gui_cognition::clipboard::GuiCrossAppConfig::from_env_default_on();
    // Task 9.1 (Requirements 10, 11, 12, 13, 14, 15, 22, 23): the
    // `gui_cog_safety_polish` flag gates the formalized per-action-type
    // verification CONTRACT (predicate + evidence source + bounded wait +
    // confidence) and the honest `inconclusive` verdict for low-confidence /
    // unreliable-evidence outcomes. It is read from the server-side environment
    // so a client cannot toggle it. Task 9.7 (Wave 7 live gate) flipped the
    // live/desktop default to ON via `from_env_default_on()` — mirroring Task 1's
    // `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
    // `gui_cog_reobserve`, Task 4's `gui_cog_wayland_focus`, Task 5's
    // `gui_cog_step_completeness`, Task 6's `gui_cog_primitives`, Task 7's
    // `gui_cog_browser`, and Task 8's `gui_cog_crossapp`. The formalized
    // per-action-type verification CONTRACT + honest `inconclusive` verdict is now
    // ON unless `KRIA_GUI_COG_SAFETY_POLISH` is an explicit opt-out
    // (`0`/`false`/`no`/`off`/empty); an absent value keeps it ON. Rollback without
    // a code change: set `KRIA_GUI_COG_SAFETY_POLISH=0` (or `false`/`no`/`off`) in
    // the desktop environment to restore the prior verification verdict
    // byte-for-byte. The deterministic T2 fixture tier is unaffected — those
    // runtimes set their safety-polish config explicitly (never through this env
    // path).
    let safety_polish =
        kria_core::agent::gui_cognition::verifier::GuiSafetyPolishConfig::from_env_default_on();

    // Phase 1 (Requirement 1): the `gui_cog_verify_live` flag changes the OpenApp
    // post-action verification predicate from `active_window_match` to
    // `window_visible` (the app's window PRESENT/visible in the desktop open-
    // window set, alias-tolerant, evidence `observation`/desktop-state) with a
    // bounded readiness wait, so genuine Wayland app launches that do not steal
    // focus are no longer falsely downgraded to PARTIAL. It is read from the
    // server-side environment so a client cannot toggle it. Like the already-
    // gated prior waves, the live/desktop path enables it by DEFAULT via
    // `from_env_default_on()`. Rollback without a code change: set
    // `KRIA_GUI_COG_VERIFY_LIVE=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior `active_window_match` verdict byte-for-
    // byte. The deterministic T2 fixture tier builds its verify-live config
    // explicitly (never through this env path).
    let verify_live =
        kria_core::agent::gui_cognition::verifier::GuiVerifyLiveConfig::from_env_default_on();

    // Task 2.1 (Requirement 2): the `gui_cog_auto_prereq` flag prepends an
    // inferred OpenApp/SwitchWindow prerequisite for a bare-primitive plan whose
    // target app is not the active/observable window (or replaces the plan with
    // an AskClarification when no app is inferable). Read from the server-side
    // environment so a client cannot toggle it. Like the already-gated prior
    // waves, the live/desktop path enables it by DEFAULT via
    // `from_env_default_on()`. Rollback without a code change: set
    // `KRIA_GUI_COG_AUTO_PREREQ=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior plan byte-for-byte.
    let auto_prereq =
        kria_core::agent::gui_cognition::llm_planner::GuiAutoPrereqConfig::from_env_default_on();

    // Task 10.1 (Requirements 16, 24): the `gui_cog_stream_ux` flag gates
    // DURING-turn streaming of `gui_cognition:event` envelopes through an mpsc
    // channel. It is read from the server-side environment so a client cannot
    // toggle it. Task 10.7 (Wave 8 live gate) flipped the live/desktop default
    // to ON via `from_env_default_on()` — mirroring Task 1's
    // `gui_cog_runtime_guards`, Task 2's `gui_cog_smart_planner`, Task 3's
    // `gui_cog_reobserve`, Task 4's `gui_cog_wayland_focus`, Task 5's
    // `gui_cog_step_completeness`, Task 6's `gui_cog_primitives`, Task 7's
    // `gui_cog_browser`, Task 8's `gui_cog_crossapp`, and Task 9's
    // `gui_cog_safety_polish`. DURING-turn streaming is now ON unless
    // `KRIA_GUI_COG_STREAM_UX` is an explicit opt-out (`0`/`false`/`no`/`off`/
    // empty); an absent value keeps it ON. Rollback without a code change: set
    // `KRIA_GUI_COG_STREAM_UX=0` (or `false`/`no`/`off`) in the desktop
    // environment to restore the prior end-of-turn batch behavior byte-for-byte.
    // Streaming still requires BOTH the flag ON and an `event_emitter` (the
    // desktop AppHandle that emits to the frontend); the deterministic T2 fixture
    // tier never supplies an emitter, so it is unaffected. While OFF (or no
    // emitter), no sink is attached and the end-of-turn batch is emitted exactly
    // as before.
    let stream_ux = GuiStreamUxConfig::from_env_default_on();
    let streaming = stream_ux.is_enabled() && event_emitter.is_some();

    // When streaming, build the sink + spawn the drain task that emits each
    // envelope live via the EXISTING `gui_cognition:event` Tauri event (the event
    // NAME is a frontend/backend contract and is unchanged). The drain mirrors
    // the end-of-turn batch loop exactly (HitlRequired → `:approval_required`,
    // then `gui_cognition:event` with a monotonic sequence), so the only
    // difference is WHEN the frontend sees each envelope, not WHAT it sees.
    let (event_sink, drain_handle) = if streaming {
        let app = event_emitter.clone().expect("emitter present when streaming");
        let (sink, mut receiver) = GuiEventStreamSink::channel();
        let stream_session = session_id.clone();
        let stream_turn = turn_id.clone();
        let stream_workflow = workflow_id.clone();
        let stream_prefix = event_scope_prefix.to_string();
        // Emit the `:thinking` state live up-front so the UI ordering matches the
        // batch path (processing state set before the first streamed envelope).
        let _ = app.emit(
            &format!("{stream_prefix}:thinking"),
            serde_json::json!({ "status": "processing", "mode": "gui_cognition" }),
        );
        let handle = tokio::spawn(async move {
            let mut sequence: u64 = 0;
            while let Some(envelope) = receiver.recv().await {
                if envelope.get("type").and_then(serde_json::Value::as_str)
                    == Some("HitlRequired")
                {
                    if let Some(approval_request) = envelope.get("approval_request").cloned() {
                        let _ =
                            app.emit(&format!("{stream_prefix}:approval_required"), approval_request);
                    }
                }
                sequence += 1;
                let _ = app.emit(
                    "gui_cognition:event",
                    gui_cognition_event_payload(
                        &stream_session,
                        &stream_turn,
                        &stream_workflow,
                        sequence,
                        envelope,
                    ),
                );
            }
        });
        (Some(sink), Some(handle))
    } else {
        (None, None)
    };

    let runtime = runtime
        .with_runtime_guards(runtime_guards)
        .with_cancel_token(Some(cancel_token))
        .with_smart_planner(smart_planner)
        .with_structured_planner(structured_planner)
        .with_reobserve(reobserve)
        .with_wayland_focus(wayland_focus)
        .with_step_completeness(step_completeness)
        .with_primitives(primitives)
        .with_browser(browser)
        .with_crossapp(crossapp)
        .with_safety_polish(safety_polish)
        .with_verify_live(verify_live)
        .with_auto_prereq(auto_prereq)
        .with_stream_ux(stream_ux)
        .with_event_sink(event_sink.clone());
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
            // Task 0.3 / Requirement 20.3: the substrate marker is derived from the
            // server-side process environment, never from the request payload, so a
            // client cannot coax the real session into auto-approving.
            execution_environment:
                kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment::from_env(),
            execution_mode: options.execution_mode,
            workflow_enabled: options.workflow_enabled,
            resume_checkpoint,
            resume_reason: options.resume_reason.clone(),
        })
        .await;

    // Task 1.2: the turn is finished; drop its cancel token from the registry so
    // a late cancel request cannot affect a future turn for this session.
    kria_core::agent::gui_cognition::cancel::gui_cancel_registry().unregister(&session_id);

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
                    let invalidated = decision.invalidated_event_payload();
                    // Task 10.1: when streaming, the late HITL-invalidation
                    // envelopes are produced AFTER the runtime returns, so push
                    // them through the sink too — the drain task emits them live
                    // in the same FIFO order, and the batch loop below skips them
                    // (no duplicate emission).
                    if let Some(sink) = &event_sink {
                        sink.send(invalidated.clone());
                    }
                    outcome.events.push(invalidated);
                }
            }
        }
    }

    // Task 10.1: close the streaming channel and wait for the drain task to flush
    // every live `gui_cognition:event` emission before returning. Dropping the
    // runtime (which holds a sink clone) and our own sink handle closes the
    // channel so the drain loop terminates. While not streaming this is a no-op.
    drop(runtime);
    drop(event_sink);
    if let Some(handle) = drain_handle {
        let _ = handle.await;
    }

    // Task 10.1: when streaming, the `:thinking` state + every `gui_cognition:event`
    // envelope (and its `:approval_required` companion) were ALREADY emitted live
    // by the drain task DURING the turn, so they are intentionally OMITTED from
    // the returned batch to avoid double-emission. While not streaming, the batch
    // is built exactly as before (byte-for-byte unchanged).
    let mut events = if streaming {
        Vec::new()
    } else {
        vec![super::chat::desktop_chat_event(
            format!("{event_scope_prefix}:thinking"),
            serde_json::json!({"status": "processing", "mode": "gui_cognition"}),
        )]
    };
    if !streaming {
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

    fn scroll_request(target_name: &str, value: Option<&str>) -> GuiActionRequest {
        GuiActionRequest {
            kind: GuiActionKind::Scroll,
            role: "scrollable".into(),
            target_name: target_name.into(),
            value: value.map(str::to_string),
            execution_hint: "scroll".into(),
            abs_click: None,
        }
    }

    #[test]
    fn scroll_keys_map_each_direction_to_correct_shortcut() {
        // Task 4 (Issue #5): direction → keys mapping for a surface scroll.
        assert_eq!(scroll_keys_for_direction("down"), vec!["page_down"]);
        assert_eq!(scroll_keys_for_direction("up"), vec!["page_up"]);
        assert_eq!(scroll_keys_for_direction("bottom"), vec!["ctrl", "end"]);
        assert_eq!(scroll_keys_for_direction("end"), vec!["ctrl", "end"]);
        assert_eq!(scroll_keys_for_direction("top"), vec!["ctrl", "home"]);
        assert_eq!(scroll_keys_for_direction("beginning"), vec!["ctrl", "home"]);
        // Unknown / empty falls back to page_down (never blind-fails).
        assert_eq!(scroll_keys_for_direction("sideways"), vec!["page_down"]);
        assert_eq!(scroll_keys_for_direction(""), vec!["page_down"]);
        // Case-insensitive.
        assert_eq!(scroll_keys_for_direction("UP"), vec!["page_up"]);
    }

    #[test]
    fn scroll_keys_from_request_read_threaded_marker_and_fallback() {
        // Threaded marker on target_name (proposal target_label).
        assert_eq!(
            scroll_keys_for_request(&scroll_request("scroll:up", None)),
            vec!["page_up"]
        );
        assert_eq!(
            scroll_keys_for_request(&scroll_request("scroll:bottom", None)),
            vec!["ctrl", "end"]
        );
        assert_eq!(
            scroll_keys_for_request(&scroll_request("scroll:top", None)),
            vec!["ctrl", "home"]
        );
        assert_eq!(
            scroll_keys_for_request(&scroll_request("scroll:down", None)),
            vec!["page_down"]
        );
        // Fallback to value when target_name carries no marker.
        assert_eq!(
            scroll_keys_for_request(&scroll_request("", Some("scroll:up"))),
            vec!["page_up"]
        );
        // Bare direction word (no marker prefix) is honored too.
        assert_eq!(
            scroll_keys_for_request(&scroll_request("up", None)),
            vec!["page_up"]
        );
        // No direction anywhere → safe default.
        assert_eq!(
            scroll_keys_for_request(&scroll_request("", None)),
            vec!["page_down"]
        );
    }

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

    // Task 3 (Issue #9): desktop-layer cache-coherence test. Drives the REAL
    // desktop `FixtureGuiPerceptionProvider` through the freshness path and
    // asserts the pre-action observe and the post-action ForceFresh re-observe
    // are DISTINCT captures (different observation_id + different screen hash) —
    // the post-action verification re-observe is never the pre-action frame.
    #[tokio::test]
    async fn desktop_provider_force_fresh_post_action_reobserve_is_distinct() {
        use kria_core::agent::gui_cognition::perception::{
            collect_observation, collect_observation_with_freshness, ObservationFreshness,
        };

        // This fixture models a real screen change after the action: the
        // pre-action capture (index 0) and post-action re-observe (index >= 1)
        // return different screen hashes.
        let provider =
            FixtureGuiPerceptionProvider::new(GuiPerceptionFixture::Step8ClickResultChanges);

        // Pre-action observe (Default — caches may serve).
        let pre = collect_observation(&provider, "obs-pre".into(), "ctx".into()).await;

        // Post-action verification re-observe (ForceFresh).
        let post = collect_observation_with_freshness(
            &provider,
            "obs-post".into(),
            "ctx".into(),
            ObservationFreshness::ForceFresh,
        )
        .await;

        assert_ne!(
            pre.observation_id, post.observation_id,
            "pre/post observations must be distinct captures"
        );
        assert_ne!(
            pre.screen_hash, post.screen_hash,
            "post-action ForceFresh re-observe must reflect the changed screen, \
             not the pre-action frame (verify-by-screen-change is sound)"
        );
    }
}

#[cfg(test)]
mod ocr_quality_tests {
    //! Task 9 (Issue #7): OCR quality + scope. Pure-function CI checks (no
    //! display/backend): the `gui_cog_ocr_quality` flag gate (default ON, falsy
    //! rollback), intent-scope partition, ROI clamping, and flag-OFF byte-for-
    //! byte parity of the OCR preprocessing against the legacy path.
    use super::{
        gui_ocr_scope_for_prompt, ocr_quality_enabled_lookup, DesktopGuiPerceptionProvider,
        GuiOcrScope, OcrRoi,
    };

    /// Encode a solid-grey RGB image of the given size to PNG bytes (deterministic).
    fn synthetic_png(width: u32, height: u32) -> Vec<u8> {
        use std::io::Cursor;
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([128, 128, 128]));
        let dynamic = image::DynamicImage::ImageRgb8(img);
        let mut buffer = Cursor::new(Vec::new());
        dynamic
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode synthetic png");
        buffer.into_inner()
    }

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        let image = image::load_from_memory(bytes).expect("decode png");
        (image.width(), image.height())
    }

    #[test]
    fn ocr_quality_flag_defaults_on_when_env_absent() {
        assert!(ocr_quality_enabled_lookup(|_| None));
    }

    #[test]
    fn ocr_quality_flag_rolls_back_on_falsy_values() {
        for raw in ["0", "false", "no", "off", "", " OFF ", "False"] {
            assert!(
                !ocr_quality_enabled_lookup(|_| Some(raw.to_string())),
                "value {raw:?} must disable OCR quality (rollback)"
            );
        }
    }

    #[test]
    fn ocr_quality_flag_stays_on_for_truthy_values() {
        for raw in ["1", "true", "yes", "on", "anything-else"] {
            assert!(
                ocr_quality_enabled_lookup(|_| Some(raw.to_string())),
                "value {raw:?} must keep OCR quality ON"
            );
        }
    }

    #[test]
    fn ocr_scope_action_intents_skip_ocr() {
        for prompt in [
            "click the OK button",
            "type \"hello world\" into the search field",
            "press the Enter key",
        ] {
            // These resolve to focus/type/click/safe-action/risk-approval intents,
            // which are action-scoped (OCR skipped under the flag).
            assert_eq!(
                gui_ocr_scope_for_prompt(prompt),
                GuiOcrScope::ActionIntent,
                "prompt {prompt:?} should be action-scoped"
            );
        }
    }

    #[test]
    fn ocr_scope_read_intents_run_ocr() {
        for prompt in [
            "summarize what is visible on the screen",
            "read the current page and tell me what it says",
            "what is shown on screen right now",
        ] {
            assert_eq!(
                gui_ocr_scope_for_prompt(prompt),
                GuiOcrScope::ReadIntent,
                "prompt {prompt:?} should be read-scoped"
            );
        }
    }

    #[test]
    fn ocr_roi_clamps_to_image_and_rejects_unfit_regions() {
        // Fully inside → unchanged.
        assert_eq!(
            OcrRoi { x: 10, y: 20, width: 200, height: 100 }.clamp_to(1920, 1080),
            Some(OcrRoi { x: 10, y: 20, width: 200, height: 100 })
        );
        // Overflowing right/bottom → clamped to the in-bounds remainder.
        assert_eq!(
            OcrRoi { x: 1800, y: 1000, width: 500, height: 500 }.clamp_to(1920, 1080),
            Some(OcrRoi { x: 1800, y: 1000, width: 120, height: 80 })
        );
        // Remainder smaller than the minimum content edge → None (use full frame).
        assert_eq!(
            OcrRoi { x: 1900, y: 1070, width: 500, height: 500 }.clamp_to(1920, 1080),
            None
        );
        // A tiny region → None.
        assert_eq!(
            OcrRoi { x: 0, y: 0, width: 32, height: 32 }.clamp_to(1920, 1080),
            None
        );
    }

    #[test]
    fn flag_off_scoped_matches_legacy_prepare_byte_for_byte() {
        // Flag OFF must be byte-for-byte identical to the legacy full-frame path,
        // and must IGNORE any supplied ROI (no crop when quality is off).
        let bytes = synthetic_png(1920, 1080);
        let legacy = DesktopGuiPerceptionProvider::prepare_ocr_png(&bytes).expect("legacy");
        let scoped_none =
            DesktopGuiPerceptionProvider::prepare_ocr_png_scoped(&bytes, None, false)
                .expect("scoped none");
        let scoped_with_roi = DesktopGuiPerceptionProvider::prepare_ocr_png_scoped(
            &bytes,
            Some(OcrRoi { x: 100, y: 100, width: 800, height: 600 }),
            false,
        )
        .expect("scoped roi ignored");

        assert_eq!(legacy, scoped_none, "flag-OFF (no ROI) must equal legacy");
        assert_eq!(
            legacy, scoped_with_roi,
            "flag-OFF must ignore the ROI and equal legacy byte-for-byte"
        );
        // Legacy downscales the 1920-wide frame to the 1000px cap.
        assert_eq!(png_dimensions(&legacy.0).0, 1000);
        assert!(legacy.1.starts_with("downscaled_1920x1080_to_1000x"));
    }

    #[test]
    fn flag_on_crops_to_roi_at_adequate_resolution() {
        // A 1280-wide ROI is below the 1600px quality cap → cropped, NOT downscaled.
        let bytes = synthetic_png(1920, 1080);
        let (out, status) = DesktopGuiPerceptionProvider::prepare_ocr_png_scoped(
            &bytes,
            Some(OcrRoi { x: 100, y: 100, width: 1280, height: 720 }),
            true,
        )
        .expect("scoped quality roi");
        assert_eq!(png_dimensions(&out), (1280, 720), "ROI cropped at full detail");
        assert!(
            status.contains("roi_1280x720+100+100") && status.contains("adequate_1280x720"),
            "status {status:?} should record the ROI + adequate (non-downscaled) resolution"
        );
    }

    #[test]
    fn flag_on_full_frame_uses_adequate_cap_not_legacy_cap() {
        // No ROI, flag ON: full 1920-wide frame downscales to the 1600px quality
        // cap (adequate), NOT the legacy 1000px cap — text stays legible.
        let bytes = synthetic_png(1920, 1080);
        let (out, status) =
            DesktopGuiPerceptionProvider::prepare_ocr_png_scoped(&bytes, None, true)
                .expect("scoped quality full");
        assert_eq!(png_dimensions(&out).0, 1600);
        assert!(
            status.contains("quality_full_frame_downscaled_1920x1080_to_1600x900"),
            "status {status:?} should record the adequate 1600px downscale"
        );
    }
}

#[cfg(test)]
mod atspi_health_tests {
    //! Task 10 (Issue #8): the `gui_cog_atspi_health` flag gate + the additive,
    //! honest AT-SPI health surfaced in the source-status payload. Pure (no
    //! display/D-Bus): builds an `AtSpiSnapshot` directly.
    use super::{atspi_health_enabled_lookup, DesktopGuiPerceptionProvider};
    use kria_core::agent::atspi_engine::{AtSpiSnapshot, AtSpiSnapshotTiming};

    fn snapshot(status: &str, omitted: usize) -> AtSpiSnapshot {
        AtSpiSnapshot {
            status: status.into(),
            applications: Vec::new(),
            application_labels: Vec::new(),
            focused_app: None,
            focused_app_label: None,
            focused_window: None,
            elements: Vec::new(),
            dialog_visible: false,
            node_count: 0,
            omitted_node_count: omitted,
            skipped_apps: Vec::new(),
            source_blockers: Vec::new(),
            remediation: Vec::new(),
            roles: Vec::new(),
            timing: AtSpiSnapshotTiming::default(),
        }
    }

    #[test]
    fn atspi_health_flag_defaults_on_when_env_absent() {
        assert!(atspi_health_enabled_lookup(|_| None));
    }

    #[test]
    fn atspi_health_flag_rolls_back_on_falsy_values() {
        for raw in ["0", "false", "no", "off", "", " Off "] {
            assert!(
                !atspi_health_enabled_lookup(|_| Some(raw.to_string())),
                "value {raw:?} must disable atspi health surfacing (rollback)"
            );
        }
    }

    #[test]
    fn flag_off_payload_omits_health_fields_byte_for_byte() {
        let snap = snapshot("degraded", 5);
        let off = DesktopGuiPerceptionProvider::snapshot_source_status_with_health(&snap, false);
        let obj = off.as_object().expect("object");
        assert!(!obj.contains_key("atspi_health"));
        assert!(!obj.contains_key("atspi_resolution_trustworthy"));
        assert!(!obj.contains_key("atspi_health_reason"));
        // The prior fields remain present.
        assert!(obj.contains_key("accessibility_source_status"));
    }

    #[test]
    fn flag_on_payload_adds_honest_health_for_degraded_snapshot() {
        let snap = snapshot("degraded", 5);
        let on = DesktopGuiPerceptionProvider::snapshot_source_status_with_health(&snap, true);
        let obj = on.as_object().expect("object");
        assert_eq!(obj.get("atspi_health").and_then(|v| v.as_str()), Some("degraded"));
        assert_eq!(
            obj.get("atspi_resolution_trustworthy").and_then(|v| v.as_bool()),
            Some(false),
            "a degraded snapshot must not be trustworthy for resolution"
        );
        assert!(obj.get("atspi_health_reason").map(|v| !v.is_null()).unwrap_or(false));
    }

    #[test]
    fn flag_on_unavailable_snapshot_is_not_trustworthy() {
        let snap = snapshot("unavailable", 0);
        let on = DesktopGuiPerceptionProvider::snapshot_source_status_with_health(&snap, true);
        let obj = on.as_object().expect("object");
        assert_eq!(obj.get("atspi_health").and_then(|v| v.as_str()), Some("unavailable"));
        assert_eq!(
            obj.get("atspi_resolution_trustworthy").and_then(|v| v.as_bool()),
            Some(false)
        );
    }
}

#[cfg(test)]
mod fast_observe_tests {
    //! Task 12 (Issue #6): the `gui_cog_fast_observe` flag gate + the intent
    //! (goal-contract action) → observation-profile partition. Pure functions.
    use super::{fast_observe_enabled_lookup, gui_observe_profile_for_prompt, GuiObserveProfile};

    #[test]
    fn fast_observe_flag_defaults_on_when_env_absent() {
        assert!(fast_observe_enabled_lookup(|_| None));
    }

    #[test]
    fn fast_observe_flag_rolls_back_on_falsy_values() {
        for raw in ["0", "false", "no", "off", "", " Off "] {
            assert!(
                !fast_observe_enabled_lookup(|_| Some(raw.to_string())),
                "value {raw:?} must disable fast observe (rollback)"
            );
        }
    }

    #[test]
    fn fast_observe_flag_stays_on_for_truthy_values() {
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(fast_observe_enabled_lookup(|_| Some(raw.to_string())));
        }
    }

    #[test]
    fn primitive_action_turns_use_fast_profile() {
        for prompt in [
            "scroll down the page",
            "press the Escape key",
            "open the calculator",
            "switch to the Chrome window",
        ] {
            assert_eq!(
                gui_observe_profile_for_prompt(prompt),
                GuiObserveProfile::FastAction,
                "prompt {prompt:?} should fast-path (skip OCR + vision)"
            );
        }
    }

    #[test]
    fn read_and_resolve_turns_use_full_profile() {
        for prompt in [
            "summarize what is visible on the screen",
            "read the current page",
            "click the OK button",
        ] {
            assert_eq!(
                gui_observe_profile_for_prompt(prompt),
                GuiObserveProfile::Full,
                "prompt {prompt:?} must keep OCR/vision (read or control resolution)"
            );
        }
    }
}

#[cfg(test)]
mod local_planner_tests {
    //! Task 11 (Issue #2): the `gui_cog_local_planner` kill-switch for the local
    //! grammar planner rung (Rung B). Default ON; falsy = rollback (no local
    //! fallback wired → ladder collapses to Rung A → Rung C, byte-for-byte). The
    //! Rung B BEHAVIOR is covered by `gui_cognition_llm_planner_tests`
    //! (`ladder_rung_b_uses_local_grammar_fallback_when_configured_rejected`).
    use super::local_planner_enabled_lookup;

    #[test]
    fn local_planner_flag_defaults_on_when_env_absent() {
        assert!(local_planner_enabled_lookup(|_| None));
    }

    #[test]
    fn local_planner_flag_rolls_back_on_falsy_values() {
        for raw in ["0", "false", "no", "off", "", " OFF "] {
            assert!(
                !local_planner_enabled_lookup(|_| Some(raw.to_string())),
                "value {raw:?} must disable the local grammar rung (rollback)"
            );
        }
    }

    #[test]
    fn local_planner_flag_stays_on_for_truthy_values() {
        for raw in ["1", "true", "yes", "on", "anything"] {
            assert!(local_planner_enabled_lookup(|_| Some(raw.to_string())));
        }
    }
}

#[cfg(test)]
mod real_vision_tests {
    //! Task 8 (Issue #1): the `gui_cog_real_vision` mode parser. `Off` restores
    //! prior perception byte-for-byte; `Vl7b` (default) + `Light` are the real-
    //! vision modes that honestly degrade on a stub/unavailable model.
    use super::{gui_real_vision_mode_from, GuiRealVisionMode};

    #[test]
    fn real_vision_defaults_to_vl7b_when_absent() {
        assert_eq!(gui_real_vision_mode_from(None), GuiRealVisionMode::Vl7b);
    }

    #[test]
    fn real_vision_off_for_falsy_values() {
        for raw in ["off", "0", "false", "no", ""] {
            assert_eq!(
                gui_real_vision_mode_from(Some(raw)),
                GuiRealVisionMode::Off,
                "value {raw:?} must select Off (prior perception byte-for-byte)"
            );
        }
    }

    #[test]
    fn real_vision_light_and_vl7b_modes() {
        assert_eq!(gui_real_vision_mode_from(Some("light")), GuiRealVisionMode::Light);
        assert_eq!(gui_real_vision_mode_from(Some("LIGHT")), GuiRealVisionMode::Light);
        assert_eq!(gui_real_vision_mode_from(Some("vl7b")), GuiRealVisionMode::Vl7b);
        assert_eq!(gui_real_vision_mode_from(Some("on")), GuiRealVisionMode::Vl7b);
        assert_eq!(gui_real_vision_mode_from(Some("true")), GuiRealVisionMode::Vl7b);
    }
}

// ============================================================================
// GUI Cognition V2 — desktop glue (Part B)
//
// Wires the decoupled kria-core V2 layers (Sight/Brain/Hands + bounded loop)
// to the real desktop substrate:
//   - `V2DesktopScreenCapturer` → KRIA's GNOME-extension screen capture
//     (`kria_ext::ext_capture_screen`), the only capture path that works on this
//     GNOME Wayland box. Records the captured PNG dimensions so the input sink
//     can normalize absolute clicks.
//   - `V2DesktopInputSink` → the existing uinput daemon backend (`YdotoolBackend`),
//     the same input substrate V1 uses. On Wayland clicks go through the daemon's
//     absolute-coordinate path ([0,65535] normalized) so they land on native
//     Wayland windows.
//   - `V2DesktopSafetyGate` → an HONEST gate over the existing global safety halt
//     + the GUI-automation master switch. It NEVER fabricates an approval; a
//     denial stops the turn. (A full HITL pause/approve round-trip is a follow-up;
//     the loop already halts safely on `Deny`.)
//   - `run_gui_cognition_v2` → builds the three layers + guards and runs ONE
//     bounded turn, streaming per-step `gui_cognition:event` envelopes on the
//     existing channel and returning the same `DesktopChatCommandCapture` shape
//     as the V1 path.
//
// Reached only when `KRIA_GUI_COG_V2` is truthy (default OFF → V1 unchanged).
// ============================================================================

/// Shared per-turn screen dimensions, written by the capturer (from the captured
/// PNG) and read by the input sink for absolute-coordinate normalization.
#[derive(Default)]
struct V2ScreenDims {
    w: std::sync::atomic::AtomicU32,
    h: std::sync::atomic::AtomicU32,
}

impl V2ScreenDims {
    fn store(&self, w: u32, h: u32) {
        self.w.store(w, std::sync::atomic::Ordering::SeqCst);
        self.h.store(h, std::sync::atomic::Ordering::SeqCst);
    }
    fn get(&self) -> Option<(u32, u32)> {
        let w = self.w.load(std::sync::atomic::Ordering::SeqCst);
        let h = self.h.load(std::sync::atomic::Ordering::SeqCst);
        (w > 0 && h > 0).then_some((w, h))
    }
}

/// Decode PNG width/height from the IHDR chunk (big-endian at bytes 16..24).
/// `None` if the buffer is not a PNG. Cheap — no full decode.
fn v2_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[0..8] != SIG {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    (w > 0 && h > 0).then_some((w, h))
}

fn v2_base64_png(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn v2_is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

fn v2_env_truthy(key: &str) -> bool {
    std::env::var(key)
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// V2 `ScreenCapturer` backed by KRIA's working GNOME-extension capture.
struct V2DesktopScreenCapturer {
    dims: StdArc<V2ScreenDims>,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::ScreenCapturer for V2DesktopScreenCapturer {
    async fn capture_png_base64(&self) -> Option<String> {
        let bytes = kria_ext::ext_capture_screen().await?;
        if let Some((w, h)) = v2_png_dimensions(&bytes) {
            self.dims.store(w, h);
        }
        Some(v2_base64_png(&bytes))
    }
}

/// Map a single combo token (e.g. `ctrl`, `shift`, `t`, `plus`) to a [`Key`].
fn v2_key_from_token(tok: &str) -> Option<kria_core::tools::gui_automation::Key> {
    use kria_core::tools::gui_automation::Key;
    Some(match tok.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" => Key::Alt,
        "super" | "win" | "cmd" | "command" | "meta" => Key::Super,
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" | "bksp" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" | "pgup" => Key::PageUp,
        "pagedown" | "page_down" | "pgdn" => Key::PageDown,
        "up" | "arrowup" => Key::ArrowUp,
        "down" | "arrowdown" => Key::ArrowDown,
        "left" | "arrowleft" => Key::ArrowLeft,
        "right" | "arrowright" => Key::ArrowRight,
        "plus" => Key::Char('+'),
        "minus" => Key::Char('-'),
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        other if other.chars().count() == 1 => Key::Char(other.chars().next().unwrap()),
        _ => return None,
    })
}

/// Parse a `+`-separated combo (e.g. `ctrl+shift+t`) into an ordered key list.
fn v2_parse_combo(combo: &str) -> Vec<kria_core::tools::gui_automation::Key> {
    combo.split('+').filter_map(v2_key_from_token).collect()
}

/// V2 `InputSink` over the existing uinput daemon backend.
struct V2DesktopInputSink {
    backend: StdArc<dyn kria_core::tools::gui_automation::GuiBackend>,
    dims: StdArc<V2ScreenDims>,
    wayland: bool,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::InputSink for V2DesktopInputSink {
    async fn click(&self, x: i32, y: i32) -> anyhow::Result<()> {
        use kria_core::tools::gui_automation::MouseButton;
        // On Wayland a relative-position click cannot be placed reliably; use the
        // daemon's absolute path with [0,65535] normalization from the current
        // screen size (the same contract V1 uses for native Wayland clicks).
        if self.wayland {
            if let Some((w, h)) = self.dims.get() {
                let nx = ((x as i64 * 65_535) / (w.max(1) as i64)).clamp(0, 65_535) as i32;
                let ny = ((y as i64 * 65_535) / (h.max(1) as i64)).clamp(0, 65_535) as i32;
                self.backend
                    .click_mouse_abs(nx, ny, MouseButton::Left)
                    .await
                    .map_err(|e| anyhow::anyhow!("abs click failed: {e}"))?;
                return Ok(());
            }
        }
        self.backend
            .click_mouse(x, y, MouseButton::Left)
            .await
            .map_err(|e| anyhow::anyhow!("click failed: {e}"))?;
        Ok(())
    }

    async fn type_text(&self, text: &str) -> anyhow::Result<()> {
        self.backend
            .type_text(text, None)
            .await
            .map_err(|e| anyhow::anyhow!("type failed: {e}"))?;
        Ok(())
    }

    async fn key(&self, combo: &str) -> anyhow::Result<()> {
        let keys = v2_parse_combo(combo);
        if keys.is_empty() {
            anyhow::bail!("unrecognized key combo: {combo}");
        }
        self.backend
            .press_shortcut(&keys, None)
            .await
            .map_err(|e| anyhow::anyhow!("key failed: {e}"))?;
        Ok(())
    }

    async fn scroll(&self, direction: &str, _amount: i32) -> anyhow::Result<()> {
        // Reuse V1's app-agnostic direction → shortcut mapping (PageDown/Up,
        // Ctrl+End/Home) so scrolling works without per-app coordinates.
        let keys: Vec<kria_core::tools::gui_automation::Key> = scroll_keys_for_direction(direction)
            .iter()
            .filter_map(|k| v2_key_from_token(k))
            .collect();
        if keys.is_empty() {
            anyhow::bail!("unsupported scroll direction: {direction}");
        }
        self.backend
            .press_shortcut(&keys, None)
            .await
            .map_err(|e| anyhow::anyhow!("scroll failed: {e}"))?;
        Ok(())
    }

    fn backend_label(&self) -> &str {
        "uinput"
    }
}

/// V2 `SafetyGate` over the existing global safety halt + automation switch.
/// Honest: it only ever `Allow`s or `Deny`s — it never fabricates a human
/// approval. A `Deny` halts the turn (the loop guarantees no execution).
struct V2DesktopSafetyGate {
    automation_enabled: bool,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::SafetyGate for V2DesktopSafetyGate {
    async fn evaluate(
        &self,
        _decision: &kria_core::agent::gui_cognition_v2::Decision,
        _observation: &kria_core::agent::gui_cognition_v2::Observation,
    ) -> kria_core::agent::gui_cognition_v2::GateDecision {
        use kria_core::agent::gui_cognition_v2::GateDecision;
        if kria_core::safety::is_halted() {
            return GateDecision::Deny(
                kria_core::safety::halt_reason()
                    .unwrap_or_else(|| "global safety halt engaged".into()),
            );
        }
        if !self.automation_enabled {
            return GateDecision::Deny("GUI automation is disabled (master switch off)".into());
        }
        GateDecision::Allow
    }
}

/// Build a minimal error capture (used when a prerequisite layer is unavailable).
fn v2_error_capture(event_scope_prefix: &str, reply: &str) -> super::chat::DesktopChatCommandCapture {
    let events = vec![
        super::chat::desktop_chat_event(
            format!("{event_scope_prefix}:token"),
            serde_json::json!({ "text": reply }),
        ),
        super::chat::desktop_chat_event(format!("{event_scope_prefix}:done"), serde_json::json!({})),
    ];
    super::chat::DesktopChatCommandCapture {
        status_code: 200,
        status: "processing".into(),
        reply: reply.to_string(),
        response: serde_json::json!({
            "gui_cognition": { "engine": "v2", "status": "stopped_error", "error": reply }
        }),
        events,
    }
}

/// Run ONE GUI Cognition V2 turn end-to-end over the real desktop substrate.
pub(super) async fn run_gui_cognition_v2(
    message: String,
    app_state: &AppState,
    session_id: String,
    event_scope_prefix: &str,
    options: Option<GuiCognitionCommandOptions>,
    event_emitter: Option<AppHandle>,
) -> Result<super::chat::DesktopChatCommandCapture, String> {
    use kria_core::agent::gui_cognition_v2 as v2;
    use kria_core::agent::gui_cognition_v2::GuiBrain as _; // for `brain.label()`

    // V2 reads its configuration from the server-side environment; the V1
    // fixture options are not (yet) modeled in V2.
    let _ = options;
    let turn_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4().to_string();

    let want_som = v2_env_truthy("KRIA_GUI_COG_V2_SOM");
    let max_steps = std::env::var("KRIA_GUI_COG_V2_MAX_STEPS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(12);
    let observe_timeout_secs = std::env::var("KRIA_GUI_COG_V2_OBSERVE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);

    // --- Sight (OmniParser sidecar + KRIA extension capture) ---
    let dims = StdArc::new(V2ScreenDims::default());
    let endpoint = std::env::var("KRIA_OMNIPARSER_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let capturer: StdArc<dyn v2::ScreenCapturer> =
        StdArc::new(V2DesktopScreenCapturer { dims: dims.clone() });
    let sight = v2::OmniParserSight::new(endpoint)
        .with_timeout(Duration::from_secs(observe_timeout_secs))
        .with_capturer(capturer);

    // --- Brain (local Qwen via the model router) ---
    let backend = match app_state.model_router.route("gui_cognition_planner").await {
        Some(backend) => backend,
        None => {
            return Ok(v2_error_capture(
                event_scope_prefix,
                "The reasoning model is not available; cannot run GUI Cognition V2.",
            ));
        }
    };
    let brain = v2::QwenBrain::new(backend).with_som(want_som);
    let brain_label = brain.label().to_string();

    // --- Hands (existing uinput daemon backend) ---
    let socket_path = kria_core::agent::gui_services::default_uinput_socket_path();
    let gui_backend: StdArc<dyn kria_core::tools::gui_automation::GuiBackend> =
        StdArc::new(kria_core::tools::gui_automation::YdotoolBackend::new(socket_path));
    let sink = V2DesktopInputSink {
        backend: gui_backend,
        dims: dims.clone(),
        wayland: v2_is_wayland(),
    };
    let hands = v2::UinputHands::new(sink);

    // --- Guards: safety gate + cancel bridge ---
    let automation_enabled = match app_state.gui_orchestrator.as_ref() {
        Some(orch) => orch.status().await.automation_enabled,
        None => false,
    };
    let gate: StdArc<dyn v2::SafetyGate> = StdArc::new(V2DesktopSafetyGate { automation_enabled });

    let cancel_flag = StdArc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_token =
        kria_core::agent::gui_cognition::cancel::gui_cancel_registry().register(&session_id);
    {
        // Bridge the existing GUI cancel token (driven by the desktop cancel
        // command) into the V2 loop's cooperative cancel flag.
        let flag = cancel_flag.clone();
        let raw = cancel_token.raw().clone();
        tokio::spawn(async move {
            raw.cancelled().await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }
    let guards = v2::LoopGuards::none()
        .with_safety(gate)
        .with_cancel(cancel_flag);

    let config = v2::LoopConfig { max_steps, want_som, no_progress_limit: 2 };

    // Emit the `:thinking` state up front so the UI ordering matches V1.
    if let Some(app) = event_emitter.as_ref() {
        let _ = app.emit(
            &format!("{event_scope_prefix}:thinking"),
            serde_json::json!({ "status": "processing", "mode": "gui_cognition" }),
        );
    }

    let outcome = v2::run_turn_v2(&sight, &brain, &hands, &message, config, &guards).await;

    kria_core::agent::gui_cognition::cancel::gui_cancel_registry().unregister(&session_id);

    // --- Build the per-step receipts + response payload ---
    let steps_json: Vec<serde_json::Value> = outcome
        .steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "step_index": s.step_index,
                "action": s.decision.action.kind(),
                "reason": s.decision.reason,
                "target_label": s.target_label,
                "ok": s.result.ok,
                "error": s.result.error,
                "backend_used": s.result.backend_used,
            })
        })
        .collect();
    let response = serde_json::json!({
        "gui_cognition": {
            "engine": "v2",
            "status": outcome.status.as_str(),
            "brain": brain_label,
            "step_count": outcome.steps.len(),
            "steps": steps_json,
        }
    });

    // --- Events: stream live when an emitter is present, else return the batch ---
    let streaming = event_emitter.is_some();
    let mut events: Vec<super::chat::DesktopChatCommandEvent> = if streaming {
        Vec::new()
    } else {
        vec![super::chat::desktop_chat_event(
            format!("{event_scope_prefix}:thinking"),
            serde_json::json!({ "status": "processing", "mode": "gui_cognition" }),
        )]
    };
    for (index, step) in outcome.steps.iter().enumerate() {
        let payload = gui_cognition_event_payload(
            &session_id,
            &turn_id,
            &workflow_id,
            (index + 1) as u64,
            serde_json::json!({
                "type": "V2Step",
                "step_index": step.step_index,
                "action": step.decision.action.kind(),
                "target_label": step.target_label,
                "ok": step.result.ok,
                "error": step.result.error,
                "backend_used": step.result.backend_used,
            }),
        );
        if streaming {
            if let Some(app) = event_emitter.as_ref() {
                let _ = app.emit("gui_cognition:event", payload);
            }
        } else {
            events.push(super::chat::desktop_chat_event("gui_cognition:event", payload));
        }
    }

    // Persist the turn to memory (mirrors the V1 path).
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
        Some(response["gui_cognition"].to_string()),
        None,
    ));

    events.push(super::chat::desktop_chat_stage_event(
        "gui_cognition_mode_handled",
        "GUI Cognition prompt handled by the V2 Sight/Brain/Hands loop",
        Some(serde_json::json!({
            "engine": "v2",
            "status": outcome.status.as_str(),
            "workflow_id": workflow_id,
        })),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:token"),
        serde_json::json!({ "text": outcome.reply.clone() }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:tool_result"),
        serde_json::json!({ "tool": "gui_cognition", "result": response.clone() }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:done"),
        serde_json::json!({}),
    ));

    Ok(super::chat::DesktopChatCommandCapture {
        status_code: 200,
        status: "processing".into(),
        reply: outcome.reply,
        response,
        events,
    })
}

#[cfg(test)]
mod gui_cognition_v2_glue_tests {
    use super::*;

    #[test]
    fn png_dimensions_reads_ihdr() {
        // Minimal 1920x1200 PNG header (signature + IHDR length/type + w/h).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes.extend_from_slice(&[0, 0, 0, 13]); // IHDR length
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1920u32.to_be_bytes());
        bytes.extend_from_slice(&1200u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]); // bit depth/color/...
        assert_eq!(v2_png_dimensions(&bytes), Some((1920, 1200)));
    }

    #[test]
    fn png_dimensions_rejects_non_png() {
        assert_eq!(v2_png_dimensions(b"not a png buffer at all...."), None);
        assert_eq!(v2_png_dimensions(&[0u8; 4]), None);
    }

    #[test]
    fn screen_dims_roundtrip() {
        let d = V2ScreenDims::default();
        assert_eq!(d.get(), None);
        d.store(1920, 1200);
        assert_eq!(d.get(), Some((1920, 1200)));
    }

    #[test]
    fn parse_combo_maps_modifiers_and_keys() {
        use kria_core::tools::gui_automation::Key;
        assert_eq!(v2_parse_combo("ctrl+t"), vec![Key::Control, Key::Char('t')]);
        assert_eq!(
            v2_parse_combo("ctrl+shift+z"),
            vec![Key::Control, Key::Shift, Key::Char('z')]
        );
        assert_eq!(v2_parse_combo("ctrl+plus"), vec![Key::Control, Key::Char('+')]);
        assert_eq!(v2_parse_combo("ctrl+l"), vec![Key::Control, Key::Char('l')]);
        assert_eq!(v2_parse_combo("enter"), vec![Key::Enter]);
    }

    #[test]
    fn parse_combo_drops_unknown_tokens() {
        // An unmappable multi-char token yields no key for that segment.
        assert!(v2_parse_combo("kaboom").is_empty());
    }
}
