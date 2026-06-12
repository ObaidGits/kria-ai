use super::executor::{stable_target_identity_hash, GuiActionExecution, GuiActionKind};
use super::perception::{sanitize_gui_text, GuiControlSummary, GuiObservationSnapshot};

/// Legacy Step 7 verification report. Retained for backward compatibility with
/// the pre-Step-8 execution path and existing tests. Step 8 callers should use
/// [`GuiPostActionVerificationResult`] instead.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuiVerificationReport {
    pub status: String,
    pub confidence: f64,
    pub after_observation_id: String,
}

pub fn verify_post_action(
    execution: &GuiActionExecution,
    post_observation: &GuiObservationSnapshot,
    success_confidence: f64,
) -> GuiVerificationReport {
    GuiVerificationReport {
        status: if execution.success {
            "completed".into()
        } else {
            "failed".into()
        },
        confidence: if execution.success {
            success_confidence
        } else {
            0.2
        },
        after_observation_id: post_observation.observation_id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Step 8: Post-Action Verification
// ---------------------------------------------------------------------------

/// Deterministic verification strategies. Visual/OCR evidence may support a
/// strategy but can never invent an executable result on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiVerificationStrategy {
    WindowVisible,
    ActiveWindowMatch,
    FocusedControl,
    TextPresent,
    StateChanged,
    ScreenChanged,
    ResultVisible,
    DialogVisible,
    FileSaved,
    DownloadStartedOrCompleted,
    ClipboardChanged,
    TargetResolved,
    VisibleContentSummarized,
    Inconclusive,
}

impl GuiVerificationStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowVisible => "window_visible",
            Self::ActiveWindowMatch => "active_window_match",
            Self::FocusedControl => "focused_control",
            Self::TextPresent => "text_present",
            Self::StateChanged => "state_changed",
            Self::ScreenChanged => "screen_changed",
            Self::ResultVisible => "result_visible",
            Self::DialogVisible => "dialog_visible",
            Self::FileSaved => "file_saved",
            Self::DownloadStartedOrCompleted => "download_started_or_completed",
            Self::ClipboardChanged => "clipboard_changed",
            Self::TargetResolved => "target_resolved",
            Self::VisibleContentSummarized => "visible_content_summarized",
            Self::Inconclusive => "inconclusive",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "window_visible" => Self::WindowVisible,
            "active_window_match" => Self::ActiveWindowMatch,
            "focused_control" => Self::FocusedControl,
            "text_present" => Self::TextPresent,
            "state_changed" => Self::StateChanged,
            "screen_changed" => Self::ScreenChanged,
            "result_visible" => Self::ResultVisible,
            "dialog_visible" => Self::DialogVisible,
            "file_saved" => Self::FileSaved,
            "download_started_or_completed" => Self::DownloadStartedOrCompleted,
            "clipboard_changed" => Self::ClipboardChanged,
            "target_resolved" => Self::TargetResolved,
            "visible_content_summarized" => Self::VisibleContentSummarized,
            _ => Self::Inconclusive,
        }
    }
}

/// Choose the action-specific verification strategy. Secret payloads never use
/// `text_present` so raw secret text is never searched for or echoed; they use
/// `state_changed` evidence instead.
pub fn select_verification_strategy(
    action: &GuiActionKind,
    is_secret_payload: bool,
) -> GuiVerificationStrategy {
    match action {
        GuiActionKind::OpenApp => GuiVerificationStrategy::ActiveWindowMatch,
        GuiActionKind::SwitchWindow => GuiVerificationStrategy::ActiveWindowMatch,
        GuiActionKind::FocusField => GuiVerificationStrategy::FocusedControl,
        GuiActionKind::TypeText | GuiActionKind::FillField => {
            if is_secret_payload {
                GuiVerificationStrategy::StateChanged
            } else {
                GuiVerificationStrategy::TextPresent
            }
        }
        GuiActionKind::Paste => {
            if is_secret_payload {
                GuiVerificationStrategy::StateChanged
            } else {
                GuiVerificationStrategy::TextPresent
            }
        }
        GuiActionKind::ClickControl => GuiVerificationStrategy::ResultVisible,
        GuiActionKind::PressKey | GuiActionKind::Hotkey => GuiVerificationStrategy::ScreenChanged,
        GuiActionKind::Scroll => GuiVerificationStrategy::ScreenChanged,
        GuiActionKind::Copy => GuiVerificationStrategy::ClipboardChanged,
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiPostActionVerificationRequest {
    pub verification_id: String,
    pub execution_id: String,
    pub proposal_id: String,
    pub proposal_hash: String,
    pub action_type: String,
    pub target_hash: String,
    pub stable_target_identity_hash: Option<String>,
    pub expected_postcondition: String,
    pub verification_strategy: String,
    pub pre_action_context_id: String,
    pub post_action_observation_id: String,
    pub post_action_context_id: String,
    pub started_at_ms: i64,
    pub is_secret_payload: bool,
    pub prompt_hash: String,
    pub target_label: Option<String>,
    pub target_role: Option<String>,
    pub target_control_id: Option<String>,
    pub expected_app_hint: Option<String>,
    pub expected_window_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GuiPostActionVerificationResult {
    pub verification_id: String,
    pub execution_id: String,
    pub proposal_id: String,
    pub status: String,
    pub verification_strategy: String,
    pub evidence: Vec<String>,
    pub pre_state_summary: String,
    pub post_state_summary: String,
    pub matched_expected_state: bool,
    pub target_still_present: bool,
    pub target_identity_matches: bool,
    pub confidence: f64,
    pub safe_error_summary: Option<String>,
    pub recovery_hint: Option<String>,
    pub can_retry: bool,
    pub prompt_hash: String,
}

pub const VERIFICATION_VERIFIED: &str = "verified";
pub const VERIFICATION_FAILED: &str = "verification_failed";
pub const VERIFICATION_INCONCLUSIVE: &str = "inconclusive";
pub const VERIFICATION_BLOCKED: &str = "blocked";

impl GuiPostActionVerificationResult {
    pub fn is_verified(&self) -> bool {
        self.status == VERIFICATION_VERIFIED
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verification_id": self.verification_id,
            "execution_id": self.execution_id,
            "proposal_id": self.proposal_id,
            "status": self.status,
            "verification_strategy": self.verification_strategy,
            "evidence": self.evidence,
            "pre_state_summary": self.pre_state_summary,
            "post_state_summary": self.post_state_summary,
            "matched_expected_state": self.matched_expected_state,
            "target_still_present": self.target_still_present,
            "target_identity_matches": self.target_identity_matches,
            "confidence": self.confidence,
            "safe_error_summary": self.safe_error_summary,
            "recovery_hint": self.recovery_hint,
            "can_retry": self.can_retry,
            "prompt_hash": self.prompt_hash,
        })
    }

    pub fn event_payload(&self) -> serde_json::Value {
        let mut payload = self.summary_json();
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "type".into(),
                serde_json::Value::String("ExecutionVerificationCompleted".into()),
            );
        }
        payload
    }
}

fn idempotent_retry(action: &GuiActionKind) -> bool {
    matches!(
        action,
        GuiActionKind::OpenApp
            | GuiActionKind::SwitchWindow
            | GuiActionKind::FocusField
            | GuiActionKind::Scroll
    )
}

fn safe_token(value: &str, limit: usize) -> String {
    sanitize_gui_text(value, limit).text
}

fn screen_hash_prefix(observation: &GuiObservationSnapshot) -> String {
    observation
        .screen_hash
        .as_deref()
        .map(|hash| hash.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "unknown".into())
}

fn state_summary(observation: &GuiObservationSnapshot) -> String {
    let focus_role = observation
        .cursor_focus
        .focused_control_role
        .as_deref()
        .map(|role| safe_token(role, 40))
        .unwrap_or_else(|| "none".into());
    format!(
        "app={}; controls={}; dialogs={}; focus_role={}; screen={}",
        safe_token(
            observation
                .active_window
                .app_name
                .as_deref()
                .unwrap_or("unknown"),
            60
        ),
        observation.visible_control_count(),
        observation.dialogs.len(),
        focus_role,
        screen_hash_prefix(observation),
    )
}

fn text_contains(haystack: &str, needle: &str) -> bool {
    if needle.trim().is_empty() {
        return false;
    }
    haystack.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

fn find_target_control<'a>(
    observation: &'a GuiObservationSnapshot,
    control_id: Option<&str>,
    label: Option<&str>,
    role: Option<&str>,
) -> Option<GuiControlSummary> {
    let controls = observation.all_controls();
    if let Some(control_id) = control_id.filter(|value| !value.trim().is_empty()) {
        if let Some(found) = controls.iter().find(|control| control.control_id == control_id) {
            return Some(found.clone());
        }
    }
    if let Some(label) = label.filter(|value| !value.trim().is_empty()) {
        if let Some(found) = controls.iter().find(|control| {
            control.name.eq_ignore_ascii_case(label)
                && role
                    .map(|role| control.role.eq_ignore_ascii_case(role))
                    .unwrap_or(true)
        }) {
            return Some(found.clone());
        }
    }
    None
}

fn screen_changed(
    pre: &GuiObservationSnapshot,
    post: &GuiObservationSnapshot,
) -> Option<bool> {
    match (pre.screen_hash.as_deref(), post.screen_hash.as_deref()) {
        (Some(before), Some(after)) => Some(before != after),
        _ => None,
    }
}

fn focus_changed(pre: &GuiObservationSnapshot, post: &GuiObservationSnapshot) -> bool {
    pre.cursor_focus.focused_control_id != post.cursor_focus.focused_control_id
}

fn window_token_match(post: &GuiObservationSnapshot, hint: &str) -> bool {
    let hint = hint.trim();
    if hint.len() < 3 {
        return false;
    }
    let label = post.active_window.label.to_ascii_lowercase();
    let app = post
        .active_window
        .app_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let hint_lower = hint.to_ascii_lowercase();
    if label.contains(&hint_lower) || app.contains(&hint_lower) {
        return true;
    }
    // Token-wise match so "Google Search - Chrome" matches a "Chrome" hint.
    hint_lower
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .any(|token| label.contains(token) || app.contains(token))
        || post.visible_windows.iter().any(|window| {
            window.title.to_ascii_lowercase().contains(&hint_lower)
                || window
                    .app_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&hint_lower)
        })
}

fn text_present_in_observation(post: &GuiObservationSnapshot, needle: &str) -> bool {
    post.all_controls()
        .iter()
        .any(|control| text_contains(&control.name, needle))
        || post
            .ocr_blocks
            .iter()
            .any(|block| text_contains(&block.safe_text_preview, needle))
}

/// Run deterministic post-action verification. The verifier re-observes are
/// passed in by the caller (`pre_observation` = pre-action context observation,
/// `post_observation` = re-observed snapshot after the action attempt).
///
/// `expected_text` is the backend-only raw payload for non-secret typing/paste
/// actions. It is used to confirm presence but is never written into the
/// result. For secret payloads, callers must pass `None` and rely on
/// `state_changed`.
pub fn verify_post_action_detailed(
    request: &GuiPostActionVerificationRequest,
    pre_observation: &GuiObservationSnapshot,
    post_observation: &GuiObservationSnapshot,
    backend_success: bool,
    expected_text: Option<&str>,
    _now_ms: i64,
) -> GuiPostActionVerificationResult {
    let action_kind = GuiActionKind::from_action_type(&request.action_type);
    let strategy = GuiVerificationStrategy::from_str(&request.verification_strategy);
    let pre_state_summary = state_summary(pre_observation);
    let post_state_summary = state_summary(post_observation);

    let target_control = find_target_control(
        post_observation,
        request.target_control_id.as_deref(),
        request.target_label.as_deref(),
        request.target_role.as_deref(),
    );
    let target_still_present = target_control.is_some();
    let target_identity_matches = match (&target_control, request.stable_target_identity_hash.as_deref()) {
        (Some(control), Some(expected_hash)) => {
            let recomputed = stable_target_identity_hash(
                Some(&control.control_id),
                Some(&control.role),
                Some(&control.name),
                control.bounds.as_ref(),
                request.expected_app_hint.as_deref(),
                request.expected_window_hint.as_deref(),
            );
            recomputed == expected_hash
        }
        // No stable identity recorded (e.g. OpenApp) -> not a mismatch.
        (_, None) => true,
        (None, Some(_)) => false,
    };

    let mut evidence: Vec<String> = Vec::new();

    // Backend did not complete: there is nothing to verify. Fail closed.
    if !backend_success {
        evidence.push("backend action did not complete; no state to verify".into());
        return GuiPostActionVerificationResult {
            verification_id: request.verification_id.clone(),
            execution_id: request.execution_id.clone(),
            proposal_id: request.proposal_id.clone(),
            status: VERIFICATION_BLOCKED.into(),
            verification_strategy: strategy.as_str().into(),
            evidence,
            pre_state_summary,
            post_state_summary,
            matched_expected_state: false,
            target_still_present,
            target_identity_matches,
            confidence: 0.2,
            safe_error_summary: Some("Backend action failed before verification.".into()),
            recovery_hint: Some(
                "Re-observe the screen and resolve a fresh target before any retry.".into(),
            ),
            can_retry: false,
            prompt_hash: request.prompt_hash.clone(),
        };
    }

    let changed = screen_changed(pre_observation, post_observation);

    // matched: Some(true|false) when the strategy could be evaluated, None when
    // the available evidence is insufficient (=> inconclusive, never blind pass).
    let matched: Option<bool> = match strategy {
        GuiVerificationStrategy::WindowVisible | GuiVerificationStrategy::ActiveWindowMatch => {
            let hint = request
                .expected_window_hint
                .as_deref()
                .filter(|value| value.trim().len() >= 3)
                .or(request.expected_app_hint.as_deref())
                .filter(|value| value.trim().len() >= 3)
                .or(request.target_label.as_deref())
                .filter(|value| value.trim().len() >= 3);
            match hint {
                Some(hint) => {
                    let ok = window_token_match(post_observation, hint);
                    if ok {
                        evidence.push(format!(
                            "active window matches expected app/window hint ({})",
                            safe_token(hint, 60)
                        ));
                    } else {
                        evidence.push(format!(
                            "active window did not match expected app/window hint ({})",
                            safe_token(hint, 60)
                        ));
                    }
                    Some(ok)
                }
                None => {
                    if post_observation.active_window_probe_ok
                        && post_observation.active_window.confidence > 0.0
                    {
                        evidence.push(
                            "active window is known but no expected app/window hint was provided"
                                .into(),
                        );
                        None
                    } else {
                        evidence.push("active window is unknown after the action".into());
                        Some(false)
                    }
                }
            }
        }
        GuiVerificationStrategy::FocusedControl => {
            let focus = &post_observation.cursor_focus;
            let id_match = match (
                focus.focused_control_id.as_deref(),
                request.target_control_id.as_deref(),
            ) {
                (Some(found), Some(expected)) if !expected.trim().is_empty() => found == expected,
                _ => false,
            };
            let label_match = match (
                focus.focused_control_label.as_deref(),
                request.target_label.as_deref(),
            ) {
                (Some(found), Some(expected)) if !expected.trim().is_empty() => {
                    found.eq_ignore_ascii_case(expected)
                }
                _ => false,
            };
            let control_focused = post_observation.all_controls().iter().any(|control| {
                control.focused
                    && request
                        .target_label
                        .as_deref()
                        .map(|label| control.name.eq_ignore_ascii_case(label))
                        .unwrap_or(false)
            });
            if focus.keyboard_focus_known || control_focused {
                let ok = id_match || label_match || control_focused;
                if ok {
                    evidence.push("expected control reports keyboard focus".into());
                } else {
                    evidence.push("focus moved to a different control after the action".into());
                }
                Some(ok)
            } else {
                evidence.push("keyboard focus is not observable after the action".into());
                None
            }
        }
        GuiVerificationStrategy::TextPresent => match expected_text {
            Some(text) if !text.trim().is_empty() => {
                let ok = text_present_in_observation(post_observation, text);
                if ok {
                    evidence.push("expected text is present in the post-action GUI state".into());
                } else {
                    evidence
                        .push("expected text was not found in the post-action GUI state".into());
                }
                Some(ok)
            }
            _ => {
                evidence.push("no observable text payload to verify".into());
                None
            }
        },
        GuiVerificationStrategy::StateChanged => {
            let focus_moved = focus_changed(pre_observation, post_observation);
            match changed {
                Some(true) => {
                    evidence.push("screen state changed after the action".into());
                    Some(true)
                }
                Some(false) if focus_moved => {
                    evidence.push("focused control changed after the action".into());
                    Some(true)
                }
                Some(false) => {
                    evidence.push("no observable state change after the action".into());
                    Some(false)
                }
                None if focus_moved => {
                    evidence.push("focused control changed after the action".into());
                    Some(true)
                }
                None => {
                    evidence.push("screen state change could not be observed".into());
                    None
                }
            }
        }
        GuiVerificationStrategy::ScreenChanged => match changed {
            Some(value) => {
                if value {
                    evidence.push("screen content changed after the action".into());
                } else {
                    evidence.push("screen content did not change after the action".into());
                }
                Some(value)
            }
            None => {
                evidence.push("screen hash unavailable; screen change not observable".into());
                None
            }
        },
        GuiVerificationStrategy::ResultVisible => {
            let dialog = !post_observation.dialogs.is_empty();
            let postcondition_visible = !request.expected_postcondition.trim().is_empty()
                && post_observation.all_controls().iter().any(|control| {
                    request
                        .expected_postcondition
                        .to_ascii_lowercase()
                        .split_whitespace()
                        .filter(|token| token.len() >= 4)
                        .any(|token| text_contains(&control.name, token))
                });
            match changed {
                Some(true) => {
                    evidence.push("screen changed and a result is visible after the action".into());
                    Some(true)
                }
                _ if dialog => {
                    evidence.push("a dialog became visible after the action".into());
                    Some(true)
                }
                _ if postcondition_visible => {
                    evidence.push("expected result content is visible after the action".into());
                    Some(true)
                }
                Some(false) => {
                    evidence.push("screen did not change and no result became visible".into());
                    Some(false)
                }
                None => {
                    evidence.push("result visibility could not be observed".into());
                    None
                }
            }
        }
        GuiVerificationStrategy::DialogVisible => {
            let dialog = !post_observation.dialogs.is_empty();
            if dialog {
                evidence.push("expected dialog is visible after the action".into());
            } else {
                evidence.push("expected dialog was not visible after the action".into());
            }
            Some(dialog)
        }
        GuiVerificationStrategy::ClipboardChanged => {
            // Clipboard contents are never read into the observation pipeline, so
            // verification relies on the backend receipt only. Never echo content.
            evidence.push("clipboard change reported by backend; content not captured".into());
            Some(true)
        }
        GuiVerificationStrategy::FileSaved
        | GuiVerificationStrategy::DownloadStartedOrCompleted => match changed {
            Some(true) => {
                evidence.push("observable state changed consistent with the expected result".into());
                Some(true)
            }
            Some(false) => {
                evidence.push("no observable change for file/download verification".into());
                Some(false)
            }
            None => {
                evidence.push("file/download result is not observable from the GUI state".into());
                None
            }
        },
        GuiVerificationStrategy::TargetResolved => {
            if target_still_present {
                evidence.push("target remains resolved after the action".into());
                Some(true)
            } else {
                evidence.push("target is no longer present after the action".into());
                Some(false)
            }
        }
        GuiVerificationStrategy::VisibleContentSummarized => {
            evidence.push("visible content summary verification is not observable here".into());
            None
        }
        GuiVerificationStrategy::Inconclusive => {
            evidence.push("no deterministic verification strategy applied".into());
            None
        }
    };

    let (status, confidence) = match matched {
        Some(true) => {
            let conf = match strategy {
                GuiVerificationStrategy::ClipboardChanged
                | GuiVerificationStrategy::StateChanged => 0.86,
                _ => 0.9,
            };
            (VERIFICATION_VERIFIED, conf)
        }
        Some(false) => (VERIFICATION_FAILED, 0.2),
        None => (VERIFICATION_INCONCLUSIVE, 0.5),
    };

    // A resolved-but-missing or identity-mismatched target downgrades a control
    // action to failure; we never claim success when the bound target moved.
    let control_action = matches!(
        action_kind,
        GuiActionKind::FocusField
            | GuiActionKind::ClickControl
            | GuiActionKind::TypeText
            | GuiActionKind::FillField
            | GuiActionKind::Paste
    );
    let (status, confidence) = if status == VERIFICATION_VERIFIED
        && control_action
        && request.target_control_id.is_some()
        && (!target_still_present || !target_identity_matches)
    {
        evidence.push("bound target identity is no longer stable after the action".into());
        (VERIFICATION_FAILED, 0.2)
    } else {
        (status, confidence)
    };

    let safe_error_summary = match status {
        VERIFICATION_FAILED => Some(format!(
            "Expected post-action state was not verified for {}.",
            safe_token(&request.action_type, 40)
        )),
        VERIFICATION_INCONCLUSIVE => Some(
            "Post-action state could not be confirmed from available evidence.".into(),
        ),
        _ => None,
    };
    let recovery_hint = match status {
        VERIFICATION_FAILED | VERIFICATION_INCONCLUSIVE => Some(
            "Re-observe and confirm the expected state before retrying; do not blind-retry."
                .into(),
        ),
        _ => None,
    };
    let can_retry = matches!(status, VERIFICATION_FAILED | VERIFICATION_INCONCLUSIVE)
        && idempotent_retry(&action_kind);

    GuiPostActionVerificationResult {
        verification_id: request.verification_id.clone(),
        execution_id: request.execution_id.clone(),
        proposal_id: request.proposal_id.clone(),
        status: status.into(),
        verification_strategy: strategy.as_str().into(),
        evidence: evidence
            .into_iter()
            .map(|value| safe_token(&value, 200))
            .collect(),
        pre_state_summary,
        post_state_summary,
        matched_expected_state: status == VERIFICATION_VERIFIED,
        target_still_present,
        target_identity_matches,
        confidence,
        safe_error_summary,
        recovery_hint,
        can_retry,
        prompt_hash: request.prompt_hash.clone(),
    }
}
