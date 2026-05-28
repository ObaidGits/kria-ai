use crate::infra::ToolResult;
use crate::platform::app_registry::InstalledAppRegistry;
use crate::platform::intent::capability::{Capability, SafeArg};
use crate::platform::intent::dispatcher::{DispatchError, IntentDispatcher};
use crate::platform::intent::scheme::{build_search_url, build_youtube_search_url};
use crate::safety::RiskLevel;
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

/// Check if a process with the given binary name hint is currently running.
/// Uses /proc scanning — works on Linux without any external tools.
/// This is a best-effort check; false negatives are possible for Flatpak/Snap apps.
///
/// Uses exact matching or prefix matching only in the safe direction:
/// - `comm == hint_lower`: exact match
/// - `comm.starts_with(&hint_lower)`: comm is a longer version of the hint (e.g., "gedit-3" matches "gedit")
/// Does NOT use `hint_lower.starts_with(&comm)` to avoid false positives
/// where a short truncated comm (e.g., "co") matches unrelated processes.
fn is_process_running_by_name(binary_hint: &str) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    let hint_lower = binary_hint.to_ascii_lowercase();
    // Minimum meaningful length to avoid false positives from short comm names
    if hint_lower.len() < 3 {
        return false;
    }
    for entry in entries.flatten() {
        let pid_dir = entry.path();
        if !pid_dir.is_dir() {
            continue;
        }
        let Some(name) = pid_dir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.parse::<u32>().is_err() {
            continue;
        }
        // Check comm (fast, truncated to 15 chars by kernel)
        let comm_path = pid_dir.join("comm");
        if let Ok(comm) = std::fs::read_to_string(&comm_path) {
            let comm = comm.trim().to_ascii_lowercase();
            // Exact match or comm is a longer version of hint (e.g., "gedit-3" matches "gedit")
            if comm == hint_lower || comm.starts_with(&hint_lower) {
                return true;
            }
        }
        // Check cmdline basename (handles long names and Flatpak)
        let cmdline_path = pid_dir.join("cmdline");
        if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
            let first_arg = cmdline.split('\0').next().unwrap_or("");
            let basename = first_arg
                .rsplit('/')
                .next()
                .unwrap_or(first_arg)
                .to_ascii_lowercase();
            if basename == hint_lower || basename.starts_with(&hint_lower) {
                return true;
            }
        }
    }
    false
}

// ─── OpenApplication ─────────────────────────────────────────────────────────
//
// Delegates to `IntentDispatcher` instead of raw `tokio::process::Command`.
// The tool name is preserved ("open_application") for LLM prompt compatibility;
// only the implementation changes.

struct OpenApplication {
    dispatcher: Arc<IntentDispatcher>,
    registry: Arc<InstalledAppRegistry>,
}

#[async_trait]
impl ToolHandler for OpenApplication {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let name = params["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            return ToolResult::err("application name is required");
        }

        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        // Resolve name alias → CanonicalAppId.
        let app_id = match self.registry.resolve_alias(name) {
            Some(id) => id,
            None => {
                return ToolResult::err(format!(
                    "application '{}' is not found in the installed app registry",
                    name
                ))
            }
        };

        // Check if the app is already running before launching a new instance.
        // This prevents duplicate windows and respects single-instance apps.
        // We use a best-effort /proc scan — if it fails, we proceed with launch.
        // Use the canonical binary name (not the user-facing alias) for accurate matching.
        let binary_hint = crate::agent::gui_substrate_planner::app_alias_to_binary_pub(name);
        let already_running = is_process_running_by_name(&binary_hint);
        if already_running {
            tracing::info!(
                target: "app_lifecycle",
                app = name,
                binary = %binary_hint,
                "App already running — attempting to focus existing window instead of launching new instance"
            );
            // Best-effort focus:
            // X11: wmctrl -a <name> (brings window to foreground)
            // Wayland: no reliable cross-compositor focus API; log and continue
            let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
            if session != "wayland" {
                let _ = tokio::process::Command::new("wmctrl")
                    .args(["-a", name])
                    .output()
                    .await;
                // Post-focus delay: X11 WM focus commits are async (EWMH round-trip).
                // Without this sleep the WindowFocused verifier polls immediately and
                // still sees the old focused window (KRIA chat), causing a false failure.
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            } else {
                // Wayland: try XWayland path (works for Electron/VSCode and other
                // XWayland-backed apps). xdotool --sync waits for WM to process the event.
                let xdotool_ok = tokio::process::Command::new("xdotool")
                    .args(["search", "--name", name, "windowactivate", "--sync"])
                    .output()
                    .await
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if xdotool_ok {
                    tracing::info!(
                        target: "app_lifecycle",
                        app = name,
                        "Wayland+XWayland: focused existing window via xdotool"
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                } else {
                    tracing::debug!(
                        target: "app_lifecycle",
                        app = name,
                        "Wayland session: xdotool focus unsuccessful (pure Wayland or app not on XWayland display)"
                    );
                }
            }
            return ToolResult::ok(serde_json::json!({
                "application": name,
                "already_running": true,
                "action": "focused_existing_window",
            }));
        }

        // Build SafeArg list from the params "args" array.
        let mut safe_args: Vec<SafeArg> = Vec::new();
        if let Some(arr) = params["args"].as_array() {
            for v in arr {
                if let Some(s) = v.as_str() {
                    match SafeArg::new(s) {
                        Ok(a) => safe_args.push(a),
                        Err(e) => return ToolResult::err(format!("invalid argument '{s}': {e}")),
                    }
                }
            }
        }

        let cap = Capability::LaunchApp {
            app_id,
            args: safe_args,
        };

        match self.dispatcher.dispatch(&cap, &session_id, false).await {
            Ok(result) => {
                if result.success {
                    ToolResult::ok(result.detail)
                } else {
                    ToolResult::err(result.message)
                }
            }
            Err(DispatchError::PolicyBlocked(reason)) => {
                ToolResult::err(format!("blocked by policy: {reason}"))
            }
            Err(DispatchError::RateLimitExceeded(action, retry)) => ToolResult::err(format!(
                "rate limit exceeded for '{action}', retry after {retry}s"
            )),
            Err(e) => ToolResult::err(format!("dispatch error: {e}")),
        }
    }
}

// ─── OpenApplicationWithFile ─────────────────────────────────────────────────
//
// Same as `open_application` but appends a single file path as a launch
// argument. Used by the substrate-aware planner to open editors with a
// generated file in one step (works on X11 AND Wayland because the OS launcher
// handles both).

struct OpenApplicationWithFile {
    dispatcher: Arc<IntentDispatcher>,
    registry: Arc<InstalledAppRegistry>,
}

#[async_trait]
impl ToolHandler for OpenApplicationWithFile {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let name = params["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            return ToolResult::err("application name is required");
        }
        let file = params["file"].as_str().unwrap_or("").trim();
        if file.is_empty() {
            return ToolResult::err("file path is required");
        }

        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        let app_id = match self.registry.resolve_alias(name) {
            Some(id) => id,
            None => {
                return ToolResult::err(format!(
                    "application '{}' is not found in the installed app registry",
                    name
                ))
            }
        };

        let safe_file = match SafeArg::new(file) {
            Ok(a) => a,
            Err(e) => return ToolResult::err(format!("invalid file argument '{file}': {e}")),
        };

        let cap = Capability::LaunchApp {
            app_id,
            args: vec![safe_file],
        };

        let dispatch = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            self.dispatcher.dispatch(&cap, &session_id, false),
        )
        .await;

        match dispatch {
            Err(_) => ToolResult::err(format!(
                "launch timeout: application '{name}' did not accept file '{file}' within 8s"
            )),
            Ok(Ok(result)) => {
                if result.success {
                    ToolResult::ok(result.detail)
                } else {
                    ToolResult::err(result.message)
                }
            }
            Ok(Err(DispatchError::PolicyBlocked(reason))) => {
                ToolResult::err(format!("blocked by policy: {reason}"))
            }
            Ok(Err(DispatchError::RateLimitExceeded(action, retry))) => ToolResult::err(format!(
                "rate limit exceeded for '{action}', retry after {retry}s"
            )),
            Ok(Err(e)) => ToolResult::err(format!("dispatch error: {e}")),
        }
    }
}

struct LegacyOpenApplicationWithFile;
#[async_trait]
impl ToolHandler for LegacyOpenApplicationWithFile {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let app = params["name"].as_str().unwrap_or("");
        let file = params["file"].as_str().unwrap_or("");
        if app.is_empty() || file.is_empty() {
            return ToolResult::err("name and file are required");
        }
        match tokio::process::Command::new(app).arg(file).spawn() {
            Ok(result) => ToolResult::ok(serde_json::json!({
            "application": app,
            "file": file,
            "pid": result.id(),
            "launched": true
            })),
            Err(e) => ToolResult::err(format!("failed to open {app} with {file}: {e}")),
        }
    }
}

// ─── OpenUrl ─────────────────────────────────────────────────────────────────

struct OpenUrl {
    dispatcher: Arc<IntentDispatcher>,
}

#[async_trait]
impl ToolHandler for OpenUrl {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let raw_url = params["url"].as_str().unwrap_or("").trim();
        if raw_url.is_empty() {
            return ToolResult::err("url is required");
        }

        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        let url = match url::Url::parse(raw_url) {
            Ok(u) => u,
            Err(e) => return ToolResult::err(format!("invalid URL '{raw_url}': {e}")),
        };

        let cap = Capability::OpenUrl { url };

        match self.dispatcher.dispatch(&cap, &session_id, false).await {
            Ok(result) => {
                if result.success {
                    ToolResult::ok(result.detail)
                } else {
                    ToolResult::err(result.message)
                }
            }
            Err(DispatchError::PolicyBlocked(reason)) => {
                ToolResult::err(format!("blocked: {reason}"))
            }
            Err(DispatchError::RateLimitExceeded(action, retry)) => {
                ToolResult::err(format!("rate limited for '{action}', retry after {retry}s"))
            }
            Err(e) => ToolResult::err(format!("{e}")),
        }
    }
}

// ─── WebSearch (via default browser) ─────────────────────────────────────────
//
// "Open Chrome and search for X" → build a safe Google search URL and dispatch.

struct WebBrowserSearch {
    dispatcher: Arc<IntentDispatcher>,
}

#[async_trait]
impl ToolHandler for WebBrowserSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params["query"].as_str().unwrap_or("").trim();

        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        // Check if a specific site is requested.
        let site = params["site"].as_str().unwrap_or("google");

        // When query is empty, navigate directly to the site homepage
        // (e.g. "open chrome and search for youtube" → open youtube.com).
        let url = if query.is_empty() {
            let homepage = match site.to_lowercase().as_str() {
                "youtube" | "yt" => "https://www.youtube.com",
                _ => "https://www.google.com",
            };
            match url::Url::parse(homepage) {
                Ok(u) => u,
                Err(e) => return ToolResult::err(format!("failed to build site URL: {e}")),
            }
        } else {
            match site.to_lowercase().as_str() {
                "youtube" | "yt" => match build_youtube_search_url(query) {
                    Ok(u) => u,
                    Err(e) => return ToolResult::err(format!("failed to build YouTube URL: {e}")),
                },
                _ => match build_search_url(query) {
                    Ok(u) => u,
                    Err(e) => return ToolResult::err(format!("failed to build search URL: {e}")),
                },
            }
        };

        let url_str = url.to_string();

        // Use xdg-open-first approach (same as managed_browser_navigate).
        // CDP is only used if already available (< 300ms check).
        if cfg!(target_os = "linux") {
            let cdp_available = tokio::time::timeout(
                std::time::Duration::from_millis(300),
                crate::agent::browser_cognition::BrowserCognitionEngine::is_available(),
            ).await.unwrap_or(false);

            if cdp_available {
                let engine = crate::agent::browser_cognition::BrowserCognitionEngine::new();
                let nav = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    engine.navigate(&url_str),
                ).await;
                if let Ok(result) = nav {
                    if result.success {
                        return ToolResult::ok(serde_json::json!({
                            "url": url_str,
                            "query": query,
                            "managed": true,
                            "method": "cdp",
                        }));
                    }
                }
                tracing::warn!(target: "app_lifecycle", "browser_search: CDP navigation failed or timed out, using IntentDispatcher");
            }
        }

        // Fallback: dispatch via IntentDispatcher (xdg-open / system default browser).
        let cap = Capability::OpenUrl { url };
        match self.dispatcher.dispatch(&cap, &session_id, false).await {
            Ok(result) => {
                if result.success {
                    ToolResult::ok(result.detail)
                } else {
                    ToolResult::err(result.message)
                }
            }
            Err(DispatchError::PolicyBlocked(reason)) => {
                ToolResult::err(format!("blocked: {reason}"))
            }
            Err(e) => ToolResult::err(format!("{e}")),
        }
    }
}

// ─── SendMessage ─────────────────────────────────────────────────────────────
//
// Opens a messaging draft. Contact resolution (ambiguity handling) is expected
// to have already happened upstream; if `contact_id` + `identifier` are provided
// directly we use them, otherwise we return an error asking for disambiguation.

struct SendMessage {
    dispatcher: Arc<IntentDispatcher>,
}

#[async_trait]
impl ToolHandler for SendMessage {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        use crate::platform::intent::capability::MessageBody;
        use crate::platform::intent::resolution::{ContactId, MessagingApp};

        let app_str = params["app"].as_str().unwrap_or("whatsapp");
        let contact_name = params["contact_name"].as_str().unwrap_or("").trim();
        let contact_identifier = params["contact_identifier"].as_str().unwrap_or("").trim();
        let body_str = params["body"].as_str().unwrap_or("").trim();
        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        if contact_name.is_empty() || contact_identifier.is_empty() {
            return ToolResult::err(
                "contact_name and contact_identifier are required; \
                 resolve contact ambiguity first by asking the user to clarify",
            );
        }
        if body_str.is_empty() {
            return ToolResult::err("message body is required");
        }

        let app = match app_str.to_lowercase().as_str() {
            "whatsapp" | "wa" => MessagingApp::WhatsApp,
            "gmail" | "email" => MessagingApp::Gmail,
            "telegram" | "tg" => MessagingApp::Telegram,
            "signal" => MessagingApp::Signal,
            other => {
                return ToolResult::err(format!(
                    "unsupported messaging app '{other}'; use: whatsapp, gmail, telegram, signal"
                ))
            }
        };

        let body = match MessageBody::new(body_str) {
            Ok(b) => b,
            Err(e) => return ToolResult::err(format!("invalid message body: {e}")),
        };

        let contact = ContactId {
            display_name: contact_name.to_string(),
            identifier: contact_identifier.to_string(),
            app: app.clone(),
        };

        let cap = Capability::SendMessage { app, contact, body };

        match self.dispatcher.dispatch(&cap, &session_id, false).await {
            Ok(result) => {
                if result.success {
                    ToolResult::ok(result.detail)
                } else {
                    ToolResult::err(result.message)
                }
            }
            Err(DispatchError::PolicyBlocked(reason)) => {
                ToolResult::err(format!("blocked: {reason}"))
            }
            Err(DispatchError::RateLimitExceeded(action, retry)) => {
                ToolResult::err(format!("rate limited for '{action}', retry after {retry}s"))
            }
            Err(e) => ToolResult::err(format!("{e}")),
        }
    }
}

// ─── Legacy stubs (no dispatcher) ────────────────────────────────────────────
//
// Used when `register_with_dispatcher` is called with `None`.
// Preserved for tests and early startup before the registry is ready.

struct LegacyOpenApplication;
#[async_trait]
impl ToolHandler for LegacyOpenApplication {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let app = params["name"].as_str().unwrap_or("");
        let args: Vec<String> = params["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        match tokio::process::Command::new(app).args(&args).spawn() {
            Ok(child) => ToolResult::ok(
                serde_json::json!({ "application": app, "pid": child.id(), "launched": true }),
            ),
            Err(e) => ToolResult::err(format!("failed to open {app}: {e}")),
        }
    }
}

struct LegacyOpenUrl;
#[async_trait]
impl ToolHandler for LegacyOpenUrl {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let raw = params["url"].as_str().unwrap_or("").trim();
        match open::that_detached(raw) {
            Ok(()) => ToolResult::ok(serde_json::json!({ "url": raw, "opened": true })),
            Err(e) => ToolResult::err(format!("failed to open '{raw}': {e}")),
        }
    }
}

struct LegacyWebBrowserSearch;
#[async_trait]
impl ToolHandler for LegacyWebBrowserSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        use crate::platform::intent::scheme::{build_search_url, build_youtube_search_url};
        let query = params["query"].as_str().unwrap_or("").trim();
        let site = params["site"].as_str().unwrap_or("google");
        let url = match site.to_lowercase().as_str() {
            "youtube" | "yt" => build_youtube_search_url(query).map_err(|e| e.to_string()),
            _ => build_search_url(query).map_err(|e| e.to_string()),
        };
        match url {
            Ok(u) => match open::that_detached(u.as_str()) {
                Ok(()) => ToolResult::ok(serde_json::json!({ "url": u.as_str(), "opened": true })),
                Err(e) => ToolResult::err(format!("{e}")),
            },
            Err(e) => ToolResult::err(e),
        }
    }
}

struct NullSendMessage;
#[async_trait]
impl ToolHandler for NullSendMessage {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        ToolResult::err("send_message is not available: IntentDispatcher not initialized yet")
    }
}

struct ListRunningApps;
#[async_trait]
impl ToolHandler for ListRunningApps {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let procs: Vec<serde_json::Value> = sys
            .processes()
            .iter()
            .filter(|(_, p)| !p.name().to_string_lossy().is_empty())
            .map(|(pid, p)| {
                serde_json::json!({
                    "pid": pid.as_u32(),
                    "name": p.name().to_string_lossy(),
                    "cpu_percent": format!("{:.1}", p.cpu_usage()),
                    "memory_mb": p.memory() / (1024 * 1024),
                })
            })
            .collect();
        ToolResult::ok(serde_json::json!({ "processes": procs, "count": procs.len() }))
    }
}

struct FocusWindow;
#[async_trait]
impl ToolHandler for FocusWindow {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let title = params["title"].as_str().unwrap_or("");
        if cfg!(target_os = "linux") {
            let output = tokio::process::Command::new("wmctrl")
                .args(["-a", title])
                .output()
                .await;
            match output {
                Ok(o) if o.status.success() => {
                    ToolResult::ok(serde_json::json!({ "focused": title }))
                }
                _ => ToolResult::err(format!(
                    "could not focus window '{title}' (wmctrl required)"
                )),
            }
        } else {
            ToolResult::err("focus_window not implemented for this OS")
        }
    }
}

struct CloseApplication;
#[async_trait]
impl ToolHandler for CloseApplication {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let name = params["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            return ToolResult::err("application name is required");
        }
        let name_lower = name.to_lowercase();
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let mut killed = 0;
        for proc_ in sys.processes().values() {
            let proc_name = proc_.name().to_string_lossy().to_lowercase();
            // FIX #33: Use exact match or starts_with, NOT contains.
            // "close_application("code")" must not kill "decode", "vscode-server", etc.
            // We match: exact name, or name is a prefix of the process name
            // (e.g., "gedit" matches "gedit-3"), but NOT substring matches.
            if proc_name == name_lower || proc_name.starts_with(&format!("{}-", name_lower)) {
                proc_.kill();
                killed += 1;
            }
        }
        if killed > 0 {
            ToolResult::ok(serde_json::json!({ "name": name, "processes_closed": killed }))
        } else {
            ToolResult::err(format!("no running process matched '{name}'"))
        }
    }
}

struct KillProcess;
#[async_trait]
impl ToolHandler for KillProcess {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let pid = params["pid"].as_u64().unwrap_or(0) as u32;
        let sys_pid = sysinfo::Pid::from_u32(pid);
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        if let Some(proc_) = sys.process(sys_pid) {
            proc_.kill();
            ToolResult::ok(serde_json::json!({ "pid": pid, "killed": true }))
        } else {
            ToolResult::err(format!("process {pid} not found"))
        }
    }
}

pub fn register(reg: &ToolRegistry) {
    register_with_dispatcher(reg, None, None);
}

/// Full registration with an `IntentDispatcher` and `InstalledAppRegistry`.
/// Called from the Tauri command setup after both are initialized.
pub fn register_with_dispatcher(
    reg: &ToolRegistry,
    dispatcher: Option<Arc<IntentDispatcher>>,
    registry: Option<Arc<InstalledAppRegistry>>,
) {
    // Fallback: if no dispatcher is provided, use the stateless legacy handlers.
    let _has_dispatcher = dispatcher.is_some();

    let open_app_handler: Arc<dyn ToolHandler> =
        if let (Some(d), Some(r)) = (dispatcher.clone(), registry.clone()) {
            Arc::new(OpenApplication {
                dispatcher: d,
                registry: r,
            })
        } else {
            // Legacy fallback (no dispatcher yet) — uses raw process::Command.
            Arc::new(LegacyOpenApplication)
        };

    let open_app_with_file_handler: Arc<dyn ToolHandler> =
        if let (Some(d), Some(r)) = (dispatcher.clone(), registry.clone()) {
            Arc::new(OpenApplicationWithFile {
                dispatcher: d,
                registry: r,
            })
        } else {
            Arc::new(LegacyOpenApplicationWithFile)
        };

    let open_url_handler: Arc<dyn ToolHandler> = if let Some(d) = dispatcher.clone() {
        Arc::new(OpenUrl {
            dispatcher: Arc::clone(&d),
        })
    } else {
        Arc::new(LegacyOpenUrl)
    };

    let search_handler: Arc<dyn ToolHandler> = if let Some(d) = dispatcher.clone() {
        Arc::new(WebBrowserSearch {
            dispatcher: Arc::clone(&d),
        })
    } else {
        Arc::new(LegacyWebBrowserSearch)
    };

    let send_message_handler: Arc<dyn ToolHandler> = if let Some(d) = dispatcher.clone() {
        Arc::new(SendMessage {
            dispatcher: Arc::clone(&d),
        })
    } else {
        Arc::new(NullSendMessage)
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "open_application".into(),
                description: "Open/launch an installed application by name. \
                              Use 'browser_search' to open a browser and search simultaneously."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "name",
                        "string",
                        "Application name (e.g., 'chrome', 'firefox', 'vscode', 'whatsapp')",
                        true,
                    ),
                    param("args", "array", "Optional launch arguments (no shell metacharacters)", false),
                    param("session_id", "string", "Session identifier for audit logging", false),
                ],
            },
            open_app_handler,
        ),
        (
            ToolDef {
                name: "open_application_with_file".into(),
                description: "Open/launch an installed application and pass a single file path as \
                              its launch argument. Use this when you have already generated content \
                              and want the editor to open with that file (works on X11 and Wayland)."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param(
                        "name",
                        "string",
                        "Application name (e.g., 'gedit', 'code', 'kate')",
                        true,
                    ),
                    param(
                        "file",
                        "string",
                        "Absolute path to the file to open in the application",
                        true,
                    ),
                    param(
                        "session_id",
                        "string",
                        "Session identifier for audit logging",
                        false,
                    ),
                ],
            },
            open_app_with_file_handler,
        ),
        (
            ToolDef {
                name: "open_url".into(),
                description: "Open a URL in the system's default handler. \
                              Only https, http, mailto, tel, and registered deep-links are allowed. \
                              file://, javascript:, data:, smb:// and similar are blocked."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("url", "string", "URL to open (must use https, http, mailto, tel, or a registered deep-link scheme)", true),
                    param("session_id", "string", "Session identifier for audit logging", false),
                ],
            },
            open_url_handler,
        ),
        (
            ToolDef {
                name: "browser_search".into(),
                description: "Open the default browser and search for a topic. \
                              Use site='youtube' to search YouTube. \
                              Example: 'Open Chrome and search for lo-fi music'."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Search query", true),
                    param("site", "string", "Search site: 'google' (default) or 'youtube'", false),
                    param("session_id", "string", "Session identifier for audit logging", false),
                ],
            },
            search_handler,
        ),
        (
            ToolDef {
                name: "send_message".into(),
                description: "Open a messaging app with a pre-filled draft. \
                              The user must press send. Does NOT auto-send. \
                              Requires contact_name AND contact_identifier (resolved phone/email). \
                              If contact is ambiguous, ask the user to clarify first."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("app", "string", "Messaging app: whatsapp, gmail, telegram, signal", true),
                    param("contact_name", "string", "Contact display name", true),
                    param("contact_identifier", "string", "Phone (E.164) or email, resolved from contacts", true),
                    param("body", "string", "Message body (max 4096 characters)", true),
                    param("session_id", "string", "Session identifier for audit logging", false),
                ],
            },
            send_message_handler,
        ),
        (
            ToolDef {
                name: "list_running_apps".into(),
                description: "List all running processes with CPU and memory usage".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListRunningApps),
        ),
        (
            ToolDef {
                name: "focus_window".into(),
                description: "Bring a window to the foreground by title".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![param(
                    "title",
                    "string",
                    "Window title (partial match)",
                    true,
                )],
            },
            Arc::new(FocusWindow),
        ),
        (
            ToolDef {
                name: "close_application".into(),
                description: "Close an application by name".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("name", "string", "Application name", true)],
            },
            Arc::new(CloseApplication),
        ),
        (
            ToolDef {
                name: "kill_process".into(),
                description: "Kill a process by PID".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("pid", "integer", "Process ID", true)],
            },
            Arc::new(KillProcess),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
