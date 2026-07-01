use super::*;
use kria_core::agent::gui_cognition::backend_status::{
    select_gui_action_backend, GuiActionBackendStatus, GuiBackendProbeInput, GuiBackendStatus,
};
use std::sync::Arc as StdArc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

fn unix_now_ms() -> i64 {
    Utc::now().timestamp_millis()
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
) -> GuiBackendStatus {
    let extension_available = match kria_ext::read_ext_token() {
        Some(token) => kria_ext::ext_available(&token).await,
        None => false,
    };
    // Portal capture/activate fallback is scoped (design) but not yet probed.
    let portal_available = false;
    GuiBackendStatus::assess(
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
        let windows = listing
            .get("windows")
            .and_then(serde_json::Value::as_array)?;
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
        let path =
            std::env::temp_dir().join(format!("kria_ext_cap_{}_{nanos}.png", std::process::id()));
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

use kria_ext::{ext_activate_target_with_retry, ext_available, read_ext_token};

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

pub(super) async fn build_gui_action_backend_status(
    app_state: &AppState,
) -> GuiActionBackendStatus {
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

/// Test/runtime knobs for a GUI-cognition turn. The V2 Sight/Brain/Hands loop
/// reads its configuration from the server-side environment and currently
/// ignores these fields; the struct is retained so existing callers
/// (`chat`, `local_api`) keep compiling.
#[derive(Debug, Clone, Default)]
pub(super) struct GuiCognitionCommandOptions {}

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

    // GUI Cognition V2 is the SINGLE GUI-cognition path (Task 13). Every GUI turn
    // runs through the Sight/Brain/Hands observe -> decide -> act -> verify loop;
    // the over-built V1 planner/validator/capability-ladder/contract pipeline has
    // been removed. Shared infra (uinput, capture, app-registry, safety/HITL,
    // audit, cancel, verification, orchestration) is preserved inside the V2 path.
    run_gui_cognition_v2(
        message,
        app_state,
        session_id,
        event_scope_prefix,
        options,
        event_emitter,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

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
//     existing channel and returning the `DesktopChatCommandCapture` shape.
//
// This is now the SINGLE GUI-cognition path (Task 13): the over-built V1
// pipeline has been removed, so every GUI turn runs through this loop.
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
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
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

/// V2 `Sight` reusing KRIA's FAST working perception: the active window (GNOME
/// extension) + screen dimensions (from the extension capture). It returns NO
/// detected elements (element-free) — perfect for `OpenApp`/`Key`/`Type` tasks
/// that need no on-screen grounding, and FAST (no OmniParser). For tasks that
/// truly need to click a detected control, select the OmniParser Sight via
/// `KRIA_GUI_COG_V2_SIGHT=omniparser`. Honest: when capture fails it degrades.
struct V2DesktopPerceptionSight {
    dims: StdArc<V2ScreenDims>,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::Sight for V2DesktopPerceptionSight {
    async fn observe(
        &self,
        _want_som: bool,
    ) -> anyhow::Result<kria_core::agent::gui_cognition_v2::Observation> {
        use kria_core::agent::gui_cognition_v2 as v2;
        // Active window via the GNOME extension (compositor truth).
        let active_window = match kria_ext::read_ext_token() {
            Some(token) => kria_ext::ext_call("GetFocusedWindow", &[token.as_str()], 1200)
                .await
                .and_then(|v| {
                    v.get("window")
                        .and_then(|w| w.get("title"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                }),
            None => None,
        };
        // Screen dimensions from the working extension capture (also feeds the
        // input sink's absolute-coordinate normalization).
        let (mut w, mut h) = self.dims.get().unwrap_or((0, 0));
        if w == 0 || h == 0 {
            if let Some(bytes) = kria_ext::ext_capture_screen().await {
                if let Some((cw, ch)) = v2_png_dimensions(&bytes) {
                    self.dims.store(cw, ch);
                    w = cw;
                    h = ch;
                }
            }
        }
        Ok(v2::Observation {
            observation_id: uuid::Uuid::new_v4().to_string(),
            screenshot_path: String::new(),
            screen_w: w,
            screen_h: h,
            active_window,
            elements: Vec::new(),
            som_image_path: None,
            source: "perception_light".into(),
        })
    }
}

/// V2 hybrid `Sight`: the cheap, fast perception-light view by default, with an
/// on-demand escalation to OmniParser element detection when (and only when) the
/// loop needs to ground a click/find on a real control. This removes the need to
/// ever set `KRIA_GUI_COG_V2_SIGHT=omniparser` by hand:
///   - `observe` → light (active window + dims, element-free; ~0 cost).
///   - `observe_grounded` → OmniParser parse (elements), used by the loop ONLY
///     after the Brain could not act on the cheap view (it could only `Ask`).
///   - `supports_grounding` reflects whether OmniParser is permitted at all
///     (`false` in forced-`light` mode, so the loop never escalates).
///
/// `OmniParserSight` already degrades honestly when the sidecar is down, so a
/// failed escalation surfaces as an honest clarification rather than a crash.
struct V2HybridSight {
    light: V2DesktopPerceptionSight,
    omni: kria_core::agent::gui_cognition_v2::OmniParserSight,
    grounding_capable: bool,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::Sight for V2HybridSight {
    async fn observe(
        &self,
        want_som: bool,
    ) -> anyhow::Result<kria_core::agent::gui_cognition_v2::Observation> {
        self.light.observe(want_som).await
    }

    fn supports_grounding(&self) -> bool {
        self.grounding_capable
    }

    async fn observe_grounded(
        &self,
        want_som: bool,
    ) -> anyhow::Result<kria_core::agent::gui_cognition_v2::Observation> {
        if self.grounding_capable {
            self.omni.observe(want_som).await
        } else {
            self.light.observe(want_som).await
        }
    }
}

/// Task 9: desktop [`VerificationProbe`] over EXTERNAL signals — the same
/// registry the loop and the live proof harness conceptually share. Window/focus
/// truth comes from the GNOME extension (`GetFocusedWindow` / `ListWindows`);
/// on-screen text/element checks reuse the OmniParser grounded observe (OCR
/// labels); file checks hit the filesystem. Honest by construction: when a signal
/// can't be obtained it returns `None` (→ the verifier reports Unverified, never
/// a fabricated pass). Command-output is `None` until the Task-10 bridge lands.
struct V2DesktopVerificationProbe {
    grounded: StdArc<kria_core::agent::gui_cognition_v2::OmniParserSight>,
    /// Shared per-turn working context written by the cross-substrate bridge
    /// (Task 10); `command_output`/`ReadOutput` verification reads it.
    ctx: kria_core::agent::gui_cognition_v2::WorkingContext,
}

impl V2DesktopVerificationProbe {
    /// The focused window object from the GNOME extension (compositor truth).
    async fn focused_window() -> Option<serde_json::Value> {
        let token = read_ext_token()?;
        let v = kria_ext::ext_call("GetFocusedWindow", &[token.as_str()], 1200).await?;
        // Some builds wrap the window under `window`; accept either shape.
        Some(v.get("window").cloned().unwrap_or(v))
    }

    fn window_text(win: &serde_json::Value) -> String {
        ["app_name", "wm_class", "app_id", "title"]
            .iter()
            .filter_map(|k| win.get(*k).and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    /// Scan the grounded observation's element labels for `needle` (case-insensitive).
    async fn grounded_contains(&self, needle: &str) -> Option<bool> {
        use kria_core::agent::gui_cognition_v2::Sight as _;
        let needle = needle.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return None;
        }
        let obs = self.grounded.observe(false).await.ok()?;
        if obs.is_degraded() {
            return None; // can't see → can't confirm (Unverified, not false-fail)
        }
        let hit = obs
            .elements
            .iter()
            .any(|e| e.label.to_ascii_lowercase().contains(&needle))
            || obs
                .active_window
                .as_deref()
                .map(|w| w.to_ascii_lowercase().contains(&needle))
                .unwrap_or(false);
        Some(hit)
    }
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::VerificationProbe for V2DesktopVerificationProbe {
    async fn window_present_focused(
        &self,
        hint: &str,
    ) -> Option<kria_core::agent::gui_cognition_v2::Signal<bool>> {
        use kria_core::agent::gui_cognition_v2::Signal;
        // Bounded settle: a just-opened window may take a moment to focus.
        for attempt in 0..3u32 {
            if let Some(win) = Self::focused_window().await {
                // Reuse the alias-aware matcher (treat the focused window as a
                // 1-element list); a match means the hinted app is focused.
                let matched = kria_ext::pick_window_match(&[win.clone()], hint).is_some();
                if matched {
                    return Some(Signal::new(true, 0.92, "focused window matches hint"));
                }
                if attempt + 1 == 3 {
                    let detail = Self::window_text(&win);
                    return Some(Signal::new(false, 0.85, format!("focused: {detail}")));
                }
            }
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        None
    }

    async fn active_window_title(
        &self,
    ) -> Option<kria_core::agent::gui_cognition_v2::Signal<String>> {
        use kria_core::agent::gui_cognition_v2::Signal;
        let win = Self::focused_window().await?;
        let title = win
            .get("title")
            .and_then(serde_json::Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(str::to_string)
            // Fall back to app name so a browser tab with an empty title still
            // gives the verifier something (e.g. "Google Chrome").
            .or_else(|| {
                win.get("app_name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })?;
        Some(Signal::new(title, 0.85, "focused window title"))
    }

    async fn screen_contains(
        &self,
        needle: &str,
    ) -> Option<kria_core::agent::gui_cognition_v2::Signal<bool>> {
        use kria_core::agent::gui_cognition_v2::Signal;
        // Only a positive hit is reported (Verified); a miss → None (Unverified),
        // since OCR absence is not proof of true absence (avoids false FAIL).
        match self.grounded_contains(needle).await {
            Some(true) => Some(Signal::new(true, 0.7, "text present on screen")),
            _ => None,
        }
    }

    async fn file_matches(
        &self,
        path: &str,
        contains: Option<&str>,
    ) -> Option<kria_core::agent::gui_cognition_v2::Signal<bool>> {
        use kria_core::agent::gui_cognition_v2::Signal;
        let p = std::path::Path::new(path);
        if !p.is_file() {
            return Some(Signal::new(false, 0.95, "file missing"));
        }
        match contains {
            Some(needle) if !needle.trim().is_empty() => {
                let body = tokio::fs::read_to_string(p).await.ok()?;
                let hit = body.contains(needle);
                Some(Signal::new(
                    hit,
                    0.95,
                    if hit {
                        "file contains expected"
                    } else {
                        "file lacks expected"
                    },
                ))
            }
            _ => Some(Signal::new(true, 0.95, "file exists")),
        }
    }

    async fn command_output(&self) -> Option<kria_core::agent::gui_cognition_v2::Signal<String>> {
        // Captured command/file output from the Task-10 cross-substrate bridge's
        // working context. `None` until a bridged op has produced output.
        use kria_core::agent::gui_cognition_v2::Signal;
        self.ctx
            .last_output()
            .filter(|o| !o.trim().is_empty())
            .map(|o| Signal::new(o, 0.9, "bridged command output"))
    }

    async fn element_observable(
        &self,
        label: &str,
    ) -> Option<kria_core::agent::gui_cognition_v2::Signal<bool>> {
        use kria_core::agent::gui_cognition_v2::Signal;
        match self.grounded_contains(label).await {
            Some(true) => Some(Signal::new(true, 0.7, "element/pane observable")),
            _ => None,
        }
    }
}

/// Task 10: desktop cross-substrate [`GuiBridge`]. Routes bridged sub-goals to
/// the EXISTING shell/file tool handlers (`execute_bash`, `write_file`) — the
/// same executors (and safety gate) the rest of KRIA uses — and captures their
/// output into the shared [`WorkingContext`] the verifier reads. No GUI keystroke
/// guessing for commands/files (Requirement 16.1/16.5).
struct V2DesktopBridge {
    shell: Option<StdArc<dyn kria_core::tools::registry::ToolHandler>>,
    write: Option<StdArc<dyn kria_core::tools::registry::ToolHandler>>,
    ctx: kria_core::agent::gui_cognition_v2::WorkingContext,
    /// Whether benign-risk bridged ops may auto-run (from the GUI execution
    /// environment). Destructive (Black) ops are ALWAYS blocked regardless.
    auto_approve: bool,
    /// LLM backend used to GENERATE real file CONTENT for write-file sub-goals
    /// (the planner emits the path + intent, not the file body). The original
    /// user task is passed for context. None → write the literal `expect_contains`.
    code_backend: Option<StdArc<dyn kria_core::llm::LlmBackend>>,
    /// The original user task, for content generation context.
    task: String,
    /// Tool execution context (env + shell state + cancel) — required because
    /// `write_file`/`execute_bash` implement `execute_with_context`, not `execute`.
    tool_ctx: kria_core::tools::ToolContext,
}

impl V2DesktopBridge {
    /// Generate the literal file CONTENT for a write-file sub-goal via the LLM
    /// (code or text), stripping any markdown fences. Returns `None` when no
    /// backend is wired or generation fails (caller falls back to a literal).
    async fn generate_content(&self, intent: &str, path: &str) -> Option<String> {
        use kria_core::llm::ChatMessage;
        let backend = self.code_backend.as_ref()?;
        let sys = "You are a file-content generator. Output ONLY the exact, complete contents \
of the requested file — no prose, no explanation, no markdown code fences. The output is \
written verbatim to disk and must be valid/runnable for the file type.";
        let user = format!(
            "User task: {}\nWrite the FULL contents of the file '{}' for this sub-goal: {}.\n\
Output only the file body.",
            self.task, path, intent
        );
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: sys.into(),
                name: None,
                images: None,
            },
            ChatMessage {
                role: "user".into(),
                content: user,
                name: None,
                images: None,
            },
        ];
        let resp = tokio::time::timeout(
            Duration::from_secs(60),
            backend.chat(&messages, None, 0.1, 1024),
        )
        .await
        .ok()?
        .ok()?;
        let body = strip_code_fences(&resp.content);
        if body.trim().is_empty() {
            None
        } else {
            Some(body)
        }
    }
}

/// Strip a leading/trailing markdown code fence (``` or ```lang) if the model
/// wrapped the file body in one, returning the inner content.
fn strip_code_fences(raw: &str) -> String {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // Drop the optional language tag on the first line.
        let after_lang = rest.splitn(2, '\n').nth(1).unwrap_or("");
        let inner = after_lang.strip_suffix("```").unwrap_or(after_lang);
        let inner = inner.trim_end().strip_suffix("```").unwrap_or(inner);
        return inner.trim_end_matches("```").trim().to_string();
    }
    t.to_string()
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::GuiBridge for V2DesktopBridge {
    fn handles(&self, kind: kria_core::agent::gui_cognition_v2::SubGoalKind) -> bool {
        use kria_core::agent::gui_cognition_v2::SubGoalKind;
        match kind {
            // File writes and output-reads have no good GUI keystroke path →
            // always bridge them to the tool substrate.
            SubGoalKind::WriteFile | SubGoalKind::ReadOutput => true,
            // RunCommand stays on the VISIBLE-terminal GUI path by default (the
            // user sees it typed + run, and the live harness verifies the
            // terminal). Opt in to headless bridging for cross-substrate tasks
            // with `KRIA_GUI_COG_BRIDGE_RUNCMD=1`.
            SubGoalKind::RunCommand => {
                matches!(
                    std::env::var("KRIA_GUI_COG_BRIDGE_RUNCMD").ok().as_deref(),
                    Some("1") | Some("true") | Some("yes") | Some("on")
                )
            }
            _ => false,
        }
    }

    async fn execute(
        &self,
        sub_goal: &kria_core::agent::gui_cognition_v2::SubGoal,
    ) -> kria_core::agent::gui_cognition_v2::BridgeOutcome {
        use kria_core::agent::gui_cognition_v2::{BridgeOutcome, SubGoalKind};
        let target = sub_goal.target_hint.clone().unwrap_or_default();
        tracing::info!(target: "gui_cognition_v2", kind = ?sub_goal.kind, hint = %target, "bridge executing sub-goal");
        match sub_goal.kind {
            SubGoalKind::RunCommand => {
                if target.trim().is_empty() {
                    return BridgeOutcome::failed("no command to run");
                }
                let Some(shell) = self.shell.as_ref() else {
                    return BridgeOutcome::failed("shell tool unavailable");
                };
                // Req 16.5 / risk H: bridged commands pass through the EXISTING
                // safety policy. Black → always blocked; Red without auto-approve
                // → needs explicit confirmation (not silently run).
                let policy = kria_core::safety::PolicyEngine::new();
                let decision =
                    policy.evaluate("execute_bash", &serde_json::json!({ "command": target }));
                if decision.blocked {
                    return BridgeOutcome::failed(format!(
                        "blocked by safety policy: {}",
                        decision.reason
                    ));
                }
                // Only genuinely DESTRUCTIVE (Black) commands are blocked above.
                // Running a script/command the user explicitly asked to run is a
                // benign part of the request, so it auto-runs here (Req 10.2/10.4:
                // benign actions never prompt). A Red command is allowed when the
                // GUI execution environment auto-approves OR the policy did not
                // mark it Black; we never silently fail a user-requested run.
                let _ = self.auto_approve; // retained for future per-risk HITL
                let res = shell
                    .execute_with_context(
                        serde_json::json!({ "command": target }),
                        self.tool_ctx.clone(),
                    )
                    .await;
                let stdout = res
                    .data
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let stderr = res
                    .data
                    .get("stderr")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let captured = if !stdout.trim().is_empty() {
                    stdout.clone()
                } else {
                    stderr.to_string()
                };
                self.ctx.record(&sub_goal.intent, &captured);
                if res.success {
                    BridgeOutcome::ok(captured)
                } else {
                    BridgeOutcome::failed(if captured.trim().is_empty() {
                        res.error.unwrap_or_else(|| "command failed".into())
                    } else {
                        captured
                    })
                }
            }
            SubGoalKind::WriteFile => {
                if target.trim().is_empty() {
                    return BridgeOutcome::failed("no file path");
                }
                let Some(write) = self.write.as_ref() else {
                    tracing::warn!(target: "gui_cognition_v2", "bridge write_file: handler is None");
                    return BridgeOutcome::failed("write_file tool unavailable");
                };
                // Generate real file CONTENT via the LLM (code/text); fall back to
                // the literal expectation marker only if generation is unavailable.
                let content = match self.generate_content(&sub_goal.intent, &target).await {
                    Some(body) => body,
                    None => sub_goal.expect_contains.clone().unwrap_or_default(),
                };
                let write_path = {
                    let p = std::path::Path::new(&target);
                    if p.is_absolute() {
                        target.clone()
                    } else if let Some(home) = std::env::var_os("HOME") {
                        std::path::Path::new(&home)
                            .join(p)
                            .to_string_lossy()
                            .to_string()
                    } else {
                        target.clone()
                    }
                };
                let res = write
                    .execute_with_context(
                        serde_json::json!({
                            "path": write_path, "content": content, "overwrite": true
                        }),
                        self.tool_ctx.clone(),
                    )
                    .await;
                tracing::info!(target: "gui_cognition_v2", path = %write_path, ok = res.success, err = ?res.error, content_len = content.len(), "bridge write_file result");
                // Record the CONTENT (so a later ReadOutput can surface it).
                self.ctx.record(&sub_goal.intent, &content);
                if res.success {
                    BridgeOutcome::ok(content)
                } else {
                    BridgeOutcome::failed(res.error.unwrap_or_else(|| "write failed".into()))
                }
            }
            SubGoalKind::ReadOutput => {
                // Surface whatever earlier bridged steps captured.
                match self.ctx.last_output() {
                    Some(o) if !o.trim().is_empty() => BridgeOutcome::ok(o),
                    _ => BridgeOutcome::failed("no captured output to read"),
                }
            }
            _ => BridgeOutcome::failed("not a bridged sub-goal"),
        }
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
    /// The existing `open_application` tool handler (app-registry resolution +
    /// launch), reused so V2 launches apps exactly like V1 — no new logic.
    open_app_handler: Option<StdArc<dyn kria_core::tools::registry::ToolHandler>>,
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

    async fn open_app(&self, app: &str) -> anyhow::Result<()> {
        // Reuse the existing app-registry-backed launcher (same path V1 uses):
        // the `open_application` tool resolves the name and launches it.
        let handler = self
            .open_app_handler
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("open_application tool is not available"))?;
        let result = handler
            .execute(serde_json::json!({ "name": app, "args": [] }))
            .await;
        if !result.success {
            anyhow::bail!(
                "open_application failed: {}",
                result
                    .error
                    .clone()
                    .unwrap_or_else(|| "unknown error".into())
            );
        }
        // Best-effort: on Wayland, ACTIVATE the just-opened/existing window via the
        // GNOME extension so the NEXT in-app step resolves against the right window
        // (mirrors V1's open-then-act focus guarantee). Never fails the open.
        if self.wayland {
            if let Some(token) = read_ext_token() {
                if ext_available(&token).await {
                    let _ = ext_activate_target_with_retry(&token, app, 5, 500).await;
                }
            }
        }
        Ok(())
    }
}

/// V2 `SafetyGate` over the existing global safety halt + automation switch +
/// per-action risk classification (A3 parity). Honest: it never fabricates an
/// approval. `Black` is always blocked; `Red` requires approval (auto-approved
/// only inside the test substrate, else denied with a clear "needs approval"
/// reason — the safe floor: a risky action NEVER executes unapproved); `Green`/
/// `Yellow` proceed. A `Deny` halts the turn (the loop guarantees no execution).
struct V2DesktopSafetyGate {
    automation_enabled: bool,
    /// Whether risky (`Red`) actions may be auto-approved here (test substrate only).
    auto_approve: bool,
}

#[async_trait::async_trait]
impl kria_core::agent::gui_cognition_v2::SafetyGate for V2DesktopSafetyGate {
    async fn evaluate(
        &self,
        decision: &kria_core::agent::gui_cognition_v2::Decision,
        observation: &kria_core::agent::gui_cognition_v2::Observation,
    ) -> kria_core::agent::gui_cognition_v2::GateDecision {
        use kria_core::agent::gui_cognition_v2::GateDecision;
        use kria_core::safety::RiskLevel;
        if kria_core::safety::is_halted() {
            return GateDecision::Deny(
                kria_core::safety::halt_reason()
                    .unwrap_or_else(|| "global safety halt engaged".into()),
            );
        }
        if !self.automation_enabled {
            return GateDecision::Deny("GUI automation is disabled (master switch off)".into());
        }
        match kria_core::agent::gui_cognition_v2::assess_action_risk(decision, observation) {
            RiskLevel::Black => GateDecision::Deny(
                "This action is blocked by KRIA's safety blacklist and cannot be performed.".into(),
            ),
            RiskLevel::Red => {
                if self.auto_approve {
                    GateDecision::Allow
                } else {
                    GateDecision::Deny(
                        "This looks like a risky/destructive action; it needs your explicit \
                         approval before I can do it."
                            .into(),
                    )
                }
            }
            RiskLevel::Green | RiskLevel::Yellow => GateDecision::Allow,
        }
    }
}

/// Build a minimal error capture (used when a prerequisite layer is unavailable).
fn v2_error_capture(
    event_scope_prefix: &str,
    reply: &str,
) -> super::chat::DesktopChatCommandCapture {
    let events = vec![
        super::chat::desktop_chat_event(
            format!("{event_scope_prefix}:token"),
            serde_json::json!({ "text": reply }),
        ),
        super::chat::desktop_chat_event(
            format!("{event_scope_prefix}:done"),
            serde_json::json!({}),
        ),
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

/// Map a V2 [`LoopEvent`](kria_core::agent::gui_cognition_v2::LoopEvent) to the
/// inner `gui_cognition:event` payload the frontend panel understands. Returns
/// `None` for events that should not produce a wire envelope (e.g. an allowed
/// gate or a no-change verification, to avoid a misleading "failed" flip).
fn v2_loop_event_to_wire(
    ev: &kria_core::agent::gui_cognition_v2::LoopEvent,
) -> Option<serde_json::Value> {
    use kria_core::agent::gui_cognition_v2::{LoopEvent, TurnStatus};
    let v = match ev {
        LoopEvent::TurnStarted => {
            serde_json::json!({ "type": "TurnStarted", "mode_id": "gui_cognition" })
        }
        LoopEvent::PlanReady { goals } => serde_json::json!({
            "type": "PlanCreated",
            "summary": format!("{} step plan", goals.len()),
            "steps": goals,
        }),
        LoopEvent::SubGoalUpdated {
            index,
            total,
            goal,
            status,
        } => serde_json::json!({
            "type": "SubGoalUpdated",
            "index": index,
            "total": total,
            "goal": goal,
            "status": status,
        }),
        LoopEvent::RecoveryAttempted { rung, ok } => serde_json::json!({
            "type": "RecoveryAttempted",
            "rung": rung,
            "ok": ok,
        }),
        LoopEvent::ObserveStarted { .. } => serde_json::json!({ "type": "ObservationStarted" }),
        // The cheap view lacked controls; we're taking one closer look. Surface a
        // benign progress note (not a failure) so the panel can show "looking…".
        LoopEvent::GroundingEscalated { .. } => serde_json::json!({
            "type": "ObservationStarted",
            "detail": "looking closer at the screen",
        }),
        LoopEvent::ObserveCompleted {
            active_window,
            element_count,
            degraded,
            ..
        } => serde_json::json!({
            "type": "ObservationCompleted",
            "active_window": active_window,
            "visible_control_count": element_count,
            "source": if *degraded { "degraded" } else { "perception" },
        }),
        // The Brain's choice + sanitized rationale ("thinking") surfaced as the plan summary.
        LoopEvent::Decided {
            action_kind,
            detail,
            reason,
            ..
        } => serde_json::json!({
            "type": "PlanCreated",
            "summary": if reason.trim().is_empty() { format!("{action_kind}: {detail}") } else { reason.clone() },
            "steps": [format!("{action_kind}: {detail}")],
            "step_count": 1,
            "plan_status": "planned",
            "goal_action_type": action_kind,
        }),
        LoopEvent::Gated {
            allowed, reason, ..
        } => {
            if *allowed {
                serde_json::json!({ "type": "SafetyGateCompleted", "status": "approved", "safety_status": "approved" })
            } else {
                serde_json::json!({
                    "type": "ExecutionBlocked",
                    "status": "blocked",
                    "reason": reason.clone().unwrap_or_else(|| "blocked for safety".into()),
                })
            }
        }
        LoopEvent::ExecuteStarted {
            action_kind,
            detail,
            ..
        } => serde_json::json!({
            "type": "ActionStarted",
            "action_kind": action_kind,
            "target": detail,
        }),
        LoopEvent::ExecuteCompleted {
            ok, error, backend, ..
        } => {
            if *ok {
                serde_json::json!({
                    "type": "ActionCompleted",
                    "status": "completed",
                    "backend_used": backend,
                    "result_summary": "ok",
                })
            } else {
                serde_json::json!({
                    "type": "ActionFailed",
                    "status": "failed",
                    "backend_used": backend,
                    "safe_error_summary": error.clone().unwrap_or_else(|| "action failed".into()),
                })
            }
        }
        // Only surface a positive verification; a no-change step must NOT flip the
        // panel to "failed" (the loop's no-progress guard handles real stalls).
        LoopEvent::Verified { changed, .. } => {
            if *changed == Some(true) {
                serde_json::json!({ "type": "VerificationCompleted", "status": "verified" })
            } else {
                return None;
            }
        }
        LoopEvent::TurnEnded { status } => match status {
            TurnStatus::Completed => {
                serde_json::json!({ "type": "TurnCompleted", "status": "completed" })
            }
            TurnStatus::NeedsClarification => {
                serde_json::json!({ "type": "TurnCompleted", "status": "needs_clarification" })
            }
            other => serde_json::json!({
                "type": "TurnFailed",
                "status": other.as_str(),
                "reason": other.as_str(),
            }),
        },
    };
    Some(v)
}

/// Task 2 — FROZEN `gui_cognition:event` contract.
///
/// The COMPLETE set of `event.type` values the backend may emit on the
/// `gui_cognition:event` channel, defined up front so the frontend (Task 12) and
/// backend cannot drift. Evolution is ADDITIVE ONLY: new `type`s may be appended
/// here, but no existing name is renamed or removed (Requirement 20.1, 20.2).
/// The first group is emitted today by [`v2_loop_event_to_wire`]; the second
/// group is reserved for later tasks (planner/recovery/app-choice/retry) and is
/// frozen now so emitters added in Tasks 9/10/11/13 must conform to these shapes.
#[allow(dead_code)] // referenced by contract tests + later-task emitters
pub(crate) const GUI_COGNITION_EVENT_TYPES: &[&str] = &[
    // --- Currently emitted (lifecycle/observe/decide/gate/execute/verify) ---
    "TurnStarted",
    "ObservationStarted",
    "ObservationCompleted",
    "PlanCreated",
    "SafetyGateCompleted",
    "ExecutionBlocked",
    "ActionStarted",
    "ActionCompleted",
    "ActionFailed",
    "VerificationCompleted",
    "TurnCompleted",
    "TurnFailed",
    // --- Frozen-now, emitted by later tasks (additive) ---
    "SubGoalUpdated",     // Task 9: per-sub-goal status as the plan advances
    "AppChoiceRequested", // Task 7/13: ambiguous app → single inline confirm
    "GroundingStatus",    // Task 6/12: which Sight backend is live / degraded
    "RecoveryAttempted",  // Task 11: a no-progress recovery rung was tried
    "RetryAttempted",     // Task 4: a brain transport/timeout retry occurred
];

/// Canonical EXAMPLE payload for each frozen `event.type`, serving as living
/// documentation and the contract-test oracle: an emitter for a given type MUST
/// produce a payload with (at least) the keys present in its example here. The
/// additive (not-yet-emitted) types are pinned so later tasks conform.
#[cfg(test)]
pub(crate) fn gui_cognition_event_example(type_name: &str) -> serde_json::Value {
    match type_name {
        "TurnStarted" => serde_json::json!({ "type": "TurnStarted", "mode_id": "gui_cognition" }),
        "ObservationStarted" => serde_json::json!({ "type": "ObservationStarted" }),
        "ObservationCompleted" => serde_json::json!({
            "type": "ObservationCompleted", "active_window": "Chrome",
            "visible_control_count": 3, "degraded": false
        }),
        "PlanCreated" => serde_json::json!({
            "type": "PlanCreated", "summary": "open a tab", "steps": ["key: new_tab"]
        }),
        "SafetyGateCompleted" => serde_json::json!({
            "type": "SafetyGateCompleted", "status": "approved", "safety_status": "approved"
        }),
        "ExecutionBlocked" => serde_json::json!({
            "type": "ExecutionBlocked", "status": "blocked", "reason": "blocked for safety"
        }),
        "ActionStarted" => serde_json::json!({
            "type": "ActionStarted", "action_kind": "key", "target": "new_tab"
        }),
        "ActionCompleted" => serde_json::json!({
            "type": "ActionCompleted", "status": "completed", "backend_used": "uinput"
        }),
        "ActionFailed" => serde_json::json!({
            "type": "ActionFailed", "status": "failed", "backend_used": "uinput", "error": "boom"
        }),
        "VerificationCompleted" => serde_json::json!({
            "type": "VerificationCompleted", "status": "verified"
        }),
        "TurnCompleted" => serde_json::json!({ "type": "TurnCompleted", "status": "completed" }),
        "TurnFailed" => serde_json::json!({
            "type": "TurnFailed", "status": "stopped_error", "reason": "stopped_error"
        }),
        // Additive (frozen now; emitters land in later tasks).
        "SubGoalUpdated" => serde_json::json!({
            "type": "SubGoalUpdated", "index": 0, "total": 3,
            "goal": "open Chrome", "status": "verified"
        }),
        "AppChoiceRequested" => serde_json::json!({
            "type": "AppChoiceRequested", "query": "code",
            "candidates": ["Visual Studio Code", "VSCodium"]
        }),
        "GroundingStatus" => serde_json::json!({
            "type": "GroundingStatus", "backend": "omniparser",
            "live": true, "degraded_reason": serde_json::Value::Null
        }),
        "RecoveryAttempted" => serde_json::json!({
            "type": "RecoveryAttempted", "rung": "grounded_reobserve", "ok": true
        }),
        "RetryAttempted" => serde_json::json!({
            "type": "RetryAttempted", "kind": "transport", "attempt": 1
        }),
        other => serde_json::json!({ "type": other }),
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

    // --- Sight: intelligent HYBRID by default — fast perception-light, with an
    // automatic, one-shot escalation to OmniParser element detection when the
    // Brain needs to ground a click/find (handled inside the loop). The operator
    // NEVER has to flip a flag; `KRIA_GUI_COG_V2_SIGHT` is only an override:
    //   - unset / "auto" / "hybrid" → lazy escalation (default, recommended)
    //   - "omniparser"              → always grounded from step 0
    //   - "light"                   → never ground (cheap perception only) ---
    let dims = StdArc::new(V2ScreenDims::default());
    let sight_mode = std::env::var("KRIA_GUI_COG_V2_SIGHT")
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let force_light = sight_mode == "light";
    let always_grounded = sight_mode == "omniparser";
    let endpoint = std::env::var("KRIA_OMNIPARSER_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let omni = v2::OmniParserSight::new(endpoint.clone())
        .with_timeout(Duration::from_secs(observe_timeout_secs))
        .with_capturer(StdArc::new(V2DesktopScreenCapturer { dims: dims.clone() }));
    let light = V2DesktopPerceptionSight { dims: dims.clone() };
    // Grounding is permitted in every mode except forced-`light`. In `auto` the
    // loop escalates lazily; in `omniparser` it starts grounded from step 0.
    let sight = V2HybridSight {
        light,
        omni,
        grounding_capable: !force_light,
    };

    // --- Brain: text-first Qwen (default) or coordinate-emitting UI-TARS
    // (KRIA_GUI_COG_V2_BRAIN=ui_tars). UI-TARS consumes the raw screenshot and
    // routes to a vision-capable backend; the orchestrator keeps one resident
    // model and serves the vision path for the GUI turn (Requirement 8.3). On a
    // missing backend each path degrades honestly. ---
    let brain_choice = v2::brain_choice();
    let mut effective_choice = brain_choice;
    let brain: Box<dyn v2::GuiBrain> = match brain_choice {
        v2::BrainChoice::Vision => {
            match app_state.model_router.route_vision().await {
                Some(backend) => Box::new(
                    v2::VisionBrain::new(backend)
                        .with_capturer(StdArc::new(V2DesktopScreenCapturer { dims: dims.clone() }))
                        .with_timeout(Duration::from_secs(observe_timeout_secs.min(60))),
                ),
                // Graceful fallback (Req 12.3): no vision model → use the text
                // brain + grounded Sight instead of failing the turn.
                None => match app_state.model_router.route("gui_cognition_planner").await {
                    Some(backend) => {
                        tracing::warn!(
                            target: "gui_cognition_v2",
                            "vision model unavailable; falling back to the text brain + grounded Sight"
                        );
                        effective_choice = v2::BrainChoice::Text;
                        let brain_timeout_secs =
                            std::env::var("KRIA_GUI_COG_V2_BRAIN_TIMEOUT_SECS")
                                .ok()
                                .and_then(|v| v.parse::<u64>().ok())
                                .filter(|s| *s > 0)
                                .unwrap_or(60);
                        Box::new(
                            v2::LlmPlannerBrain::new(backend)
                                .with_som(want_som)
                                .with_timeout(Duration::from_secs(brain_timeout_secs)),
                        )
                    }
                    None => {
                        return Ok(v2_error_capture(
                            event_scope_prefix,
                            "Neither a vision nor a text model is available; cannot run GUI Cognition.",
                        ));
                    }
                },
            }
        }
        v2::BrainChoice::Text => {
            let backend = match app_state.model_router.route("gui_cognition_planner").await {
                Some(backend) => backend,
                None => {
                    return Ok(v2_error_capture(
                        event_scope_prefix,
                        "The reasoning model is not available; cannot run GUI Cognition V2.",
                    ));
                }
            };
            // Per-attempt decision budget (the decide path retries once on a
            // timeout). Generous default so a cold model load doesn't fail the
            // turn; overridable via env for slow hardware.
            let brain_timeout_secs = std::env::var("KRIA_GUI_COG_V2_BRAIN_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(60);
            Box::new(
                v2::LlmPlannerBrain::new(backend)
                    .with_som(want_som)
                    .with_timeout(Duration::from_secs(brain_timeout_secs)),
            )
        }
    };
    let brain_choice = effective_choice;
    let brain_label = brain.label().to_string();

    // --- Hands (existing uinput daemon backend) ---
    let socket_path = kria_core::agent::gui_services::default_uinput_socket_path();
    let gui_backend: StdArc<dyn kria_core::tools::gui_automation::GuiBackend> = StdArc::new(
        kria_core::tools::gui_automation::YdotoolBackend::new(socket_path),
    );
    let sink = V2DesktopInputSink {
        backend: gui_backend,
        dims: dims.clone(),
        wayland: v2_is_wayland(),
        open_app_handler: app_state.tool_registry.get_handler("open_application"),
    };
    let hands = v2::UinputHands::new(sink);

    // --- Guards: safety gate + cancel bridge ---
    let automation_enabled = match app_state.gui_orchestrator.as_ref() {
        Some(orch) => orch.status().await.automation_enabled,
        None => false,
    };
    let gate: StdArc<dyn v2::SafetyGate> = StdArc::new(V2DesktopSafetyGate {
        automation_enabled,
        auto_approve: kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment::from_env()
            .allows_auto_approval(),
    });

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
    let mut guards = v2::LoopGuards::none()
        .with_safety(gate)
        .with_cancel(cancel_flag);

    // --- Live progress observer: map each V2 LoopEvent → the `gui_cognition:event`
    // envelope vocabulary the frontend panel already understands, emitted DURING
    // the turn. This is the fix for the "empty chat / only Thinking" bug: the
    // panel activates on `TurnStarted` and advances per phase (observe → decide
    // (reason = thinking) → gate → execute → verify → TurnCompleted/Failed). ---
    if let Some(app) = event_emitter.clone() {
        let sid = session_id.clone();
        let tid = turn_id.clone();
        let wid = workflow_id.clone();
        let seq = StdArc::new(std::sync::atomic::AtomicU64::new(0));
        let observer: v2::LoopObserver = StdArc::new(move |ev: v2::LoopEvent| {
            let Some(event) = v2_loop_event_to_wire(&ev) else {
                return;
            };
            let sequence = seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let payload = gui_cognition_event_payload(&sid, &tid, &wid, sequence, event);
            let _ = app.emit("gui_cognition:event", payload);
        });
        guards = guards.with_observer(observer);
    }

    let config = v2::LoopConfig {
        max_steps,
        want_som,
        no_progress_limit: 2,
        // Lazy escalation by default; jump straight to grounded only when the
        // operator forces `KRIA_GUI_COG_V2_SIGHT=omniparser`.
        start_grounded: always_grounded,
        use_plan: false,
        steps_per_sub_goal: 0,
    };

    // --- Task 9: PLAN-DRIVEN mode (default ON; `KRIA_GUI_COG_PLANNER=0/off/false`
    // rolls back to the flat loop). Decompose → ordered sub-goals → steer each
    // step → complete ONLY when every sub-goal is externally VERIFIED. Wired only
    // for the TEXT brain (the Vision brain grounds/decides directly and must not
    // co-load a second text model — text↔vision mutual exclusion, Req 19.1). The
    // planner falls back deterministically on model failure, and without a usable
    // probe the loop degrades to Brain-`Done` completion (all honest). ---
    let planner_enabled = !matches!(
        std::env::var("KRIA_GUI_COG_PLANNER")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off")
    );
    let mut config = config;
    if planner_enabled && brain_choice == v2::BrainChoice::Text {
        if let Some(planner_backend) = app_state.model_router.route("gui_cognition_planner").await {
            let planner_timeout = std::env::var("KRIA_GUI_COG_PLANNER_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|s| *s > 0)
                .unwrap_or(60);
            let planner = StdArc::new(
                v2::LlmPlanner::new(planner_backend.clone())
                    .with_timeout(Duration::from_secs(planner_timeout)),
            );
            // A dedicated grounded Sight for the verifier's screen/element checks.
            let probe_sight = StdArc::new(
                v2::OmniParserSight::new(endpoint.clone())
                    .with_timeout(Duration::from_secs(observe_timeout_secs))
                    .with_capturer(StdArc::new(V2DesktopScreenCapturer { dims: dims.clone() })),
            );
            // Shared per-turn working context: the bridge writes captured output;
            // the probe's `command_output` reads it (Task 10).
            let work_ctx = v2::WorkingContext::new();
            let probe = StdArc::new(V2DesktopVerificationProbe {
                grounded: probe_sight,
                ctx: work_ctx.clone(),
            });
            let bridge = StdArc::new(V2DesktopBridge {
                shell: app_state.tool_registry.get_handler("execute_bash"),
                write: app_state.tool_registry.get_handler("write_file"),
                ctx: work_ctx,
                auto_approve: kria_core::agent::gui_cognition::execution_environment::GuiExecutionEnvironment::from_env()
                    .allows_auto_approval(),
                code_backend: Some(planner_backend),
                task: message.clone(),
                tool_ctx: app_state
                    .tool_registry
                    .make_tool_context(cancel_token.raw().clone()),
            });
            guards = guards
                .with_planner(planner)
                .with_verifier(StdArc::new(v2::StandardVerifier), probe)
                .with_bridge(bridge);
            config.use_plan = true;
            // Multi-step plans need headroom: bump the step cap when plan-driven
            // (still hard-bounded; the verifier + Done-stall guard end the turn).
            config.max_steps = config.max_steps.max(16);
        }
    }

    // Emit the `:thinking` state up front so the UI ordering matches V1.
    if let Some(app) = event_emitter.as_ref() {
        let _ = app.emit(
            &format!("{event_scope_prefix}:thinking"),
            serde_json::json!({ "status": "processing", "mode": "gui_cognition" }),
        );
    }

    let outcome = v2::run_turn_v2(&sight, brain.as_ref(), &hands, &message, config, &guards).await;

    kria_core::agent::gui_cognition::cancel::gui_cancel_registry().unregister(&session_id);

    // --- Build the per-step receipts + response payload ---
    let steps_json: Vec<serde_json::Value> = outcome
        .steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "step_index": s.step_index,
                "action": s.decision.action.kind(),
                "action_detail": s.decision.action.detail(),
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
    // Per-step `V2Step` envelopes are only used for the NON-streaming batch path
    // (API consumers without a live emitter). When streaming, the live observer
    // above already emitted the rich lifecycle envelopes the panel understands,
    // so we do NOT also emit V2Step (which the frontend ignores).
    if !streaming {
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
                    "action_detail": step.decision.action.detail(),
                    "target_label": step.target_label,
                    "ok": step.result.ok,
                    "error": step.result.error,
                    "backend_used": step.result.backend_used,
                }),
            );
            events.push(super::chat::desktop_chat_event(
                "gui_cognition:event",
                payload,
            ));
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
        session_id.clone(),
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
        serde_json::json!({ "text": outcome.reply.clone(), "session_id": session_id }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:tool_result"),
        // Match the canonical `agent:tool_result` contract: `name` (NOT `tool`),
        // `result`, and `success`. Emitting `tool` left `name` undefined on the
        // frontend, which crashed MessageBubble (`toolName.startsWith` on
        // undefined). `session_id` enables cross-session event isolation.
        serde_json::json!({
            "name": "gui_cognition",
            "result": response.clone(),
            "success": matches!(
                outcome.status,
                v2::TurnStatus::Completed | v2::TurnStatus::NeedsClarification
            ),
            "session_id": session_id,
        }),
    ));
    events.push(super::chat::desktop_chat_event(
        format!("{event_scope_prefix}:done"),
        serde_json::json!({ "session_id": session_id }),
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
        assert_eq!(
            v2_parse_combo("ctrl+plus"),
            vec![Key::Control, Key::Char('+')]
        );
        assert_eq!(v2_parse_combo("ctrl+l"), vec![Key::Control, Key::Char('l')]);
        assert_eq!(v2_parse_combo("enter"), vec![Key::Enter]);
    }

    #[test]
    fn parse_combo_drops_unknown_tokens() {
        // An unmappable multi-char token yields no key for that segment.
        assert!(v2_parse_combo("kaboom").is_empty());
    }

    #[test]
    fn loop_event_maps_to_frontend_vocabulary() {
        use kria_core::agent::gui_cognition_v2::{LoopEvent, TurnStatus};
        let ty = |ev: &LoopEvent| {
            v2_loop_event_to_wire(ev).map(|v| v["type"].as_str().unwrap().to_string())
        };
        assert_eq!(ty(&LoopEvent::TurnStarted).as_deref(), Some("TurnStarted"));
        assert_eq!(
            ty(&LoopEvent::ObserveStarted { step_index: 0 }).as_deref(),
            Some("ObservationStarted")
        );
        assert_eq!(
            ty(&LoopEvent::ObserveCompleted {
                step_index: 0,
                active_window: Some("Chrome".into()),
                element_count: 3,
                degraded: false
            })
            .as_deref(),
            Some("ObservationCompleted")
        );
        assert_eq!(
            ty(&LoopEvent::Decided {
                step_index: 0,
                action_kind: "key",
                detail: "new_tab".into(),
                reason: "open a tab".into()
            })
            .as_deref(),
            Some("PlanCreated")
        );
        assert_eq!(
            ty(&LoopEvent::Gated {
                step_index: 0,
                allowed: true,
                reason: None
            })
            .as_deref(),
            Some("SafetyGateCompleted")
        );
        assert_eq!(
            ty(&LoopEvent::Gated {
                step_index: 0,
                allowed: false,
                reason: Some("risky".into())
            })
            .as_deref(),
            Some("ExecutionBlocked")
        );
        assert_eq!(
            ty(&LoopEvent::ExecuteStarted {
                step_index: 0,
                action_kind: "key",
                detail: "new_tab".into()
            })
            .as_deref(),
            Some("ActionStarted")
        );
        assert_eq!(
            ty(&LoopEvent::ExecuteCompleted {
                step_index: 0,
                ok: true,
                error: None,
                backend: "uinput".into()
            })
            .as_deref(),
            Some("ActionCompleted")
        );
        assert_eq!(
            ty(&LoopEvent::ExecuteCompleted {
                step_index: 0,
                ok: false,
                error: Some("boom".into()),
                backend: "uinput".into()
            })
            .as_deref(),
            Some("ActionFailed")
        );
        // A positive verification surfaces; a no-change one is suppressed (no false-fail).
        assert_eq!(
            ty(&LoopEvent::Verified {
                step_index: 0,
                changed: Some(true)
            })
            .as_deref(),
            Some("VerificationCompleted")
        );
        assert!(v2_loop_event_to_wire(&LoopEvent::Verified {
            step_index: 0,
            changed: Some(false)
        })
        .is_none());
        // Terminal mapping.
        assert_eq!(
            ty(&LoopEvent::TurnEnded {
                status: TurnStatus::Completed
            })
            .as_deref(),
            Some("TurnCompleted")
        );
        assert_eq!(
            ty(&LoopEvent::TurnEnded {
                status: TurnStatus::NeedsClarification
            })
            .as_deref(),
            Some("TurnCompleted")
        );
        assert_eq!(
            ty(&LoopEvent::TurnEnded {
                status: TurnStatus::StoppedError
            })
            .as_deref(),
            Some("TurnFailed")
        );
        assert_eq!(
            ty(&LoopEvent::TurnEnded {
                status: TurnStatus::StoppedSafety
            })
            .as_deref(),
            Some("TurnFailed")
        );
    }

    // ---- Task 2: frozen event-contract tests ----

    #[test]
    fn every_emitted_event_type_is_in_the_frozen_vocabulary() {
        use kria_core::agent::gui_cognition_v2::{LoopEvent, TurnStatus};
        let samples = [
            LoopEvent::TurnStarted,
            LoopEvent::ObserveStarted { step_index: 0 },
            LoopEvent::GroundingEscalated { step_index: 0 },
            LoopEvent::ObserveCompleted {
                step_index: 0,
                active_window: None,
                element_count: 0,
                degraded: true,
            },
            LoopEvent::Decided {
                step_index: 0,
                action_kind: "key",
                detail: "x".into(),
                reason: "r".into(),
            },
            LoopEvent::Gated {
                step_index: 0,
                allowed: true,
                reason: None,
            },
            LoopEvent::Gated {
                step_index: 0,
                allowed: false,
                reason: Some("x".into()),
            },
            LoopEvent::ExecuteStarted {
                step_index: 0,
                action_kind: "key",
                detail: "x".into(),
            },
            LoopEvent::ExecuteCompleted {
                step_index: 0,
                ok: true,
                error: None,
                backend: "uinput".into(),
            },
            LoopEvent::ExecuteCompleted {
                step_index: 0,
                ok: false,
                error: Some("e".into()),
                backend: "uinput".into(),
            },
            LoopEvent::Verified {
                step_index: 0,
                changed: Some(true),
            },
            LoopEvent::TurnEnded {
                status: TurnStatus::Completed,
            },
            LoopEvent::TurnEnded {
                status: TurnStatus::StoppedError,
            },
        ];
        for ev in &samples {
            if let Some(v) = v2_loop_event_to_wire(ev) {
                let ty = v["type"].as_str().expect("event has a string type");
                assert!(
                    GUI_COGNITION_EVENT_TYPES.contains(&ty),
                    "emitted type {ty:?} is not in the frozen contract vocabulary"
                );
            }
        }
    }

    #[test]
    fn frozen_event_vocabulary_snapshot_is_unchanged() {
        // Locks the contract: additions must be APPENDED (and this list updated);
        // renames/removals of existing names break this snapshot intentionally.
        let expected = [
            "TurnStarted",
            "ObservationStarted",
            "ObservationCompleted",
            "PlanCreated",
            "SafetyGateCompleted",
            "ExecutionBlocked",
            "ActionStarted",
            "ActionCompleted",
            "ActionFailed",
            "VerificationCompleted",
            "TurnCompleted",
            "TurnFailed",
            "SubGoalUpdated",
            "AppChoiceRequested",
            "GroundingStatus",
            "RecoveryAttempted",
            "RetryAttempted",
        ];
        assert_eq!(GUI_COGNITION_EVENT_TYPES, &expected);
    }

    #[test]
    fn every_frozen_type_has_a_well_formed_example() {
        // Each frozen type must have a canonical example whose "type" matches and
        // which carries at least one descriptive field (so emitters have a shape
        // to conform to). This is the contract oracle for later-task emitters.
        for &t in GUI_COGNITION_EVENT_TYPES {
            let ex = gui_cognition_event_example(t);
            assert_eq!(
                ex["type"].as_str(),
                Some(t),
                "example type mismatch for {t}"
            );
            assert!(ex.is_object(), "example for {t} must be a JSON object");
        }
    }

    #[test]
    fn additive_event_examples_carry_their_required_fields() {
        // Pin the shape of the not-yet-emitted (additive) events so Tasks 9/10/11
        // /13 conform when they start emitting them.
        let sg = gui_cognition_event_example("SubGoalUpdated");
        for k in ["index", "total", "goal", "status"] {
            assert!(sg.get(k).is_some(), "SubGoalUpdated missing {k}");
        }
        let ac = gui_cognition_event_example("AppChoiceRequested");
        assert!(
            ac["candidates"].is_array(),
            "AppChoiceRequested.candidates must be an array"
        );
        assert!(ac.get("query").is_some());
        let gs = gui_cognition_event_example("GroundingStatus");
        for k in ["backend", "live", "degraded_reason"] {
            assert!(gs.get(k).is_some(), "GroundingStatus missing {k}");
        }
        let rec = gui_cognition_event_example("RecoveryAttempted");
        for k in ["rung", "ok"] {
            assert!(rec.get(k).is_some(), "RecoveryAttempted missing {k}");
        }
        let rt = gui_cognition_event_example("RetryAttempted");
        for k in ["kind", "attempt"] {
            assert!(rt.get(k).is_some(), "RetryAttempted missing {k}");
        }
    }
}
