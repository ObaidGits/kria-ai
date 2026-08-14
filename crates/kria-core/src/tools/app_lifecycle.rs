use crate::infra::ToolResult;
use crate::platform::app_registry::InstalledAppRegistry;
use crate::platform::intent::capability::{Capability, SafeArg};
use crate::platform::intent::dispatcher::{DispatchError, IntentDispatcher};
use crate::platform::intent::scheme::{build_search_url, build_youtube_search_url};
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
use crate::tools::ToolContext;
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

        // Resolve name alias → CanonicalAppId. On an exact-miss, fall back to a
        // fuzzy match (Requirement 6: mistyped/synonym/closest) and, when nothing
        // is confident, return an HONEST "not installed" with nearest suggestions
        // instead of a bare error. `name` is shadowed with the resolved alias so
        // the focus/launch path below uses the corrected name.
        let (app_id, resolved_name) = match self.registry.resolve_alias(name) {
            Some(id) => (id, name.to_string()),
            None => match self.registry.fuzzy_match(name) {
                crate::platform::app_registry::AppMatch::Closest { alias, score } => {
                    match self.registry.resolve_alias(&alias) {
                        Some(id) => {
                            tracing::info!(
                                target: "app_lifecycle",
                                requested = name, resolved = %alias, score,
                                "open_application: fuzzy-resolved an inexact app name"
                            );
                            (id, alias)
                        }
                        None => {
                            return ToolResult::err(format!(
                                "application '{}' is not installed",
                                name
                            ))
                        }
                    }
                }
                crate::platform::app_registry::AppMatch::Ambiguous(cands) => {
                    return ToolResult::err(format!(
                        "Several apps match '{}': {}. Which one did you mean?",
                        name,
                        cands.join(", ")
                    ))
                }
                crate::platform::app_registry::AppMatch::None(suggestions) => {
                    let hint = if suggestions.is_empty() {
                        String::new()
                    } else {
                        format!(" Did you mean: {}?", suggestions.join(", "))
                    };
                    return ToolResult::err(format!("'{}' is not installed.{}", name, hint));
                }
            },
        };
        let name = resolved_name.as_str();

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
            let focus_confirmed = if session != "wayland" {
                let ok = tokio::process::Command::new("wmctrl")
                    .args(["-a", name])
                    .output()
                    .await
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                // Post-focus delay: X11 WM focus commits are async (EWMH round-trip).
                if ok {
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                }
                ok
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
                }
                xdotool_ok
            };
            if focus_confirmed {
                return ToolResult::ok(serde_json::json!({
                    "application": name,
                    "already_running": true,
                    "action": "focused_existing_window",
                }));
            }
            // Focus could NOT be confirmed — the "running" process may be a
            // background/helper process (e.g. Chrome's crashpad/GPU helpers) with
            // NO visible window, or a native-Wayland window xdotool cannot reach.
            // Returning a false "focused" here is the bug that left the user with
            // no visible window. FALL THROUGH to launch: for a single-instance app
            // this raises/creates a window; for a stale background process it
            // brings up a fresh, visible instance.
            tracing::info!(
                target: "app_lifecycle",
                app = name,
                "running process had no focusable window — launching a visible instance instead"
            );
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

// ─── OpenWithApplication (canonical Task 3.3 name: `open_with_application`) ──
//
// Same behavior as `open_application_with_file`; the frozen manifest names
// this tool `open_with_application(app_id, path)`. Both names are registered
// (OSC-009.3: hard cutover requires updating every reference atomically —
// `open_application_with_file` has many existing call sites across the
// GUI-substrate planner/executor, so this task adds the canonical name
// alongside rather than renaming in place).

struct OpenWithApplication {
    dispatcher: Arc<IntentDispatcher>,
    registry: Arc<InstalledAppRegistry>,
}

#[async_trait]
impl ToolHandler for OpenWithApplication {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let app_id = params["app_id"].as_str().unwrap_or("").trim();
        if app_id.is_empty() {
            return ToolResult::err("app_id is required");
        }
        let path = params["path"].as_str().unwrap_or("").trim();
        if path.is_empty() {
            return ToolResult::err("path is required");
        }

        let session_id = params["session_id"]
            .as_str()
            .unwrap_or("no-session")
            .to_string();

        let resolved_app_id = match self.registry.resolve_alias(app_id) {
            Some(id) => id,
            None => {
                return ToolResult::err(format!(
                    "application '{app_id}' is not found in the installed app registry"
                ))
            }
        };

        let safe_path = match SafeArg::new(path) {
            Ok(a) => a,
            Err(e) => return ToolResult::err(format!("invalid path argument '{path}': {e}")),
        };

        let cap = Capability::LaunchApp {
            app_id: resolved_app_id,
            args: vec![safe_path],
        };

        let dispatch = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            self.dispatcher.dispatch(&cap, &session_id, false),
        )
        .await;

        match dispatch {
            Err(_) => ToolResult::err(format!(
                "launch timeout: application '{app_id}' did not accept path '{path}' within 8s"
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

// ─── ListInstalledApps ────────────────────────────────────────────────────────
//
// Wraps the existing `InstalledAppRegistry`'s already-scanned manifests
// (design §9.2: "do not duplicate .desktop parsing") rather than re-scanning
// `.desktop` files. A pure read, outside any mutation lifecycle.

struct ListInstalledApps {
    registry: Arc<InstalledAppRegistry>,
}

#[async_trait]
impl ToolHandler for ListInstalledApps {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params["query"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let limit = params["limit"].as_u64().unwrap_or(256).clamp(1, 256) as usize;

        let apps = self.registry.snapshot_for_listing();
        let mut items: Vec<serde_json::Value> = apps
            .into_iter()
            .filter(|app| query.is_empty() || app.display_name.to_ascii_lowercase().contains(&query) || app.app_id.to_ascii_lowercase().contains(&query))
            .map(|app| {
                serde_json::json!({
                    "app_id": app.app_id,
                    "display_name": app.display_name,
                    "desktop_entry_digest": app.desktop_entry_digest,
                    "available": app.available,
                })
            })
            .collect();
        let truncated = items.len() > limit;
        items.truncate(limit);

        ToolResult::ok(serde_json::json!({
            "items": items,
            "truncated": truncated,
        }))
    }
}

// ─── SetDefaultApplication / ManageAutostart ──────────────────────────────────
//
// linux-os-control-production **Task 3.3**: freedesktop MIME-default and
// XDG-autostart mutations. Reach host effects **only** through the injected
// `OsControlRuntime` + `os_control::applications::DesktopAssociationControl`
// provider. Until a live provider is composed, both fail closed with the
// frozen `Unavailable` envelope.

struct SetDefaultApplication;
#[async_trait]
impl ToolHandler for SetDefaultApplication {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "set_default_application")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let mime = params["mime"].as_str().unwrap_or("").trim();
        if mime.is_empty() {
            return ToolResult::err("mime is required");
        }
        let app_id = params["app_id"].as_str().unwrap_or("").trim();
        if app_id.is_empty() {
            return ToolResult::err("app_id is required");
        }
        // The governed DesktopAssociationControl provider owns the actual
        // mimeapps.list before-state capture + write + verification through
        // the runtime.
        let resolved = match gov::resolve(&ctx, "set_default_application") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.desktop_association("set_default_application") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "set_default_application") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::applications::AssociationRequest {
            action: "set_default_application".to_string(),
            params: params.clone(),
            op: crate::os_control::applications::AssociationOp::SetDefaultApplication {
                mime: params["mime"].as_str().unwrap_or_default().to_string(),
                app_id: params["app_id"]
                    .as_str()
                    .or_else(|| params["application"].as_str())
                    .unwrap_or_default()
                    .to_string(),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "set_default_application",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct ManageAutostart;
#[async_trait]
impl ToolHandler for ManageAutostart {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "manage_autostart")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let app_id = params["app_id"].as_str().unwrap_or("").trim();
        if app_id.is_empty() {
            return ToolResult::err("app_id is required");
        }
        if params["enabled"].as_bool().is_none() {
            return ToolResult::err("enabled (boolean) is required");
        }
        // The governed DesktopAssociationControl provider owns the actual
        // XDG autostart entry before-state capture + write + verification
        // through the runtime.
        let resolved = match gov::resolve(&ctx, "manage_autostart") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.desktop_association("manage_autostart") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "manage_autostart") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let request = crate::os_control::applications::AssociationRequest {
            action: "manage_autostart".to_string(),
            params: params.clone(),
            op: crate::os_control::applications::AssociationOp::SetAutostart {
                app_id: params["app_id"]
                    .as_str()
                    .or_else(|| params["application"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                enabled: params["enabled"].as_bool().unwrap_or(true),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "manage_autostart",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
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
            )
            .await
            .unwrap_or(false);

            if cdp_available {
                let engine = crate::agent::browser_cognition::BrowserCognitionEngine::new();
                let nav = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    engine.navigate(&url_str),
                )
                .await;
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

struct NullListInstalledApps;
#[async_trait]
impl ToolHandler for NullListInstalledApps {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        ToolResult::ok(serde_json::json!({ "items": [], "truncated": false }))
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

/// Return the governed OS-control `Unavailable` envelope for an application/
/// process tool.
///
/// linux-os-control-production **Task 2.5**: `close_application` (mapped to
/// the canonical `graceful_close_application` operation) and `kill_process`
/// no longer call `sysinfo::Process::kill()` directly (an unconditional
/// `SIGKILL` for *both* the "close" and "kill" tools — the exact
/// graceful-vs-forced conflation this task's "split graceful close from
/// kill" requirement targets). They reach host effects **only** through the
/// injected [`crate::os_control::OsControlRuntime`] +
/// `os_control::applications::ApplicationCloseControl` /
/// `os_control::processes::ProcessControl` providers. Until a live
/// native-syscall provider is composed into the runtime, the handlers fail
/// closed with this frozen envelope and never fall back to an ungoverned
/// `kill()` call.
fn os_process_unavailable(
    runtime: Option<&std::sync::Arc<crate::os_control::OsControlRuntime>>,
    tool: &str,
) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => crate::os_control::OsControlError::Unavailable {
            provider: None,
            reason: crate::os_control::contract::SafeText::new(
                "OS control runtime is not injected in this build",
            ),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct CloseApplication;
#[async_trait]
impl ToolHandler for CloseApplication {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "close_application")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let name = params["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            return ToolResult::err("application name is required");
        }
        // The governed ApplicationCloseControl provider owns the actual
        // SIGTERM-only signal loop (never SIGKILL — that escalation is the
        // separate `kill_process` operation) + liveness verification through
        // the runtime.
        let resolved = match gov::resolve(&ctx, "close_application") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.application_close("close_application") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "close_application") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let name = params["name"]
            .as_str()
            .or_else(|| params["app"].as_str())
            .unwrap_or_default()
            .to_string();
        let request = crate::os_control::applications::ApplicationCloseRequest {
            action: "close_application".to_string(),
            params: params.clone(),
            name,
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "close_application",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

struct KillProcess;
#[async_trait]
impl ToolHandler for KillProcess {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_process_unavailable(None, "kill_process")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let pid = params["pid"].as_u64().unwrap_or(0);
        if pid == 0 {
            return ToolResult::err("pid is required");
        }
        // The governed ProcessControl provider owns the actual PID-reuse-safe
        // `kill(2)` (SIGKILL) + verification through the runtime.
        let resolved = match gov::resolve(&ctx, "kill_process") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.processes("kill_process") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "kill_process") {
            Ok(call) => call,
            Err(result) => return result,
        };
        let pid = params["pid"].as_u64().unwrap_or(0) as u32;
        // start_time binds the identity so a recycled PID cannot be mistaken for
        // the original process.
        let start_time = params["start_time"].as_u64().unwrap_or(0);
        let identity = crate::os_control::processes::ProcessIdentity::new(pid, start_time);
        // `kill_process` is the escalated path: SIGKILL is unconditional.
        let request = crate::os_control::processes::ProcessRequest {
            action: "kill_process".to_string(),
            params: params.clone(),
            op: crate::os_control::processes::ProcessOp::Terminate {
                identity,
                force: params["force"].as_bool().unwrap_or(true),
            },
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "kill_process",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
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

    let open_with_application_handler: Arc<dyn ToolHandler> =
        if let (Some(d), Some(r)) = (dispatcher.clone(), registry.clone()) {
            Arc::new(OpenWithApplication {
                dispatcher: d,
                registry: r,
            })
        } else {
            Arc::new(LegacyOpenApplicationWithFile)
        };

    let list_installed_apps_handler: Arc<dyn ToolHandler> = if let Some(r) = registry.clone() {
        Arc::new(ListInstalledApps { registry: r })
    } else {
        Arc::new(NullListInstalledApps)
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
                name: "open_with_application".into(),
                description: "Open/launch an installed application with a file path argument (canonical Task 3.3 name; same behavior as open_application_with_file)."
                    .into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("app_id", "string", "Canonical application id (from list_installed_apps)", true),
                    param("path", "string", "Absolute path to the file to open in the application", true),
                    param("session_id", "string", "Session identifier for audit logging", false),
                ],
            },
            open_with_application_handler,
        ),
        (
            ToolDef {
                name: "list_installed_apps".into(),
                description: "List installed desktop applications discovered from .desktop entries, with stable canonical app ids, display names, and availability.".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query", "string", "Filter by name/id substring", false),
                    param("cursor", "string", "Pagination cursor from a previous call", false),
                    param("limit", "integer", "Maximum applications to return per page", false),
                ],
            },
            list_installed_apps_handler,
        ),
        (
            ToolDef {
                name: "set_default_application".into(),
                description: "Set the default application for a MIME type (e.g. text/plain, text/html) using freedesktop MIME associations.".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("mime", "string", "MIME type to associate (e.g. text/plain)", true),
                    param("app_id", "string", "Canonical application id to make the default handler", true),
                ],
            },
            Arc::new(SetDefaultApplication),
        ),
        (
            ToolDef {
                name: "manage_autostart".into(),
                description: "Enable or disable an application's autostart entry for the current user (XDG autostart).".into(),
                category: "app_lifecycle".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![
                    param("app_id", "string", "Canonical application id", true),
                    param("enabled", "boolean", "Whether the application should autostart", true),
                ],
            },
            Arc::new(ManageAutostart),
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
                parameters: vec![
                    param("pid", "integer", "Process ID", true),
                    param("start_time", "integer", "Process start time in ms since epoch, for PID-reuse-safe targeting (optional; from get_process_info)", false),
                ],
            },
            Arc::new(KillProcess),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
